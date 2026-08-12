#!/usr/bin/env bash
# Full LP-0002 lifecycle against a REAL LEZ sequencer running locally in
# standalone mode, with RISC0_DEV_MODE=0.
#
# WHY THIS EXISTS SEPARATELY FROM demo.sh
#
# `demo.sh` is the fast tour: it runs the circuit and the built verifier through
# the sequencer's *executor*, which is the same code the chain runs but linked
# in-process. That is enough to show what is rejected and why, in five seconds,
# with no network.
#
# It is not the same claim as "works against a real sequencer". This script
# makes that one: it starts the actual `sequencer_service` binary in standalone
# mode, points a throwaway wallet at it, and drives the whole lifecycle over
# JSON-RPC — deploy, create, propose, gather a threshold of real Risc0
# approvals, execute — then reads the resulting accounts back off that local
# chain. Nothing is mocked and RISC0_DEV_MODE is 0 throughout.
#
#   ./scripts/e2e-local-sequencer.sh
#   MEMBERS=2 THRESHOLD=1 ./scripts/e2e-local-sequencer.sh     # the CI shape
#
# Env:
#   LEZ_SRC     checkout of logos-execution-zone (default: ./_external/lez)
#   MEMBERS     member set size      (default 3)
#   THRESHOLD   approvals required   (default 2)
#   PORT        sequencer RPC port   (default: first free from 3141)
#   KEEP        set to 1 to leave the sequencer running for inspection
#
# Budget: one approval is a real proof, measured at ~150 s. A 2-of-3 run is
# about eight minutes end to end. THRESHOLD=1 still exercises every
# integration point and is what CI uses.

set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

LEZ_SRC="${LEZ_SRC:-$ROOT/_external/lez}"
MEMBERS="${MEMBERS:-3}"
THRESHOLD="${THRESHOLD:-2}"
KEEP="${KEEP:-0}"

# The signing key of the genesis-funded test account, published in clear in
# LEZ's own justfile (`wallet-import-test-accounts`). It is a shared test
# account, not a secret, and it only exists on chains whose genesis lists it.
TEST_SIGNER_KEY=7f273098f25b71e6c005a9519f2678da8d1c7f01f6a27778e2d9948abdf901fb
TEST_SIGNER=CbgR6tj5kWx5oziiFptM7jMvrQeYY3Mzaao6ciuhSr2r

die() { echo "error: $*" >&2; exit 1; }
say() { printf '\n\033[1m%s\033[0m\n' "$*"; }

[ -d "$LEZ_SRC" ] || die "no LEZ checkout at $LEZ_SRC. Clone logos-execution-zone there, or set LEZ_SRC."
CONFIG_SRC="$LEZ_SRC/lez/sequencer/service/configs/debug/sequencer_config.json"
[ -f "$CONFIG_SRC" ] || die "no standalone config at $CONFIG_SRC"

SEQ_BIN="$LEZ_SRC/target/release/sequencer_service"
if [ ! -x "$SEQ_BIN" ]; then
  say "building the standalone sequencer (once; a few minutes)"
  ( cd "$LEZ_SRC" && cargo build --release --features standalone -p sequencer_service ) \
    || die "sequencer build failed"
fi
WALLET_BIN="$LEZ_SRC/target/release/wallet"
[ -x "$WALLET_BIN" ] || ( cd "$LEZ_SRC" && cargo build --release -p wallet ) || die "wallet build failed"

# The wallet links Python 3.9 and dies with `Library not loaded:
# @rpath/Python3.framework/... no LC_RPATH's found` without this. Exporting it
# is not enough: macOS SIP strips DYLD_* whenever bash execs another script, so
# it has to be set on the wallet's own exec. Same reason as in
# deploy-and-run.sh, which does it for the calls it makes.
WALLET_ENV=()
if [ "$(uname)" = "Darwin" ]; then
  WALLET_ENV=(env "DYLD_FALLBACK_FRAMEWORK_PATH=${DYLD_FALLBACK_FRAMEWORK_PATH:-/Library/Developer/CommandLineTools/Library/Frameworks}")
fi
wallet_run() { "${WALLET_ENV[@]}" "$WALLET_BIN" "$@"; }

# A free port, so this never fights a sequencer the developer already has up.
PORT="${PORT:-}"
if [ -z "$PORT" ]; then
  for p in $(seq 3141 3200); do
    if ! nc -z localhost "$p" 2>/dev/null; then PORT=$p; break; fi
  done
fi
[ -n "$PORT" ] || die "no free port in 3141-3200"

WORK="$(mktemp -d)"
SEQ_HOME="$WORK/sequencer"; mkdir -p "$SEQ_HOME"
WALLET_HOME="$WORK/wallet";  mkdir -p "$WALLET_HOME"
RPC="http://localhost:$PORT"
SEQ_PID=""

cleanup() {
  if [ -n "$SEQ_PID" ] && [ "$KEEP" != "1" ]; then
    kill "$SEQ_PID" 2>/dev/null
    wait "$SEQ_PID" 2>/dev/null
  fi
  if [ "$KEEP" = "1" ]; then
    echo
    echo "left running: sequencer pid $SEQ_PID on $RPC"
    echo "  wallet home $WALLET_HOME"
    echo "  logs        $WORK/sequencer.log"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT INT TERM

say "[1/5] starting a real sequencer in standalone mode on $RPC"
python3 - "$CONFIG_SRC" "$SEQ_HOME" <<'PY'
import json, sys
cfg = json.load(open(sys.argv[1]))
cfg["home"] = sys.argv[2]
json.dump(cfg, open(sys.argv[2] + "/sequencer_config.json", "w"), indent=2)
PY
RUST_LOG=info "$SEQ_BIN" --port "$PORT" "$SEQ_HOME/sequencer_config.json" \
  > "$WORK/sequencer.log" 2>&1 &
SEQ_PID=$!

for _ in $(seq 1 60); do
  if curl -s -m 2 -X POST "$RPC" -H 'Content-Type: application/json' \
       -d '{"jsonrpc":"2.0","id":1,"method":"getAccount","params":["'"$TEST_SIGNER"'"]}' \
       | grep -q '"result"'; then
    echo "  up, pid $SEQ_PID"; break
  fi
  kill -0 "$SEQ_PID" 2>/dev/null || { tail -20 "$WORK/sequencer.log"; die "sequencer died on startup"; }
  sleep 1
done
curl -s -m 3 -X POST "$RPC" -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getAccount","params":["'"$TEST_SIGNER"'"]}' \
  | grep -q '"result"' || die "sequencer never answered on $RPC"

say "[2/5] a throwaway wallet pointed at it"
# LEZ v0.2.2 made the wallet multi-sequencer: the single `sequencer_addr` became
# a `sequencers` array. An old-format config does not fail over to a default —
# the wallet refuses to deserialize it and dies before doing anything.
# `multi_sequencer_client_config` is `#[serde(default)]`, so it is left out.
cat > "$WALLET_HOME/wallet_config.json" <<EOF
{ "sequencers": [ { "sequencer_addr": "$RPC/" } ],
  "seq_poll_timeout": "30s", "seq_tx_poll_max_blocks": 15,
  "seq_poll_max_retries": 10, "seq_block_poll_max_amount": 100 }
EOF
export LEE_WALLET_HOME_DIR="$WALLET_HOME"
printf 'lp0002\n' | wallet_run account import public --private-key "$TEST_SIGNER_KEY" >/dev/null 2>&1 \
  || die "could not import the test signer"
echo "  imported Public/$TEST_SIGNER"

say "[3/5] funding it from the genesis vault"
wallet_run vault claim --account-id "Public/$TEST_SIGNER" --amount 5000 </dev/null >/dev/null 2>&1
for _ in $(seq 1 30); do
  bal=$(curl -s -m 3 -X POST "$RPC" -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"getAccount","params":["'"$TEST_SIGNER"'"]}' \
        | python3 -c 'import json,sys; print(json.load(sys.stdin).get("result",{}).get("balance",0))' 2>/dev/null)
  [ "${bal:-0}" -gt 0 ] 2>/dev/null && { echo "  balance $bal"; break; }
  sleep 2
done
[ "${bal:-0}" -gt 0 ] 2>/dev/null || die "vault claim did not land"

say "[4/5] one private account per approval"
# Diff the account list around each creation instead of taking the tail of it.
#
# `account list` prints a derivation tree — `/`, `/0`, `/0/0`, `/1` — so a newly
# created account can appear in the middle. Taking the last N lines picks by
# display order, which is not creation order, and on a wallet with history that
# silently selects *already spent* accounts. A privacy transaction consumes its
# submitter's commitment, so the run would then fail deep in the lifecycle with
# "Invalid account_identities length" after minutes of proving.
list_private() {
  wallet_run account list </dev/null 2>/dev/null \
    | grep -oE 'Private/[1-9A-HJ-NP-Za-km-z]+' | sed 's|Private/||' | sort
}
APPROVER_LIST=()
for _ in $(seq 1 "$THRESHOLD"); do
  before=$(list_private)
  wallet_run account new private </dev/null >/dev/null 2>&1 || die "could not create a private account"
  fresh=$(comm -13 <(printf '%s\n' "$before") <(list_private) | head -1)
  [ -n "$fresh" ] || die "account new private produced nothing new"
  APPROVER_LIST+=("$fresh")
done
APPROVERS=$(IFS=,; echo "${APPROVER_LIST[*]}")
echo "  $APPROVERS"

say "[5/5] the lifecycle, against that sequencer"
echo "  RISC0_DEV_MODE=0, $THRESHOLD real proof(s), ~150 s apiece"
SIGNER="$TEST_SIGNER" \
APPROVERS="$APPROVERS" \
MEMBERS="$MEMBERS" \
THRESHOLD="$THRESHOLD" \
SEQUENCER_URL="$RPC" \
WALLET_BIN="$WALLET_BIN" \
LEE_WALLET_HOME_DIR="$WALLET_HOME" \
WORK="$WORK/lifecycle" \
  ./scripts/deploy-and-run.sh
rc=$?

if [ $rc -ne 0 ]; then
  echo
  echo "lifecycle failed; last sequencer output:" >&2
  tail -30 "$WORK/sequencer.log" >&2
  exit $rc
fi

PROP_ID=$(awk -F'\t' '$1=="create_proposal"{print $2}' "$WORK/lifecycle/lifecycle.tsv" >/dev/null 2>&1; \
          python3 -c "
import json,glob,os
d='$WORK/lifecycle/proposals'
print(os.path.splitext(os.path.basename(sorted(glob.glob(d+'/*.json'))[0]))[0])" 2>/dev/null)

say "verifying the accounts on the local chain"
SEQUENCER_URL="$RPC" ./scripts/verify-onchain.sh "$WORK/lifecycle" "$PROP_ID"
rc=$?

echo
if [ $rc -eq 0 ]; then
  printf '\033[1me2e against a real local sequencer: PASS\033[0m  (%s, %s-of-%s, RISC0_DEV_MODE=0)\n' \
    "$RPC" "$THRESHOLD" "$MEMBERS"
else
  echo "e2e against a real local sequencer: FAIL" >&2
fi
exit $rc
