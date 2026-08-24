#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Nothing in this tree points at another submission.

    ./scripts/check-no-cross-submission.py

WHY THIS EXISTS

This repository is one submission. A reviewer reading it should find the thing
it is about and nothing else — not the identifier of a different prize entry,
not a competing bid, not a note about how some other submission was received.
That is a judgement about what belongs in a deliverable, and it is not the kind
of thing anyone notices while writing code.

It has already been wrong here. `scripts/check-transcript.py` shipped an anchor
table with an entry for three different films, two of them belonging to other
submissions, and their filenames sat in the source for anyone who opened the
script. Nothing flagged it, because the only check that looks for this lived in
one repository out of three and had never been run.

WHAT IT LOOKS FOR

Identifiers of other prize entries, and the vocabulary of comparison. The
patterns are deliberately blunt: a false positive costs one line of reading, and
a miss costs a reviewer finding a competitor's name in a file that is supposed
to be about this work.

WHAT IT DOES NOT DO

Judge intent. `LP-0019` in a sentence about future work is caught here and has
to be looked at, because a script cannot tell a roadmap from a leak. When a
mention is deliberate, say so in EXPECTED below with the reason, so the next
person reads the reason instead of re-deciding.
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# This repository's own prize. Everything else in the LP- space is somebody's
# else's entry, or another one of these, and neither belongs here.
OWN = os.environ.get("OWN_PRIZE") or ""
PATTERNS = [
    r"competing submission", r"another submission", r"other submission",
    r"rival (bid|submission)", r"was closed with", r"reviewer (said|rejected)",
]
# Deliberate mentions, each with the reason it is not a leak. An entry that
# stops matching anything is itself reported: a suppression nobody needs is a
# suppression that has outlived what it was hiding.
EXPECTED = {}


def main():
    if not OWN:
        print("OWN_PRIZE is not set, so this cannot tell this repository's own prize\n"
              "identifier from another one. Set it in the workflow and in the script's\n"
              "invocation — guessing it from the directory name is how this check would\n"
              "quietly start allowing the thing it exists to forbid.")
        return 1

    others = [p for p in ("LP-0002", "LP-0003", "LP-0005", "LP-0008", "LP-0009",
                          "LP-0010", "LP-0012", "LP-0013", "LP-0016", "LP-0017")
              if p.lower() != OWN.lower()]
    pattern = "|".join(PATTERNS + [re.escape(p) for p in others]
                       + [re.escape(p.lower()) for p in others])

    out = subprocess.run(["git", "grep", "-nIE", pattern, "--", "."],
                         cwd=ROOT, capture_output=True, text=True)
    if out.returncode not in (0, 1):
        print("git grep failed: " + (out.stderr or "").strip())
        return 1

    hits = [l for l in out.stdout.splitlines()
            if os.path.basename(__file__) not in l
            and not any(e in l for e in EXPECTED)]

    unused = [e for e in EXPECTED if e not in out.stdout]
    scanned = subprocess.run(["git", "ls-files"], cwd=ROOT,
                             capture_output=True, text=True).stdout.split()
    print("scanned %d tracked file(s) for %d other prize identifier(s) and %d "
          "comparison phrase(s)" % (len(scanned), len(others), len(PATTERNS)))

    if unused:
        print("\n%d allowance(s) match nothing any more:" % len(unused))
        for e in unused:
            print("  " + e)
        print("An allowance that suppresses nothing is one nobody re-reads. Remove it.")
        return 1
    if hits:
        print("\n%d line(s) point at another submission:\n" % len(hits))
        for h in hits[:20]:
            print("  " + h[:150])
        if len(hits) > 20:
            print("  … and %d more" % (len(hits) - 20))
        print("\nEach is either a leak to delete or a deliberate mention to record in\n"
              "EXPECTED with its reason. Deciding once and writing it down is the point.")
        return 1
    print("nothing in this tree points at another submission")
    return 0


if __name__ == "__main__":
    sys.exit(main())
