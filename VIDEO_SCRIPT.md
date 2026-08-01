# Narrated demo script

The prize requires a **narrated** walkthrough in which the builder explains what
they built and why, walks through the architecture and key implementation
decisions, and shows the full end-to-end flow with terminal output confirming
`RISC0_DEV_MODE=0`. A silent screencast is not sufficient — this is the single
most common reason submissions in this program get closed.

Target: 10–14 minutes. Speak over every step; do not just let the terminal scroll.

---

## 0. Setup before recording

```bash
cd lp-0002-private-multisig
git log --oneline | head -5      # show it is real work with history
rm -rf .demo
```

Have two terminal tabs: one for the demo, one for `docs/security.md` open in a
pager or editor for the architecture segment.

---

## 1. What this is and why (1 min, talking head or slide)

> "This is LP-0002, a private M-of-N multisig for the Logos Execution Zone.
> Members hold shielded accounts. When a threshold approves a proposal, the chain
> records that the threshold was met — and nothing about who met it. Not to
> outside observers, and, the part I care most about, not to the other members
> either. The prize asks for exactly that: without revealing identity to on-chain
> observers *or other members*."

---

## 2. The decision that shaped everything (2 min)

Open `docs/security.md` at *"Why the proof is genuinely verified on chain"*.

> "The first thing I checked was whether a LEZ public transaction actually
> verifies a proof. It does not. The sequencer re-executes the program host-side
> — here, in `program.rs`, the comment literally says *execute the program,
> without proving*. So a multisig on the public path would be a signature check
> wearing a zero-knowledge costume.
>
> The path that works is the privacy-preserving transaction. The client proves
> locally, and LEZ's privacy circuit composes each chained call with a real
> `env::verify` — this line — and the sequencer checks the receipt against the
> pinned circuit id. So my `approve` instruction declares a chained call to a
> membership program, and that membership proof is verified on chain as a
> precondition of the transaction being accepted.
>
> For that to work the callee has to *be* a LEZ program emitting a
> `ProgramOutput`, not a standalone Risc0 guest. That is why `membership_lez`
> exists in the shape it does."

---

## 3. Anchoring: the attacks that shaped the design (2–3 min)

Open `crates/multisig-verifier-spel/methods/guest/src/bin/multisig_verifier.rs`.

> "A membership proof proves membership against whatever root the statement
> names. On its own that is worthless — I can build a one-leaf tree containing
> myself. So the multisig account is a PDA whose address derives from the
> multisig id and a config hash, and the config hash commits to the member root
> *and the threshold*. Only `create_multisig` initialises that address. An
> invented member set gives a different address that was never created, so its
> owner is the default, and I reject it.
>
> Putting the threshold inside the same hash is what stops the other obvious
> attack: supplying `threshold = 1` at execution against a 3-of-5 set. Different
> threshold, different config hash, different address, never created. There is no
> code path anywhere that reads a threshold from caller-supplied data.
>
> The third one took me longer to see. If approvals were scoped to a proposal id
> alone, I could publish a harmless action, collect three approvals, then publish
> a second proposal under the same id with a malicious action — and the approvals
> would count for it. So `proposal_ref` folds in the action hash. Approvals bind
> to exact bytes. And symmetrically, someone publishing a junk action under my
> proposal id cannot burn my markers, because they land at different addresses."

Scroll to `execute`, checks 6 and 7.

> "And this is what makes M markers mean M distinct *members*. Each marker
> address is a function of a nullifier, each nullifier is a function of a member's
> secret. Two different addresses imply two different secrets. So I check the
> nullifiers are pairwise distinct, that each account is the marker PDA for the
> nullifier it was paired with on *this* proposal, and that my program owns it —
> which it can only do if a membership proof was verified on chain."

---

## 4. The demo (4–5 min)

```bash
./scripts/demo.sh
```

Narrate as it runs. Points to hit, in order:

- **Step 0** — pause on `RISC0_DEV_MODE=0`. Say it out loud: no mock receipts.
- **Step 1** — "22 adversarial tests on the circuit logic. Non-members, borrowed
  Merkle paths, invented roots, forged nullifiers, bait-and-switch actions, and
  the padding sentinel — because padding leaves are real leaves and I had to
  prove nobody can approve as one."
- **Step 2** — "These 15 run the *built binary* through the sequencer's own
  executor. Same executor, same input order, same 32-megabyte session limit. A
  rejection here is the rejection the chain performs. Two honest controls, twelve
  attacks, each required to fail with its documented code."
- **Steps 3–5** — the 3-of-5 lifecycle, three members approving.
- **Step 6** — "Member zero tries again and is refused, with the reason."
- **Step 7** — "And the action cannot be swapped under the same id."
- **Step 9** — *slow down here.* "This is the whole point. On chain this proposal
  is three addresses. Each one is a hash of a nullifier, and each nullifier is a
  hash of a member secret. Someone who knows all five members — including the
  other four members — cannot tell which three of them these came from."
- **Step 10** — the measured compute cost.

---

## 5. Reliability and the SDK (1–2 min)

```bash
cat .demo/proposals/00...01.json
```

> "Approvals are recorded before the command returns, so a partial set survives a
> client restart. That is the reliability criterion, and it falls out naturally:
> proving takes ten minutes an approval, so a real threshold *is* gathered across
> days and separate sessions."

Show `crates/multisig-sdk/src/lib.rs` briefly.

> "The SDK is the crate a Logos module depends on. It opens no socket and touches
> no filesystem — give it data, it gives you instruction arguments. The CLI and
> the Basecamp app are both thin shells over it."

---

## 6. The Basecamp app (1 min)

Show it loaded, run one approval through the GUI.

> "Every button runs an `msig` subcommand, so the GUI and the chain compute the
> same commitments from the same code. And notice the approval list shows marker
> addresses, never member names — because that is all the chain records."

---

## 7. Honest limits (1 min)

Open `docs/security.md` at *"What is not hidden"*.

> "N is public. M is public. The count of approvals so far is public — each one is
> an account. The action is public. And two things I want to name rather than
> bury: timing correlation is real, because approvals land minutes apart and a
> member whose account activity is otherwise visible can be linked by timing
> rather than by cryptography. And small sets leak — in a 1-of-2 multisig the
> other member knows it was not them, so they know exactly who it was.
> Unlinkability scales with the member count. I would rather state that than
> claim a property I do not have."

---

## 8. Close (30 s)

> "Everything I have shown runs from a clean clone with `./scripts/demo.sh` — no
> network, no funded account. The testnet lifecycle is `scripts/deploy-and-run.sh`
> and the transaction hashes are in `docs/DEPLOYMENT.md`."

---

## Checklist before uploading

- [ ] Your voice, explaining — not a silent screencast
- [ ] `RISC0_DEV_MODE=0` visible in the terminal
- [ ] Real proof generation shown or its cost stated honestly
- [ ] Architecture and the *why* behind decisions, not just a feature tour
- [ ] Full end-to-end flow
- [ ] Unlisted or public on YouTube, link added to `README.md` and the solution file
