#!/usr/bin/env bash
# Build both LP-0002 guests reproducibly and pin the membership program id.
#
# The verifier hard-codes MEMBERSHIP_LEZ_PROGRAM_ID so that a chained call can
# only ever reach the audited membership binary. That constant and the built
# binary must agree; this script rebuilds both and fails loudly if they drift,
# which is the check that stops a verifier from silently chaining to something
# else after a guest edit.
#
# Requires: docker, cargo-risczero 3.0.5, spel >= 0.6.0.
#
#   ./scripts/build-programs.sh          build both, verify the pin
#   ./scripts/build-programs.sh --check  verify the pin only, no rebuild

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

MEMBERSHIP_MANIFEST=crates/membership-circuit/methods/guest-lez/Cargo.toml
VERIFIER_MANIFEST=crates/multisig-verifier-spel/methods/guest/Cargo.toml
MEMBERSHIP_OUT=crates/membership-circuit/methods/guest-lez/target/riscv32im-risc0-zkvm-elf/docker/membership_lez.bin
VERIFIER_OUT=crates/multisig-verifier-spel/methods/guest/target/riscv32im-risc0-zkvm-elf/docker/multisig_verifier.bin
VERIFIER_SRC=crates/multisig-verifier-spel/methods/guest/src/bin/multisig_verifier.rs

CHECK_ONLY=0
[ "${1:-}" = "--check" ] && CHECK_ONLY=1

# The decimal ProgramId words the verifier source currently pins.
pinned_id() {
  awk '/pub const MEMBERSHIP_LEZ_PROGRAM_ID/,/\];/' "$VERIFIER_SRC" \
    | grep -oE '[0-9]{4,}' | paste -sd, -
}

# The decimal ProgramId of a built binary, per spel.
built_id() {
  spel program-id "$1" | awk -F': *' '/ProgramId \(decimal\)/ {print $2}' | tr -d ' '
}

if [ "$CHECK_ONLY" = 0 ]; then
  echo "[1/4] building the LEZ-native membership guest"
  cargo risczero build --manifest-path "$MEMBERSHIP_MANIFEST"
  mkdir -p artifacts/programs
  cp "$MEMBERSHIP_OUT" artifacts/programs/
fi

echo "[2/4] checking the membership program id pin"
BUILT=$(built_id artifacts/programs/membership_lez.bin)
PINNED=$(pinned_id)
if [ "$BUILT" != "$PINNED" ]; then
  cat >&2 <<EOF
MEMBERSHIP_LEZ_PROGRAM_ID drift.

  pinned in $VERIFIER_SRC:
    $PINNED
  built from $MEMBERSHIP_MANIFEST:
    $BUILT

The verifier would chain to a binary other than the one just built. Update the
constant to the built value and rebuild the verifier.
EOF
  exit 1
fi
echo "      ok, verifier pins the built membership binary"

if [ "$CHECK_ONLY" = 0 ]; then
  echo "[3/4] building the SPEL verifier guest"
  cargo risczero build --manifest-path "$VERIFIER_MANIFEST"
  cp "$VERIFIER_OUT" artifacts/programs/

  echo "[4/4] regenerating the IDL"
  spel generate-idl "$VERIFIER_SRC" > idl/multisig_verifier.idl.json
fi

echo
echo "artifacts:"
for b in artifacts/programs/*.bin; do
  echo "  $b"
  spel program-id "$b" | sed 's/^/    /' | grep -E 'ProgramId \(hex\)|ImageID'
done
