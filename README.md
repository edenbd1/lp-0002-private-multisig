# LP-0002 — Private M-of-N Multisig for the Logos Execution Zone

A threshold multisig where members hold shielded accounts, approvals leave no
public trace of who voted, and the chain records only that a threshold was met —
not which members met it. **Including from the other members.**

Submission for [λPrize LP-0002](https://github.com/logos-co/lambda-prize/blob/master/prizes/LP-0002.md).

---

## What makes this different

**The membership proof is genuinely verified on chain.** A LEZ *public*
transaction proves and verifies nothing — the sequencer re-executes the program
host-side (`lee/state_machine/src/program/mod.rs:73-77`, commented *"Execute the
program (without proving)"*). A multisig built on that path is a signature check
wearing a zero-knowledge costume. This one targets the privacy-preserving path,
where LEZ's circuit composes each chained call with a real `env::verify`
(`lee/privacy_preserving_circuit/src/execution_state.rs:149-155`) and the sequencer
checks the receipt against the node-pinned `PRIVACY_PRESERVING_CIRCUIT_ID`.

**The member set and the threshold are anchored by address.** The multisig
account is a PDA seeded by `[multisig_id, config_hash]` where
`config_hash = H(member_root ‖ threshold)`. An invented member set, or a
threshold lowered at execution time, resolves to an address nobody ever
initialised. There is no code path that reads a threshold from caller-supplied
data.

**Approvals bind to the exact action, and the action moves money.** Each
multisig owns a treasury PDA. A proposal names a recipient and an amount;
`proposal_ref` commits to `(multisig_id, config_hash, proposal_id, action_hash)`
and `action_hash` commits to those fields, so approvals gathered for a benign
payment are worthless for a malicious one published under the same proposal id.
Reaching the threshold **debits the treasury and credits the recipient**, in the
`execute` transaction. That works with no transfer authority over anybody: LEZ
forbids a program from *decreasing* a balance it does not own and permits any
program to *increase* any balance, and the treasury is this program's own
account.

**Every account it claims carries a record.** A `getAccount` on the multisig
returns the member root, the threshold and the treasury address; on the proposal,
the recipient, the amount and an executed flag; on the execution marker, the
exact nullifiers consumed. The byte offsets are published in
[docs/account-layout.md](docs/account-layout.md), and `scripts/decode-account.py`
reads them with no dependency on this repository's Rust.

**M markers are M distinct members.** Each marker address is a function of a
secret-bound nullifier, so two addresses imply two secrets. `execute` checks
pairwise distinctness, checks each account is the marker PDA for the nullifier it
was paired with *on this proposal*, and checks the verifier owns it — which it
can only do if a membership proof was verified on chain.

---

## Quick start

```bash
git clone https://github.com/edenbd1/lp-0002-private-multisig
cd lp-0002-private-multisig
./scripts/demo.sh
```

No network, no funded account, no sequencer required. The demo runs the 25
circuit tests, the 59 tests against the built verifier binary through the
sequencer's own executor — the forgeries it refuses, the balances it moves, and
the records it writes — a full 3-of-5 lifecycle, and reports the measured
compute cost.

The only hard requirement is a Rust toolchain. `r0vm`, `spel`, `python3` and
`curl` are each optional and each gates exactly one step: without one, that step
prints `(skipped: …)`, is listed again in a summary at the end, and the script
still exits 0. A skip is never reported as a pass. The two steps that need
`r0vm` — the verifier suite and the compute-cost measurement — run in CI on
every push against these same committed binaries.

### Against a real sequencer

`demo.sh` is fast because it drives the sequencer's *executor* in-process. That
is the same code the chain runs, but it is not the same claim as "works against
a sequencer". This makes that one:

```bash
./scripts/e2e-local-sequencer.sh
```

It starts the actual `sequencer_service` binary in standalone mode, points a
throwaway wallet at it, and drives the whole lifecycle over JSON-RPC — deploy,
create, propose, gather a threshold of **real Risc0 approvals** with
`RISC0_DEV_MODE=0`, execute — then reads the resulting accounts back off that
local chain. About 22 minutes for a 2-of-3 on LEZ v0.2.4 — measured, not
estimated: 1333 s end to end on an Apple-silicon laptop, of which 444 s and
704 s were the two real proofs. It prints its own per-approval wall clock, which
is the number to trust on your machine. Needs a `logos-execution-zone` checkout
(`LEZ_SRC`, default `_external/lez`) and builds the sequencer once.

The same script runs in CI on a schedule — see
[`.github/workflows/e2e-local-sequencer.yml`](.github/workflows/e2e-local-sequencer.yml).

Before pushing, run what CI runs:

```bash
./scripts/preflight.sh
```

It is the same four commands `.github/workflows/ci.yml` uses, not an
approximation of them — `fmt --check`, `clippy --all-targets -- -D warnings`,
build, test. Wire it up as a pre-push hook with:

```bash
printf '#!/usr/bin/env bash\nexec "$(git rev-parse --show-toplevel)/scripts/preflight.sh"\n' \
  > .git/hooks/pre-push && chmod +x .git/hooks/pre-push
```

To rebuild the on-chain programs (needs Docker and `cargo-risczero`):

```bash
./scripts/build-programs.sh
```

### Test inventory

`cargo test --workspace` runs **97** tests. Counted, so you can check the claim:

| Suite | Count | What it establishes |
|---|---:|---|
| `multisig-core` — `approve_adversarial` | 25 | The circuit-side bindings: non-members, borrowed Merkle paths, invented roots, forged nullifiers, bait-and-switch actions, the padding sentinel, tree construction |
| `multisig-core` — `state` unit tests | 9 | The published account layouts: exact lengths, refused formats, refused trailing bytes, and a round trip through the offsets `docs/account-layout.md` gives |
| `multisig-verifier-tests` — `verifier_rejects` | 30 | The **built verifier binary** through the sequencer's own executor: 5 honest controls and 25 rejections — 19 asserting a documented error code, 6 caught one layer earlier by SPEL's address validation or LEZ's init guard |
| `multisig-verifier-tests` — `state_and_transfer` | 22 | What passing the gate *does*: the treasury falls by the proposed amount and the recipient rises by it, read out of the guest's own journal; every claimed account decodes at the published offsets; and fifteen ways of redirecting, draining, replaying or corrupting the payment are refused |
| `multisig-verifier-tests` — `idl_contract` | 5 | The IDL carries every error code and every record layout the guest declares, the instruction order has not moved, `docs/error-codes.md` documents every code, and `scripts/pda.py` derives the same treasury address the program does |
| `multisig-verifier-tests` — `program_id_pin` | 2 | The verifier pins the committed membership binary, and the pin is not a placeholder |
| `multisig-sdk` — `cross_check` + doctest | 2 | Every SDK derivation equals the `multisig-core` one the chain re-derives, and the client-side guards hold |
| `multisig-cli` — `resumable` | 2 | Through the **built binary**, one process per step: a partial set of approvals survives client restarts, and a non-member is refused in milliseconds instead of after minutes of proving |

25 + 9 + 30 + 22 + 5 + 2 + 2 + 2 = 97, and every row's own breakdown adds up to
its count.

One further test is `#[ignore]`d: it reports the measured compute cost rather
than asserting a property. Run it with
`cargo test -p multisig-verifier-tests --test verifier_rejects -- --ignored --nocapture`.

[`docs/error-codes.md`](docs/error-codes.md) counts **59** rather than 97. That
is `multisig-verifier-tests` alone — the suites that run the built binary — which
is the inventory behind the error-code table. 97 is what `cargo test --workspace`
runs. Both numbers are in CI: the `workspace` job runs `multisig-core`,
`multisig-sdk` and `multisig-cli`, the `verifier` job runs
`multisig-verifier-tests`, and between them all 97 run on every push.

**The deployed program is genuinely under test.** The two guest crates are
excluded from the host workspace because they build for
`riscv32im-risc0-zkvm-elf`, but `multisig-verifier-tests` is a workspace member
and it exercises the *built binary* — so the on-chain program is not a component
that ships untested.

---

## The lifecycle

```bash
msig new-multisig --members 5 --threshold 3 --out ms/
msig create-multisig-args --dir ms/ --out ms/create.args
msig fund-treasury-args --dir ms/ --amount 500 --out ms/fund.args
msig propose --dir ms/ --proposal-id 01..01 \
     --recipient <64-hex account id> --amount 250 --memo "pay the auditors"
msig create-proposal-args --dir ms/ --proposal-id 01..01 --out ms/prop.args
msig approve-args --dir ms/ --proposal-id 01..01 --member 0 --out ms/a0.args
msig approve-args --dir ms/ --proposal-id 01..01 --member 3 --out ms/a3.args
msig approve-args --dir ms/ --proposal-id 01..01 --member 4 --out ms/a4.args
msig status --dir ms/ --proposal-id 01..01
msig execute-args --dir ms/ --proposal-id 01..01 --out ms/exec.args
```

Each `.args` file is submitted with `spel`; see [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).
A member who holds only their own key uses `--msk <hex>` instead of `--member <i>`.

Partial approvals are recorded in `ms/proposals/<id>.json` as they are generated,
so a threshold can be gathered across restarts, days, and separate member
sessions.

### The same lifecycle from the Basecamp app

The GUI is the CLI: every button runs an `msig` subcommand, so the two cannot
compute different commitments. Verified on **LogosBasecamp 0.2.2**.

1. **Install the module.** The package is committed at
   `app/lp-0002-multisig.lgx` and carries a `darwin-arm64` and a `linux-amd64`
   variant. Extract the one matching your host into Basecamp's user plugins
   directory:

   ```bash
   lgx extract app/lp-0002-multisig.lgx --variant darwin-arm64 --output /tmp/x
   D=~/Library/Application\ Support/Logos/LogosBasecamp/plugins/lp-0002-multisig
   mkdir -p "$D" && cp -R /tmp/x/darwin-arm64/. "$D/"
   printf darwin-arm64 > "$D/variant"
   tar xzOf app/lp-0002-multisig.lgx manifest.json > "$D/manifest.json"
   ```

   On Linux the directory is `~/.local/share/Logos/LogosBasecamp/plugins/` and
   the variant is `linux-amd64`. Restart Basecamp; the module appears in the
   left rail. Full notes, including why the Qt version and the plugin interface
   are load-bearing, are in [app/README.md](app/README.md).

2. **Open it** — click the tile. The `msig` CLI ships inside the package and the
   plugin resolves it from its own directory, so the *msig binary* field stays
   empty unless you are running out of a build tree.

3. **Point *Multisig folder*** at a directory made by `msig new-multisig`.
   (`artifacts/testnet` is the record of a *superseded* deployment and predates
   the typed action, so the current `msig` will not parse its proposal file —
   see the README inside that directory.)

4. **Create** — set members and threshold, press *New multisig*. Writes
   `multisig.json` and `members.json` and prints the member root and the config
   hash that anchors the pair on chain.

5. **Propose** — enter a proposal id, the recipient's account id, the amount and
   the memo, press *Bind*. All four are bound into the proposal reference, so
   re-binding the same id to a different payment is refused and the message
   explains why.

6. **Approve** — pick a member index, or paste a member's own secret, and press
   *Build approval*. The `.args` file it writes is submitted with `spel` on the
   privacy-preserving path; proving takes about two and a half minutes
   (measured — see [docs/cu-costs.md](docs/cu-costs.md)).

7. **Status** — shows how many approvals have been gathered, the payment they
   authorise, and the marker addresses. It reads the resumable state file, so it
   survives a Basecamp restart.

8. **Build execution** — emits the execution arguments once the threshold is met.

The approval list shows marker addresses, never member names, because that is
all the chain records and all the other members can see.

---

## Layout

| Path | What |
|---|---|
| `crates/multisig-core` | Shared primitives, the in-circuit approval logic, and the account-layout codec. `no_std`. 34 tests |
| `crates/membership-circuit/methods/guest-lez` | The membership proof as a native LEZ program, so the privacy circuit composes it with `env::verify` |
| `crates/multisig-verifier-spel/methods/guest` | The on-chain verifier: `create_multisig`, `fund_treasury`, `create_proposal`, `approve`, `execute` |
| `crates/multisig-verifier-tests` | 59 tests against the built binary, through the sequencer's own executor: 30 rejections and controls, 22 on the state it writes and the value it moves, 5 on the IDL and the docs, 2 pinning the membership binary |
| `crates/multisig-sdk` | The reusable client library for Logos modules. Transport-agnostic |
| `crates/multisig-cli` | `msig`, the command line client |
| `app/` | The Basecamp GUI |
| `idl/` | The SPEL IDL, generated from source |
| `scripts/` | Build, demo, testnet lifecycle, on-chain verification |

## Documentation

- **[docs/security.md](docs/security.md)** — the threat model, what is hidden
  from whom, what is deliberately public, and the residual risks
- **[docs/account-layout.md](docs/account-layout.md)** — what every account
  holds, field by field, with byte offsets, so a stranger with `getAccount` can
  decode one by hand
- [docs/error-codes.md](docs/error-codes.md) — every rejection, the attack it
  stops, the test that proves it
- **[docs/lez-account-model.md](docs/lez-account-model.md)** — how the nonce and
  `program_owner` constraints are handled, which is the incompatibility this
  prize exists to solve
- [docs/cu-costs.md](docs/cu-costs.md) — measured compute cost per instruction,
  and measured proof generation time
- [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) — deployed program ids, transaction
  hashes, and how to verify them independently
- [app/README.md](app/README.md) — building and loading the Basecamp app

## Toolchain

| Component | Version |
|---|---|
| LEZ | `v0.2.4` |
| SPEL | `v0.6.0` |
| risc0 | `3.0.5` |
| Rust (host) | stable |
| Rust (guest) | `1.88` via `cargo risczero` Docker |

`enum-ordinalize` and `enum-ordinalize-derive` are pinned to `4.3.2` in both
guest lockfiles; `4.4.2` breaks the guest toolchain with *"rustc 1.88.0-dev is
not supported"*.

## On the public LEZ testnet — superseded, pending a redeploy

**Read [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) before following any address
here.** Giving the accounts state and the threshold a treasury changed the
verifier guest, and on LEZ a program's identity *is* its ImageID. The seven
transactions below are still on chain and still valid; what they are no longer is
*this* program's, and the five account addresses they produced belong to the
previous ImageID. `./scripts/deploy-and-run.sh` re-runs the whole lifecycle, and
DEPLOYMENT.md lists what has to be renumbered when it does.

The record of the run that happened, from ImageID `5bb40082…`:

```bash
./scripts/verify-onchain-lifecycle.sh
```

It recomputes each deploy hash from the committed bytecode rather than trusting
the table below, and asserts the variant every transaction carries — the two
approvals are `PrivacyPreserving` and not `Public`, which is the part a block
explorer cannot show you. A hash that was never deployed runs first as the
control; if it ever resolves the run is abandoned rather than reported green.

| Step | Transaction |
|---|---|
| deploy `membership_lez` | `fb8eb10f7f394286c109cb6502a1c95294180523f30d06f707fc087a589bea98` |
| deploy `multisig_verifier` | `517efe12a0b592abe4d21a03246866b95c4379483e87af62fd9f26f7b8fe45ff` |
| `create_multisig` | `2930c1db4521b7c0b912278f4025e430704cfb9a7ebfcb5d22c374fd7ce85b70` |
| `create_proposal` | `68d5127e1e5570936f8d78e9a2da4d485562566cd8b7487a59322bf059406978` |
| `approve` (member A, **privacy tx**) | `41f5bb99346a0bef6aa0c69243473a554b84f0f0ad65e460bbb6890b11644942` |
| `approve` (member B, **privacy tx**) | `ae006465f5f945b8ba2666f28a5357d0a2aab4af05508c9c2811e0101d0ac649` |
| `execute` | `b43e46505f571e31d6051f7da43563db605b6a74b90c670da2d3582d53412ecd` |

The two approval markers — `DaG2Qan1ie5YhEpcti2LMCsvbkYi7WjWxnNKvxiqxi7B` and
`FMj5yL8cpcrQzN7xhENHC2vysTrNwbtokPbTYjr98rPt` — and the execution marker
`CpiuicNDii6uCeMXtjd1W6hek6Vq35HJ7k3mz1Q82Fui` are all owned by the verifier
program. Neither approval marker could exist without a membership proof having
been verified on chain, and neither names a member.

Full detail, including how to re-verify each one yourself, in
[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

## Programs

| Program | ImageID |
|---|---|
| `membership_lez` | `56f784d6b37f5cbac85d2eca3e28f56346e8739e6c22cb15a1b7165616758e31` |
| `multisig_verifier` | `1346b65293ac9b11d4b1029a0d02559462238582124062925a3ad24298ff4e1e` |

The verifier pins the membership program id as a constant, so a chained call can
only ever reach the audited binary. `./scripts/build-programs.sh --check` fails if
the two drift.

## Licence

MIT or Apache-2.0, at your option.
