# Solution: LP-0002 — Private M-of-N Multisig

**Submitted by:** edenbd1

## Summary

A threshold multisig for LEZ where members hold shielded accounts, approvals
leave no public trace of who voted, and the chain records only that a threshold
was met — not which members met it, **including to the other members**.

The membership proof is genuinely verified on chain. Each `approve` declares a
`ChainedCall` to a LEZ-native membership program, so LEZ's privacy circuit
composes it with a real `env::verify` and the sequencer checks the receipt
against the node-pinned `PRIVACY_PRESERVING_CIRCUIT_ID`. The member set and the
threshold are anchored by PDA address, so neither can be invented or lowered.

## Repository

- **Repo:** <https://github.com/edenbd1/lp-0002-private-multisig>
- **Video:** <https://youtu.be/cPVsWWBVEeE>

## Approach

### The decision that shaped everything: which transaction path

The first thing I checked was whether a LEZ **public** transaction verifies a
proof. It does not — the sequencer re-executes the program host-side
(`lee/state_machine/src/program.rs:73-77`, commented *"Execute the program
(without proving)"*). A multisig built there would be a signature check wearing
a zero-knowledge costume, which is precisely the ground on which earlier
submissions in this program were rejected.

The path that works is the **privacy-preserving transaction**: the client proves
locally, LEZ's privacy circuit composes each chained call with a real
`env::verify` (`lee/privacy_preserving_circuit/src/execution_state.rs:149`), and
the sequencer verifies the receipt against the pinned circuit id.

For that composition to happen, the callee must *be* a LEZ program emitting a
`ProgramOutput` — a standalone Risc0 guest commits a bespoke journal that cannot
decode as one, and the sequencer rejects the call with `ProgramExecutionFailed`.
That is why `membership_lez` exists in the shape it does rather than as a plain
guest.

It also has a privacy consequence I depend on: a privacy `Message` publishes
commitments and nullifiers and carries neither `program_id` nor
`instruction_data`, so the witness can travel in the instruction. On the public
path the same bytes would be published verbatim — so the membership program must
never be invoked there, and is not.

### Anchoring, and the three attacks it closes

A membership proof establishes membership against whatever root the statement
names, which on its own is worthless: anyone can build a one-leaf tree holding
themselves. So:

**The member set is anchored by address.** The multisig account is a PDA seeded
by `[multisig_id, config_hash]`, and only `create_multisig` initialises it. An
invented root gives a different address that was never created, whose owner is
the default — rejected (`5003`).

**The threshold is inside the same hash.** `config_hash = H(member_root ‖ threshold)`.
Supplying `threshold = 1` against a 3-of-5 set resolves to a PDA nobody created —
rejected (`5003`). There is no code path anywhere that reads a threshold from
caller-supplied data. I considered storing the threshold in the account's data
instead; folding it into the address is strictly stronger, because it makes the
forgery unrepresentable rather than merely checked.

**Approvals bind to the exact action.** This one took me longest to see. If
approvals were scoped to a proposal id alone, a proposer could publish a harmless
action, collect M approvals, then publish a second proposal under the same id
carrying a malicious action — and the approvals already gathered would count for
it. So `proposal_ref = H(multisig_id ‖ proposal_id ‖ action_hash)`, and both the
nullifier and the marker seed derive from it. It closes the mirror-image
griefing vector too: a junk action published under a real proposal id cannot burn
that proposal's markers, because they land at different addresses.

### Making M markers mean M distinct members

Each approval occupies a PDA seeded by
`H(APPROVAL_MARKER_PREFIX ‖ proposal_ref ‖ nullifier)` where
`nullifier = H(APPROVAL_NULLIFIER_PREFIX ‖ proposal_ref ‖ msk)`. Two different
addresses therefore imply two different secrets. `execute` checks the nullifiers
are pairwise distinct (`5011`), that each account presented is the marker PDA for
the nullifier it was paired with *on this proposal* (`5012`), and that the
verifier program owns it (`5013`) — which it can only do if a membership proof
was verified on chain.

The distinctness check is quadratic in M deliberately: M is a multisig
threshold, a small number, and a sort would cost more cycles than it saves. The
measured cost confirms it — `execute` scales at ~48,600 user cycles per approval.

### What did not work

- **Storing the threshold in account data.** Workable, but it makes the forgery
  *representable* and then rejected, rather than unrepresentable. Address
  anchoring is the stronger construction and needs no data writes at all.
- **Scoping approvals to `proposal_id`.** See the bait-and-switch above.
- **Enabling risc0's `prove` feature** so the tests execute in-process rather
  than through an `r0vm` subprocess. It drags in the GPU backends, and on macOS
  that means Metal, which needs an Xcode component most machines lack. CI
  installs `r0vm` instead.

### Why the Logos stack

The whole design rests on two properties nothing centralised provides. First,
**trustless execution with real proof composition**: the threshold is enforced by
a circuit the sequencer verifies, not by a server that could be asked to lie
about who voted. Second, **shielded accounts as a first-class primitive**: a
member's approval is bound to a secret that never appears on chain, so
unlinkability holds against the other members and not just against outsiders. A
centralised multisig service can hide the member list from the public; it cannot
hide it from itself, and it cannot prove to a third party that it counted
honestly.

### Targeting the current testnet, and the SPEL patch that took

The prize says it targets the *current* LEZ testnet. That testnet was reset onto
a newer chain in August: everything deployed against **v0.2.0** stopped being
accepted — not "went stale", but rejected outright, with submitted transactions
returning a hash whose `getTransaction` is `null`. Everything here was rebuilt
against **LEZ v0.2.4**, redeployed, and the whole lifecycle re-run. Every hash
and address in this submission comes from that run.

That upgrade is also why this repository vendors SPEL. Its latest release,
**v0.6.0**, still pins LEZ v0.2.0 in about twenty places, and there is no
released SPEL that builds against a current LEZ — so "use SPEL" and "transact on
the current testnet" could not both be satisfied with anything published.
`vendor/spel` is upstream v0.6.0 with the minimum changes to do both, and
[`vendor/spel/PATCH.md`](https://github.com/edenbd1/lp-0002-private-multisig/blob/main/vendor/spel/PATCH.md)
documents every one of them, how to reproduce the directory, and when it should
be deleted. Three things broke: private-PDA derivation gained an ML-KEM 768
viewing key, `WalletCore::from_env()` became async, and the wallet went
multi-sequencer.

Worth flagging because it is the kind of thing that wastes a reviewer's day:
released SPEL *builds* cleanly against v0.2.4's testnet and then fails at run
time on every instruction with `missing field 'sequencer_addr'`, because the
wallet config format changed. The private-PDA API question is left to SPEL's
maintainers rather than patched over here — this project uses only public PDAs,
whose derivation is unchanged in v0.2.4.

## Success Criteria Checklist

### Functionality

- [x] **Any M-of-N member holding a shielded LEZ account can submit an approval
      without revealing their identity to on-chain observers or other members.**
      The only trace is a marker PDA seeded by a secret-bound nullifier. See
      [`docs/security.md`](docs/security.md), which states the threat model
      against three adversaries including the insider.
- [x] **The on-chain verifier confirms a threshold of M approvals was reached
      without recording which members approved.** `execute`, checks 5–7.
- [x] **A member cannot approve the same proposal twice.** The nullifier is
      deterministic per `(proposal, member)`, so the second approval targets an
      occupied PDA and `init` refuses.
- [x] **A completed execution is unlinkable to any individual member's shielded
      account.** `execute` consumes marker addresses and is signed by an executor
      who need not be a member at all.
- [x] **Proof generation runs client-side on a standard laptop.** Approvals are
      built by `msig approve-args` and proved locally; **149 s and 154 s** for the
      two approvals of a measured 2-of-3 run, timed by the script itself.
- [x] **A reference integration on LEZ testnet.** A 2-of-3 multisig created, a
      proposal published, two approvals gathered on the privacy-preserving path,
      and executed against the fixed verifier. Seven transactions, all live; see
      [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md).

      **What "threshold-gated action" means here**, since the criterion offers
      *treasury transfer or parameter change* as examples and it is fair to ask
      which one this is. The gated action is `transfer 100 LEZ to the grants
      treasury`, bound into `proposal_ref` by its hash, and what the chain gates
      is the **right to act on it**: the execution marker PDA comes into
      existence only when M distinct secret-bound nullifiers resolve to markers
      the verifier owns, and it can never be produced any other way. Performing
      the transfer is one CPI away and deliberately out of scope — a multisig is
      an authorisation primitive, and coupling it to one action type would make
      it a treasury program instead. Any program can gate on that marker: it is
      a PDA at a derivable address, owned by the verifier, whose existence *is*
      the proof that the threshold was met. The submission demonstrates the
      part the prize is about — reaching the threshold privately — rather than
      re-implementing a transfer that LEZ already has.

      One thing to flag before you click, because it would otherwise look like
      dead links: **none of the seven transactions renders on the explorer right
      now**, and all seven are on chain — `getTransaction` returns every one.
      The explorer's indexer is behind: its own front page listed block 4351 as
      the most recent while the chain was at 4496, and every transaction here was
      included after that. It affects anything recently submitted, by anyone.

      Worth stating plainly, because the obvious check misleads: the explorer
      is a WASM app that serves an identical page shell for every hash and
      renders client-side, so comparing response sizes cannot distinguish an
      indexed transaction from one that does not exist.
      `./scripts/check-explorer.py` renders the pages in a headless browser
      instead, with an impossible hash as the control, and prints what it finds.
      Re-run it; the answer is a measurement with a date on it, not a claim.

      `docs/DEPLOYMENT.md` gives a one-line `curl` per hash, and
      `./scripts/verify-onchain.sh` does the stronger check — it reads the five
      accounts the lifecycle created and confirms the verifier program owns
      them, which no transaction lookup can fake.
- [x] **At least 1 multisig instance on testnet with a proposal submitted,
      approved by threshold, and executed.** Multisig
      `4wqJXoEhqqqYknt1s7gHcgBL6pkfwNJDfhbVVeAqwtnX`, proposal
      `E11Awng7j59dVft83VVrwftXp41roJPKY5QRMb45Zcoe`, execution marker
      `CpiuicNDii6uCeMXtjd1W6hek6Vq35HJ7k3mz1Q82Fui` — all owned by the verifier.
      Re-verify with `./scripts/verify-onchain.sh`.
- [x] **Full documentation and a clean public repository.**

### Usability

- [x] **Module/SDK for building Logos modules.** `crates/multisig-sdk`,
      transport-agnostic: it opens no socket and touches no filesystem.
- [x] **Basecamp app GUI with local build instructions and loadable assets.**
      `app/`, with both the `logos-module-builder` path and a standalone Qt path.
      The packaged module is committed at `app/lp-0002-multisig.lgx` (2.4 MB),
      built with the **real `lgx`** from `logos-co/logos-package` — the same tool
      `nix-bundle-lgx` drives — and it passes `lgx verify`. It carries **two
      variants**, `darwin-arm64` and `linux-amd64`, so it opens on the machine
      the evaluator actually reviews on; the Linux half is built in Docker by
      `scripts/build-linux-variant.sh` and the packaged Linux `msig` was run
      inside a Linux container to confirm it works, not just that it exists.

      **The Linux variant is load-tested against Basecamp's own Qt, not just
      built.** Basecamp ships an `x86_64.AppImage` in the same 0.2.2 release;
      extracted, it bundles the same Qt 6.9.2 as macOS. Running what Basecamp's
      PluginLoader runs — `QPluginLoader::instance()` then
      `qobject_cast<IComponent*>` — linked against *that* Qt gives
      `SUCCESS: loaded + cast to IComponent`. The harness was shown able to
      fail: a real Qt plugin with a different IID gives `CAST FAILED`. Basecamp
      itself also boots on Linux with the package installed, reaching
      `Logos Core started successfully!` and scanning the plugins directory.
      What is *not* claimed is the click: a UI app's widget loads on click, and
      headless there is nothing to click, so that half was exercised on macOS.
      Method in [`app/README.md`](app/README.md).

      **It was installed and driven in Logos Basecamp 0.2.2**, not merely
      declared loadable: the committed package extracts into the user plugins
      directory, the module loads (`Successfully loaded UI module`), and
      pressing *Status* against `artifacts/testnet` returns the live
      deployment's `2-of-3 · 2/2 READY TO EXECUTE` with both approval markers.
      Doing that found three defects in the packaging, all fixed and all
      described in `app/README.md`: the plugin was built against a Qt newer than
      the one Basecamp runs, so Qt refused it outright; `Q_DECLARE_INTERFACE`
      used a private IID instead of `com.logos.component.IComponent`, so the
      host's `qobject_cast` returned null; and the `IComponent` declaration
      carried an extra virtual, which shifted every later vtable slot.
      `scripts/package-lgx.py` now refuses to package a binary with any of
      those properties.
- [x] **SPEL IDL.** `idl/multisig_verifier.idl.json`, generated from source by
      `spel generate-idl`.

### Reliability

- [x] **Proof generation failures surface a clear error.** `approve-args`
      verifies the witness locally before emitting anything, so a bad witness
      fails in microseconds rather than after minutes of proving.
- [x] **A partial set of approvals is preserved and resumable across client
      restarts.** Every approval is recorded in `proposals/<id>.json` before the
      command returns. This is not incidental: proving takes minutes, not an
      approval, so a real threshold *is* gathered across sessions and days.
- [x] **Deterministic, documented error codes.** Thirteen, `5001`–`5013`, in
      [`docs/error-codes.md`](docs/error-codes.md), each mapped to the attack it
      stops and the test that proves it.

### Performance

- [x] **CU cost of each on-chain operation documented.**
      [`docs/cu-costs.md`](docs/cu-costs.md): `approve` 335,564 user cycles;
      `execute` linear in M at ~48,600 per approval; all inside one segment at
      1.56 % of the public budget. Measured by replaying through the sequencer's
      own executor, reproducible with one command.

### Supportability

- [x] **Deployed and tested on LEZ devnet/testnet.** Both programs deployed;
      full lifecycle run and independently re-verifiable over JSON-RPC.
- [x] **E2E integration tests run against a LEZ sequencer (standalone mode), in
      CI.** Two layers, because they answer different questions.
      `multisig-verifier-tests` runs the built binary through the sequencer's own
      **executor** on every push — same executor, same input order, same 32M
      session limit — which is what makes 28 adversarial rejections cheap enough
      to gate a commit. On top of that,
      [`.github/workflows/e2e-local-sequencer.yml`](.github/workflows/e2e-local-sequencer.yml)
      starts the actual `sequencer_service` binary in **standalone mode** and
      drives the whole lifecycle against it over JSON-RPC. It is scheduled
      rather than per-push because it builds the LEZ workspace and then
      generates a real proof; both are CI, and only one of them can be fast.
- [x] **CI green on the default branch.** Ubuntu and macOS both.
- [x] **README documents end-to-end usage** — deployment steps, program
      addresses, and step-by-step instructions for both the CLI and the Basecamp
      app, the latter verified on LogosBasecamp 0.2.2.
- [x] **Reproducible demo script working against a real local sequencer with
      `RISC0_DEV_MODE=0`.** `scripts/e2e-local-sequencer.sh` starts a real
      standalone sequencer, funds a throwaway wallet from its genesis vault,
      deploys both programs onto that fresh chain, and runs create → propose →
      *real Risc0 approvals* → execute, then reads the five resulting accounts
      back off it. Nothing mocked, `RISC0_DEV_MODE=0` throughout, about eight
      minutes for a 2-of-3. `scripts/demo.sh` remains the five-second tour for
      readers who want the rejections and the cost table without a sequencer.
- [x] **Recorded narrated video demo.** <https://youtu.be/cPVsWWBVEeE> — the
      terminal is on screen throughout, `RISC0_DEV_MODE=0` is visible before any
      proving starts, and the approval is a real proof generated in one
      uninterrupted take, with its wall clock printed at the end.

## FURPS Self-Assessment

### Functionality

Four instructions: `create_multisig`, `create_proposal`, `approve`, `execute`.
Seven documented bindings, each with adversarial coverage. **Limitations, stated
rather than buried:** the member set is fixed at creation (no rotation);
proposals do not expire; the action payload is opaque to this protocol and a
consuming program must validate it itself.

### Usability

Three surfaces over one library: the SDK, the `msig` CLI, and the Basecamp app.
The app shells out to the CLI, so the GUI and the chain compute the same
commitments from the same code — there is no second implementation to drift.
Onboarding is `git clone && ./scripts/demo.sh`. The CLI's refusals are written to
be read by a human and explain *why*, and the GUI passes them straight through.

### Reliability

Local pre-verification before proving; resumable partial approvals; thirteen
documented error codes. Failure modes I know about and have not solved: a
transaction whose proving outruns the wallet's polling window reports "NOT
confirmed" while landing anyway — the scripts poll `getTransaction` rather than
trust the CLI's verdict, and the docs say so.

### Performance

`approve` at 1.56 % of the public compute budget, with headroom to ~524k user
cycles before a second segment. Wall-clock is dominated entirely by proving:
**149 s and 154 s** for the two approvals of a 2-of-3 run against a local
standalone sequencer, timed by `scripts/deploy-and-run.sh` itself rather than
estimated, and written into `lifecycle.tsv` alongside the transaction hashes.
Both numbers are in [`docs/cu-costs.md`](docs/cu-costs.md) with the machine they
were taken on.

That figure is a correction. This submission previously said "~10 minutes per
approval" in six places, carried over from an early estimate that no one had
gone back and measured. Measuring it was part of closing the "proof generation
time benchmark" requirement, and it turned out the estimate was four times too
pessimistic — which is worth stating plainly, because a benchmark nobody
re-runs is just a claim.

### Supportability

61 tests across five suites, counted and itemised in the README. Every one of
the thirteen documented error codes is exercised against the built binary, and
the bindings they enforce are written up in `docs/security.md`. The two guest
crates are excluded from the host workspace because they target
`riscv32im-risc0-zkvm-elf` — but the deployed program is still under test,
because `multisig-verifier-tests` is a workspace member exercising the built
binary. CI runs on Ubuntu and macOS, and a dedicated job fails if
`MEMBERSHIP_LEZ_PROGRAM_ID` drifts from the committed membership binary, since
that constant is what stops a chained call from reaching an unaudited program.

## Write-up, by the topics the brief names

The Submission Requirements ask for a write-up covering six specific things, and
the Scope section adds a seventh. Rather than leave them to be hunted for:

| Required topic | Where |
|---|---|
| **Threshold proof scheme** | *Approach* above — membership against an anchored Merkle root, proved in a Risc0 guest, composed on chain by LEZ's privacy circuit via `env::verify` |
| **Nullifier design** | *Making M markers mean M distinct members* above, and bindings 4–7 in [`docs/security.md`](docs/security.md) |
| **LEZ account model compatibility** — specifically the **nonce** and **`program_owner`** constraints | [`docs/lez-account-model.md`](docs/lez-account-model.md), in full. Short version: the nonce constraint never binds on membership because a member is a Merkle leaf and not an account this program touches; it binds one level up, on the account that *submits*, which is unrelated to the member and may be a relayer. `program_owner` is load-bearing rather than an obstacle — it is how an uninitialised PDA is detectable, which is what makes a forged member set unrepresentable instead of merely refused |
| **Trusted setup** | **None.** Risc0 is a STARK-based, transparent proving system: there is no structured reference string, no ceremony, and no toxic waste to trust or destroy. The only trusted parameters are the two program ImageIDs, and those are content-addressed — [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) shows a clean rebuild reproducing `5bb40082…` exactly, so an evaluator who rebuilds gets the same identity or knows something is wrong |
| **Security assumptions** | [`docs/security.md`](docs/security.md) — three adversaries in increasing order of knowledge, ending at the insider, which is the interesting one |
| **Known limitations** | [`docs/security.md`](docs/security.md), *What is not hidden* and *Residual risks and non-goals* — timing correlation, small anonymity sets, and a 1-of-2 multisig hiding nothing from its other member |
| **Integration instructions** | [`README.md`](README.md) — the CLI lifecycle, the Basecamp walkthrough, and [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) for deployment |
| **Proof generation time and gas cost benchmarks** | [`docs/cu-costs.md`](docs/cu-costs.md) — per-instruction CU, and per-approval wall clock measured by the lifecycle script itself |

## Supporting Materials

- [`docs/security.md`](docs/security.md) — threat model, what is hidden from
  whom, what is deliberately public, residual risks
- [`docs/lez-account-model.md`](docs/lez-account-model.md) — the nonce and
  `program_owner` constraints, which are the incompatibility this prize exists
  to solve
- [`docs/error-codes.md`](docs/error-codes.md) — all 13 codes, the attack each
  stops, and the test behind it
- [`docs/cu-costs.md`](docs/cu-costs.md)
- [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md)
- Narrated demo video: <https://youtu.be/cPVsWWBVEeE>

## Two LEZ behaviours worth knowing

Both are invisible until you hit them, and neither is a program bug — they are
noted here because anyone deploying on this stack will meet them:

1. **One approver account cannot serve several approvals.** A privacy
   transaction consumes the approver's commitment, so a second approval from the
   same account panics in the *client-side* circuit — `Invalid
   account_identities length, left: 4, right: 3` — before anything is submitted.
   One private account per approval fixes it, and that is how a real deployment
   works anyway.
2. **Variadic accounts take one comma-separated flag.** `spel-cli` resolves them
   with `last_value()`, so a repeated `--approvals` silently keeps only the last.
   The program then saw one account against two nullifiers and rejected with
   `E_APPROVAL_COUNT_MISMATCH` (5009) — the check doing exactly its job, which is
   how the cause was found.

Both are handled in `scripts/deploy-and-run.sh` and documented in
`docs/DEPLOYMENT.md`.

## Terms & Conditions

By submitting this solution, I confirm that I have read and agree to the
[Terms & Conditions](../TERMS.md).
