#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Every test count README.md states is what that suite actually passes.

    ./scripts/check-test-count.py

WHY THIS EXISTS

The README states three counts, and all three are quoted outward as the
shorthand for how much of this is checked rather than asserted. Nothing compared
them to the suites. A suite that grows does not fail — it runs more tests than
the sentence about it admits, and CI stays green through the whole drift.

The counts are what `cargo test` reports passing, not the number of `#[test]`
attributes in the tree. The two agree on two of these three claims and
DISAGREE on the third by design: `crates/multisig-verifier-tests` holds 60 test
functions and the README says 59, because one of them is `#[ignore]`d — it
reports a measurement rather than asserting a property, and the workflow runs it
separately with `--ignored`. Counting attributes would call the README wrong
about a number that is right. What cargo prints is what a reader reproduces.

Each claim names its own suite, because "the 25 circuit tests" is not a package
total: it is `tests/approve_adversarial.rs` inside `multisig-core`, whose whole
suite is 34. A gate comparing both to one number would have to be wrong about
one of them.

A SUITE THAT FAILS TO BUILD PASSES ZERO TESTS, and zero against a sentence is a
confident mismatch about entirely the wrong thing, so a non-zero cargo exit is
surfaced as itself before any comparison.

It also checks the one other count the README states about this repository: how
many steps `scripts/preflight.sh` runs. That sentence has already been wrong
once — it said "the same four commands" while the script ran seven — and the
number moves every time a step is added, which is precisely when nobody is
looking at the README.
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RUNNING = re.compile(r"^\s*Running (?:unittests )?(\S+)", re.M)
RESULT = re.compile(r"^test result: ok\. (\d+) passed", re.M)

# (what the README says, which package, which suite — None for the whole package)
#
# Anchored on the words around each number, not on the number itself: a README
# that drops a claim fails here rather than passing because there is nothing
# left to compare.
CLAIMS = [
    (r"\bthe (\d+) circuit tests\b", "multisig-core", "approve_adversarial"),
    (r"\|\s*`crates/multisig-core`\s*\|[^|]*?\b(\d+) tests\b", "multisig-core", None),
    (r"\|\s*`crates/multisig-verifier-tests`\s*\|\s*(\d+) tests\b",
     "multisig-verifier-tests", None),
    (r"\bthe (\d+) tests against the built verifier binary\b",
     "multisig-verifier-tests", None),
]
_ran = {}


def suites(pkg):
    """Per-suite passing counts for a package, memoised. Suites keyed by the
    binary's source file stem, which is what `Running tests/foo.rs` names."""
    if pkg not in _ran:
        r = subprocess.run(["cargo", "test", "-p", pkg, "--release"],
                           cwd=ROOT, capture_output=True, text=True)
        out = r.stdout + r.stderr
        if r.returncode != 0:
            _ran[pkg] = (None, out)
        else:
            names = [os.path.splitext(os.path.basename(m))[0].split(" ")[0]
                     for m in RUNNING.findall(out)]
            counts = [int(n) for n in RESULT.findall(out)]
            _ran[pkg] = (dict(zip(names, counts)) if names else {}, out)
    return _ran[pkg]


def main():
    with open(os.path.join(ROOT, "README.md"), encoding="utf-8") as fh:
        text = re.sub(r"\s+", " ", fh.read())

    failures = []
    checked = 0
    for pattern, pkg, suite in CLAIMS:
        found = re.findall(pattern, text)
        if not found:
            failures.append("README.md no longer states the claim matched by %r. That "
                            "sentence is what this gate defends; losing it silently is "
                            "a check going away, not a tree getting cleaner." % pattern)
            continue
        if len(set(found)) > 1:
            failures.append("the claim %r appears with different numbers (%s); a reader "
                            "cannot tell which is meant" % (pattern, ", ".join(sorted(set(found)))))
            continue
        claimed = int(found[0])
        per, out = suites(pkg)
        if per is None:
            print("`cargo test -p %s --release` did not build. A suite that does not\n"
                  "build passes zero tests, and zero against a README is a mismatch\n"
                  "about the wrong thing. The build is the failure:\n" % pkg)
            print("\n".join(out.strip().splitlines()[-12:]))
            return 1
        if suite is None:
            actual, what = sum(per.values()), "the whole %s suite" % pkg
        elif suite not in per:
            failures.append("suite %r was not run by `cargo test -p %s` — it was "
                            "renamed or removed, so the claim of %d cannot be checked "
                            "at all" % (suite, pkg, claimed))
            continue
        else:
            actual, what = per[suite], "%s/tests/%s.rs" % (pkg, suite)
        checked += 1
        mark = "ok  " if actual == claimed else "FAIL"
        print("  %s README says %-3d for %s; it passes %d" % (mark, claimed, what, actual))
        if actual != claimed:
            failures.append("README.md says %d where %s passes %d. The README is what a "
                            "reader takes for how much is checked, and it is quoted "
                            "outward, so it is the one that moves." % (claimed, what, actual))

    # The README says how many steps preflight.sh runs. Counted from the script,
    # not from the sentence: `step "` is how each one is declared.
    with open(os.path.join(ROOT, "scripts", "preflight.sh"), encoding="utf-8") as fh:
        steps = fh.read().count('step "')
    words = {"four": 4, "five": 5, "six": 6, "seven": 7, "eight": 8, "nine": 9,
             "ten": 10, "eleven": 11, "twelve": 12}
    m = re.search(r"\b(%s)\s+steps\b" % "|".join(words), text, re.I)
    if not m:
        failures.append("README.md no longer says how many steps preflight.sh runs. "
                        "That sentence has been wrong before; losing it is not the "
                        "same as fixing it.")
    else:
        said = words[m.group(1).lower()]
        checked += 1
        print("  %s README says %s (%d) steps in preflight.sh; it declares %d"
              % ("ok  " if said == steps else "FAIL", m.group(1), said, steps))
        if said != steps:
            failures.append("README.md says preflight.sh runs %d steps; it declares %d."
                            % (said, steps))

    print("checked %d claim(s): %d test count(s) and the preflight step count"
          % (checked, len(CLAIMS)))
    # The README also says the demo runs "a full 3-of-5 lifecycle" and reports a
    # compute cost. Neither is a count, and neither is checked here; they are
    # gated by the demo script's own exits.
    if failures:
        print("\n%d claim(s) the suites do not support:\n" % len(failures))
        for f in failures:
            print("  " + f)
        return 1
    print("every stated count is what its suite passes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
