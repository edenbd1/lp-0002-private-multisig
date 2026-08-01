# LP-0002 — Private M-of-N Multisig for the Logos Execution Zone

A threshold multisig where members hold shielded accounts, approvals leave no
public trace of who voted, and the chain records only that a threshold was met —
not which members met it. **Including from the other members.**

Submission for [λPrize LP-0002](https://github.com/logos-co/lambda-prize/blob/master/prizes/LP-0002.md).

---

## What makes this different

**The membership proof is genuinely verified on chain.** A LEZ *public*
transaction proves and verifies nothing — the sequencer re-executes the program
host-side (`lee/state_machine/src/program.rs:73-77`, commented *"Execute the
program (without proving)"*). A multisig built on that path is a signature check
wearing a zero-knowledge costume. This one targets the privacy-preserving path,
where LEZ's circuit composes each chained call with a real `env::verify`
(`lee/privacy_preserving_circuit/src/execution_state.rs:149`) and the sequencer
checks the receipt against the node-pinned `PRIVACY_PRESERVING_CIRCUIT_ID`.

**The member set and the threshold are anchored by address.** The multisig
account is a PDA seeded by `[multisig_id, config_hash]` where
`config_hash = H(member_root ‖ threshold)`. An invented member set, or a
threshold lowered at execution time, resolves to an address nobody ever
initialised. There is no code path that reads a threshold from caller-supplied
data.

**Approvals bind to the exact action.** `proposal_ref` commits to
`(multisig_id, proposal_id, action_hash)`. Approvals gathered for a benign
action are worthless for a malicious one published under the same proposal id —
and, symmetrically, a junk action cannot burn the real proposal's markers.

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

No network, no funded account, no sequencer required. The demo runs the 22
circuit tests, the 28 adversarial tests against the built verifier binary through
the sequencer's own executor, a full 3-of-5 lifecycle, and reports the measured
compute cost.

To rebuild the on-chain programs (needs Docker and `cargo-risczero`):

```bash
./scripts/build-programs.sh
```

### Test inventory

`cargo test --workspace` runs **53** tests. Counted, so you can check the claim:

| Suite | Count | What it establishes |
|---|---:|---|
| `multisig-core` — `approve_adversarial` | 22 | The circuit-side bindings: non-members, borrowed Merkle paths, invented roots, forged nullifiers, bait-and-switch actions, the padding sentinel, tree construction |
| `multisig-verifier-tests` — `verifier_rejects` | 28 | The **built verifier binary** through the sequencer's own executor: 4 honest controls (one per instruction), 22 attacks, 2 framework-layer checks |
| `multisig-verifier-tests` — `program_id_pin` | 2 | The verifier pins the committed membership binary, and the pin is not a placeholder |
| `multisig-sdk` doctest | 1 | The documented API compiles and runs |

One further test is `#[ignore]`d: it reports the measured compute cost rather
than asserting a property. Run it with
`cargo test -p multisig-verifier-tests -- --ignored --nocapture`.

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
msig propose --dir ms/ --proposal-id 01..01 --action "transfer 100 to the treasury"
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

---

## Layout

| Path | What |
|---|---|
| `crates/multisig-core` | Shared primitives and the in-circuit approval logic. `no_std`. 22 adversarial tests |
| `crates/membership-circuit/methods/guest-lez` | The membership proof as a native LEZ program, so the privacy circuit composes it with `env::verify` |
| `crates/multisig-verifier-spel/methods/guest` | The on-chain verifier: `create_multisig`, `create_proposal`, `approve`, `execute` |
| `crates/multisig-verifier-tests` | 28 adversarial tests against the built binary, through the sequencer's own executor |
| `crates/multisig-sdk` | The reusable client library for Logos modules. Transport-agnostic |
| `crates/multisig-cli` | `msig`, the command line client |
| `app/` | The Basecamp GUI |
| `idl/` | The SPEL IDL, generated from source |
| `scripts/` | Build, demo, testnet lifecycle, on-chain verification |

## Documentation

- **[docs/security.md](docs/security.md)** — the threat model, what is hidden
  from whom, what is deliberately public, and the residual risks
- [docs/error-codes.md](docs/error-codes.md) — every rejection, the attack it
  stops, the test that proves it
- [docs/cu-costs.md](docs/cu-costs.md) — measured compute cost per instruction
- [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) — deployed program ids, transaction
  hashes, and how to verify them independently
- [app/README.md](app/README.md) — building and loading the Basecamp app

## Toolchain

| Component | Version |
|---|---|
| LEZ | `v0.2.0` |
| SPEL | `v0.6.0` |
| risc0 | `3.0.5` |
| Rust (host) | stable |
| Rust (guest) | `1.88` via `cargo risczero` Docker |

`enum-ordinalize` and `enum-ordinalize-derive` are pinned to `4.3.2` in both
guest lockfiles; `4.4.2` breaks the guest toolchain with *"rustc 1.88.0-dev is
not supported"*.

## Live on the public LEZ testnet

A **2-of-3** multisig, created, proposed, approved to threshold on the
privacy-preserving path, and executed. Every hash is live — check any of them
with `getTransaction` against `https://testnet.lez.logos.co`.

| Step | Transaction |
|---|---|
| deploy `membership_lez` | `64098974b7d28f4facf1218e771d27c6163f7fb7ce3bd4f218df6db42ace6dde` |
| deploy `multisig_verifier` | `e24f5367521616f235acd26e3ee8937e8fc071335f187fab3ad282b6a691192f` |
| `create_multisig` | `de22a8c917774643969fec0b566082f701867b1359f18574fc8ee390badb3cdd` |
| `create_proposal` | `b7a4f74534cf9efc5da734c50b3c3ace7d1e7aa35aeaf86ca8f736dd566ea832` |
| `approve` (member A, **privacy tx**) | `6e035e4e702bcd241faa2ac304bc32178193bef25d66036a8bf7b0915d716347` |
| `approve` (member B, **privacy tx**) | `968c5d1ba1b828f93ee44a037b313f1c8c2b3ea30afb9302857b18fd8619dc55` |
| `execute` | `5817c49ce6ab86b5349ca2d55b95662f4cf7192b89be924d1f72c74f5d0e8b74` |

The two approval markers — `9q31RPufMoRe6pXcxrcuwFEJQN2Wnr2qV4HhXnV8a42r` and
`GMgP7TMKoFVimMxX7PmtbeYG1dhGTGHUDh4F1yJmc8pv` — and the execution marker
`EsV6LpVUfR1iunep8g4etg1qTGQfzxbA1J7PjDBsFV5b` are all owned by the verifier
program. Neither approval marker could exist without a membership proof having
been verified on chain, and neither names a member.

Full detail, including how to re-verify each one yourself, in
[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

## Programs

| Program | ImageID |
|---|---|
| `membership_lez` | `a48ecc5289404ad01fd6d6fd1d79eaebb8d2f0fe4f2dc2ebbc85003ee82af3d6` |
| `multisig_verifier` | `cf5724b0e8dabd4a1519f8d9ea7371d69e1d7e2f6d8c931f1e4b3110150d7982` |

The verifier pins the membership program id as a constant, so a chained call can
only ever reach the audited binary. `./scripts/build-programs.sh --check` fails if
the two drift.

## Licence

MIT or Apache-2.0, at your option.
