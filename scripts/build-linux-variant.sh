#!/usr/bin/env bash
# Build the Linux variant of the Basecamp module, in Docker.
#
# WHY
#
# The prize asks for a Basecamp module that is loadable, and review happens on
# Linux. A `.lgx` carrying only `darwin-arm64` is a package the reviewer cannot
# open at all, which makes the Basecamp deliverable worth nothing to the person
# assessing it.
#
# So this produces the `linux-amd64` half: the Qt plugin as an ELF shared
# object, and the `msig` CLI the plugin's bridge shells out to. Both are built
# inside `linux/amd64` containers, so the result does not depend on anything
# about the host beyond Docker.
#
#   ./scripts/build-linux-variant.sh
#   python3 scripts/package-lgx.py --add-variant linux-amd64:.linux-variant
#
# Set ARCH=arm64 for `linux-arm64` instead. Everything below is arch-agnostic;
# only the platform flag and the variant directory name change.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

ARCH="${ARCH:-amd64}"
PLATFORM="linux/${ARCH}"
OUT="$ROOT/.linux-variant"

command -v docker >/dev/null || { echo "docker not found" >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo "docker is not running" >&2; exit 1; }

rm -rf "$OUT"; mkdir -p "$OUT"

echo "[1/3] image with Qt6 (${PLATFORM})"
docker build --platform "$PLATFORM" -t "lp0002-linux-build-${ARCH}" - <<'DOCKERFILE' >/dev/null
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential cmake qt6-base-dev qt6-declarative-dev \
      libgl1-mesa-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
DOCKERFILE

echo "[2/3] Qt plugin"
docker run --rm --platform "$PLATFORM" -v "$ROOT/app":/src -w /src \
  "lp0002-linux-build-${ARCH}" \
  sh -c 'rm -rf build-linux && cmake -B build-linux -S . >/dev/null && cmake --build build-linux -j"$(nproc)" >/dev/null'
cp "$ROOT/app/build-linux/lp_0002_multisig.so" "$OUT/"

echo "[3/3] msig CLI"
# The plugin's bridge shells out to msig, so the variant is only usable if the
# CLI ships with it — a macOS binary next to a Linux plugin would load and then
# fail on the first button press.
docker run --rm --platform "$PLATFORM" -v "$ROOT":/w -w /w rust:1.90-slim-bookworm \
  sh -c 'apt-get update -qq && apt-get install -y -qq pkg-config libssl-dev >/dev/null 2>&1
         cargo build --release -p multisig-cli --target-dir /tmp/lt >/dev/null 2>&1
         cp /tmp/lt/release/msig /w/.linux-variant/msig'
chmod +x "$OUT/msig"

echo
echo "wrote $OUT:"
for f in "$OUT"/*; do printf "  %-28s %s\n" "$(basename "$f")" "$(file -b "$f" | cut -c1-60)"; done
echo
echo "next:  python3 scripts/package-lgx.py --add-variant linux-${ARCH}:.linux-variant"
