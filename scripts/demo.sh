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
# The last step reads the public testnet straight off the chain, from the hashes
# committed in this repository — so it works from a clean clone with no local
# state. It reports the *pending* state honestly: the membership program on chain
# is the binary committed here, and the verifier committed here has not been
# deployed yet. See docs/DEPLOYMENT.md.
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
ROT_ID=0000000000000000000000000000000000000000000000000000000000000002
MEMO="transfer 250 LEZ to the grants treasury"
# A payee, and an amount. A proposal that moves nothing would give the threshold
# nothing to gate, which is why the program refuses one.
RECIPIENT=5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e
AMOUNT=250

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

rule "1. the nine bindings, in the circuit — and the account layouts"
echo "25 adversarial tests over the approval logic: non-members, borrowed paths,"
echo "invented roots, forged nullifiers, bait-and-switch actions, padding leaves,"
echo "and a member listed twice still getting exactly one vote."
echo "Plus 9 over the published account layouts: exact lengths, refused formats,"
echo "refused trailing bytes, and a round trip through the documented offsets."
cargo test -p multisig-core --quiet 2>&1 \
  | grep -E 'test result' | grep -v ' 0 passed' | sed 's/^/   /'

rule "2. the on-chain checks, through the sequencer's executor"
echo "Tests against the built verifier binary, in three groups:"
echo "  · what cannot be forced — 25 rejections and five honest controls."
echo "  · what the threshold DOES — the treasury falls by the proposed amount"
echo "    and the recipient rises by it, read out of the guest's own journal,"
echo "    which is the state the sequencer would write."
echo "  · what a stranger can read afterwards — every account decoded from the"
echo "    byte offsets docs/account-layout.md publishes, by a decoder that has"
echo "    never heard of borsh."
echo "Plus 2 that pin the verifier to the exact membership binary it chains to,"
echo "and 5 that hold the IDL and the error-code table to the guest's source."
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

rule "4. a proposal to pay 250 out of the treasury, bound to its exact action"
$M propose --dir "$WORK" --proposal-id "$PROP_ID" \
  --recipient "$RECIPIENT" --amount "$AMOUNT" --memo "$MEMO"

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
echo "Same proposal id, same memo — a different payee and a larger amount."
echo "That is a different proposal_ref, so the approvals above do not carry."
if $M propose --dir "$WORK" --proposal-id "$PROP_ID" \
     --recipient "6666666666666666666666666666666666666666666666666666666666666666" \
     --amount 1000000 --memo "$MEMO" 2>&1 | sed 's/^/   /'; then
  echo "   UNEXPECTED: the action was swapped" >&2; exit 1
fi

rule "8. threshold reached, execution arguments emitted"
$M status --dir "$WORK" --proposal-id "$PROP_ID"
echo
$M execute-args --dir "$WORK" --proposal-id "$PROP_ID" --out "$WORK/exec.args"

rule "9. a spending tier: the same members, a cheaper small transfer"
echo "Below the cap two approvals suffice where the default asks three. A tier may"
echo "only ever LOWER the bar: caps must strictly increase, thresholds must not"
echo "fall, and none may be zero or above the default — so no legal table makes a"
echo "larger transfer easier than a smaller one."
$M new-multisig --members 5 --threshold 3 --tier 300:2 --id "$MSIG_ID" --out "$WORK/tiered" \
  | sed 's/^/   /'
$M propose --dir "$WORK/tiered" --proposal-id "$PROP_ID" \
  --recipient "$RECIPIENT" --amount "$AMOUNT" --memo "$MEMO" >/dev/null
for i in 0 3; do
  $M approve-args --dir "$WORK/tiered" --proposal-id "$PROP_ID" --member "$i" \
    --out "$WORK/tiered/a$i.args" >/dev/null
done
echo
echo "Two approvals, and the arguments are emitted — the tier priced it:"
$M execute-args --dir "$WORK/tiered" --proposal-id "$PROP_ID" --out "$WORK/tiered/exec.args" \
  | sed 's/^/   /'
echo
echo "A table that would RAISE the bar is refused before any proving happens:"
if $M new-multisig --members 5 --threshold 3 --tier 300:4 --out "$WORK/badtier" 2>&1 \
   | sed 's/^/   /'; then
  echo "   UNEXPECTED: a tier above the default threshold was accepted" >&2; exit 1
fi

rule "10. rotating the member set, without rewriting it"
echo "A rotation anchors a SECOND configuration at its own address, with its own"
echo "treasury, and marks the first superseded. Every guarantee the address gives"
echo "the old configuration, the new one has by the same construction — and"
echo "proposals raised under the old one live at addresses the new one never reads."
$M new-multisig --members 5 --threshold 4 --id "$MSIG_ID" --out "$WORK/next" | sed 's/^/   /'
echo
$M propose-rotation --dir "$WORK" --proposal-id "$ROT_ID" --to "$WORK/next" | sed 's/^/   /'
for i in 0 3 4; do
  $M approve-args --dir "$WORK" --proposal-id "$ROT_ID" --member "$i" \
    --out "$WORK/r$i.args" >/dev/null
done
echo
echo "Three approvals — the DEFAULT threshold, never a tier. Pricing governance by"
echo "tier would make the cheapest action available the one that rewrites who may act."
$M rotate-args --dir "$WORK" --proposal-id "$ROT_ID" --to "$WORK/next" --out "$WORK/rot.args" \
  | sed 's/^/   /'
echo
echo "And the two action shapes cannot be spent by each other's instruction:"
if $M execute-args --dir "$WORK" --proposal-id "$ROT_ID" --out "$WORK/wrong.args" 2>&1 \
   | sed 's/^/   /'; then
  echo "   UNEXPECTED: a rotation was spendable by execute" >&2; exit 1
fi

rule "11. what an observer sees"
if ! have python3; then
  skip "11. what an observer sees" "python3"
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

rule "12. compute cost"
# Measured by executing the guest, so it needs r0vm for the same reason step 2
# does. The recorded figures are in docs/cu-costs.md.
if have r0vm; then
  cargo test -p multisig-verifier-tests --quiet -- --ignored --nocapture 2>&1 \
    | grep -E 'approve |execute ' | sed 's/^/   /'
else
  skip "12. compute cost" "the risc0 VM r0vm ('cargo risczero install')"
  echo "   Nothing is measured here without it. The figures this step prints when"
  echo "   it runs are recorded in docs/cu-costs.md and re-measured by CI."
fi

rule "13. the Basecamp package"
if ! have python3; then
  skip "13. the Basecamp package" "python3"
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

rule "14. the chain, read straight off the chain"
echo "The multisig above is local. This checks what is actually on the public"
echo "testnet, from hashes committed in this repository — no local state needed."
echo
# No narration about what the check is *going to* say. This paragraph used to
# announce "it will report the verifier as NOT YET DEPLOYED" — true when it was
# written, and false from the redeploy onwards, so a reviewer running the demo
# from a clean clone read a prediction of failure and then watched every line
# come back ok. A script that tells you what to expect has to be re-read every
# time the thing it describes changes, and nothing was re-reading it. What the
# check finds is printed by the check.
echo "Whatever it reports is what the chain says right now; if a program is not"
echo "there, docs/DEPLOYMENT.md has the redeploy checklist."
echo
# verify-onchain.sh refuses to run without spel/python3/curl rather than derive
# wrong addresses and report a false negative. Check the same three here so a
# missing tool reads as a skip and not as an unreachable chain.
MISSING=""
for t in jq python3 curl; do have "$t" || MISSING="$MISSING $t"; done
if [ -n "$MISSING" ]; then
  skip "12. the chain" "${MISSING# }"
  echo "   The transaction hashes are in docs/DEPLOYMENT.md and can be checked"
  echo "   with any JSON-RPC client."
elif ! ./scripts/verify-onchain-lifecycle.sh; then
  echo
  echo "If this failed, the public testnet may be unreachable from here."
  echo "The transaction hashes are in docs/DEPLOYMENT.md and can be checked with"
  echo "any JSON-RPC client."
  SKIPPED="${SKIPPED}   - 12. the chain — not verified: the public testnet did not answer
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
