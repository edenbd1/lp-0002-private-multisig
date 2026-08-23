# LEZ account model compatibility

The prize exists because of a specific incompatibility. The public PoC,
[lez-multisig](https://github.com/jimmy-claw/lez-multisig), requires each member
account to be a **fresh zero-nonce keypair claimed by the multisig program**.
Private accounts cannot be that: they are owned by the privacy protocol and
their nonce advances on every use. This document states exactly how LP-0002
handles the two constraints the brief names — **nonce** and **`program_owner`**
— and where each one does and does not bind.

## The nonce constraint

### On membership: it never binds, because a member is not an account

The design does not work around the nonce constraint. It removes the situation
in which the constraint applies.

A member of an LP-0002 multisig is **not an account the program touches**. It is
a leaf in a Merkle tree:

```
npk        = SHA256(NPK_PREFIX || msk)
account_id = SHA256(PRIVATE_ACCOUNT_ID_PREFIX || npk || identifier)
leaf       = SHA256(LEAF_PREFIX || account_id || salt)
```

`account_id` is derived exactly as LEZ derives a private account id — same
prefix, same construction — so a member entry *denotes* a real shielded account.
But the verifier program never claims it, never writes to it, never reads its
nonce, and never requires it to be fresh. It never appears in an instruction's
account list at all. The only thing committed on chain is the Merkle root, and
the only thing proved is that some leaf under that root is owned by the secret
behind a nullifier.

So the PoC's requirement — *fresh, zero-nonce, claimed by the program* — has
nothing to attach to here. A member may hold an account that has been used a
thousand times, may use it elsewhere between approvals, and may never touch it
at all: approving does not consume, modify, or reference it.

### Where it does bind: the account that submits the transaction

The constraint reappears one level up, at submission, and this is the part that
cost real time to learn.

An approval is carried by a **privacy-preserving transaction**, which spends the
submitting account's commitment and produces a new one. Submitting two approvals
from the same private account fails *client-side*, before anything reaches the
sequencer, with

```
Invalid account_identities length, left: 4, right: 3
```

because the second proof is built against a commitment the first one already
consumed. The operational rule is therefore **one private account per
approval**, and it is enforced in `scripts/deploy-and-run.sh` through the
`APPROVERS` list rather than left to be rediscovered.

### Why that is not a privacy leak, and not a membership constraint either

The submitting account is **unrelated to the member who approved**. In the
`approve` instruction the approver is declared only as

```rust
#[account(signer)] approver: AccountWithMetadata,
```

and no check anywhere binds it to a leaf, a nullifier, or the member set. The
membership proof travels in the witness of the chained call; the signer merely
pays for and carries the transaction. Three consequences follow:

- A member can approve from **any** private account, including a fresh one made
  for the purpose, so the one-account-per-approval rule is a bookkeeping cost
  rather than a limit on who can participate.
- A member can have someone else submit for them. A relayer holding no member
  secret can carry an approval, which is what keeps timing correlation from
  being structural.
- `execute` is signed by an executor **who need not be a member at all** — which
  is what makes the completed execution unlinkable to any member's shielded
  account, as the criterion requires.

## The `program_owner` constraint

`program_owner` is not an obstacle here; it is load-bearing. The design uses it
for two distinct jobs.

### 1. Anchoring — an uninitialised PDA is detectable

Every account LP-0002 reads is a PDA, and a PDA that was never initialised
carries `DEFAULT_PROGRAM_ID` as its owner. That single fact is what turns
"invent your own member set" into a dead end:

```rust
if multisig.account.program_owner == nssa_core::program::DEFAULT_PROGRAM_ID {
    return Err(SpelError::custom(E_MULTISIG_NOT_ANCHORED, ...));   // 5003
}
```

A forged `member_root` or a lowered `threshold` changes `config_hash`, which
changes the multisig PDA address, which resolves to an account nobody ever
created — owner `DEFAULT_PROGRAM_ID`, rejected. The same test anchors the
proposal (`5004`). The forgery is not *checked and refused*; it is
**unrepresentable**, and `program_owner` is how the program can tell.

### 2. Proof of provenance — only `approve` can produce a marker the verifier owns

`execute` requires each approval marker to be owned by this program (`5013`):

```
E_APPROVAL_NOT_ANCHORED: an approval marker exists at the right address but was
never claimed by this program, so no membership proof was ever verified for it.
```

An attacker can compute any marker address they like — the seed derivation is
public. What they cannot do is make the **verifier program** the owner of that
account, because the only code path that initialises an approval marker is
`approve`, and `approve` only runs after LEZ's privacy circuit has composed the
chained membership call with a real `env::verify` whose receipt the sequencer
checked against the node-pinned `PRIVACY_PRESERVING_CIRCUIT_ID`.

So `program_owner` on a marker is not metadata. It is the on-chain evidence that
a membership proof was verified, and it is why counting markers is equivalent to
counting proofs.

### What `program_owner` publishes, and why that is acceptable

Decoding a real `approve` transaction from the testnet run, the verifier's
program id appears eight times — in the `program_owner` field of the accounts
carried in `public_post_states`. That is necessary and harmless: the whole point
of a marker is that it exists and is owned by this program, so its owner must be
public. *Which program a multisig uses* is public by design; *which member
approved* is what has to stay hidden, and it does. See
[`security.md`](security.md) for the full statement of what is and is not
hidden.

## Balances and data: what a program may write

`validate_execution` (`lee/state_machine/core/src/program/mod.rs:670-760`) is
eight rules. Three of them decide whether a threshold can move money at all, and
they are worth quoting because the first reading of them is usually wrong.

**Rule 5 — a program may not *decrease* a balance it does not own.**

```rust
if post.account.balance < pre.account.balance
    && account_program_owner != executing_program_id
{
    return Err(ExecutionValidationError::UnauthorizedBalanceDecrease { .. });
}
```

Note what is *not* there: no condition on increases. Any program may raise any
account's balance, and the payee needs no signature and no relationship with the
payer. So a payout does not need a transfer authority over anybody — provided the
payer is the program itself.

That is why each multisig owns a **treasury PDA**. Debiting it is this program
debiting its own account, which rule 5 permits; crediting the recipient is the
direction rule 5 says nothing about. `execute` therefore moves value with no
chained call and no second signature, and the recipient is not a signer.

The mirror image is `fund_treasury`, where the payer is a *user's* account. That
decrease is not this program's to make, so it is declared as a chained call into
the program that owns the balance — `authenticated_transfer`, **pinned by
ProgramId** rather than read off the account, because LEZ deployment is
permissionless and a caller could otherwise hand over an account owned by a
program they wrote.

**Rule 8 — total balance is preserved across the instruction.** The debit and the
credit must both appear in the same post-state list and must be equal. They are,
and `execute_preserves_the_total_balance` asserts it against the guest's own
journal, because a program that fails rule 8 fails it *after* a proof has been
generated.

**Rule 6 — data changes only if the program owns the account, or the pre-state is
default.**

```rust
if pre.account.data != post.account.data
    && pre.account != Account::default()
    && account_program_owner != executing_program_id
{
    return Err(ExecutionValidationError::UnauthorizedDataModification { .. });
}
```

The second clause is what lets an `init` account be claimed and written in one
instruction: its pre-state *is* default. Every record this program writes is
either into a PDA it is claiming right now (`create_multisig`, `create_proposal`,
`approve`, `execute`'s marker) or into one it already owns (`execute` flipping the
proposal's status). The layouts are in [`account-layout.md`](account-layout.md).

### Two shapes the platform refuses, learned by being refused

**An account cannot be initialised and paid into in the same transaction.** The
chained transfer reads a pre-state the initialisation has not written yet. So
`create_multisig` opens the treasury and `fund_treasury` fills it, in a second
transaction — the same split `lez-payment-streams` makes between
`initialize_vault` and `deposit`.

**A transaction cannot both chain a call that credits an account and write that
account's balance itself**: two competing post-states for one account, refused.
`fund_treasury` therefore declares the chained call and touches no balance of its
own, which `fund_treasury_chains_into_the_pinned_transfer_program` asserts by
comparing the pre and post balances it emits.

## Summary

| Constraint | Where it binds | How LP-0002 handles it |
|---|---|---|
| Private accounts are not zero-nonce | Would bind if members were program-claimed accounts | Members are Merkle leaves; the program never touches a member account |
| A privacy tx spends the submitter's commitment | The submitting account, once per approval | One private account per approval; the submitter is unrelated to the member |
| Private accounts are owned by the privacy protocol | Would bind if the program had to own member accounts | It does not — it owns only PDAs it creates itself |
| `program_owner` on an uninitialised PDA | Every account the program reads | Used as the anchoring test: forged roots and lowered thresholds resolve to unowned addresses (`5003`, `5004`) |
| `program_owner` on an approval marker | `execute` | Required to be this program (`5013`), which only `approve` can bring about, and only after a verified membership proof |
| Rule 5: no decreasing a balance you do not own | `execute`'s payout | The treasury is a PDA of this program, so the debit is self-owned; the credit needs no authority because increases are unrestricted |
| Rule 5, from the other side | `fund_treasury` | The funder's balance belongs to `authenticated_transfer`, so the decrease is a chained call into it — pinned by ProgramId (`5016`) |
| Rule 8: total balance preserved | `execute` | Debit and credit are equal and both post-states are emitted together, asserted against the guest's journal |
| Rule 6: data only on owned or default accounts | Every record written | Written either into a PDA being claimed in the same instruction, or into one the program already owns |
