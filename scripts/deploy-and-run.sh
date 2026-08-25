#!/usr/bin/env bash
# LP-0002 full lifecycle on the public LEZ testnet: deploy both programs, create
# a multisig instance and its treasury, fund the treasury, publish a proposal to
# pay somebody out of it, gather the threshold of approvals on the
# privacy-preserving path, and execute — which moves the money.
#
# Each approval is a real Risc0 proof composed on chain via env::verify, with
# RISC0_DEV_MODE=0. This is the script behind the "1 multisig instance, one
# proposal submitted, approved by threshold, and executed" criterion, and behind
# "a reference threshold-gated action": the treasury balance falls by exactly the
# proposed amount and the recipient's rises by it, in the execute transaction.
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
#   RECIPIENT    Public account id the proposal pays. It must be held by the
#                native transfer program, or the verifier refuses to pay it
#                (E_RECIPIENT_UNUSABLE, 5020) — money in an account nobody can
#                spend from is a burn, not a payment. Make one with:
#                  wallet account new public
#                  wallet auth-transfer init --account-id Public/<id>
#                It must NOT be the signer: one account cannot appear twice in a
#                transaction.
#   MEMBERS      member set size          (default 5)
#   THRESHOLD    approvals required       (default 3)
#   FUND         how much to put in the treasury (default 500)
#   AMOUNT       how much the proposal pays      (default 250)
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
: "${RECIPIENT:?set RECIPIENT to a Public account id held by the native transfer program}"
FUND="${FUND:-500}"
AMOUNT="${AMOUNT:-250}"
# Spending tiers, as `MAX:THRESHOLD` separated by spaces — e.g. TIERS="300:2".
# Empty means none, which is what every deployment before tiers existed
# anchored. A tier may only lower the bar, and the CLI refuses an illegal
# table before anything reaches the chain.
TIERS="${TIERS:-}"
# Set to 1 to append a rotation: a second configuration anchored at its own
# address, the first marked superseded. Off by default because it costs one
# real proof per approval and one private approver account each.
ROTATE="${ROTATE:-0}"
if [ "$RECIPIENT" = "$SIGNER" ]; then
  echo "RECIPIENT must differ from SIGNER: LEZ refuses a transaction naming one account twice" >&2
  exit 1
fi
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
# One account's balance, or the empty string if the chain does not know it.
balance_of() {
  curl -s -m 20 -X POST "$RPC" -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getAccount\",\"params\":[\"$1\"]}" \
    | python3 -c 'import json,sys
try:
    r = json.load(sys.stdin).get("result")
    print(r["balance"] if r else "")
except Exception:
    print("")' 2>/dev/null
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

echo "[1/8] deploy both programs (content-addressed, so this is idempotent)"
deploy "$MEMBERSHIP" "deploy:membership_lez" || exit 1
deploy "$VERIFIER"   "deploy:multisig_verifier" || exit 1

echo "[2/8] build a ${THRESHOLD}-of-${MEMBERS} member set"
MSIG_ID=$(python3 -c "import os;print(os.urandom(32).hex())")
TIER_FLAGS=()
for t in $TIERS; do TIER_FLAGS+=(--tier "$t"); done
"$CLI" new-multisig --members "$MEMBERS" --threshold "$THRESHOLD" \
  ${TIER_FLAGS[@]+"${TIER_FLAGS[@]}"} --id "$MSIG_ID" --out "$WORK"
"$CLI" create-multisig-args --dir "$WORK" --out "$WORK/create.args" >/dev/null

echo "[3/8] commit the multisig and open its treasury"
read_args "$WORK/create.args"
"$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
  -- create_multisig --authority "Public/$SIGNER" "${ARGS[@]}" \
  2>&1 | tee "$WORK/create.out" | tail -3
require_tx "$WORK/create.out" "create_multisig"

# The treasury's address, derived the same way the program derives it. Recorded
# now so every later step and every reader uses one value.
read -r SEED_ID SEED_CFG SEED_LIT <<EOF
$("$CLI" treasury-seeds --dir "$WORK")
EOF
TREASURY=$(python3 scripts/pda.py "$VERIFIER" "$SEED_ID" "$SEED_CFG" "$SEED_LIT")
echo "      treasury  $TREASURY"
printf 'treasury\t%s\n' "$TREASURY" >> "$LOG"

echo "[4/8] fund the treasury (a chained call into the native transfer program)"
# Separate from creation, and not by preference: an account cannot be
# initialised and paid into in one transaction, because the chained transfer
# reads a pre-state the initialisation has not written yet.
"$CLI" fund-treasury-args --dir "$WORK" --amount "$FUND" --out "$WORK/fund.args" >/dev/null
read_args "$WORK/fund.args"
"$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
  -- fund_treasury --funder "Public/$SIGNER" "${ARGS[@]}" \
  2>&1 | tee "$WORK/fund.out" | tail -3
require_tx "$WORK/fund.out" "fund_treasury"
echo "      treasury balance now $(balance_of "$TREASURY")"

echo "[5/8] publish a proposal to pay $AMOUNT out of it"
PROP_ID=$(python3 -c "import os;print(os.urandom(32).hex())")
MEMO="transfer $AMOUNT LEZ to the grants treasury"
# `spel` takes a base58 account id; the protocol commits to the 32 raw bytes.
RECIPIENT_HEX=$(python3 -c "
import sys; sys.path.insert(0,'scripts')
from importlib import import_module
print(import_module('pda').b58decode('$RECIPIENT').hex())")
"$CLI" propose --dir "$WORK" --proposal-id "$PROP_ID" \
  --recipient "$RECIPIENT_HEX" --amount "$AMOUNT" --memo "$MEMO"
"$CLI" create-proposal-args --dir "$WORK" --proposal-id "$PROP_ID" --out "$WORK/prop.args" >/dev/null
read_args "$WORK/prop.args"
"$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
  -- create_proposal --authority "Public/$SIGNER" "${ARGS[@]}" \
  2>&1 | tee "$WORK/prop.out" | tail -3
require_tx "$WORK/prop.out" "create_proposal"

echo "[6/8] gather approvals on the privacy-preserving path"
echo "      each is a real proof composed on chain. Cost is per machine and"
echo "      depends on what else is proving: timed below, not predicted."
# Gather until the proposal has what it needs, not a fixed count. With no tiers
# that is the threshold and this loop runs exactly THRESHOLD times. With a tier
# covering this amount it stops earlier — and stopping earlier is the tier doing
# something, measured in proofs not asserted in prose. `execute-args` is the
# oracle because it applies `required_threshold`, the same function the chain
# applies.
GATHERED=0
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
  GATHERED=$((GATHERED + 1))
  if "$CLI" execute-args --dir "$WORK" --proposal-id "$PROP_ID" \
       --out "$WORK/exec.args" >/dev/null 2>&1; then
    if [ "$GATHERED" -lt "$THRESHOLD" ]; then
      echo "      $GATHERED approvals carry $AMOUNT — a tier priced it below the"
      echo "      default threshold of $THRESHOLD, so the remaining proofs are not needed"
      printf 'tiered:approvals_used\t%s of %s\n' "$GATHERED" "$THRESHOLD" >> "$LOG"
    fi
    break
  fi
done

echo "[7/8] execute — the step that moves the money"
T_BEFORE=$(balance_of "$TREASURY")
R_BEFORE=$(balance_of "$RECIPIENT")
echo "      before: treasury $T_BEFORE, recipient $R_BEFORE"
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
  -- execute --executor "Public/$SIGNER" --recipient "Public/$RECIPIENT" \
     --approvals "$MARKERS" "${ARGS[@]}" \
  2>&1 | tee "$WORK/exec.out" | tail -3
require_tx "$WORK/exec.out" "execute"

echo "[8/8] read the balances back off the chain"
# The check that matters. A marker PDA proves the threshold was reached; only
# these two numbers prove it did anything.
T_AFTER=$(balance_of "$TREASURY")
R_AFTER=$(balance_of "$RECIPIENT")
printf '      treasury  %s -> %s\n' "$T_BEFORE" "$T_AFTER"
printf '      recipient %s -> %s\n' "$R_BEFORE" "$R_AFTER"
printf 'balance:treasury\t%s -> %s\n'  "$T_BEFORE" "$T_AFTER"  >> "$LOG"
printf 'balance:recipient\t%s -> %s\n' "$R_BEFORE" "$R_AFTER" >> "$LOG"
if [ "$((T_BEFORE - AMOUNT))" != "$T_AFTER" ] || [ "$((R_BEFORE + AMOUNT))" != "$R_AFTER" ]; then
  echo "  BALANCES DID NOT MOVE BY $AMOUNT — the execution did not pay out" >&2
  exit 1
fi
echo "      moved $AMOUNT, both sides, in the execute transaction"

if [ "$ROTATE" = "1" ]; then
  echo
  echo "[9/11] define the configuration to rotate into"
  # Same multisig id, a different member set and threshold. A rotation does not
  # rewrite this multisig: it anchors a SECOND one at its own address, with its
  # own treasury, and marks this one superseded.
  NEXT_THRESHOLD="${NEXT_THRESHOLD:-$MEMBERS}"
  "$CLI" new-multisig --members "$MEMBERS" --threshold "$NEXT_THRESHOLD" \
    --id "$MSIG_ID" --out "$WORK/next"
  ROT_ID=$(python3 -c "import os;print(os.urandom(32).hex())")
  "$CLI" propose-rotation --dir "$WORK" --proposal-id "$ROT_ID" --to "$WORK/next"
  read_args "$WORK/prop_rot.args" 2>/dev/null || true
  "$CLI" create-proposal-args --dir "$WORK" --proposal-id "$ROT_ID" \
    --out "$WORK/prop_rot.args" >/dev/null
  read_args "$WORK/prop_rot.args"
  "$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
    -- create_proposal --authority "Public/$SIGNER" "${ARGS[@]}" \
    2>&1 | tee "$WORK/prop_rot.out" | tail -3
  require_tx "$WORK/prop_rot.out" "create_proposal:rotation"

  echo "[10/11] gather $THRESHOLD approvals for the rotation"
  echo "        the DEFAULT threshold, never a tier: pricing governance by tier"
  echo "        would make the cheapest action the one that rewrites who may act"
  for i in $(seq 0 $((THRESHOLD-1))); do
    echo "-- member $i"
    T0=$(date +%s)
    "$CLI" approve-args --dir "$WORK" --proposal-id "$ROT_ID" --member "$i" \
      --out "$WORK/rot_approve_$i.args" | sed 's/^/   /'
    wallet_run account sync-private </dev/null >/dev/null 2>&1 || true
    # One approver account per approval, and the rotation needs its own: a
    # privacy transaction consumes the approver's commitment, so the accounts
    # that carried the transfer cannot carry this too.
    APPROVER="${APPROVER_LIST[$((i + GATHERED))]:-${APPROVER_LIST[$i]}}"
    read_args "$WORK/rot_approve_$i.args"
    "$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" --bin-membership "$MEMBERSHIP" \
      -- approve --approver "Private/$APPROVER" "${ARGS[@]}" \
      2>&1 | tee "$WORK/rot_approve_$i.out" | tail -3
    require_tx "$WORK/rot_approve_$i.out" "approve:rotation:member_$i"
    printf 'timing:approve:rotation:member_%s\t%ss\n' "$i" "$(( $(date +%s) - T0 ))" >> "$LOG"
  done

  echo "[11/11] rotate — the old configuration is retired, not rewritten"
  "$CLI" rotate-args --dir "$WORK" --proposal-id "$ROT_ID" --to "$WORK/next" \
    --out "$WORK/rot.args"
  ROT_MARKERS=""
  while read -r seed; do
    [ -z "$seed" ] && continue
    ROT_MARKERS="${ROT_MARKERS:+$ROT_MARKERS,}$(python3 scripts/pda.py "$VERIFIER" "$seed")"
  done < "$WORK/rot.markers"
  read -r N_ID N_CFG N_LIT <<EOF
$("$CLI" treasury-seeds --dir "$WORK/next")
EOF
  NEW_MULTISIG=$(python3 scripts/pda.py "$VERIFIER" "$N_ID" "$N_CFG")
  NEW_TREASURY=$(python3 scripts/pda.py "$VERIFIER" "$N_ID" "$N_CFG" "$N_LIT")
  read_args "$WORK/rot.args"
  "$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
    -- rotate_config --executor "Public/$SIGNER" --approvals "$ROT_MARKERS" "${ARGS[@]}" \
    2>&1 | tee "$WORK/rot.out" | tail -3
  require_tx "$WORK/rot.out" "rotate_config"
  printf 'account:new_multisig\t%s\n' "$NEW_MULTISIG" >> "$LOG"
  printf 'account:new_treasury\t%s\n' "$NEW_TREASURY" >> "$LOG"
  echo "        new configuration  $NEW_MULTISIG"
  echo "        its treasury       $NEW_TREASURY  (empty: a rotation moves no value)"
fi

echo
echo "lifecycle recorded in $LOG"
column -t "$LOG" 2>/dev/null || cat "$LOG"
echo
echo "verify independently with:  ./scripts/verify-onchain.sh $WORK $PROP_ID"
