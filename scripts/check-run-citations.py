#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Every CI run this repository cites must have run on a commit this branch has.

WHY THIS EXISTS

Several criteria here rest on a workflow having gone green — the end-to-end run
against a standalone LEZ sequencer with `RISC0_DEV_MODE=0` above all. Nobody
re-runs that: it needs a whole logos-execution-zone build and about an hour of
proving. What a reader does instead is follow the link. So the link IS the
deliverable, and a link that lands on a commit this repository does not contain
is worse than no link at all — it reads as a history rewritten to hide
something.

WHAT IT CHECKS

For every `actions/runs/<id>` in the documents: resolve the run's head commit
through the GitHub API, require it to be `success` unless the surrounding
sentences say plainly that it is not, and require its commit to be an ancestor
of the current branch. Run ids written as bare numbers are resolved and
*reported* rather than enforced — a number in a table is a claim that a run
happened, not something a reader clicks.

WHY THE DOCUMENT SCAN IS RECURSIVE

The first version of this script listed `docs/` with `os.listdir`, which does
not descend. In this repository that skipped `docs/benchmarks/` and
`docs/decisions/` — and `docs/benchmarks/cu-budget.md` is the only file here
that cites a CI run at all. A gate that reads none of the citations reports
"no citation is broken", which is true and worthless. It walks now.

NO SKIP PATH. If `gh` is missing or unauthenticated this exits non-zero and says
so. A citation checker that quietly passes when it cannot check is the failure it
was written to prevent, one level up.
"""
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RUN_RE = re.compile(r"actions/runs/(\d+)")
# GitHub run ids are 11 digits today. Anchored on word boundaries so block
# heights, byte counts and hashes cannot be mistaken for one.
BARE_RE = re.compile(r"\b(\d{11})\b")
CONTEXT_LINES = 6
# A cited run that is not green, and is not SAID to be not green, is the worst
# citation in the set: a reader clicks it and sees a cross under a sentence
# claiming a pass. These are the phrasings that count as saying so out loud.
FAILURE_IS_THE_POINT = (
    "is not a pass", "killed at the", "ran past its", "cancelled rather than",
    "at the cap", "is red", "that run is red", "did not finish",
)


def documents():
    """README, any top-level markdown, and everything under docs/ — recursively.

    Vendored trees are excluded: `vendor/` carries somebody else's documents,
    and their citations are not claims this repository is making.
    """
    found = []
    for name in sorted(os.listdir(ROOT)):
        if name.endswith(".md") and os.path.isfile(os.path.join(ROOT, name)):
            found.append(name)
    docs_dir = os.path.join(ROOT, "docs")
    for dirpath, dirnames, filenames in os.walk(docs_dir):
        dirnames[:] = [d for d in sorted(dirnames) if d != "vendor"]
        for f in sorted(filenames):
            if f.endswith(".md"):
                found.append(os.path.relpath(os.path.join(dirpath, f), ROOT))
    return found


def sh(*args):
    """Run a command, and treat "it is not installed" as a failed run.

    subprocess.run RAISES FileNotFoundError when the binary is absent rather
    than returning a non-zero code, which would turn the sentence explaining
    what went wrong into a traceback. Exiting non-zero by crashing is not the
    same as saying so.
    """
    try:
        return subprocess.run(args, cwd=ROOT, capture_output=True, text=True)
    except (FileNotFoundError, PermissionError) as exc:
        return subprocess.CompletedProcess(args, 127, "", "%s: %s" % (args[0], exc))


def excused(run_id, sites):
    """True when a document citing a non-green run says out loud that it is."""
    for site in sites:
        path = os.path.join(ROOT, site.split(":")[0])
        if not os.path.exists(path):
            continue
        with open(path, encoding="utf-8") as fh:
            lines = fh.read().splitlines()
        for i, line in enumerate(lines):
            if run_id in line:
                window = " ".join(
                    lines[max(0, i - CONTEXT_LINES):i + CONTEXT_LINES + 1]
                ).lower()
                if any(w in window for w in FAILURE_IS_THE_POINT):
                    return True
    return False


def main():
    docs = documents()

    # PROBE WITH THE THING THIS SCRIPT ACTUALLY DOES, not with `gh auth status`.
    # That validates the token against /user, which a GitHub Actions job token
    # legitimately cannot read — it is repository-scoped, not a user token. So
    # the probe is one real query of the same kind as the checks below.
    probe = sh("gh", "run", "list", "--limit", "1", "--json", "databaseId")
    if probe.returncode != 0:
        print("gh cannot list this repository's runs, so no citation could be\n"
              "checked. This gate has no skip path on purpose: passing here without\n"
              "having looked is the exact failure it exists to prevent.\n"
              + (probe.stderr or probe.stdout).strip())
        return 1

    # A SHALLOW CLONE CANNOT ANSWER THIS QUESTION, and must not pretend to.
    # `actions/checkout@v4` fetches depth 1 by default, so on a runner HEAD has
    # no ancestors and `merge-base --is-ancestor` is false for every commit ever
    # made — which would report perfectly good citations as orphaned. "I could
    # not resolve it" and "it is orphaned" are a checkout problem and a
    # repository problem; they do not get the same message.
    shallow = sh("git", "rev-parse", "--is-shallow-repository")
    if shallow.stdout.strip() == "true":
        print("this is a SHALLOW clone: HEAD has no ancestors here, so every\n"
              "citation would read as orphaned and none of that would be true.\n"
              "Fetch the full history before running this — in a workflow that\n"
              "is `fetch-depth: 0` on the checkout step.")
        return 1

    cited, bare = {}, {}
    for doc in docs:
        path = os.path.join(ROOT, doc)
        if not os.path.exists(path):
            continue
        with open(path, encoding="utf-8") as fh:
            for lineno, line in enumerate(fh, 1):
                linked = RUN_RE.findall(line)
                for run_id in linked:
                    cited.setdefault(run_id, []).append("%s:%d" % (doc, lineno))
                # A run id written as a bare number is still evidence, and a
                # gate that only reads links misses whatever is load-bearing in
                # a table.
                for run_id in BARE_RE.findall(line):
                    if run_id not in linked:
                        bare.setdefault(run_id, []).append("%s:%d" % (doc, lineno))

    if not cited:
        print("no CI run is cited in any of the %d document(s) scanned, so there is\n"
              "nothing to check. That is suspicious rather than clean: the criteria\n"
              "that rest on a workflow being green have no link for a reader to\n"
              "follow, and this gate cannot tell an honest repository from one\n"
              "whose evidence has quietly gone missing." % len(docs))
        return 1

    failures = []
    for run_id, sites in sorted(cited.items()):
        out = sh("gh", "run", "view", run_id, "--json",
                 "headSha,workflowName,conclusion,status")
        if out.returncode != 0:
            failures.append("run %s (cited at %s) could not be resolved: %s"
                            % (run_id, ", ".join(sites), out.stderr.strip()[:120]))
            continue
        info = json.loads(out.stdout)
        sha = info.get("headSha", "")
        # "The commit is not an ancestor" and "this repository has never heard
        # of it" are different problems; the second is a fetch problem wearing
        # the first's face.
        if sh("git", "cat-file", "-e", sha + "^{commit}").returncode != 0:
            failures.append(
                "run %s ran on %s, which this checkout does not have AT ALL — that "
                "is a fetch problem, not necessarily an orphaned commit; cited at %s"
                % (run_id, sha[:7], ", ".join(sites)))
            continue
        # Green, or said not to be. Checked before ancestry, because a red run a
        # reader clicks is a worse citation than one they cannot check out.
        # "Still running" and "finished badly" are different mistakes and get
        # different sentences. A document that cites an unfinished run was
        # written ahead of its evidence — which is how a duration measured on
        # an EARLIER run ends up attached to a later one, a claim no reader can
        # falsify without opening both.
        concl = info.get("conclusion") or ""
        if info.get("status") != "completed":
            failures.append(
                "run %s has not finished (%s) and is already cited at %s as "
                "evidence. Whatever this document says about it — that it is "
                "green, how long it took — was written before the run said so"
                % (run_id, info.get("status", "?"), ", ".join(sites)))
        elif concl != "success" and not excused(run_id, sites):
            failures.append(
                "run %s concluded %r and is cited at %s as evidence, with nothing "
                "near the citation saying it is not a pass. A reader clicks that "
                "link and sees a cross under a sentence claiming it is green — "
                "cite the run that is, or say what this one is"
                % (run_id, concl, ", ".join(sites)))
        if sh("git", "merge-base", "--is-ancestor", sha, "HEAD").returncode != 0:
            failures.append(
                "run %s ran on %s, which is not an ancestor of HEAD — the reader is "
                "sent to a commit this branch does not contain (%s %s), cited at %s"
                % (run_id, sha[:7], info.get("workflowName", "?"),
                   info.get("conclusion", "?"), ", ".join(sites)))

    print("checked %d cited CI run(s) across %d document(s)" % (len(cited), len(docs)))

    # Bare ids are held to a different standard on purpose. A URL is something a
    # reader clicks, so it must land inside this branch's history. A number in a
    # table is a claim about a run that happened, and the honest requirement is
    # that it exists and says what the document says it says. Ancestry is
    # REPORTED for them, never enforced — what would be dishonest is not
    # disclosing it.
    if bare:
        print("\n%d run id(s) mentioned without a link:" % len(bare))
        for run_id, sites in sorted(bare.items()):
            out = sh("gh", "run", "view", run_id, "--json",
                     "headSha,workflowName,conclusion,status")
            if out.returncode != 0:
                failures.append(
                    "run %s is named at %s and DOES NOT RESOLVE — a number that "
                    "looks like evidence and is not" % (run_id, ", ".join(sites)))
                continue
            info = json.loads(out.stdout)
            sha = info.get("headSha", "")
            known = sh("git", "cat-file", "-e", sha + "^{commit}").returncode == 0
            anc = known and sh("git", "merge-base", "--is-ancestor",
                               sha, "HEAD").returncode == 0
            print("  %s  %s  %s  on %s — %s"
                  % (run_id, info.get("workflowName", "?"),
                     info.get("conclusion", "?"), sha[:7],
                     "in this branch" if anc
                     else "NOT in this branch; the document must say so"))

    if failures:
        print("\n%d citation(s) a reader could not follow:\n" % len(failures))
        for f in failures:
            print("  " + f)
        return 1
    print("\nevery cited run ran on a commit this branch contains")
    return 0


if __name__ == "__main__":
    sys.exit(main())
