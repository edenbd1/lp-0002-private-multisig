#!/usr/bin/env bash
# Run exactly what CI runs, before pushing.
#
# WHY THIS EXISTS
#
# On 2026-08-03 three pushes in four minutes each broke CI and each sent a
# failure email. Every one was avoidable: the checks had been run, but as
# *approximations* of the CI commands rather than the commands themselves —
# `cargo clippy` with the output grepped for lines starting with "error", where
# CI runs `--all-targets -- -D warnings` and a lint is an error; tests without
# `cargo fmt --check`, which is the first step of the job.
#
# The lesson is not "be careful". It is that a check which differs from the
# gate does not test the gate. So this file is generated from ci.yml's steps
# and nothing else: if the two drift, that is a bug in this script.
#
#   ./scripts/preflight.sh          # before every push
#
# It does not run the e2e-vs-local-sequencer workflow, which needs a LEZ
# checkout and generates a real proof — use ./scripts/e2e-local-sequencer.sh
# for that one, and note that it is the workflow no local check can stand in
# for.

set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

fail=0
step() {
  printf '\n\033[1m%s\033[0m\n' "$1"; shift
  if "$@"; then
    printf '  \033[32mok\033[0m\n'
  else
    printf '  \033[31mFAILED\033[0m — this is what CI will say\n'
    fail=1
  fi
}

step "Format        cargo fmt --all -- --check"   cargo fmt --all -- --check
step "Clippy        cargo clippy --workspace --all-targets -- -D warnings" \
     cargo clippy --workspace --all-targets -- -D warnings
step "Build         cargo build --workspace"      cargo build --workspace --quiet
step "Tests         cargo test --workspace"       cargo test --workspace --quiet

# The workflow files are only exercised on the runner, so at least catch the
# syntax here rather than in an email.
step "Hashes        every quoted sha256 is that file's" \
     python3 scripts/check-quoted-hashes.py

step "IDL           carries the error codes the guest declares" \
     python3 scripts/idl-errors.py --check

step "Workflows     yaml parse" python3 - <<'PY'
import glob, sys
try:
    import yaml
except ImportError:
    print("  (pyyaml not installed, skipped)"); sys.exit(0)
for f in glob.glob(".github/workflows/*.yml"):
    yaml.safe_load(open(f))
PY

echo
if [ "$fail" -eq 0 ]; then
  printf '\033[1mpreflight: clean — safe to push\033[0m\n'
else
  printf '\033[1;31mpreflight: do not push\033[0m\n' >&2
fi
exit "$fail"
