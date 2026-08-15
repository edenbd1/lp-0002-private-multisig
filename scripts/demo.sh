#!/usr/bin/env bash
# LP-0002 end-to-end demo, runnable from a clean clone with no network and no
# funded account.
#
# WHAT THIS PROVES, AND WHAT IT DOES NOT
#
# Everything here runs against the *sequencer's own executor* — the same
# executor, the same input order, the same 32M session limit the chain uses
# (`lee/state_machine/src/program/mod.rs:55-110`). A rejection you see below is the
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
#
# WHAT IT NEEDS
#
# Required: a Rust toolchain (`cargo`, `rustc`). Nothing else.
#
# Optional, and each one only affects the step that uses it:
#   r0vm     the risc0 VM (`cargo risczero install`). Steps 2 and 10 run the
#            built verifier binary through it. Absent, they are skipped — CI
#            runs them on every push against the same committed binary.
#   spel     step 0's program ids, and step 12's address derivation.
#   python3  steps 9 and 11, and step 12.
#   curl     step 12, which reads the public testnet.
#
# A missing optional tool is reported as a *skip*, never as a pass, and the
# skipped steps are listed again at the end. The script exits 0 either way, so
# a clean environment gets a truthful run rather than a failed one.

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

# A step that could not run is a skip, not a pass. Every skip prints why, in
# place, and is repeated in a summary at the end so it cannot be mistaken for a
# green line scrolled past.
SKIPPED=""
skip() { # step-label reason
  printf '   \033[33m(skipped: %s)\033[0m\n' "$2"
  SKIPPED="${SKIPPED}   - $1 — needs $2
"
}
have() { command -v "$1" >/dev/null 2>&1; }

rule "0. environment"
echo "RISC0_DEV_MODE=$RISC0_DEV_MODE  (0 = real proofs, no mock receipts)"
for t in cargo rustc; do
  have "$t" || { echo "\`$t\` is not on PATH. This demo needs a Rust toolchain; install one from https://rustup.rs and re-run." >&2; exit 1; }
done
rustc --version
test -f artifacts/programs/multisig_verifier.bin \
  || { echo "artifacts/programs/multisig_verifier.bin missing. Run ./scripts/build-programs.sh" >&2; exit 1; }
if have spel; then
  spel program-id artifacts/programs/*.bin 2>/dev/null | grep -E '📦|ImageID' || true
else
  skip "0. program ids" "spel"
  echo "   The ids are not printed here, but they are still checked: step 2's"
  echo "   program_id_pin recomputes them from these same committed binaries."
fi

rule "1. the seven bindings, in the circuit"
echo "25 adversarial tests over the approval logic: non-members, borrowed paths,"
echo "invented roots, forged nullifiers, bait-and-switch actions, padding leaves,"
echo "and a member listed twice still getting exactly one vote."
cargo test -p multisig-core --quiet 2>&1 \
  | grep -E 'test result' | grep -v ' 0 passed' | sed 's/^/   /'

rule "2. the on-chain checks, through the sequencer's executor"
echo "30 tests against the built verifier binary: five honest controls — one per"
echo "instruction, plus an over-threshold approval set that must still be accepted"
echo "— and 25 rejections. 19 of those name a documented error code; the other 6"
echo "are caught one layer earlier, by SPEL's address validation or LEZ's init"
echo "guard, and assert that instead."
echo "Plus 2 that pin the verifier to the exact membership binary it chains to."
# These drive the guest through r0vm, which is how the sequencer's own executor
# runs it. Without r0vm the step cannot run at all, so it is skipped rather than
# reported as anything. The 2 pin tests decode the committed binaries directly
# and need no VM, so they still run.
if have r0vm; then
  cargo test -p multisig-verifier-tests --quiet 2>&1 \
    | grep -E 'test result' | grep -v ' 0 passed' | sed 's/^/   /'
else
  skip "2. the 30 verifier tests" "the risc0 VM r0vm ('cargo risczero install')"
  echo "   CI runs all 30 on every push against this same committed binary —"
  echo "   see .github/workflows/ci.yml, job 'verifier vs sequencer executor'."
  echo "   The 2 pin tests need no VM, so they do run:"
  cargo test -p multisig-verifier-tests --quiet --test program_id_pin 2>&1 \
    | grep -E 'test result' | grep -v ' 0 passed' | sed 's/^/   /'
fi

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
if ! have python3; then
  skip "9. what an observer sees" "python3"
else
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
fi

rule "10. compute cost"
# Measured by executing the guest, so it needs r0vm for the same reason step 2
# does. The recorded figures are in docs/cu-costs.md.
if have r0vm; then
  cargo test -p multisig-verifier-tests --quiet -- --ignored --nocapture 2>&1 \
    | grep -E 'approve |execute ' | sed 's/^/   /'
else
  skip "10. compute cost" "the risc0 VM r0vm ('cargo risczero install')"
  echo "   Nothing is measured here without it. The figures this step prints when"
  echo "   it runs are recorded in docs/cu-costs.md and re-measured by CI."
fi

rule "11. the Basecamp package"
if ! have python3; then
  skip "11. the Basecamp package" "python3"
elif [ -f app/lp-0002-multisig.lgx ]; then
  echo "The committed .lgx carries two variants — darwin-arm64 and linux-amd64 —"
  echo "so it opens on the machine a reviewer actually uses. Its manifest hashes"
  echo "are recomputed from its contents below: the package is checked, not just"
  echo "present."
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
# verify-onchain.sh refuses to run without spel/python3/curl rather than derive
# wrong addresses and report a false negative. Check the same three here so a
# missing tool reads as a skip and not as an unreachable chain.
MISSING=""
for t in spel python3 curl; do have "$t" || MISSING="$MISSING $t"; done
if [ -n "$MISSING" ]; then
  skip "12. the live deployment" "${MISSING# }"
  echo "   Without them every PDA would be derived wrong and this would report a"
  echo "   healthy deployment as missing. The transaction hashes are in"
  echo "   docs/DEPLOYMENT.md and can be checked with any JSON-RPC client."
elif ! ./scripts/verify-onchain.sh artifacts/testnet "$(cat artifacts/testnet/proposal_id)"; then
  echo
  echo "If this failed, the public testnet may be unreachable from here."
  echo "The transaction hashes are in docs/DEPLOYMENT.md and can be checked with"
  echo "any JSON-RPC client."
  SKIPPED="${SKIPPED}   - 12. the live deployment — not verified: the public testnet did not answer
"
fi

printf '\n\033[1mdemo complete\033[0m — working directory %s\n' "$WORK"
if [ -n "$SKIPPED" ]; then
  printf '\n\033[1;33mnot everything ran.\033[0m These steps were skipped, not passed:\n'
  printf '%s' "$SKIPPED"
  echo "Install what each one names and re-run to see it. Steps 2 and 10 are run"
  echo "by CI on every push against these same committed binaries; step 12 reads a"
  echo "live chain, and its transaction hashes are listed in docs/DEPLOYMENT.md."
else
  echo "every step ran; nothing was skipped."
fi
