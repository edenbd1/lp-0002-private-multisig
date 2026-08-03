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

## Summary

| Constraint | Where it binds | How LP-0002 handles it |
|---|---|---|
| Private accounts are not zero-nonce | Would bind if members were program-claimed accounts | Members are Merkle leaves; the program never touches a member account |
| A privacy tx spends the submitter's commitment | The submitting account, once per approval | One private account per approval; the submitter is unrelated to the member |
| Private accounts are owned by the privacy protocol | Would bind if the program had to own member accounts | It does not — it owns only PDAs it creates itself |
| `program_owner` on an uninitialised PDA | Every account the program reads | Used as the anchoring test: forged roots and lowered thresholds resolve to unowned addresses (`5003`, `5004`) |
| `program_owner` on an approval marker | `execute` | Required to be this program (`5013`), which only `approve` can bring about, and only after a verified membership proof |
