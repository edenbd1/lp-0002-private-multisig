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
#   SPEL_BIN     spel >= 0.6.0            (default: spel on PATH)
#   WALLET_BIN   wallet from LEZ v0.2.0   (default: wallet on PATH)
#   SEQUENCER_URL                          (default: https://testnet.lez.logos.co)
#
# The wallet may print "Transaction NOT confirmed" for a privacy transaction
# whose proving outruns its polling window; the transaction lands anyway. This
# script checks getTransaction, not the CLI's verdict.
#
# Budget: proving an approval takes over ten minutes. A 3-of-5 run is a couple
# of hours. Run it in a session you can leave alone.

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

confirmed() {
  curl -s -m 20 -X POST "$RPC" -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getTransaction\",\"params\":[\"$1\"]}" \
    | grep -q '"result":"'
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

echo "[1/6] deploy both programs (content-addressed, so this is idempotent)"
"$WALLET_BIN" deploy-program "$MEMBERSHIP" >/dev/null 2>&1 || true
"$WALLET_BIN" deploy-program "$VERIFIER"   >/dev/null 2>&1 || true
wait_tx "$(deploy_hash "$MEMBERSHIP")" "deploy:membership_lez"
wait_tx "$(deploy_hash "$VERIFIER")"   "deploy:multisig_verifier"

echo "[2/6] build a ${THRESHOLD}-of-${MEMBERS} member set"
MSIG_ID=$(python3 -c "import os;print(os.urandom(32).hex())")
"$CLI" new-multisig --members "$MEMBERS" --threshold "$THRESHOLD" --id "$MSIG_ID" --out "$WORK"
"$CLI" create-multisig-args --dir "$WORK" --out "$WORK/create.args" >/dev/null

echo "[3/6] commit the multisig on chain"
read_args "$WORK/create.args"
"$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
  -- create_multisig --authority "Public/$SIGNER" "${ARGS[@]}" \
  2>&1 | tee "$WORK/create.out" | tail -3
CREATE_TX=$(grep -oE '[0-9a-f]{64}' "$WORK/create.out" | head -1)
[ -n "$CREATE_TX" ] && wait_tx "$CREATE_TX" "create_multisig"

echo "[4/6] publish a proposal"
PROP_ID=$(python3 -c "import os;print(os.urandom(32).hex())")
ACTION="transfer 100 LEZ to the grants treasury"
"$CLI" propose --dir "$WORK" --proposal-id "$PROP_ID" --action "$ACTION"
"$CLI" create-proposal-args --dir "$WORK" --proposal-id "$PROP_ID" --out "$WORK/prop.args" >/dev/null
read_args "$WORK/prop.args"
"$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
  -- create_proposal --authority "Public/$SIGNER" "${ARGS[@]}" \
  2>&1 | tee "$WORK/prop.out" | tail -3
PROP_TX=$(grep -oE '[0-9a-f]{64}' "$WORK/prop.out" | head -1)
[ -n "$PROP_TX" ] && wait_tx "$PROP_TX" "create_proposal"

echo "[5/6] gather $THRESHOLD approvals on the privacy-preserving path"
echo "      each is a real proof composed on chain; budget ten minutes apiece"
for i in $(seq 0 $((THRESHOLD-1))); do
  echo "-- member $i"
  "$CLI" approve-args --dir "$WORK" --proposal-id "$PROP_ID" --member "$i" \
    --out "$WORK/approve_$i.args" | sed 's/^/   /'
  # Re-sync before each approval: a privacy transaction spends commitments, and
  # a stale view produces a proof the sequencer drops.
  "$WALLET_BIN" account sync-private >/dev/null 2>&1 || true
  # One approver account per approval — see the APPROVERS note above.
  APPROVER="${APPROVER_LIST[$i]}"
  read_args "$WORK/approve_$i.args"
  "$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" --bin-membership "$MEMBERSHIP" \
    -- approve --approver "Private/$APPROVER" "${ARGS[@]}" \
    2>&1 | tee "$WORK/approve_$i.out" | tail -3
  TX=$(grep -oE '[0-9a-f]{64}' "$WORK/approve_$i.out" | head -1)
  [ -n "$TX" ] && wait_tx "$TX" "approve:member_$i"
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
EXEC_TX=$(grep -oE '[0-9a-f]{64}' "$WORK/exec.out" | head -1)
[ -n "$EXEC_TX" ] && wait_tx "$EXEC_TX" "execute"

echo
echo "lifecycle recorded in $LOG"
column -t "$LOG" 2>/dev/null || cat "$LOG"
echo
echo "verify independently with:  ./scripts/verify-onchain.sh $WORK $PROP_ID"
