#!/usr/bin/env bash
# Independently verify an LP-0002 proposal's on-chain state over JSON-RPC.
#
# WHY THIS EXISTS
#
# A privacy-preserving transaction publishes commitments and nullifiers, not
# `program_id` or `instruction_data`, so the block explorer's indexer has
# nothing to show for an approval. That is the privacy property working as
# designed, and it means "look it up on the explorer" is not a verification
# path. The real check is to read the marker accounts directly and confirm the
# verifier program owns them — which is what this does.
#
#   ./scripts/verify-onchain.sh <work-dir> <proposal-id-hex>
#
# Env: SEQUENCER_URL (default https://testnet.lez.logos.co).
#
# Addresses are derived by `scripts/pda.py`, whose output is checked against the
# live chain by this very script: if the derivation were wrong, the multisig and
# proposal accounts would read as absent.

set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

WORK="${1:?usage: verify-onchain.sh <work-dir> <proposal-id-hex>}"
PROP_ID="${2:?usage: verify-onchain.sh <work-dir> <proposal-id-hex>}"
RPC="${SEQUENCER_URL:-https://testnet.lez.logos.co}"
VERIFIER=artifacts/programs/multisig_verifier.bin

PROPOSAL_JSON="$WORK/proposals/$PROP_ID.json"
[ -f "$PROPOSAL_JSON" ] || { echo "no local record at $PROPOSAL_JSON" >&2; exit 1; }
[ -f "$VERIFIER" ] || { echo "missing $VERIFIER — run ./scripts/build-programs.sh" >&2; exit 1; }

PROPOSAL_REF=$(python3 -c "import json;print(json.load(open('$PROPOSAL_JSON'))['proposal_ref_hex'])")
CONFIG_HASH=$(python3 -c "import json;print(json.load(open('$WORK/multisig.json'))['config_hash_hex'])")
MSIG_ID=$(python3 -c "import json;print(json.load(open('$WORK/multisig.json'))['id_hex'])")
THRESHOLD=$(python3 -c "import json;print(json.load(open('$WORK/multisig.json'))['threshold'])")

echo "sequencer   $RPC"
echo "proposal    $PROPOSAL_REF"
echo "threshold   $THRESHOLD"
echo

fail=0

check() { # label address
  local label="$1" addr="$2"
  local out state
  out=$(curl -s -m 20 -X POST "$RPC" -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getAccount\",\"params\":[\"$addr\"]}" 2>/dev/null || echo '{}')
  state=$(printf '%s' "$out" | VERIFIER_BIN="$VERIFIER" python3 -c "
import json,os,subprocess,sys,struct
# The verifier's own ProgramId, read from the committed binary — so this script
# checks ownership against the program in this repo, not a hard-coded constant.
out = subprocess.run(['spel','program-id',os.environ['VERIFIER_BIN']],
                     capture_output=True, text=True).stdout
words = None
for line in out.splitlines():
    if 'ProgramId (decimal)' in line:
        words = [int(w) for w in line.split(':',1)[1].strip().split(',')]
try:
    d = json.load(sys.stdin).get('result')
except Exception:
    print('RPC-ERROR'); raise SystemExit
if not d:
    print('ABSENT'); raise SystemExit
po = d.get('program_owner')
if po == [0]*8:      print('UNCLAIMED')
elif words and po == words: print('VERIFIER-OWNED')
else:                print('OWNED-BY-ANOTHER-PROGRAM')
" 2>/dev/null)
  case "$state" in
    VERIFIER-OWNED) printf '  \033[32m✅\033[0m %-20s %s\n' "$label" "$addr" ;;
    *)              printf '  \033[31m❌\033[0m %-20s %s   (%s)\n' "$label" "$addr" "$state"; fail=1 ;;
  esac
}

echo "multisig instance — its address anchors the member root AND the threshold"
check "multisig" "$(python3 scripts/pda.py "$VERIFIER" "$MSIG_ID" "$CONFIG_HASH")"

echo "proposal — its address anchors the exact action"
check "proposal" "$(python3 scripts/pda.py "$VERIFIER" "$PROPOSAL_REF")"

echo "approval markers — each is an approval whose membership proof the privacy"
echo "circuit verified on chain, and none of them names a member"
n=0
while read -r seed; do
  [ -z "$seed" ] && continue
  check "approval $n" "$(python3 scripts/pda.py "$VERIFIER" "$seed")"
  n=$((n+1))
done < <(python3 -c "
import json
for a in json.load(open('$PROPOSAL_JSON'))['approvals']: print(a['marker_seed_hex'])")

echo "execution marker — present means the threshold was met and executed"
EXEC_SEED=$(python3 -c "
import hashlib
pref=bytes('/lp-0002/v0.1/ExecMark/','ascii').ljust(32,b'\x00')
print(hashlib.sha256(pref+bytes.fromhex('$PROPOSAL_REF')).hexdigest())")
check "execution" "$(python3 scripts/pda.py "$VERIFIER" "$EXEC_SEED")"

echo
if [ "$fail" = 0 ]; then
  echo "all accounts present and owned by the verifier"
  echo "$n approval(s) recorded against a threshold of $THRESHOLD"
else
  echo "one or more accounts are missing or not owned by the verifier" >&2
fi
exit "$fail"
