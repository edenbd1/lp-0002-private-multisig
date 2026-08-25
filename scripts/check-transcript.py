#!/usr/bin/env python3
"""Prove the committed transcript belongs to the film it claims to transcribe.

    ./scripts/check-transcript.py recordings/lp-0002-threshold-moves-value.srt <film.mp4>

A transcript shipped next to a video is a convenience, and a convenience nobody
checks is a claim nobody checks. This asserts three things, and none of them is
"the file exists":

  STRUCTURE   cues are numbered from 1, strictly ordered, never overlapping, and
              the last one ends inside the film rather than past its end. A
              transcript of a *different*, longer cut fails here.

  FIT         the last cue ends within TAIL seconds of the film's end. A
              transcript of the first half of the film passes every structural
              check ever written and is still the wrong transcript.

  ANCHORS     the narration is spoken, so it is not on screen — but what it
              talks about is. For each anchor below, the frame at the moment the
              line is spoken must show the matching text. That is what ties the
              words to the picture, and it is the only check here that could not
              be satisfied by a plausible file written from memory.

No skip path: a missing ffmpeg, ffprobe or tesseract exits non-zero. A checker
that passes when it cannot look is the failure it exists to prevent.

THIS ONE RUNS BY HAND, AND THAT IS NOT AN OVERSIGHT

It needs the film, and the film is not in this repository: it is an attachment
on the pull request, where a reader watches it. Committing a video to carry a
check would be the tail wagging the dog. So no workflow runs this, and saying so
here is the point — a gate nobody schedules and nobody mentions is indis-
tinguishable, from the outside, from one that passes. Run it against the file
you uploaded, before you upload it.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

TAIL = 30.0          # the last cue may end at most this far before the film does
WINDOW = 25.0        # how far either side of a cue an anchor may appear on screen

# Anchors are per film, because films do not show the same things: looking for
# "r0vm" in a film that never proves anything reports "not testable" on every
# line and then passes, which is the failure this file exists to prevent.
#
# Each entry is (a phrase in the narration, a regex the picture must show while
# it is spoken). Deliberately few and deliberately loose: this ties two
# artefacts together, it is not an OCR accuracy contest. The digit classes are
# not decoration — tesseract renders 0 as @ or O in this terminal font, so the
# prover line comes back as `RISC@_DEV_MODE=0 r@vm 1129% cpu`.
ANCHORS = {
    "lp-0002-threshold-moves-value.srt": [
        ("privacy-preserving variant", r"variant\s*1|PrivacyPreserving"),
        ("approval marker",         r"approval\s*[01]|approval markers"),
        # The rotation is the half of the lifecycle a transfer-only film never
        # shows, so the anchor demands `rotate_config` on screen rather than any
        # variant column: the latter is satisfied by the first eight rows alone.
        ("Thirteen transactions",   r"rotate_config"),
        # `balance 2` and not `recipient`: the word is in the screen's own
        # caption, so matching on it would pass whatever the number said.
        ("recipient",               r"balance\s*2"),
    ],
}

# A film whose transcript is not listed above cannot be anchored, and this
# refuses rather than passing on structure alone.
MIN_ANCHORS = 2


def need(binary):
    if shutil.which(binary) is None:
        sys.exit("  %s is not on PATH. Refusing to report on a film I cannot read." % binary)


def cues(path):
    out = []
    for block in open(path, encoding="utf-8").read().strip().split("\n\n"):
        lines = [l for l in block.split("\n") if l.strip()]
        if len(lines) < 3 or "-->" not in lines[1]:
            continue

        def secs(t):
            h, m, s = t.split(":")
            return int(h) * 3600 + int(m) * 60 + float(s.replace(",", "."))

        a, b = (secs(x.strip()) for x in lines[1].split("-->"))
        out.append((int(lines[0]), a, b, " ".join(lines[2:])))
    return out


def duration(film):
    o = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                        "-of", "csv=p=0", film], capture_output=True, text=True).stdout.strip()
    if not o:
        sys.exit("  ffprobe could not read %s" % film)
    return float(o)


def frame_text(film, t, tmp):
    p = os.path.join(tmp, "f.png")
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-ss", "%.3f" % t,
                    "-i", film, "-frames:v", "1", p], check=True)
    return " ".join(subprocess.run(["tesseract", p, "-"], capture_output=True)
                    .stdout.decode("utf-8", "replace").split())


def main(argv):
    if len(argv) < 3:
        sys.exit("  usage: check-transcript.py <transcript.srt> <film.mp4>")
    srt, film = argv[1], argv[2]
    for b in ("ffmpeg", "ffprobe", "tesseract"):
        need(b)

    cs = cues(srt)
    if not cs:
        sys.exit("  %s carries no cue" % srt)
    dur = duration(film)
    print("  %s: %d cue(s), %s: %.1f s" % (os.path.basename(srt), len(cs),
                                           os.path.basename(film), dur))

    bad = 0
    for i, (n, a, b, _) in enumerate(cs):
        if n != i + 1:
            print("    cue %d is numbered %d" % (i + 1, n)); bad += 1
        if b <= a:
            print("    cue %d ends before it starts" % n); bad += 1
        if i and a < cs[i - 1][2]:
            print("    cue %d starts before cue %d ends" % (n, cs[i - 1][0])); bad += 1
    end = cs[-1][2]
    if end > dur:
        print("    the last cue ends at %.1f s, past the film's %.1f s" % (end, dur)); bad += 1
    elif dur - end > TAIL:
        print("    the last cue ends %.1f s before the film does (limit %.0f): "
              "this looks like a transcript of a different cut" % (dur - end, TAIL)); bad += 1
    else:
        print("    ok    structure, and the last cue lands %.1f s before the end" % (dur - end))

    # If tesseract reads nothing anywhere, the transcript is not what failed.
    # Blaming it would be the checker reporting on a film it could not see —
    # and in this environment that happens for real: tesseract silently returns
    # an empty string for images under some temporary directories.
    found = 0
    with tempfile.TemporaryDirectory(dir=os.path.dirname(os.path.abspath(film))) as tmp:
        probe = [frame_text(film, dur * f, tmp) for f in (0.25, 0.5, 0.75)]
        if not any(probe):
            sys.exit("  tesseract read nothing from three frames of %s. The tool cannot see "
                     "the film; this says nothing about the transcript." % os.path.basename(film))
        table = ANCHORS.get(os.path.basename(srt))
        if table is None:
            sys.exit("  no anchors are defined for %s. Add them to ANCHORS rather than "
                     "letting a structural pass stand in for a check."
                     % os.path.basename(srt))
        for phrase, pattern in table:
            hit = next((c for c in cs if phrase.lower() in c[3].lower()), None)
            if hit is None:
                print("    the narration never says %r — anchor not testable" % phrase)
                continue
            _, a, b, text = hit
            mid = (a + b) / 2
            ok = False
            # Sweep the window at a fixed step rather than probing five points
            # in it. Five points miss evidence that is genuinely there: the
            # thirteen-row table prints six seconds after the line that names
            # it, inside the same shot, and landed between two probes — the
            # check then reported a true caption as unmatched. A miss that says
            # "not on screen" when it is on screen is the failure mode that
            # matters here, because it is the one that gets argued away.
            offs = [0.0]
            step = 2.0
            k = step
            while k <= WINDOW:
                offs += [-k, k]
                k += step
            for off in offs:
                t = min(max(mid + off, 0), dur - 1)
                if re.search(pattern, frame_text(film, t, tmp), re.I):
                    ok = True
                    break
            if ok:
                found += 1
                print("    ok    %-26s spoken at %.0fs, and on screen there" % (phrase, mid))
            else:
                print("    %-26s spoken at %.0fs, but no %s within %.0fs of it"
                      % (phrase, mid, pattern, WINDOW)); bad += 1

    if found < MIN_ANCHORS:
        print("    only %d anchor(s) could be tested (need %d): the part of this check "
              "that ties words to pictures did not run, so it proved nothing." % (found, MIN_ANCHORS))
        bad += 1
    if bad:
        print("  %d problem(s): this transcript is not this film's." % bad)
        return 1
    print("  transcript matches the film: structure, fit, and %d anchor(s) tied to the picture." % found)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
