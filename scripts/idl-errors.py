#!/usr/bin/env python3
"""Put the program's error codes into its IDL, and keep them there.

WHY THIS EXISTS

`spel generate-idl` emits `errors: vec![]` unconditionally
(`vendor/spel/spel-framework-core/src/idl_gen.rs:228` and
`spel-cli/src/generate_idl.rs`), whatever the program declares. So a client
generated from this IDL knows every instruction, every account and every
argument, and nothing at all about the twenty-two rejections it will actually
meet. The codes exist, are stable, and are documented for humans — they were
simply missing from the machine-readable artefact.

The source of truth is the guest itself. This script reads the `const E_*: u32 =
5NNN;` block out of the verifier source together with its `///` doc comments, so
the IDL cannot drift from the program: there is no second list to update.

    scripts/idl-errors.py            # rewrite idl/…json with the errors merged in
    scripts/idl-errors.py --check    # exit non-zero if it would change anything

`--check` is what `scripts/preflight.sh` runs. It is the difference between "we
regenerate the IDL" and "the IDL is regenerated", which is exactly the kind of
gap that survives a review.
"""
import json
import pathlib
import re
import sys

SOURCE = pathlib.Path("crates/multisig-verifier-spel/methods/guest/src/bin/multisig_verifier.rs")
IDL = pathlib.Path("idl/multisig_verifier.idl.json")

# A run of `///` lines immediately followed by `const E_NAME: u32 = 1234;`.
DECL = re.compile(
    r"((?:^[ \t]*///[^\n]*\n)+)^[ \t]*const[ \t]+(E_[A-Z0-9_]+)[ \t]*:[ \t]*u32[ \t]*=[ \t]*(\d+)[ \t]*;",
    re.MULTILINE,
)


def doc_to_sentence(block: str) -> str:
    """Join a `///` block into one line, the way a tooltip would show it."""
    lines = [ln.strip().removeprefix("///").strip() for ln in block.strip().splitlines()]
    return " ".join(part for part in lines if part)


def collect(source: pathlib.Path) -> list[dict]:
    text = source.read_text()
    errors = []
    seen: dict[int, str] = {}
    for doc, name, code in DECL.findall(text):
        code_i = int(code)
        if code_i in seen:
            raise SystemExit(
                f"error code {code_i} is declared twice, as {seen[code_i]} and {name}. "
                "Codes are a public interface; two meanings for one code is a bug."
            )
        seen[code_i] = name
        errors.append({"code": code_i, "name": name, "msg": doc_to_sentence(doc)})
    if not errors:
        raise SystemExit(f"found no `const E_*: u32` declarations in {source}")
    errors.sort(key=lambda e: e["code"])
    return errors


def main() -> int:
    check_only = "--check" in sys.argv[1:]
    root = pathlib.Path(__file__).resolve().parent.parent
    source, idl_path = root / SOURCE, root / IDL

    errors = collect(source)
    idl = json.loads(idl_path.read_text())

    # Key order matters only for the diff being readable; place `errors` after
    # `types` when it exists, and at the end otherwise, which is where the SPEL
    # struct declares it.
    merged = dict(idl)
    merged["errors"] = errors
    rendered = json.dumps(merged, indent=2) + "\n"

    if rendered == idl_path.read_text():
        print(f"  {len(errors)} error code(s) in {IDL}, up to date")
        return 0
    if check_only:
        have = {e["code"] for e in idl.get("errors", [])}
        want = {e["code"] for e in errors}
        print(f"  {IDL} does not carry the error codes {source} declares.")
        if want - have:
            print("    missing: " + ", ".join(str(c) for c in sorted(want - have)))
        if have - want:
            print("    stale:   " + ", ".join(str(c) for c in sorted(have - want)))
        if have == want:
            print("    the codes match but a message or the ordering differs.")
        print("    Run: scripts/idl-errors.py")
        return 1
    idl_path.write_text(rendered)
    print(f"  merged {len(errors)} error code(s) into {IDL}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
