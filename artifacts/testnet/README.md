# The deployed lifecycle

These files are the client-side record of the LP-0002 lifecycle that ran on the
public LEZ testnet on **2026-08-25**, against verifier ImageID
`a8a87f8b456299144236f42f194f1b85c11265763a976c055a7f471b61500750` — a **3-of-3**
multisig with a spending tier, its treasury funded, a proposal published, **two**
approvals gathered on the privacy-preserving path, and executed. Eight
transactions, blocks 23028-23066. [`docs/DEPLOYMENT.md`](../../docs/DEPLOYMENT.md)
has every hash and every address.

**Two approvals against a threshold of three**, because the tier this multisig
anchors — at or below 1, two approvals — covers the amount proposed. That is the
tier doing something, counted in proofs not generated rather than asserted in
prose, and `msig status` below reports both numbers.

No member secret is here. `members.json` holds the member secret keys and is
deliberately not committed. One thing that *is* here deserves naming rather than
hiding behind "public": each approval record carries `member_index` beside its
`nullifier_hex`, so a reader of this directory learns which member produced which
nullifier. The chain records no such link — that is the whole point of the
design, and `scripts/verify-onchain.sh` reads the markers back with nothing but
`proposal_ref` and a nullifier in them. This is the *creator's* bookkeeping for a
member set the creator generated, and it is committed because pointing `msig` or
the Basecamp module at this directory is the only way to see the deployed state
without secrets. A real deployment's operator would not publish it.

Point `msig` at this directory and it reads:

```bash
msig status --dir artifacts/testnet \
    --proposal-id 86bb53f6851ef77b03cdfc2e04f9ff2d930e11dbee1b33492018b95a3d65560d
```

which prints the payment, the threshold, and the two marker seeds — the same
values the chain re-derived. The Basecamp module reads the same two files, so
pointing **Multisig folder** here shows the deployed state.

Read the chain rather than these files:

```bash
./scripts/verify-onchain-lifecycle.sh          # the eight transactions
./scripts/verify-onchain.sh artifacts/testnet 86bb53f6851ef77b03cdfc2e04f9ff2d930e11dbee1b33492018b95a3d65560d
```

**An earlier run's files used to live here** and were replaced rather than kept
alongside, because two `multisig.json` in one directory is a directory `msig`
cannot read. The addresses and hashes that run produced are not lost: they are
listed under *Superseded addresses* in
[`docs/DEPLOYMENT.md`](../../docs/DEPLOYMENT.md), which is where a stale link
found elsewhere can be recognised for what it is.
