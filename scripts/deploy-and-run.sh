#!/usr/bin/env bash
# LP-0002 full lifecycle on the public LEZ testnet: deploy both programs, create
# a multisig instance, publish a proposal, gather the threshold of approvals on
# the privacy-preserving path, and execute.
#
# Each approval is a real Risc0 proof composed on chain via env::verify, with
# RISC0_DEV_MODE=0. This is the script behind the "1 multisig instance, one
# proposal submitted, approved by threshold, and executed" criterion.
#
# Idempotent on the deploys (content-addressed: re-deploying a byte-identical
# binary reproduces the same transaction hash) and on create_multisig /
# create_proposal (init fails harmlessly if the account already exists).
#
# Env:
#   SIGNER       Public account id that pays and authors (must be funded)
#   APPROVERS    Comma-separated Private account ids, ONE PER APPROVAL.
#                Not one account reused: a privacy transaction consumes the
#                approver's commitment, so a second approval submitted from the
#                same account panics in the client-side circuit with
#                "Invalid account_identities length" before it is ever sent.
#                Create them with `wallet account new private`.
#   MEMBERS      member set size          (default 5)
#   THRESHOLD    approvals required       (default 3)
#   SPEL_BIN     spel built from vendor/spel (default: spel on PATH). NOT the
#                released spel: it targets LEZ v0.2.0 and fails every
#                instruction here with `missing field 'sequencer_addr'`.
#                See vendor/spel/PATCH.md.
#   WALLET_BIN   wallet from LEZ v0.2.4   (default: wallet on PATH)
#   SEQUENCER_URL                          (default: https://testnet.lez.logos.co)
#
# The wallet may print "Transaction NOT confirmed" for a privacy transaction
# whose proving outruns its polling window; the transaction lands anyway. This
# script checks getTransaction, not the CLI's verdict.
#
# Budget: an approval is a real proof. Measured at 149-154 s per approval on an
# M-series laptop against a local sequencer, and 440-469 s per approval against
# the public testnet, which adds block time, network latency and contention. The
# script prints its own per-approval wall clock — and that number is the check
# that the run was real: a lifecycle whose approvals take seconds proved
# nothing.

set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export RISC0_DEV_MODE=0
export LEE_WALLET_HOME_DIR="${LEE_WALLET_HOME_DIR:-$HOME/.lee/wallet}"

SPEL_BIN="${SPEL_BIN:-spel}"
WALLET_BIN="${WALLET_BIN:-wallet}"
RPC="${SEQUENCER_URL:-https://testnet.lez.logos.co}"
MEMBERS="${MEMBERS:-5}"
THRESHOLD="${THRESHOLD:-3}"
: "${SIGNER:?set SIGNER to a funded Public account id}"
: "${APPROVERS:?set APPROVERS to a comma-separated list of Private account ids, one per approval}"
IFS=',' read -r -a APPROVER_LIST <<< "$APPROVERS"
if [ "${#APPROVER_LIST[@]}" -lt "$THRESHOLD" ]; then
  echo "need at least $THRESHOLD approver accounts, got ${#APPROVER_LIST[@]}" >&2
  exit 1
fi

IDL=idl/multisig_verifier.idl.json
VERIFIER=artifacts/programs/multisig_verifier.bin
MEMBERSHIP=artifacts/programs/membership_lez.bin
WORK="${WORK:-$ROOT/.testnet}"
LOG="$WORK/lifecycle.tsv"

CLI=target/release/msig
cargo build --release --quiet -p multisig-cli

mkdir -p "$WORK"
: > "$LOG"

# Read a `*.args` file into the ARGS array, one shell word per element.
#
# The CLI quotes its values, which it has to: an action is
# `--action 'transfer 100 LEZ to the grants treasury'`, four words in one
# argument. Interpolating the file unquoted splits that into four arguments;
# interpolating it quoted makes the whole file one argument. Both are wrong,
# and the first fails confusingly — the quotes arrive as literal characters and
# spel reports a 32-byte field as "66 bytes", counting the two apostrophes.
#
# xargs applies the quoting rules the file was written in, which is exactly the
# parse we want. Kept as a function so all four call sites share it, since the
# bug was invisible until every one of them had been written the wrong way.
read_args() {
  ARGS=()
  while IFS= read -r a; do
    [ -n "$a" ] && ARGS+=("$a")
  done < <(xargs -n1 printf '%s\n' < "$1")
}

# Pull the submitted transaction hash out of a spel transcript.
#
# Not the first 64-hex string in it: spel prints the resolved accounts first,
# and an account id is also 64 hex. Grepping loosely picks up the authority's
# id, then wait_tx polls for a hash that is not a transaction and burns its
# full ten-minute budget before reporting TIMEOUT on a step that actually
# succeeded. Anchor on the label spel prints instead.
tx_hash_from() {
  grep -oE 'tx_hash: [0-9a-f]{64}' "$1" | awk '{print $2}' | head -1
}

# Submit-and-confirm, where "spel printed no tx_hash" is FATAL.
#
# spel can exit 0 having submitted nothing at all — a wallet config it cannot
# read, an argument it rejects — and simply print no hash. The previous form here
# was `[ -n "$TX" ] && wait_tx ...`, which reads that as "nothing to wait for"
# and moves on. A whole lifecycle then completes with exit 0, a printed summary,
# and an empty chain.
#
# That is the worst failure this script can have, because it is the one an
# evaluator would trust. It happened: against LEZ v0.2.4 the released spel failed
# every instruction with `missing field 'sequencer_addr'`, and the run still
# reported success — with approvals timed at 1s and 32s, where a real proof takes
# minutes. The timings were the only tell.
require_tx() { # out-file label
  local hash; hash=$(tx_hash_from "$1")
  if [ -z "$hash" ]; then
    echo "  NO TRANSACTION for $2 — spel submitted nothing. Its output:" >&2
    tail -12 "$1" >&2
    exit 1
  fi
  wait_tx "$hash" "$2"
}

# Run the wallet with the framework path it needs, set on its own exec.
#
# The wallet links Python 3.9 and dies with
# `Library not loaded: @rpath/Python3.framework/... no LC_RPATH's found`
# unless DYLD_FALLBACK_FRAMEWORK_PATH points at the CommandLineTools frameworks.
# Exporting that in a calling shell is not enough: macOS System Integrity
# Protection strips every DYLD_* variable when bash execs another script, so a
# wrapper script's export never reaches a wallet invoked from the script it
# calls. Setting it on the wallet's own exec survives, because the wallet is not
# a protected binary.
#
# This cost an hour of a deploy failing with SIGABRT and no message, because the
# output was going to /dev/null.
WALLET_ENV=()
if [ "$(uname)" = "Darwin" ]; then
  WALLET_ENV=(env "DYLD_FALLBACK_FRAMEWORK_PATH=${DYLD_FALLBACK_FRAMEWORK_PATH:-/Library/Developer/CommandLineTools/Library/Frameworks}")
fi
wallet_run() { "${WALLET_ENV[@]}" "$WALLET_BIN" "$@"; }

# A transaction is confirmed iff `getTransaction` returns a non-null result.
#
# Do NOT test the *shape* of that result. This used to grep for `"result":"`,
# which quietly assumed the node returns a string; on LEZ v0.2.4 it returns a
# decoded object, so the check failed for transactions that were demonstrably in
# a block. That reads exactly like a dead deploy — three retries, then a hard
# failure — for a deploy that landed the first time.
confirmed() {
  curl -s -m 20 -X POST "$RPC" -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getTransaction\",\"params\":[\"$1\"]}" \
    | python3 -c 'import json,sys
try:    sys.exit(0 if json.load(sys.stdin).get("result") is not None else 1)
except Exception: sys.exit(1)' 2>/dev/null
}
wait_tx() { # hash label
  for _ in $(seq 1 60); do
    confirmed "$1" && { echo "  confirmed  $2  $1"; printf '%s\t%s\n' "$2" "$1" >> "$LOG"; return 0; }
    sleep 10
  done
  echo "  TIMEOUT    $2  $1" >&2
  printf '%s\t%s\tTIMEOUT\n' "$2" "$1" >> "$LOG"
  return 1
}
deploy_hash() {
  python3 -c "
import hashlib,struct,sys
b=open(sys.argv[1],'rb').read()
print(hashlib.sha256(struct.pack('<I',len(b))+b).hexdigest())" "$1"
}

# Deploy one program, retrying until the chain says it is there.
#
# `wallet deploy-program` reports nothing useful: it exits 0 and prints nothing
# whether it worked or not, and it sometimes dies with SIGABRT while reading
# stdin, which is why stdin is pinned to /dev/null here rather than inherited.
# Neither its exit code nor its output can be trusted, so the only real test is
# `getTransaction` on the content-addressed hash — and the only sane response to
# a failed attempt is another attempt. Discarding the output entirely, as this
# did before, turned a dead deploy into a ten-minute silent timeout.
deploy() { # file label
  local hash; hash=$(deploy_hash "$1")
  if confirmed "$hash"; then
    echo "  already on chain  $2  $hash"
    printf '%s\t%s\n' "$2" "$hash" >> "$LOG"
    return 0
  fi
  local attempt
  for attempt in 1 2 3; do
    wallet_run deploy-program "$1" </dev/null > "$WORK/deploy_$2.out" 2>&1
    for _ in $(seq 1 12); do
      confirmed "$hash" && { echo "  deployed  $2  $hash"; printf '%s\t%s\n' "$2" "$hash" >> "$LOG"; return 0; }
      sleep 5
    done
    echo "  attempt $attempt did not land for $2, retrying" >&2
  done
  echo "  FAILED to deploy $2 after 3 attempts; wallet output:" >&2
  tail -5 "$WORK/deploy_$2.out" >&2
  return 1
}

echo "[1/6] deploy both programs (content-addressed, so this is idempotent)"
deploy "$MEMBERSHIP" "deploy:membership_lez" || exit 1
deploy "$VERIFIER"   "deploy:multisig_verifier" || exit 1

echo "[2/6] build a ${THRESHOLD}-of-${MEMBERS} member set"
MSIG_ID=$(python3 -c "import os;print(os.urandom(32).hex())")
"$CLI" new-multisig --members "$MEMBERS" --threshold "$THRESHOLD" --id "$MSIG_ID" --out "$WORK"
"$CLI" create-multisig-args --dir "$WORK" --out "$WORK/create.args" >/dev/null

echo "[3/6] commit the multisig on chain"
read_args "$WORK/create.args"
"$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
  -- create_multisig --authority "Public/$SIGNER" "${ARGS[@]}" \
  2>&1 | tee "$WORK/create.out" | tail -3
require_tx "$WORK/create.out" "create_multisig"

echo "[4/6] publish a proposal"
PROP_ID=$(python3 -c "import os;print(os.urandom(32).hex())")
ACTION="transfer 100 LEZ to the grants treasury"
"$CLI" propose --dir "$WORK" --proposal-id "$PROP_ID" --action "$ACTION"
"$CLI" create-proposal-args --dir "$WORK" --proposal-id "$PROP_ID" --out "$WORK/prop.args" >/dev/null
read_args "$WORK/prop.args"
"$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
  -- create_proposal --authority "Public/$SIGNER" "${ARGS[@]}" \
  2>&1 | tee "$WORK/prop.out" | tail -3
require_tx "$WORK/prop.out" "create_proposal"

echo "[5/6] gather $THRESHOLD approvals on the privacy-preserving path"
echo "      each is a real proof composed on chain; ~150 s on a laptop and"
echo "      ~20 min on a shared CI runner. Timed below, per machine."
for i in $(seq 0 $((THRESHOLD-1))); do
  echo "-- member $i"
  # Timed, because "proof generation time" is a required benchmark and a number
  # the script measures beats a number the README remembers.
  T_WITNESS_START=$(date +%s)
  "$CLI" approve-args --dir "$WORK" --proposal-id "$PROP_ID" --member "$i" \
    --out "$WORK/approve_$i.args" | sed 's/^/   /'
  # Re-sync before each approval: a privacy transaction spends commitments, and
  # a stale view produces a proof the sequencer drops.
  wallet_run account sync-private </dev/null >/dev/null 2>&1 || true
  # One approver account per approval — see the APPROVERS note above.
  APPROVER="${APPROVER_LIST[$i]}"
  read_args "$WORK/approve_$i.args"
  "$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" --bin-membership "$MEMBERSHIP" \
    -- approve --approver "Private/$APPROVER" "${ARGS[@]}" \
    2>&1 | tee "$WORK/approve_$i.out" | tail -3
  require_tx "$WORK/approve_$i.out" "approve:member_$i"
  T_APPROVE_END=$(date +%s)
  printf '   approval %d: %ds wall clock (witness + proof + submit + confirm)\n' \
    "$i" "$((T_APPROVE_END - T_WITNESS_START))"
  printf 'timing:approve:member_%s\t%ss\n' "$i" "$((T_APPROVE_END - T_WITNESS_START))" >> "$LOG"
done

echo "[6/6] execute"
"$CLI" status --dir "$WORK" --proposal-id "$PROP_ID"
"$CLI" execute-args --dir "$WORK" --proposal-id "$PROP_ID" --out "$WORK/exec.args" >/dev/null
# The approval marker accounts, in the same order as the nullifiers.
#
# `approvals` is a variadic (rest) account, and spel parses those as ONE
# comma-separated flag: spel-cli/src/tx.rs uses last_value(), so repeating
# --approvals silently keeps only the last one and the program then rejects the
# call with E_APPROVAL_COUNT_MISMATCH (5009) because the account count no longer
# matches the nullifier count.
MARKERS=""
while read -r seed; do
  [ -z "$seed" ] && continue
  ADDR=$(python3 scripts/pda.py "$VERIFIER" "$seed")
  MARKERS="${MARKERS:+$MARKERS,}$ADDR"
done < "$WORK/exec.markers"
read_args "$WORK/exec.args"
"$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
  -- execute --executor "Public/$SIGNER" --approvals "$MARKERS" "${ARGS[@]}" \
  2>&1 | tee "$WORK/exec.out" | tail -3
require_tx "$WORK/exec.out" "execute"

echo
echo "lifecycle recorded in $LOG"
column -t "$LOG" 2>/dev/null || cat "$LOG"
echo
echo "verify independently with:  ./scripts/verify-onchain.sh $WORK $PROP_ID"
