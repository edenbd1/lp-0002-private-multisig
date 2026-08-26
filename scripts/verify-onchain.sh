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
#
# It also *decodes* each account, through `scripts/decode-account.py`, which
# parses by byte offset out of docs/account-layout.md and knows nothing about
# this repository's Rust. Ownership says the verifier claimed the address; the
# decoded record says what it claimed it for, and the treasury's balance says
# whether the threshold actually moved anything.

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

# Fail loudly on a missing prerequisite rather than quietly deriving the wrong
# addresses. Without `spel` this script cannot read the verifier's ProgramId,
# every PDA comes out wrong, and it reports a perfectly healthy deployment as
# "accounts missing" — a false negative, and the worst possible one, because
# this is the check you run before trusting the deployment.
for tool in spel python3 curl; do
  command -v "$tool" >/dev/null || {
    echo "missing \`$tool\`, which this script needs to derive addresses." >&2
    echo "Every PDA would be wrong and the report would be a false negative," >&2
    echo "so it stops here instead. Put it on PATH and re-run." >&2
    exit 2
  }
done

PROPOSAL_REF=$(python3 -c "import json;print(json.load(open('$PROPOSAL_JSON'))['proposal_ref_hex'])")
CONFIG_HASH=$(python3 -c "import json;print(json.load(open('$WORK/multisig.json'))['config_hash_hex'])")
MSIG_ID=$(python3 -c "import json;print(json.load(open('$WORK/multisig.json'))['id_hex'])")
THRESHOLD=$(python3 -c "import json;print(json.load(open('$WORK/multisig.json'))['threshold'])")
# What this proposal actually costs, which is not always the default. A tier that
# covers the amount lowers it, and the closing line used to report the default
# alone: "2 approval(s) recorded against a threshold of 3" reads as an execution
# that went through under-approved, when it is the anchored tier doing its job.
# The tier table is in the same record the threshold came from.
REQUIRED=$(python3 -c "
import json
m=json.load(open('$WORK/multisig.json'))
p=json.load(open('$WORK/proposals/$PROP_ID.json'))
amount=p.get('amount')
req=m['threshold']; why=''
if amount is not None:
    for cap, thr in m.get('tiers') or []:
        if amount <= cap:
            req, why = thr, ' (an anchored tier prices %s at %s, below the default of %s)' % (amount, thr, m['threshold'])
            break
print('%d\t%s' % (req, why))" 2>/dev/null)
REQ_N=${REQUIRED%%$'\t'*}
REQ_WHY=${REQUIRED#*$'\t'}

echo "sequencer   $RPC"
echo "proposal    $PROPOSAL_REF"
echo "threshold   $THRESHOLD"
echo

fail=0

check() { # label kind address
  local label="$1" kind="$2" addr="$3"
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
  # An owned account with no data is the defect this submission had: five
  # addresses the verifier owned and not one byte of state behind them. Decoding
  # is therefore part of the verdict, not a nicety.
  if [ "$state" = VERIFIER-OWNED ]; then
    if ! printf '%s' "$out" | python3 scripts/decode-account.py "$kind" --stdin \
         | sed 's/^/    /'; then
      printf '  \033[31m❌\033[0m %-20s the account holds no readable %s record\n' "$label" "$kind"
      fail=1
    fi
  fi
}

echo "multisig instance — its address anchors the member root AND the threshold,"
echo "and its record makes both readable to somebody who knows neither"
check "multisig" multisig "$(python3 scripts/pda.py "$VERIFIER" "$MSIG_ID" "$CONFIG_HASH")"

echo
echo "treasury — the account the threshold spends from. Its balance is the only"
echo "number here that cannot be produced by anything except a real execution"
TREASURY=$(python3 scripts/pda.py "$VERIFIER" "$MSIG_ID" "$CONFIG_HASH" str:treasury)
check "treasury" treasury "$TREASURY"

echo
echo "proposal — its address anchors the exact action AND its multisig"
# The proposal PDA is seeded by the config too, so a proposal belongs to one
# (member_root, threshold) and not merely to a multisig id. See docs/security.md.
check "proposal" proposal "$(python3 scripts/pda.py "$VERIFIER" "$MSIG_ID" "$CONFIG_HASH" "$PROPOSAL_REF")"

echo "approval markers — each is an approval whose membership proof the privacy"
echo "circuit verified on chain, and none of them names a member"
n=0
while read -r seed; do
  [ -z "$seed" ] && continue
  check "approval $n" approval "$(python3 scripts/pda.py "$VERIFIER" "$seed")"
  n=$((n+1))
done < <(python3 -c "
import json
for a in json.load(open('$PROPOSAL_JSON'))['approvals']: print(a['marker_seed_hex'])")

echo "execution marker — present means the threshold was met and executed"
EXEC_SEED=$(python3 -c "
import hashlib
pref=bytes('/lp-0002/v0.1/ExecMark/','ascii').ljust(32,b'\x00')
print(hashlib.sha256(pref+bytes.fromhex('$PROPOSAL_REF')).hexdigest())")
check "execution" execution "$(python3 scripts/pda.py "$VERIFIER" "$EXEC_SEED")"

# The payee. Not owned by the verifier — it is an ordinary public account held by
# the native transfer program — so it is reported rather than asserted on.
RECIPIENT_HEX=$(python3 -c "import json;print(json.load(open('$PROPOSAL_JSON'))['recipient_hex'])" 2>/dev/null || true)
if [ -n "${RECIPIENT_HEX:-}" ]; then
  echo
  echo "recipient — where the money went. Owned by the native transfer program,"
  echo "not by the verifier, which is what makes the balance spendable"
  RECIPIENT_B58=$(python3 -c "
import sys
sys.path.insert(0, 'scripts')
from importlib import import_module
pda = import_module('pda')
print(pda.b58encode(bytes.fromhex('$RECIPIENT_HEX')))")
  curl -s -m 20 -X POST "$RPC" -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getAccount\",\"params\":[\"$RECIPIENT_B58\"]}" \
    | python3 -c "
import json,sys
r = json.load(sys.stdin).get('result')
print('    %-20s %s' % ('$RECIPIENT_B58', 'ABSENT' if not r else 'balance %s' % r['balance']))"
fi

echo
if [ "$fail" = 0 ]; then
  echo "all accounts present and owned by the verifier"
  echo "$n approval(s) recorded against a requirement of ${REQ_N:-$THRESHOLD}${REQ_WHY}"
else
  echo "one or more accounts are missing or not owned by the verifier" >&2
fi
exit "$fail"
