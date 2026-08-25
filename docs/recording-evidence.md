# What the recording proves, measured rather than described

    ./scripts/check-transcript.py recordings/lp-0002-threshold-moves-value.srt <film.mp4>

The short film attached to the pull request is one terminal session running the
two on-chain verifiers this repository ships. Its narration is committed beside
it as
[`recordings/lp-0002-threshold-moves-value.srt`](../recordings/lp-0002-threshold-moves-value.srt),
so the film can be read and grepped instead of watched.

## Measured 2026-08-26

```
  lp-0002-threshold-moves-value.srt: 15 cue(s), lp-0002-threshold-moves-value-sub.mp4: 46.5 s
    ok    structure, and the last cue lands 1.2 s before the end
    ok    privacy-preserving variant spoken at 12s, and on screen there
    ok    approval marker            spoken at 21s, and on screen there
    ok    Thirteen transactions      spoken at 9s, and on screen there
    ok    recipient                  spoken at 28s, and on screen there
  transcript matches the film: structure, fit, and 4 anchor(s) tied to the picture.
```

The anchors are the part that matters. The narration is spoken, so its words are
never on screen — but what it *talks about* is, and each anchor requires the
picture to show it while the line is being said. Where the narration says "five
approvals carry the privacy-preserving variant", the frame at that second must
show the variant column. A transcript written from memory passes every
structural check and fails these.

The checker refuses to pass on structure alone: a transcript it has no anchors
for is rejected outright, and fewer than two tested anchors is a failure. That
rule is there because the first version of this check did exactly what it exists
to prevent — run against a film whose narration used none of its phrases, it
reported "anchor not testable" four times and then printed "transcript matches
the film".

## Which commit the film shows

The film was shot at
[`dda7d2a`](https://github.com/edenbd1/lp-0002-private-multisig/commit/dda7d2a)
and shows that hash on screen in its opening seconds, over a clean tree. The
reviewed commit is later, because the transcript of a film can only be committed
after the film exists, and the checks that followed found things worth fixing.
Everything between the two is documentation and repository gates: this file, the
transcript and its checker, the README's test tally — which had not followed the
suite from 130 to 131 — and the gate that now reads that table rather than four
remembered rows of it. **No program, no script the demo runs, no artefact the
chain sees.** Rather than trust that list to stay complete,
`git diff dda7d2a..HEAD --stat` prints what actually moved; the two program
binaries under `artifacts/programs/` are not in it, and `scripts/preflight.sh`
fails if either ever hashes differently from what is deployed.

## What this does not establish

- **Not every frame.** Up to 25 frames are read per anchor: the window is now
  swept at a fixed two-second step rather than probed at five points, because
  five points missed the thirteen-row table that prints six seconds after the
  line naming it, and reported a true caption as unmatched. Widening a check is
  the wrong instinct when the claim is false, so the negative control was rerun
  afterwards: this transcript against a different film still fails, on five
  counts. Nothing here claims the frames between the samples.
- **Not the wording.** Nothing checks the transcript's sentences against the
  audio. Structure, fit and four anchors are what is proved.
- **An OCR behaviour routed around, not diagnosed.** Tesseract renders `0` as
  `@` in this terminal font, and on the machine these were run on it returns an
  empty string for images under some temporary directories, silently. The
  checker probes three frames before reporting and refuses to blame the
  transcript if it can read none of them. Why tesseract does that has not been
  diagnosed, and is not guessed at here.
