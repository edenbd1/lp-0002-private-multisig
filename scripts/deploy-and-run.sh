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
#   APPROVER     Private account id that signs approvals (authorized)
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
: "${APPROVER:?set APPROVER to an authorized Private account id}"

IDL=idl/multisig_verifier.idl.json
VERIFIER=artifacts/programs/multisig_verifier.bin
MEMBERSHIP=artifacts/programs/membership_lez.bin
WORK="${WORK:-$ROOT/.testnet}"
LOG="$WORK/lifecycle.tsv"

CLI=target/release/msig
cargo build --release --quiet -p multisig-cli

mkdir -p "$WORK"
: > "$LOG"

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
# shellcheck disable=SC2046
"$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
  -- create_multisig --authority "Public/$SIGNER" $(tr '\n' ' ' < "$WORK/create.args") \
  2>&1 | tee "$WORK/create.out" | tail -3
CREATE_TX=$(grep -oE '[0-9a-f]{64}' "$WORK/create.out" | head -1)
[ -n "$CREATE_TX" ] && wait_tx "$CREATE_TX" "create_multisig"

echo "[4/6] publish a proposal"
PROP_ID=$(python3 -c "import os;print(os.urandom(32).hex())")
ACTION="transfer 100 LEZ to the grants treasury"
"$CLI" propose --dir "$WORK" --proposal-id "$PROP_ID" --action "$ACTION"
"$CLI" create-proposal-args --dir "$WORK" --proposal-id "$PROP_ID" --out "$WORK/prop.args" >/dev/null
# shellcheck disable=SC2046
"$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
  -- create_proposal --authority "Public/$SIGNER" $(tr '\n' ' ' < "$WORK/prop.args") \
  2>&1 | tee "$WORK/prop.out" | tail -3
PROP_TX=$(grep -oE '[0-9a-f]{64}' "$WORK/prop.out" | head -1)
[ -n "$PROP_TX" ] && wait_tx "$PROP_TX" "create_proposal"

echo "[5/6] gather $THRESHOLD approvals on the privacy-preserving path"
echo "      each is a real proof composed on chain; budget ten minutes apiece"
for i in $(seq 0 $((THRESHOLD-1))); do
  echo "-- member $i"
  "$CLI" approve-args --dir "$WORK" --proposal-id "$PROP_ID" --member "$i" \
    --out "$WORK/approve_$i.args" | sed 's/^/   /'
  # A privacy transaction spends the signer's commitment, so the approver's
  # private account must be re-synced before each approval or its membership
  # proof is stale and the sequencer drops the transaction.
  "$WALLET_BIN" account sync-private >/dev/null 2>&1 || true
  # shellcheck disable=SC2046
  "$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" --bin-membership "$MEMBERSHIP" \
    -- approve --approver "Private/$APPROVER" $(tr '\n' ' ' < "$WORK/approve_$i.args") \
    2>&1 | tee "$WORK/approve_$i.out" | tail -3
  TX=$(grep -oE '[0-9a-f]{64}' "$WORK/approve_$i.out" | head -1)
  [ -n "$TX" ] && wait_tx "$TX" "approve:member_$i"
done

echo "[6/6] execute"
"$CLI" status --dir "$WORK" --proposal-id "$PROP_ID"
"$CLI" execute-args --dir "$WORK" --proposal-id "$PROP_ID" --out "$WORK/exec.args" >/dev/null
# The trailing approval marker accounts, in the same order as the nullifiers.
MARKERS=""
while read -r seed; do
  [ -z "$seed" ] && continue
  ADDR=$("$SPEL_BIN" pda --program "$($SPEL_BIN program-id "$VERIFIER" | awk -F': *' '/hex/{print $2;exit}')" "$seed")
  MARKERS="$MARKERS --approvals $ADDR"
done < "$WORK/exec.markers"
# shellcheck disable=SC2046,SC2086
"$SPEL_BIN" --idl "$IDL" --program "$VERIFIER" \
  -- execute --executor "Public/$SIGNER" $(tr '\n' ' ' < "$WORK/exec.args") $MARKERS \
  2>&1 | tee "$WORK/exec.out" | tail -3
EXEC_TX=$(grep -oE '[0-9a-f]{64}' "$WORK/exec.out" | head -1)
[ -n "$EXEC_TX" ] && wait_tx "$EXEC_TX" "execute"

echo
echo "lifecycle recorded in $LOG"
column -t "$LOG" 2>/dev/null || cat "$LOG"
echo
echo "verify independently with:  ./scripts/verify-onchain.sh $WORK $PROP_ID"
