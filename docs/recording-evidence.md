# What the recording proves, measured rather than described

    ./scripts/check-transcript.py recordings/lp-0002-threshold-moves-value.srt <film.mp4>

The short film attached to the pull request is one terminal session running the
two on-chain verifiers this repository ships. Its narration is committed beside
it as
[`recordings/lp-0002-threshold-moves-value.srt`](../recordings/lp-0002-threshold-moves-value.srt),
so the film can be read and grepped instead of watched.

## Measured 2026-08-28, on the reshot film

```
  lp-0002-threshold-moves-value.srt: 24 cue(s), lp-0002-threshold-moves-value-sub.mp4: 76.1 s
    ok    structure, and the last cue lands 1.2 s before the end
    ok    privacy-preserving variant spoken at 12s, and on screen there
    ok    approval marker            spoken at 28s, and on screen there
    ok    Thirteen transactions      spoken at 8s, and on screen there
    ok    recipient                  spoken at 51s, and on screen there
    ok    requirement of two         spoken at 35s, and on screen there
  transcript matches the film: structure, fit, and 5 anchor(s) tied to the picture.
```

**Why there is a fifth anchor now.** The film this replaces showed
`2 approval(s) recorded against a threshold of 3` for 27 of its 46 seconds, while
the narration said "passing the gate moves value" — read literally, an execution
that went through under-approved. `verify-onchain.sh` had already been corrected
to print `requirement of 2 (an anchored tier prices 1 at 2, below the default of
3)`; the film predated the fix. The transcript check passed that film 4 anchors
out of 4, because its anchors only test lines the narration names and the
narration never mentioned the approval count. The wrong line sat in a blind spot
by construction. The new narration says it aloud, so an anchor can hold it.

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

## Which commit each film shows

There are two films, and they were shot an hour apart, so they show different
commits. Saying which is which costs a paragraph and saves a reviewer the
trouble of wondering.

The **end-to-end film** was shot at
[`b4f53ba`](https://github.com/edenbd1/lp-0002-private-multisig/commit/b4f53ba6ae9bdd4722d8437c7beb36b626e4d28d)
and shows that hash in full in its opening seconds, over a clean tree. The
reviewed commit is later — necessarily, because a film's transcript and the
measurement of that transcript can only be written once the film exists. Run
`git diff b4f53ba <reviewed commit> --stat` for the exact list; it is this file,
the transcript, and the anchor added to hold one to the other.

The **76-second reading of the chain** was shot at
[`631efab`](https://github.com/edenbd1/lp-0002-private-multisig/commit/631efab731e29ac49e6e972713ce0c7b1b94d1d9),
one commit earlier. The reason is structural rather than an oversight: a film's
transcript can only be committed once the film exists, so the commit carrying the
transcript is necessarily later than the take it transcribes.
`git diff 631efab b4f53ba --stat` is two files — that transcript, and the anchor
added to `scripts/check-transcript.py` to hold it to the picture.

**No program, no script either film runs, and no artefact the chain sees** is in
that diff. The two binaries under `artifacts/programs/` are untouched, and
`scripts/preflight.sh` fails if either ever hashes differently from what is
deployed.

## What the films show, checked against what the scripts print

The previous pair of films had drifted from the scripts they filmed, and the
drift inverted a claim: the closing line read
`2 approval(s) recorded against a threshold of 3`, which is what an execution
that went through under-approved looks like, while the narration said "passing
the gate moves value". The other film's narration said the privacy-preserving
variant is "the part a block explorer cannot show you" — and the explorer shows
it, at `Proof Size: 264907 bytes`.

Both films were reshot rather than relabelled, and the check is not "does the
transcript match" but "does the picture still say what the scripts say today".
Every frame of all three films was read with OCR and searched for the wordings
that are now wrong:

| wording | must not appear | cycle | claim | e2e |
|---|---|---:|---:|---:|
| `threshold of` | yes | 0 | 0 | 0 |
| `cannot show you` | yes | 0 | 0 | 0 |
| `Five checks` / `[n/5]` | yes | 0 | 0 | 0 |
| `requirement of 2` | expected | 43 | — | 26 |
| `RISC0_DEV_MODE=0` | expected | — | 49 | 120 |

Counts are frames. A transcript check anchors only on lines the narration names;
this reads everything on screen, which is the half that let the old defect
through.

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
