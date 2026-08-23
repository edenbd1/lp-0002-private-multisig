#!/usr/bin/env python3
"""Every sha256 quoted next to a file path must be that file's.

A published digest beside a path is a checkable claim, and this repository
published a wrong one: `app/README.md` named a package hash that had been
correct three commits earlier, before the package was rebuilt. A reviewer who
recomputes it finds a submission contradicting itself before reading any code.

Run from the repository root. Exits non-zero and names every mismatch.
"""
import hashlib
import pathlib
import re
import sys

SKIP = {"vendor", "_external", "target", "node_modules", ".git"}
# `path/to/thing.lgx` … `<64 hex>` — the path and the digest on the same line,
# in either order, which is how both of this repository's docs write them.
PAT = [
    re.compile(r"`([\w./-]+\.(?:lgx|bin|so|dylib))`[^`\n]{0,100}?`([0-9a-f]{64})`"),
    re.compile(r"`([0-9a-f]{64})`[^`\n]{0,100}?`([\w./-]+\.(?:lgx|bin|so|dylib))`"),
]


def main() -> int:
    root = pathlib.Path(".")
    bad = checked = 0
    for md in sorted(root.rglob("*.md")):
        if SKIP & set(md.parts):
            continue
        text = md.read_text(errors="replace")
        for i, pat in enumerate(PAT):
            for m in pat.finditer(text):
                path, want = (m.group(1), m.group(2)) if i == 0 else (m.group(2), m.group(1))
                f = root / path
                if not f.is_file():
                    continue          # a path this checkout does not carry
                checked += 1
                real = hashlib.sha256(f.read_bytes()).hexdigest()
                if real != want:
                    line = text[: m.start()].count("\n") + 1
                    print("  %s:%d says %s… for %s, which is %s…"
                          % (md, line, want[:16], path, real[:16]))
                    bad += 1
    if bad:
        print("  %d quoted digest(s) do not match the file beside them." % bad)
        return 1
    print("  %d quoted digest(s), all matching" % checked)
    return 0


if __name__ == "__main__":
    sys.exit(main())
