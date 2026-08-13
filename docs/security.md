# Security and privacy model

This document states precisely what LP-0002 hides, from whom, and what it does
not hide. Claims of privacy are only meaningful relative to a threat model, so
the model comes first.

## Threat model

Three adversaries, in increasing order of what they know.

**A — the passive observer.** Reads the chain. Sees every account, every
transaction, every PDA address. Does not know the member set.

**B — the informed observer.** Everything A has, plus the full member list: every
member's `account_id` and every per-entry salt. This is the realistic adversary
for a DAO whose membership is public, or for anyone who was handed the member
CSV.

**C — the insider.** A member of the multisig. Everything B has, plus their own
secret `msk`, their own Merkle path, and whatever they observed while
participating.

The interesting claim is against **C**. A multisig that hides approvals from the
outside world but leaks them to the other signers has not solved the problem
this prize describes: *"without revealing their identity to on-chain observers
**or other members**."*

## What is hidden

**Which members approved — from A, B, and C.**

An approval's only on-chain trace is a marker account at a PDA seeded by

```
approval_marker_seed = SHA256(APPROVAL_MARKER_PREFIX || proposal_ref || nullifier)
nullifier            = SHA256(APPROVAL_NULLIFIER_PREFIX || proposal_ref || msk)
```

`msk` is the member's secret. To link a marker to a member, an adversary must
find the `msk` that produces its nullifier. B knows every `account_id`, but
`account_id = SHA256(prefix || npk || identifier)` and `npk = SHA256(prefix || msk)`
— both one-way. Knowing who *could* have approved gives no way to test who *did*.

C is in exactly B's position for every member but themselves. A member can
recognise their own marker, because they can compute their own nullifier. They
learn nothing about anyone else's. Three markers on a 3-of-5 proposal tell a
member "two of the other four approved" and nothing more.

**Whether two approvals on different proposals came from the same member.**

The nullifier folds in `proposal_ref`, so the same member produces unrelated
nullifiers on different proposals. There is no cross-proposal correlation
handle. An adversary cannot build a voting history for a pseudonymous member.

**Which member the execution came from.**

`execute` consumes marker addresses and a signature from an executor who need
not be a member at all. The transaction that lands on chain carries the
executor's identity, and the executor can be a disinterested relayer. This is
what makes the completed execution unlinkable to any member's shielded account.

**The witness itself.**

The Merkle path, the salt, the identifier and `msk` travel in the instruction
data of a **privacy-preserving** transaction. The privacy `Message` struct
(`lee/state_machine/src/privacy_preserving_transaction/message.rs`) has fields
for public account ids, nonces, public post-states, encrypted private
post-states, commitments, nullifiers and the validity windows — and **no
`program_id` field and no `instruction_data` field**. On the public path the
same bytes would be published verbatim, which is why the membership program must
never be invoked there, and is not: the verifier reaches it only through a
`ChainedCall` inside a privacy transaction.

*Verified against a real transaction rather than asserted.* Decoding the
`approve` transaction from the testnet run and searching its 230 kB payload:
no member `msk`, no salt, no `account_id`, and not even the `member_root`
appears anywhere in it. What *does* appear is the verifier's program id — eight
times, in the `program_owner` field of the accounts carried in
`public_post_states`. That is necessary and harmless: the whole point of the
marker is that it exists and is owned by this program, so its owner must be
published. Which program a multisig uses is public by design. **Which member
approved is what has to stay hidden, and it does.**

## What is not hidden

Stated plainly, because a privacy claim that overreaches is worse than none.

| Public | Why |
|---|---|
| **N**, the member-set size | Committed at creation; the Merkle tree's shape is derivable |
| **M**, the threshold | Inside `config_hash`, which is in the multisig PDA address |
| **The number of approvals so far** | Each is a marker account; counting them is trivial |
| **The proposed action** | `action_hash` is inside `proposal_ref`, which is in the proposal PDA address; the preimage is published so members can decide |
| **That *some* member approved at time T** | The marker account appears in a block |
| **The executor's identity** | They sign and pay for the execution transaction |

Two consequences worth naming:

**Timing correlation.** A member who submits an approval from an account whose
activity is otherwise observable may be linked to their marker by timing, not by
cryptography. Approvals land minutes apart because proving is slow, which widens
rather than narrows this window. Mitigation is operational: submit through a
relayer, or batch. The protocol does not solve it and does not claim to.

**Small anonymity sets.** A 1-of-2 multisig hides almost nothing: one marker on a
two-member set means one of two people, and the other member knows it was not
them, so they know exactly who it was. Unlinkability is bounded by `N - 1` from
an insider's perspective and by `N` from an observer's. The property is real but
it scales with the member count, and for N=2 an insider learns everything.

## The seven bindings

Each is enforced in `multisig-core`'s `approve`, or on chain by PDA anchoring.
Every one has adversarial test coverage; see `crates/multisig-core/tests/` for
the circuit half and `crates/multisig-verifier-tests/` for the on-chain half.

1. **Membership is against an anchored root.** The multisig PDA address derives
   from `[multisig_id, config_hash]`, and only `create_multisig` initialises it.
   An invented member set gives a different address, never initialised, whose
   owner is the default — rejected with `5003`.

2. **The threshold cannot be lowered.** `threshold` is inside `config_hash`, so
   it is anchored by the same address. Supplying `threshold = 1` against a 3-of-5
   set resolves to a PDA nobody created — rejected with `5003`. There is no code
   path that reads a threshold from caller-supplied data.

3. **Approvals are bound to the exact action.** `proposal_ref` commits to
   `(multisig_id, proposal_id, action_hash)`, and the nullifier and marker seeds
   derive from it. Approvals gathered for a benign action are worthless for a
   malicious one published under the same id. This also removes the mirror-image
   griefing vector: a junk action cannot burn the real proposal's markers,
   because they live at different addresses.

4. **Double-approval is caught by a secret-bound nullifier.** Deterministic per
   `(proposal, member)`, so the second approval targets an occupied PDA and
   `init` refuses.

5. **Approvals do not cross multisigs or proposals.** `proposal_ref` carries
   `multisig_id`, and `action_hash` is itself multisig-scoped. Crucially the
   proposal's **PDA address** is also seeded by `[multisig_id, proposal_ref]`, so
   the binding is enforced by the address rather than only committed inside a
   hash the program cannot invert. See *An audit finding* below for why that
   distinction is not academic.

6. **The public key ties the secret to the committed leaf.** The circuit proves
   `npk = H(prefix || msk)` and `account_id = derive_account_id(npk, identifier)`,
   so the nullifier's secret owns the committed entry. Without it, a prover could
   pair someone else's leaf with their own nullifier.

7. **M distinct markers are M distinct members.** Each marker address is a
   function of a nullifier; each nullifier is a function of a secret. Two
   addresses imply two secrets. `execute` checks the nullifiers are pairwise
   distinct (`5011`), that each account is the marker PDA for the nullifier it
   was paired with *on this proposal* (`5012`), and that the verifier owns it
   (`5013`) — which it can only do if a membership proof was verified on chain.

## Why the proof is genuinely verified on chain

This is the criterion most submissions in this program have failed, so it is
worth being exact about the mechanism.

A LEZ **public** transaction proves and verifies nothing. The sequencer
re-executes the program host-side — `lee/state_machine/src/program.rs:73-77`,
commented *"Execute the program (without proving)"*. A multisig built on that
path would be a signature check wearing a zero-knowledge costume.

The privacy-preserving path is different. The client proves locally
(`lez/wallet/src/lib.rs:578`); LEZ's privacy circuit composes each chained call
with a real verification —

```
env::verify(chained_call.program_id, program_output_words)
    -- lee/privacy_preserving_circuit/src/execution_state.rs:149
```

— and the sequencer checks the resulting receipt against the node-pinned
`PRIVACY_PRESERVING_CIRCUIT_ID`. So when `approve` declares a `ChainedCall` to
`membership_lez`, the membership proof is verified on chain as a precondition of
the transaction being accepted.

For that composition to happen the callee must be a real LEZ program emitting a
`ProgramOutput`. That is why `membership_lez` exists as a LEZ program rather than
a standalone Risc0 guest: a standalone guest commits a bespoke journal that
cannot decode as a `ProgramOutput`, and the sequencer rejects the call with
`ProgramExecutionFailed`.

The sequencer runs `receipt.verify(PRIVACY_PRESERVING_CIRCUIT_ID)` and has no
`RISC0_DEV_MODE` in its environment, so acceptance implies a genuine receipt even
if a client were in dev mode.

## Why a proposal belongs to exactly one configuration

This is the binding that took the most care to get right, so it is worth stating
in full — including the attack it exists to stop.

**The attack it defends against.** Anyone may create a multisig; that is by
design, and `create_multisig` places no ownership constraint on `multisig_id`.
An attacker can therefore create their own **1-of-1 configuration under a
victim's `multisig_id`**: a fresh `(id, config_hash)` pair, so it initialises
cleanly. If the proposal's identity did not mention the configuration, both
accounts an approval needs would still resolve — the multisig PDA
`[victim_id, attacker_config]` because they created it, and the proposal PDA
because its seeds would not mention the config. They would approve against their
own member root, mint a marker on the victim's proposal, and execute at
threshold 1. A 3-of-5 proposal would execute on one outsider's approval.

**Why both mechanisms are needed.** `config_hash` is folded into `proposal_ref`
*and* into the proposal PDA's seeds. Each alone leaves a hole:

- **Seeds only.** The attacker cannot approve on the victim's proposal, but a
  proposal of their own under the same `(multisig_id, proposal_id, action)`
  would produce the *same* `proposal_ref`, so their markers would land in the
  victim's marker address space and `execute` would count them.
- **`proposal_ref` only.** Marker addresses separate, but `approve` takes
  `proposal_ref` opaquely and never re-derives it, so an attacker naming the
  victim's ref with their own config would still resolve both accounts.

Together, a proposal belongs to one `(member_root, threshold)`, and naming any
other configuration resolves to an account nobody ever created — rejected before
the program body runs.

**How it is checked.** Four tests hold `multisig_id` constant and vary the
configuration, which is the axis this binding covers:
`approving_under_a_second_config_of_the_same_multisig_id_is_rejected`,
`executing_under_a_second_config_of_the_same_multisig_id_is_rejected`,
`approving_a_proposal_under_a_foreign_multisig_is_rejected` and
`executing_a_proposal_under_a_foreign_multisig_is_rejected`. All four run the
built binary through the sequencer's own executor.

## Trusted setup

**There is none.** Risc0 is STARK-based and transparent: no structured reference
string, no ceremony, no toxic waste. Nothing in this system's security depends
on a parameter that someone had to generate honestly and then forget.

What *is* trusted is smaller and checkable: the two program ImageIDs. Those are
content-addressed, so an evaluator who rebuilds from source either gets the same
identity or learns immediately that the committed binary is not the source. A
clean `cargo risczero build` reproduces `5bb40082…` exactly — see
[`DEPLOYMENT.md`](DEPLOYMENT.md), which also shows the deployed bytes hashing to
the deployment transaction id.

## Residual risks and non-goals

- **A malicious creator.** Whoever builds the member set knows every salt, and
  in the demo flow also generates every secret. A real deployment has each member
  generate their own `msk` and hand over only the derived `account_id`; the
  creator never needs a secret. The protocol protects members from each other and
  from observers, not from a creator who fabricates the set. The set's honesty is
  attested by the fact that it is committed publicly before any approval.
- **No membership rotation.** The member set is fixed at creation. Adding or
  removing a member means a new multisig. This is out of scope by design;
  rotation with forward-privacy is a genuinely harder problem.
- **No proposal expiry.** Approvals do not decay. A proposal approved to
  threshold can be executed at any later time by anyone. If that is undesirable,
  bind a deadline into the action bytes and enforce it in the consuming program.
- **The action is opaque to this protocol.** `multisig-core` guarantees approvals
  bind to exact bytes; it does not interpret them. A downstream integration that
  reads the execution marker must parse the action itself and is responsible for
  its own validation.
- **No audit.** This is prize work, not audited production code.
