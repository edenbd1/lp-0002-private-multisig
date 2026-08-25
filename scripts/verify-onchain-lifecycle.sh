#!/usr/bin/env bash
# Re-check this submission's on-chain claims from a clean clone, with nothing
# local to set up.
#
#   ./scripts/verify-onchain-lifecycle.sh
#
# The other verifier, scripts/verify-onchain.sh, reads a work directory left by
# a live run: it is for the person who just deployed, not for a reviewer who has
# only the repository and the public sequencer. This one takes no arguments.
#
# What it decides, and why each part is here:
#
#   - the deployed programs are the binaries committed here, by recomputing the
#     deploy transaction hash from the bytecode rather than trusting a hash
#     written down next to it;
#   - the thirteen transactions of the lifecycle and the rotation resolve;
#   - and each carries the variant it must carry. That last one is the point of
#     the submission: if the two approvals were Public, threshold approval would
#     be linkable and there would be nothing here worth reviewing. A run that
#     only proved the transactions exist would pass just as green.
#
# The control runs first. A hash that was never deployed must not resolve; if it
# ever does, the sequencer is answering something other than what it is asked and
# nothing below means anything, so the run is abandoned rather than reported.
#
# Needs curl, jq and python3. Exits non-zero on any failure.
set -uo pipefail
cd "$(dirname "$0")/.."

RPC="${SEQUENCER_URL:-https://testnet.lez.logos.co}"
NEVER=dededededededededededededededededededededededededededededededede

fail=0
ok()  { printf '  \033[32mok\033[0m    %s\n' "$1"; }
bad() { printf '  \033[31mFAIL\033[0m  %s\n' "$1"; fail=1; }

rpc() {
  curl -s -m 30 -X POST "$RPC" -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":$2}"
}

# v0.2.4 getTransaction returns [transaction, block_id]; the transaction is [0].
tx_field() { # hash -> "<variant> <bytes>" or empty
  rpc getTransaction "[\"$1\"]" \
    | jq -r '.result[0] // empty' \
    | python3 -c 'import sys,base64
b = sys.stdin.read().strip()
if b:
    raw = base64.b64decode(b)
    print(raw[0], len(raw))'
}

deploy_tx_hash() { # path/to.bin -> the hash its deployment would have
  python3 -c "
import hashlib, struct, sys
b = open(sys.argv[1], 'rb').read()
print(hashlib.sha256(struct.pack('<I', len(b)) + b).hexdigest())" "$1"
}

echo "LP-0002 on the public LEZ testnet — $RPC"
echo

echo "[1/3] the control: a hash that was never deployed must not resolve"
if [ -n "$(tx_field "$NEVER")" ]; then
  echo "  the never-deployed hash resolved. The comparison below would be" >&2
  echo "  meaningless, so nothing is concluded." >&2
  exit 2
fi
ok "it returns null, so a resolving hash below means something"
echo

echo "[2/3] the deployed programs are the binaries committed here"
for prog in membership_lez multisig_verifier; do
  bin="artifacts/programs/$prog.bin"
  if [ ! -f "$bin" ]; then bad "$prog  $bin is missing"; continue; fi
  h=$(deploy_tx_hash "$bin")
  if [ -n "$(tx_field "$h")" ]; then
    ok "$prog  SHA256(len‖bytecode) = ${h:0:16}… is on chain"
  else
    bad "$prog  computed ${h:0:16}… is NOT on chain"
  fi
done
echo

echo "[3/3] the lifecycle, and the variant each transaction must carry"
# variant 0 is Public, 1 is PrivacyPreserving; the deploys carry 2. The
# approvals are the two that must not be 0 — that is the claim being made.
check() { # label expected-variant hash
  read -r got size <<<"$(tx_field "$3")"
  if [ -z "${got:-}" ]; then
    bad "$(printf '%-26s' "$1") not on chain"
  elif [ "$got" != "$2" ]; then
    bad "$(printf '%-26s' "$1") variant $got, expected $2"
  else
    printf -v n '%-26s' "$1"
    ok "$n variant $got, $size bytes"
  fi
}
check "deploy membership_lez"    2 fb8eb10f7f394286c109cb6502a1c95294180523f30d06f707fc087a589bea98
check "deploy multisig_verifier" 2 268834b601f78b59090e90f8f10fd8ce3b526528e1224983edba95224be31aa3
check "create_multisig"          0 dce8fd4dc4b53216d7271466ba66290b3bbfb2cf125701cd7c97a68cb69d1db0
check "fund_treasury"            0 993d1f7c2b27fcab1abf759513f7bf7c64449a547b608e692deae67ec94f640b
check "create_proposal"          0 54a300eb8c0bec27adb40b3ab36ff653b6234dc4deab2a47ac89a0a665c4fdd3
check "approve (member A)"       1 1a5e529d8b9c87ec781b6e8cc2d4bc71c149e7f45f9ce66711108a22dbf6fcd5
check "approve (member B)"       1 28a07e8df970322b643d7d5d6c74640f49e6d0260964255909181fdc815e8397
check "execute"                  0 d0bab2943f09d3a27a10610c49c2a6cce1a2c94b93ded6f341ce29005ba8ca7c

# The rotation. Three approvals rather than two: the tier that priced the
# transfer does not apply to governance, and `rotate_config` never reads it for
# its own count. These are variant 1 for the same reason the transfer's are —
# an approval is an approval, whatever it approves.
check "rotation: proposal"       0 7a9331a9db536f710689b31861e7a3c94462fa1e090140e9fc3af66bc9eee773
check "rotation: approve A"      1 664fa866c041d39733dadadf84e266b1ec9a36e033a2cc271d2032c87dc2b563
check "rotation: approve B"      1 87aeffe96c98d4ed58601bbb5707f5c6516ca602e4a7aff1e274ba2188bb58a0
check "rotation: approve C"      1 6fcc674ae1c198651181afc74b47e005dc90aee4729301e79fb709019a314160
check "rotate_config"            0 cb8a3f8b07db5039b48aefc32e3770e9919b2ce7af0792ef31fb8b47ea942554
echo

if [ "$fail" -eq 0 ]; then
  echo "Both programs on chain are the bytecode in this repository, the thirteen"
  echo "transactions resolve, and the five approvals are PrivacyPreserving rather"
  echo "than Public — which is the part a block explorer cannot show you."
else
  echo "Something above did not hold." >&2
fi
exit "$fail"
