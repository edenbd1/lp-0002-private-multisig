# The deployed lifecycle

These files are the client-side record of the LP-0002 lifecycle that ran on the
public LEZ testnet on **2026-08-24**, against verifier ImageID
`1346b65293ac9b11d4b1029a0d02559462238582124062925a3ad24298ff4e1e` — a 2-of-3
multisig created, its treasury funded, a proposal published, two approvals
gathered on the privacy-preserving path, and executed. Eight transactions, blocks
20856-20880. [`docs/DEPLOYMENT.md`](../../docs/DEPLOYMENT.md) has every hash and
every address.

No member secret is here. `members.json` holds the member secret keys and is
deliberately not committed; what is here is what a stranger is allowed to hold.

Point `msig` at this directory and it reads:

```bash
msig status --dir artifacts/testnet \
    --proposal-id 5bc829bb9a9efab4adb7446201f3a94113293bc51b63ef9356a254671f2b96fc
```

which prints the payment, the threshold, and the two marker seeds — the same
values the chain re-derived. The Basecamp module reads the same two files, so
pointing **Multisig folder** here shows the deployed state.

Read the chain rather than these files:

```bash
./scripts/verify-onchain-lifecycle.sh          # the eight transactions
./scripts/verify-onchain.sh .testnet 5bc829bb9a9efab4adb7446201f3a94113293bc51b63ef9356a254671f2b96fc
```

**An earlier run's files used to live here** and were replaced rather than kept
alongside, because two `multisig.json` in one directory is a directory `msig`
cannot read. The addresses and hashes that run produced are not lost: they are
listed under *Superseded addresses* in
[`docs/DEPLOYMENT.md`](../../docs/DEPLOYMENT.md), which is where a stale link
found elsewhere can be recognised for what it is.
