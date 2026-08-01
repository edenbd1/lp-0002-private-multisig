#!/usr/bin/env bash
# LP-0002 end-to-end demo, runnable from a clean clone with no network and no
# funded account.
#
# WHAT THIS PROVES, AND WHAT IT DOES NOT
#
# Everything here runs against the *sequencer's own executor* — the same
# executor, the same input order, the same 32M session limit the chain uses
# (`lee/state_machine/src/program.rs:55-110`). A rejection you see below is the
# rejection the chain performs, byte for byte, because it is the same binary
# being fed the same inputs.
#
# What it does not do is *prove*. Proving a single approval takes minutes and
# establishes nothing extra about which inputs are accepted, so the adversarial
# suite executes rather than proves. The real proofs, generated with
# RISC0_DEV_MODE=0 and verified on chain by LEZ's privacy circuit, are what
# `scripts/deploy-and-run.sh` produces against the public testnet — see
# docs/DEPLOYMENT.md for the transaction hashes.
#
# The last step reads the live deployment straight off the public testnet, using
# the manifest committed under artifacts/testnet/ — so it works from a clean
# clone with no local state.
#
#   ./scripts/demo.sh

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export RISC0_DEV_MODE=0

WORK="${WORK:-$ROOT/.demo}"
rm -rf "$WORK"; mkdir -p "$WORK"

MSIG_ID=00000000000000000000000000000000000000000000000000000000000000a1
PROP_ID=0000000000000000000000000000000000000000000000000000000000000001
ACTION="transfer 100 LEZ to the grants treasury"

rule() { printf '\n\033[1m== %s\033[0m\n' "$1"; }

rule "0. environment"
echo "RISC0_DEV_MODE=$RISC0_DEV_MODE  (0 = real proofs, no mock receipts)"
rustc --version
test -f artifacts/programs/multisig_verifier.bin \
  || { echo "artifacts/programs/multisig_verifier.bin missing. Run ./scripts/build-programs.sh" >&2; exit 1; }
spel program-id artifacts/programs/*.bin | grep -E '📦|ImageID'

rule "1. the seven bindings, in the circuit"
echo "25 adversarial tests over the approval logic: non-members, borrowed paths,"
echo "invented roots, forged nullifiers, bait-and-switch actions, padding leaves,"
echo "and a member listed twice still getting exactly one vote."
cargo test -p multisig-core --quiet 2>&1 \
  | grep -E 'test result' | grep -v ' 0 passed' | sed 's/^/   /'

rule "2. the on-chain checks, through the sequencer's executor"
echo "28 tests against the built verifier binary: four honest controls, one per"
echo "instruction, and 22 attacks each required to be rejected with its documented"
echo "error code — including the threshold bypass a third audit pass found."
echo "Plus 2 that pin the verifier to the exact membership binary it chains to."
cargo test -p multisig-verifier-tests --quiet 2>&1 \
  | grep -E 'test result' | grep -v ' 0 passed' | sed 's/^/   /'

rule "3. a 3-of-5 multisig, client side"
cargo build --release --quiet -p multisig-cli
M=target/release/msig
$M new-multisig --members 5 --threshold 3 --id "$MSIG_ID" --out "$WORK"

rule "4. a proposal, bound to its exact action"
$M propose --dir "$WORK" --proposal-id "$PROP_ID" --action "$ACTION"

rule "5. three members approve, independently"
for i in 0 3 4; do
  echo "-- member $i"
  $M approve-args --dir "$WORK" --proposal-id "$PROP_ID" --member "$i" --out "$WORK/a$i.args" \
    | sed 's/^/   /'
done

rule "6. a fourth attempt by member 0 is refused"
if $M approve-args --dir "$WORK" --proposal-id "$PROP_ID" --member 0 --out "$WORK/dup.args" 2>&1 \
   | sed 's/^/   /'; then
  echo "   UNEXPECTED: a double approval was accepted" >&2; exit 1
fi

rule "7. the action cannot be swapped under the same id"
if $M propose --dir "$WORK" --proposal-id "$PROP_ID" --action "transfer 1000000 LEZ to the attacker" 2>&1 \
   | sed 's/^/   /'; then
  echo "   UNEXPECTED: the action was swapped" >&2; exit 1
fi

rule "8. threshold reached, execution arguments emitted"
$M status --dir "$WORK" --proposal-id "$PROP_ID"
echo
$M execute-args --dir "$WORK" --proposal-id "$PROP_ID" --out "$WORK/exec.args"

rule "9. what an observer sees"
python3 - "$WORK/proposals/$PROP_ID.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
print("On chain, this proposal is three marker PDAs at these addresses:\n")
for a in d["approvals"]:
    print("   ", a["marker_seed_hex"])
print("""
Each address is SHA256(prefix || proposal_ref || nullifier), and each nullifier
is SHA256(prefix || proposal_ref || msk) for a member secret nobody else holds.
An observer who knows all five members — including the other four members —
cannot tell which three of them these came from. That is the unlinkability
criterion, and it holds against insiders, not just outsiders.""")
PY

rule "10. compute cost"
cargo test -p multisig-verifier-tests --quiet -- --ignored --nocapture 2>&1 \
  | grep -E 'approve |execute ' | sed 's/^/   /'

rule "11. the Basecamp package"
if [ -f app/lp-0002-multisig.lgx ]; then
  echo "The committed .lgx, with its own manifest hashes recomputed from its"
  echo "contents — so the package is checked, not just present."
  python3 scripts/package-lgx.py --verify app/lp-0002-multisig.lgx | sed 's/^/   /'
else
  echo "app/lp-0002-multisig.lgx missing. Build it with:"
  echo "  cd app && cmake -B build -S . -DCMAKE_PREFIX_PATH=\$(brew --prefix qt) && cmake --build build"
  echo "  python3 scripts/package-lgx.py"
fi

rule "12. the live deployment, read straight off the chain"
echo "The multisig above is local. This checks the one that is actually deployed:"
echo "its manifest is committed under artifacts/testnet/, so anyone can re-run this."
echo
./scripts/verify-onchain.sh artifacts/testnet "$(cat artifacts/testnet/proposal_id)" || {
  echo
  echo "If this failed, the public testnet may be unreachable from here."
  echo "The transaction hashes are in docs/DEPLOYMENT.md and can be checked with"
  echo "any JSON-RPC client."
}

printf '\n\033[1mdemo complete\033[0m — working directory %s\n' "$WORK"
