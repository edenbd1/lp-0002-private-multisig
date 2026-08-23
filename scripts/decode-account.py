#!/usr/bin/env python3
"""Read one LP-0002 account off the chain and decode it, field by field.

    scripts/decode-account.py <kind> <address>       # fetch, then decode
    scripts/decode-account.py <kind> --stdin         # decode a getAccount reply

    kind: multisig | treasury | proposal | approval | execution

WHY THIS EXISTS

The claim this program makes is that a stranger, holding nothing but an address,
can read what the multisig is and what it did. This script is that claim, in
executable form — and it is deliberately written the long way: it parses by byte
offset out of `docs/account-layout.md`, with no dependency on this repository's
Rust, on borsh, or on an IDL. If it can read the chain, so can anyone.

`treasury` and `approval` are both 65 bytes, so the kind is a required argument
rather than guessed: a decoder that guesses will one day guess wrong and print a
confident answer.

Env: SEQUENCER_URL (default https://testnet.lez.logos.co).
"""
import json
import os
import subprocess
import sys

RPC = os.environ.get("SEQUENCER_URL", "https://testnet.lez.logos.co")

STATE_FORMAT_V1 = 1
STATUS = {0: "Open", 1: "Executed"}

# (name, length, renderer). `None` length means "the rest, in 32-byte items".
LAYOUTS = {
    "multisig": [
        ("format", 1, "u8"),
        ("multisig_id", 32, "hex"),
        ("member_root", 32, "hex"),
        ("threshold", 4, "u32"),
        ("treasury", 32, "hex"),
        ("authority", 32, "hex"),
    ],
    "treasury": [
        ("format", 1, "u8"),
        ("multisig_id", 32, "hex"),
        ("config_hash", 32, "hex"),
    ],
    "proposal": [
        ("format", 1, "u8"),
        ("multisig_id", 32, "hex"),
        ("config_hash", 32, "hex"),
        ("proposal_id", 32, "hex"),
        ("action_hash", 32, "hex"),
        ("recipient", 32, "hex"),
        ("amount", 16, "u128"),
        ("memo_hash", 32, "hex"),
        ("status", 1, "status"),
    ],
    "approval": [
        ("format", 1, "u8"),
        ("proposal_ref", 32, "hex"),
        ("nullifier", 32, "hex"),
    ],
    "execution": [
        ("format", 1, "u8"),
        ("proposal_ref", 32, "hex"),
        ("recipient", 32, "hex"),
        ("amount", 16, "u128"),
        ("status", 1, "status"),
        ("nullifier_count", 4, "u32"),
        ("nullifiers", None, "list32"),
    ],
}


def render(kind: str, raw: bytes, value: bytes, at: int) -> str:
    if kind == "u8":
        return str(value[0])
    if kind == "u32":
        return str(int.from_bytes(value, "little"))
    if kind == "u128":
        return str(int.from_bytes(value, "little"))
    if kind == "status":
        return STATUS.get(value[0], f"UNKNOWN({value[0]})")
    if kind == "list32":
        rest = raw[at:]
        if len(rest) % 32:
            raise SystemExit(f"trailing {len(rest) % 32} byte(s) after the nullifier list")
        return "\n".join(
            f"      [{i}] {rest[i * 32:(i + 1) * 32].hex()}" for i in range(len(rest) // 32)
        )
    return value.hex()


def decode(kind: str, raw: bytes) -> int:
    layout = LAYOUTS[kind]
    fixed = sum(n for _, n, _ in layout if n is not None)
    if len(raw) < fixed:
        print(f"  account holds {len(raw)} byte(s); a {kind} record needs {fixed}")
        if not raw:
            print("  an empty account is one nobody has written to")
        return 1
    if raw[0] != STATE_FORMAT_V1:
        print(f"  format byte is {raw[0]}, not {STATE_FORMAT_V1}: this decoder does not know it")
        return 1

    at = 0
    declared = None
    for name, length, how in layout:
        if length is None:
            print(f"  {name:<16} @{at}")
            print(render(how, raw, b"", at))
            at = len(raw)
            break
        value = raw[at : at + length]
        print(f"  {name:<16} @{at:<4} {render(how, raw, value, at)}")
        if name == "nullifier_count":
            declared = int.from_bytes(value, "little")
        at += length

    if declared is not None:
        actual = (len(raw) - fixed) // 32
        if declared != actual:
            print(f"  MISMATCH: header declares {declared} nullifier(s), {actual} follow")
            return 1
    elif at != len(raw):
        print(f"  MISMATCH: {len(raw) - at} trailing byte(s) after a fixed-width record")
        return 1
    return 0


def fetch(address: str) -> dict:
    body = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": "getAccount", "params": [address]}
    )
    out = subprocess.run(
        ["curl", "-s", "-m", "20", "-X", "POST", RPC, "-H", "Content-Type: application/json",
         "-d", body],
        capture_output=True, text=True, check=False,
    )
    if out.returncode != 0 or not out.stdout.strip():
        raise SystemExit(f"no answer from {RPC}")
    return json.loads(out.stdout)


def main() -> int:
    args = sys.argv[1:]
    if len(args) != 2 or args[0] not in LAYOUTS:
        raise SystemExit(__doc__)
    kind, where = args
    reply = json.load(sys.stdin) if where == "--stdin" else fetch(where)
    result = reply.get("result")
    if not result:
        print("  no such account on this chain")
        return 1
    print(f"  program_owner    {result.get('program_owner')}")
    print(f"  balance          {result.get('balance')}")
    return decode(kind, bytes(result.get("data") or []))


if __name__ == "__main__":
    sys.exit(main())
