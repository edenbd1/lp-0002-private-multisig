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
#   - the two deployed programs are the binaries committed here, by recomputing
#     the deploy transaction hash from the bytecode rather than trusting a hash
#     written down next to it;
#   - the seven transactions of the lifecycle resolve;
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
check "deploy multisig_verifier" 2 517efe12a0b592abe4d21a03246866b95c4379483e87af62fd9f26f7b8fe45ff
check "create_multisig"          0 2930c1db4521b7c0b912278f4025e430704cfb9a7ebfcb5d22c374fd7ce85b70
check "create_proposal"          0 68d5127e1e5570936f8d78e9a2da4d485562566cd8b7487a59322bf059406978
check "approve (member A)"       1 41f5bb99346a0bef6aa0c69243473a554b84f0f0ad65e460bbb6890b11644942
check "approve (member B)"       1 ae006465f5f945b8ba2666f28a5357d0a2aab4af05508c9c2811e0101d0ac649
check "execute"                  0 b43e46505f571e31d6051f7da43563db605b6a74b90c670da2d3582d53412ecd
echo

if [ "$fail" -eq 0 ]; then
  echo "Both programs on chain are the bytecode in this repository, the seven"
  echo "transactions resolve, and the two approvals are PrivacyPreserving rather"
  echo "than Public — which is the part a block explorer cannot show you."
else
  echo "Something above did not hold." >&2
fi
exit "$fail"
