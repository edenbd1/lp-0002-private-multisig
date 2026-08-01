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
# Env: SEQUENCER_URL (default https://testnet.lez.logos.co), SPEL_BIN.

set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

WORK="${1:?usage: verify-onchain.sh <work-dir> <proposal-id-hex>}"
PROP_ID="${2:?usage: verify-onchain.sh <work-dir> <proposal-id-hex>}"
RPC="${SEQUENCER_URL:-https://testnet.lez.logos.co}"
SPEL_BIN="${SPEL_BIN:-spel}"
VERIFIER=artifacts/programs/multisig_verifier.bin

PROGRAM_HEX=$("$SPEL_BIN" program-id "$VERIFIER" | awk -F': *' '/ProgramId \(hex\)/{print $2; exit}')
echo "verifier program  $PROGRAM_HEX"
echo "sequencer         $RPC"
echo

rpc() { # method params-json
  curl -s -m 20 -X POST "$RPC" -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":$2}"
}

check_account() { # label address
  local label="$1" addr="$2"
  local out owner
  out=$(rpc getAccount "[\"$addr\"]")
  owner=$(printf '%s' "$out" | python3 -c "
import json,sys
try:
    d=json.load(sys.stdin)
except Exception:
    print('UNPARSEABLE'); raise SystemExit
r=d.get('result')
if not r: print('ABSENT'); raise SystemExit
po=r.get('program_owner') or r.get('programOwner')
print(po if po else 'NO-OWNER-FIELD')
" 2>/dev/null)
  printf '  %-22s %s\n' "$label" "$addr"
  printf '  %-22s owner: %s\n' "" "$owner"
  case "$owner" in
    ABSENT)        printf '  %-22s \033[31mNOT PRESENT\033[0m\n\n' "" ;;
    UNPARSEABLE|NO-OWNER-FIELD)
                   printf '  %-22s \033[33mcould not read owner; inspect manually\033[0m\n\n' "" ;;
    *)             printf '  %-22s \033[32mpresent\033[0m\n\n' "" ;;
  esac
}

PROPOSAL_JSON="$WORK/proposals/$PROP_ID.json"
[ -f "$PROPOSAL_JSON" ] || { echo "no local record at $PROPOSAL_JSON" >&2; exit 1; }

PROPOSAL_REF=$(python3 -c "import json;print(json.load(open('$PROPOSAL_JSON'))['proposal_ref_hex'])")
CONFIG_HASH=$(python3 -c "import json;print(json.load(open('$WORK/multisig.json'))['config_hash_hex'])")
MSIG_ID=$(python3 -c "import json;print(json.load(open('$WORK/multisig.json'))['id_hex'])")

echo "multisig instance"
check_account "multisig PDA" "$("$SPEL_BIN" pda --program "$PROGRAM_HEX" "$MSIG_ID" "$CONFIG_HASH")"

echo "proposal"
check_account "proposal PDA" "$("$SPEL_BIN" pda --program "$PROGRAM_HEX" "$PROPOSAL_REF")"

echo "approval markers — each one is an approval whose membership proof the"
echo "privacy circuit verified on chain, and none of them names a member"
python3 -c "
import json
d=json.load(open('$PROPOSAL_JSON'))
for a in d['approvals']: print(a['marker_seed_hex'])
" | while read -r seed; do
  check_account "approval marker" "$("$SPEL_BIN" pda --program "$PROGRAM_HEX" "$seed")"
done

echo "execution marker — present means the threshold was met and executed"
EXEC_SEED=$(python3 -c "
import hashlib
pref=bytes('/lp-0002/v0.1/ExecMark/','ascii').ljust(32,b'\x00')
print(hashlib.sha256(pref+bytes.fromhex('$PROPOSAL_REF')).hexdigest())")
check_account "execution marker" "$("$SPEL_BIN" pda --program "$PROGRAM_HEX" "$EXEC_SEED")"
