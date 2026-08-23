# Account layout

Every account the verifier program claims carries a record of what it stands
for. This document is the record's specification: a byte offset and a width per
field, so a reader with `getAccount` and nothing else can decode one by hand.

That is not a figure of speech. `crates/multisig-core/src/state.rs` decodes from
these offsets with no `borsh` dependency, and
`crates/multisig-verifier-tests/tests/state_and_transfer.rs` runs the real
program and decodes its real output through that module. If the program's
encoding and this table ever disagree, those tests fail. The table is checked,
not described.

`scripts/decode-account.py` is the same thing for the command line:

```bash
scripts/decode-account.py proposal <address>
```

## Conventions

* Written with `borsh`, which for these types means **fields back to back, no
  padding, no length prefix on fixed-width fields, little-endian integers**.
* Offsets are absolute, from byte 0 of the account's `data`.
* Every record opens with a **format byte**. It is `1` for every layout below.
  A decoder that meets any other value must stop rather than guess.
* Fixed-width records have an exact length. Trailing bytes mean the writer and
  the reader disagree, so the fields already parsed cannot be trusted either.
* An account that was never written reads as `data: []` — zero bytes, which is
  "nobody has claimed this" and not "this record is corrupt".

## `MultisigRecord` — 133 bytes

At the PDA seeded by `[multisig_id, config_hash]`.

| Offset | Width | Field | Type | Meaning |
|---:|---:|---|---|---|
| 0 | 1 | `format` | `u8` | `1` |
| 1 | 32 | `multisig_id` | bytes | the instance id |
| 33 | 32 | `member_root` | bytes | Merkle root over the member leaves |
| 65 | 4 | `threshold` | `u32` LE | M, the approvals required |
| 69 | 32 | `treasury` | account id | the PDA this multisig pays from |
| 101 | 32 | `authority` | account id | who created it |

`member_root` and `threshold` are already *anchored* by the address —
`config_hash = SHA256(MULTISIG_CONFIG_PREFIX ‖ member_root ‖ threshold_le)`, and
that hash is a PDA seed. Storing them adds no security and a great deal of
readability: without the record, a stranger holding the address can *confirm* a
guess at the configuration and cannot *discover* it.

`authority` is recorded and never trusted. Creation is permissionless, and being
named here grants nothing: no instruction reads this field.

## `TreasuryRecord` — 65 bytes

At the PDA seeded by `[multisig_id, config_hash, "treasury"]`, where the literal
is its ASCII bytes zero-padded to 32 (SPEL's `seed_from_str`). Derive it with:

```bash
scripts/pda.py artifacts/programs/multisig_verifier.bin \
    <multisig_id> <config_hash> str:treasury
```

| Offset | Width | Field | Type | Meaning |
|---:|---:|---|---|---|
| 0 | 1 | `format` | `u8` | `1` |
| 1 | 32 | `multisig_id` | bytes | which multisig it belongs to |
| 33 | 32 | `config_hash` | bytes | under which configuration |

The interesting number in this account is not in `data` at all: it is
`balance`. The treasury exists so that the program owns a balance, because LEZ
refuses a post-state that *decreases* a balance the executing program does not
own and permits any program to *increase* any balance
(`lee/state_machine/core/src/program/mod.rs:706-717`). A payout is therefore the
program debiting its own account and crediting the payee, with no transfer
authority over anybody.

## `ProposalRecord` — 210 bytes

At the PDA seeded by `[multisig_id, config_hash, proposal_ref]`.

| Offset | Width | Field | Type | Meaning |
|---:|---:|---|---|---|
| 0 | 1 | `format` | `u8` | `1` |
| 1 | 32 | `multisig_id` | bytes | |
| 33 | 32 | `config_hash` | bytes | |
| 65 | 32 | `proposal_id` | bytes | chosen by the proposer |
| 97 | 32 | `action_hash` | bytes | `SHA256(ACTION_PREFIX ‖ multisig_id ‖ action)` |
| 129 | 32 | `recipient` | account id | who the treasury pays |
| 161 | 16 | `amount` | `u128` LE | how much |
| 177 | 32 | `memo_hash` | bytes | `SHA256(ACTION_MEMO_PREFIX ‖ memo)` |
| 209 | 1 | `status` | `u8` | `0` open, `1` executed |

**The action, and why it is stored as fields rather than as bytes.** `action`
used to be an opaque blob the protocol bound by hash and never looked inside.
That is enough to make approvals unforgeable and not enough to make an execution
*do* anything. A v1 action is an 81-byte record:

```
format(1) ‖ recipient(32) ‖ amount_le(16) ‖ memo_hash(32)
```

`create_proposal` refuses fields that do not hash to the `action_hash` it was
handed, and refuses an `action_hash` that does not re-derive the `proposal_ref`
its own address is seeded by. `execute` repeats both checks against the stored
record before it pays. So the bytes here are not believed: they are re-tied to
the address every approval was bound to, on every execution.

**The memo** is the sentence a member reads before approving — "transfer 250 LEZ
to the grants treasury". It is bound by its digest so that it is as unforgeable
as the recipient and the amount, without letting an arbitrary-length string into
a fixed-width record. The text itself lives in the client's
`proposals/<id>.json`.

**`status`** is a readable mirror, not a second gate. The authoritative replay
guard is that `execute` claims the execution marker with `init`, and LEZ refuses
to claim an account that is no longer default-owned. The flag is checked as well,
so the two cannot disagree about whether a proposal has been paid.

## `ApprovalMarkerRecord` — 65 bytes

At the PDA seeded by `SHA256(APPROVAL_MARKER_PREFIX ‖ proposal_ref ‖ nullifier)`.

| Offset | Width | Field | Type | Meaning |
|---:|---:|---|---|---|
| 0 | 1 | `format` | `u8` | `1` |
| 1 | 32 | `proposal_ref` | bytes | which proposal was approved |
| 33 | 32 | `nullifier` | bytes | which approval was spent |

Both values are already implied by the address; writing them makes the marker
self-describing to somebody who has only found the account. It carries **nothing
else**, and that is a privacy requirement rather than a space saving: this is a
public account, and anything derived from the witness — the member's secret,
identifier, salt or leaf index — would be readable by everyone, including the
other members.

The nullifier is `SHA256(APPROVAL_NULLIFIER_PREFIX ‖ proposal_ref ‖ msk)`. An
observer who knows the entire candidate member set still cannot invert it.

## `ExecutionMarkerRecord` — 86 bytes, plus 32 per nullifier

At the PDA seeded by `SHA256(EXECUTION_MARKER_PREFIX ‖ proposal_ref)`.

| Offset | Width | Field | Type | Meaning |
|---:|---:|---|---|---|
| 0 | 1 | `format` | `u8` | `1` |
| 1 | 32 | `proposal_ref` | bytes | which proposal executed |
| 33 | 32 | `recipient` | account id | who was paid |
| 65 | 16 | `amount` | `u128` LE | how much was paid |
| 81 | 1 | `status` | `u8` | `1` — the record exists only after the payout |
| 82 | 4 | `nullifier_count` | `u32` LE | M, as presented |
| 86 | 32 × M | `nullifiers` | bytes | the nullifiers consumed, in order |

The only variable-length record here, and the audit trail: M distinct
secret-bound values, each of which had to resolve to a marker PDA the program
itself owns before this account could come into existence.

A 3-of-5 execution therefore writes 86 + 96 = **182 bytes**.

## Reading one by hand

```bash
curl -s -X POST https://testnet.lez.logos.co \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getAccount","params":["<address>"]}'
```

`result.data` is a JSON array of byte values. Index into it with the table above.
`result.balance` on the treasury is the number that says whether a threshold ever
moved anything.
