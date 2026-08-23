#!/usr/bin/env python3
"""Derive a public PDA account id, the way LEZ and SPEL do.

    scripts/pda.py <program.bin|program-id-hex> <seed-hex> [<seed-hex> ...]

Why this exists rather than shelling out to `spel pda`: the scripts need the
address as a bare base58 string on stdout, and this keeps the derivation
visible and auditable next to the code that depends on it.

A seed is 32 bytes of hex, or `str:<text>` for SPEL's `literal("...")` form —
the ASCII bytes zero-padded to 32, exactly as `spel_framework::pda::seed_from_str`
builds them. The treasury PDA uses one:

    scripts/pda.py program.bin <multisig_id> <config_hash> str:treasury

The derivation, byte for byte:

    combined = seed                       if a single seed
             = SHA256(seed0 || seed1 ...) if several
    id       = SHA256("/LEE/v0.2/AccountId/PDA/" padded to 32
                      || program_id as 8 u32 little-endian words
                      || combined)

It is verified against the live chain by `scripts/verify-onchain.sh`, which
reads the accounts these addresses name and reports their owner.
"""

import hashlib
import struct
import subprocess
import sys
from pathlib import Path

B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"

PDA_PREFIX = b"/LEE/v0.2/AccountId/PDA/".ljust(32, b"\x00")


def b58encode(raw: bytes) -> str:
    n = int.from_bytes(raw, "big")
    out = ""
    while n:
        n, r = divmod(n, 58)
        out = B58[r] + out
    return "1" * (len(raw) - len(raw.lstrip(b"\0"))) + out


def b58decode(text: str) -> bytes:
    """32 bytes from a base58 account id, leading zeros included.

    Each leading '1' is a leading zero byte — dropping them would silently
    produce a short id that hashes to something else entirely.
    """
    n = 0
    for ch in text:
        try:
            n = n * 58 + B58.index(ch)
        except ValueError:
            raise SystemExit(f"{ch!r} is not a base58 character") from None
    body = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    raw = b"\x00" * (len(text) - len(text.lstrip("1"))) + body
    if len(raw) != 32:
        raise SystemExit(f"{text} decodes to {len(raw)} bytes, expected 32")
    return raw


def program_id_bytes(spec: str) -> bytes:
    """Accept either a path to a program binary or a comma-separated hex id."""
    path = Path(spec)
    if path.is_file():
        # `spel program-id` prints the hex words; parse those.
        out = subprocess.run(
            ["spel", "program-id", str(path)],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        for line in out.splitlines():
            if "ProgramId (hex)" in line:
                spec = line.split(":", 1)[1].strip()
                break
        else:
            raise SystemExit(f"could not read a ProgramId from {path}")
    words = [int(w, 16) for w in spec.replace(" ", "").split(",")]
    if len(words) != 8:
        raise SystemExit(f"a ProgramId is 8 words, got {len(words)}")
    return b"".join(struct.pack("<I", w) for w in words)


def seed_bytes(spec: str) -> bytes:
    """One 32-byte seed, from hex or from SPEL's `literal("...")` padding."""
    if spec.startswith("str:"):
        raw = spec[4:].encode()
        if len(raw) > 32:
            raise SystemExit(f"literal seed {spec[4:]!r} exceeds 32 bytes")
        return raw.ljust(32, b"\x00")
    raw = bytes.fromhex(spec)
    if len(raw) != 32:
        raise SystemExit(f"a seed is 32 bytes, got {len(raw)} in {spec!r}")
    return raw


def pda(program: bytes, seeds: list[str]) -> str:
    if not seeds:
        raise SystemExit("at least one seed is required")
    parts = [seed_bytes(s) for s in seeds]
    if len(parts) == 1:
        combined = parts[0]
    else:
        h = hashlib.sha256()
        for part in parts:
            h.update(part)
        combined = h.digest()
    return b58encode(hashlib.sha256(PDA_PREFIX + program + combined).digest())


if __name__ == "__main__":
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    print(pda(program_id_bytes(sys.argv[1]), sys.argv[2:]))
