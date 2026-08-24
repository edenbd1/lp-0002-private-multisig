# Deployment

**Deployed to the public LEZ testnet on 2026-08-24**, in blocks 20856-20880.
Every hash and every address below was read back off the chain rather than
derived on paper: `./scripts/verify-onchain-lifecycle.sh` re-checks the
transactions from a clean clone, and `./scripts/verify-onchain.sh` re-reads the
accounts and decodes each record.

**Only the verifier was redeployed, and keeping it to one program took work.**
`membership_lez` links `multisig-core`, and with `lto = "fat"` even unreachable
code in a linked crate shifts the ELF — the first build of this revision moved
the membership ImageID to `f369cff3…` purely because the crate had gained an
account-layout module the membership guest never calls. Putting that vocabulary
behind a `records` feature the membership guest does not enable brings it back to
`56f784d6…` byte for byte.

So the deployed membership binary is still exactly the source committed here, the
deployment transaction it has had since block 4459 is still the live one, and
`MEMBERSHIP_LEZ_PROGRAM_ID` — the pin that stops a chained call reaching anything
other than the audited membership program — never moved. **One program needed
redeploying, not two.**

The verifier's ImageID did move, because giving the accounts state and giving the
threshold a treasury to spend changed the guest. On LEZ a program's identity
**is** its ImageID and every PDA address is derived from it, so the previous
deployment's five accounts sit at five different addresses. They are empty, they
are reachable forever, and they are listed under
[Superseded addresses](#superseded-addresses) rather than quietly dropped.

## The lifecycle, on chain

A **2-of-3** multisig: created, its treasury funded, a proposal published, two
approvals gathered on the privacy-preserving path, and executed — and executing
moved the money. Eight transactions, every one of them live on the public
sequencer.

| Step | Transaction | Block |
|---|---|---|
| deploy `membership_lez` | [`fb8eb10f7f394286c109cb6502a1c95294180523f30d06f707fc087a589bea98`](https://explorer.testnet.lez.logos.co/transaction/fb8eb10f7f394286c109cb6502a1c95294180523f30d06f707fc087a589bea98) | 4459 |
| deploy `multisig_verifier` | [`2d6f720e3c6dd8d876c8617eada5ddcd3c13a978b2edcb1921a3de73231e82e2`](https://explorer.testnet.lez.logos.co/transaction/2d6f720e3c6dd8d876c8617eada5ddcd3c13a978b2edcb1921a3de73231e82e2) | 20856 |
| `create_multisig` | [`a8d8422ae2c46566b15c31954647974d3e95eadbe0b560eac4ab609c9a25ab55`](https://explorer.testnet.lez.logos.co/transaction/a8d8422ae2c46566b15c31954647974d3e95eadbe0b560eac4ab609c9a25ab55) | 20857 |
| `fund_treasury` | [`2844eef12695ab0d3c6d55832e94ae316638dd7400735d2f393875a30bb6a5c2`](https://explorer.testnet.lez.logos.co/transaction/2844eef12695ab0d3c6d55832e94ae316638dd7400735d2f393875a30bb6a5c2) | 20858 |
| `create_proposal` | [`b194da9ba24e1a17a7bec0d64da0d252a96c6edc96208a778a7e77e71fed9826`](https://explorer.testnet.lez.logos.co/transaction/b194da9ba24e1a17a7bec0d64da0d252a96c6edc96208a778a7e77e71fed9826) | 20859 |
| `approve` (member A, **privacy tx**) | [`d13813094f36c1b60c02350adbc272ce5aa88dd7d87ab409a3e36436e70a91c0`](https://explorer.testnet.lez.logos.co/transaction/d13813094f36c1b60c02350adbc272ce5aa88dd7d87ab409a3e36436e70a91c0) | 20869 |
| `approve` (member B, **privacy tx**) | [`9f7c541c187c6ed284f67b0f5c6f0942de0ed98ff7e589dc81955ceed7219719`](https://explorer.testnet.lez.logos.co/transaction/9f7c541c187c6ed284f67b0f5c6f0942de0ed98ff7e589dc81955ceed7219719) | 20879 |
| `execute` | [`00ea68384758097dba8b648605b4ecf65d9535ba6b497af335d4fcf2be7f75ae`](https://explorer.testnet.lez.logos.co/transaction/00ea68384758097dba8b648605b4ecf65d9535ba6b497af335d4fcf2be7f75ae) | 20880 |

`membership_lez` sits three sequencer-lifetimes back at block 4459 because it was
never redeployed. A LEZ deployment hash is `SHA256(borsh(bytecode))`, the
membership bytecode is byte-identical to what is committed here, and deploying it
again would reproduce that same hash rather than mint a new one.

The two approvals took **614 s** and **609 s** of wall clock. `deploy-and-run.sh`
times each one and writes the figure into `.testnet/lifecycle.tsv` beside its
hash, so the timing comes out of the same run as the evidence. That number is
also the check that the run was real: proving on the privacy path costs minutes,
and an approval that returns in seconds proved nothing.

### What the explorer shows, measured rather than asserted

**Measured 2026-08-24, hours after the redeploy: the explorer has not indexed
these yet.** Its own index sat at block **20769** while the lifecycle above ran
in blocks 20856-20880, so seven of the eight hashes return the 2416-byte
`Failed to load transaction: error running server function: Transaction not
found` page, and the account view answers
`Program Owner: 11111111111111111111111111111111 … Data: 0 bytes` for accounts
the sequencer returns in full. Transactions from the earlier run — hundreds of
blocks older — render normally, so this is the indexer lagging the sequencer
rather than anything about these transactions. It affects everything recent by
anyone; it is not specific to this submission and not a property of privacy
transactions.

That is written down rather than waited out, because "renders" is a claim about
a moving index and would be stale either way. Re-measure the lag instead of
trusting this paragraph:

```bash
# where the explorer's index is
curl -s https://explorer.testnet.lez.logos.co/ | grep -o 'block_id[^,]*' | head -1
# where the sequencer is
curl -s -X POST https://testnet.lez.logos.co -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getLastBlockId","params":[]}'
```

**A correction, because this document once said the opposite.** It used to state
that the explorer was a WASM application serving the same 2416-byte shell for
every `/transaction/<hash>` URL and rendering client-side, so that `curl` could
not distinguish an indexed transaction from an impossible one. That was true when
`scripts/check-explorer.py` was written — it is why the script drives a browser
at all — and it is not true now. Re-measured **2026-08-15** and again
**2026-08-24**, the explorer server-side renders, so a one-line `curl` separates
an *indexed* transaction from an impossible one:

```bash
# an indexed transaction: ~366 kB, and the body carries its type and proof size
curl -s https://explorer.testnet.lez.logos.co/transaction/41f5bb99346a0bef6aa0c69243473a554b84f0f0ad65e460bbb6890b11644942 | wc -c

# a hash that cannot exist: 2416 bytes, and the body says why
curl -s "https://explorer.testnet.lez.logos.co/transaction/$(python3 -c 'print("ff"*32)')" | wc -c
```

Compare the bodies rather than only the sizes — a size is a weaker signal that
happens to work today. And note the limit of the whole method: a hash the
explorer has *not indexed yet* and a hash that *cannot exist* produce the same
2416-byte page. The explorer cannot tell those two apart at all, which is
precisely why the RPC is the primary check here and the explorer is the second
opinion rather than the other way round.

`scripts/check-explorer.py` is the stronger version of that second opinion: it
renders each page headless and compares it against the same impossible hash as a
control, so it reads the DOM a reviewer actually sees, and it keeps working if
the explorer returns to client-side rendering. If that control ever renders as a
*found* transaction the script aborts rather than report anything, because the
baseline every verdict rests on would be invalid. It reads its hash list out of
the table above, so it cannot drift from this document. It also prints the
explorer's most recent indexed block, so the current lag is read off rather than
quoted from here.

So do not judge these by clicking alone, in either direction: a link that does
not render is evidence about the indexer, and a link that does render does not
prove ownership of anything. Check the chain directly:

```bash
curl -s -X POST https://testnet.lez.logos.co -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getTransaction","params":["<hash>"]}'
```

A non-`null` `"result"` means the transaction is on chain — on LEZ v0.2.4 that
result is a decoded object, not a string, so test whether it is null rather than
what type it is. `./scripts/verify-onchain.sh` is the stronger check: it reads
the six accounts the lifecycle produced, confirms the verifier program owns them,
and decodes every field — which no amount of transaction-fetching can fake.

If the testnet is reset again and the hashes go cold, `./scripts/deploy-and-run.sh`
re-runs the whole lifecycle. The two deployment hashes come back identical,
because a deployment hash is `SHA256(borsh(bytecode))` and the binaries are
committed; the six lifecycle hashes are signed with a nonce and will be new.

Multisig id `95d44c351a586e2e37d748ca14e4108904cf1c627b3d8ec51bf9b38851662e04`,
member root `5372f377b5c1b94df954d7c0751edfa4bd2d76d6ab0f30f76ff9bc442bbc5970`,
config hash `3310d0fb1321e48f0a8a8fc7f539aa232dc783ce02d4fa384da6b360aad25269`
(which is what anchors the root *and* the threshold in the multisig's address),
proposal id `5bc829bb9a9efab4adb7446201f3a94113293bc51b63ef9356a254671f2b96fc`.
The threshold was **2 of 3**, and the action was *transfer 1 to
`8kexXda8j5hPegPeHXzUM9PhvjYNFLpN8wN8PvG5iDhn`*, memo
`transfer 1 LEZ to the grants treasury`. Executing it moved the amount: the
treasury went **2 → 1** and the recipient **0 → 1**.

## The accounts, and what they prove

All six are owned by the verifier program
(ProgramId `52b64613,119bac93,9a02b1d4,9455020d,82852362,92624012,42d23a5a,1e4eff98`).
Read them yourself with `./scripts/verify-onchain.sh`, or derive the addresses
with `scripts/pda.py` and query `getAccount`.

| Account | Address | Balance |
|---|---|---|
| multisig | `Hx7Ni2riURJfgng4QAXb3RBq9ZMjrvwj7JREjut9PuBC` | 0 |
| treasury | `xtngupTp3tcQU9faCbND73KhCpYqfBqKhmoepQAXoVx` | **2 → 1** |
| proposal | `9EjLSnKjXB4r8SgBEHcgqf36nkqfyUVXuqSsDzCTRvg7` | 0 |
| approval marker A | `Awd6RNVpMdPkvALi6ubdZyffKb3Bw3HimbUKtUP7YV6F` | 0 |
| approval marker B | `6dmnyiGZu6KtVy6jxyEMvb4HG8N59AuKey7wB9RjbCTV` | 0 |
| execution marker | `BP6buTDdFThPYWU3NFGSsD8k3ymewStQo4hj86tAwq1a` | 0 |

A seventh account is not the program's and is the point of the exercise: the
**recipient**, `8kexXda8j5hPegPeHXzUM9PhvjYNFLpN8wN8PvG5iDhn`, went **0 → 1**. It
is owned by the native transfer program rather than by the verifier, which is
what makes the balance it received spendable instead of burnt.

The treasury's balance is the one number here that nothing except a real
execution could produce. `fund_treasury` put 2 into it and `execute` moved 1 out,
and the two sides of that move are visible in two accounts owned by two different
programs.

Every one of the six decodes, field by field, per
[`account-layout.md`](account-layout.md):

```bash
scripts/decode-account.py multisig <address>
scripts/decode-account.py treasury <address>
scripts/decode-account.py proposal <address>
```

The treasury's address is derived like any other PDA, with the `literal` seed
spelled `str:`:

```bash
scripts/pda.py artifacts/programs/multisig_verifier.bin \
    <multisig_id> <config_hash> str:treasury
```

**The two approval markers are the whole claim.** Each exists only because
`approve` ran and claimed it; `approve` declares a `ChainedCall` to
`membership_lez`; and on the privacy path LEZ's circuit composes that call with a
real `env::verify` whose receipt the sequencer checks against the node-pinned
`PRIVACY_PRESERVING_CIRCUIT_ID`. So neither marker could exist without a
membership proof having been verified on chain.

Their addresses are `SHA256(prefix ‖ proposal_ref ‖ nullifier)` where each
nullifier is `SHA256(prefix ‖ proposal_ref ‖ msk)` for a member secret. Nothing
in the pair names a member. Read the accounts and you learn that two distinct
members approved, and nothing about which two.

The execution marker exists only because `execute` re-derived both marker
addresses from the nullifiers it was handed, found the verifier owns both, and
counted them against the threshold anchored in the multisig's address.

## Built artifacts

| Program | ImageID | ProgramId (hex) |
|---|---|---|
| `membership_lez` | `56f784d6b37f5cbac85d2eca3e28f56346e8739e6c22cb15a1b7165616758e31` | `d684f756,ba5c7fb3,ca2e5dc8,63f5283e,9e73e846,15cb226c,5616b7a1,318e7516` |
| `multisig_verifier` | `1346b65293ac9b11d4b1029a0d02559462238582124062925a3ad24298ff4e1e` | `52b64613,119bac93,9a02b1d4,9455020d,82852362,92624012,42d23a5a,1e4eff98` |

Verify for yourself:

```bash
spel program-id artifacts/programs/membership_lez.bin
spel program-id artifacts/programs/multisig_verifier.bin
```

**The build is reproducible.** Rebuilding the verifier guest from a clean
`cargo risczero build` reproduces ImageID
`1346b65293ac9b11d4b1029a0d02559462238582124062925a3ad24298ff4e1e` exactly —
checked, not assumed. The membership guest reproduces
`56f784d6b37f5cbac85d2eca3e28f56346e8739e6c22cb15a1b7165616758e31` — the id it had before this
revision and the one already on chain — which is the check that the `records`
feature gate does what it claims. That matters because the ImageID *is* the program's
identity on chain: an evaluator who rebuilds and gets the same id knows the
committed binary is the source in this repository, compiled.

**And the deployed bytes are the committed bytes.** The exact contents of each
committed binary appear inside its deployment transaction — offset 5, with the
payload exactly five bytes longer than the file. Fetch either deployment
transaction with `getTransaction`, base64-decode it, and search for the bytes of
`artifacts/programs/<name>.bin`: they are there verbatim.

A LEZ program-deployment transaction hash is `SHA256(borsh(bytecode))` —
content-addressed. Re-deploying a byte-identical binary reproduces the identical
hash, which is why the deploy step in `scripts/deploy-and-run.sh` is idempotent
and why a deploy link survives a re-run unchanged.

## Prerequisites

| Tool | Version | Note |
|---|---|---|
| `wallet` | LEZ `v0.2.4` | Wallet home is `LEE_WALLET_HOME_DIR`, default `~/.lee/wallet`. A pre-v0.2.0 wallet looks in `~/.nssa/wallet` and will not work |
| `spel` | `>= 0.6.0` | Older versions fail with `missing field 'accounts'` |
| `cargo-risczero` | `3.0.5` | Needs Docker for the guest builds |

Fund the signer with `wallet vault claim --amount <n>`.

## Rebuilding the programs

```bash
./scripts/build-programs.sh
```

This rebuilds both guests, copies them into `artifacts/programs/`, regenerates
the IDL, and **fails if `MEMBERSHIP_LEZ_PROGRAM_ID` in the verifier source has
drifted from the built membership binary**. That constant is what stops a chained
call from reaching anything other than the audited membership program, so drift is
a security bug rather than a nuisance. `--check` verifies the pin without
rebuilding.

## Running the lifecycle

```bash
export LEE_WALLET_HOME_DIR=~/.lee/wallet
export SIGNER=<funded Public account id>
# ONE private account per approval, comma separated — see the warning below.
export APPROVERS=<private-id-1>,<private-id-2>,<private-id-3>
# The account the proposal pays. It must be held by the native transfer program
# and must not be the signer.
export RECIPIENT=<Public account id>

./scripts/deploy-and-run.sh
```

It deploys both programs, creates a multisig **and its treasury**, funds the
treasury, publishes a proposal to pay `AMOUNT` out of it, gathers `THRESHOLD`
approvals on the privacy-preserving path, executes, and then reads both balances
back off the chain and **fails if they did not move by exactly `AMOUNT`**. Every
transaction hash is appended to `.testnet/lifecycle.tsv`, along with the treasury
address and the before/after balances.

### Making a recipient

The verifier refuses to pay an account the native transfer program does not own
(`E_RECIPIENT_UNUSABLE`, 5020): a balance in an account nobody can spend from is
a burn wearing a payment's clothes. A fresh public account is default-owned, so
it needs one instruction to become payable:

```bash
wallet account new public                                 # note the id
wallet auth-transfer init --account-id Public/<that id>   # claims it
```

Confirm it before running the lifecycle — `getAccount` must show a non-zero
`program_owner`:

```bash
curl -s -X POST https://testnet.lez.logos.co -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getAccount","params":["<recipient>"]}'
```

`scripts/e2e-local-sequencer.sh` does all of this itself against its throwaway
wallet, which is the reference for the sequence.

**Budget several hours.** Proving one approval with `RISC0_DEV_MODE=0` takes
about two and a half minutes; the run is dominated entirely by proving. Set `MEMBERS`
and `THRESHOLD` to shrink it.

> The wallet may print `Transaction NOT confirmed` for a privacy-preserving
> transaction whose proving outruns its polling window. The transaction lands
> anyway. The script polls `getTransaction` rather than trusting the CLI's
> verdict, and so should you.

> **Use a different private account for each approval.** A privacy transaction
> consumes the approver's commitment, so a second approval submitted from the
> same account fails in the *client-side* circuit, before anything is sent, with
>
> ```
> Guest panicked: assertion `left == right` failed: Invalid account_identities length
>   left: 4
>  right: 3
> ```
>
> Create them with `wallet account new private`. This also matches how a real
> deployment works: each member submits from their own account.

> Re-sync (`wallet account sync-private`) before each approval as well: a privacy
> transaction spends commitments, and a stale view produces a proof the sequencer
> drops. The script does both.

## Verifying independently

```bash
./scripts/verify-onchain.sh .testnet <proposal-id-hex>
```

This reads the multisig PDA, the treasury, the proposal PDA, each approval
marker, the execution marker and the recipient over JSON-RPC, reports each one's
owner, and decodes every record at the published offsets.

**The accounts are readable in a browser too, once the indexer catches up**, and
the distinction matters because a reader who takes the warning below as "the
explorer will not help" stops looking one click too early. The explorer has an
account view:

- multisig `/account/Hx7Ni2riURJfgng4QAXb3RBq9ZMjrvwj7JREjut9PuBC`
- treasury `/account/xtngupTp3tcQU9faCbND73KhCpYqfBqKhmoepQAXoVx`
- proposal `/account/9EjLSnKjXB4r8SgBEHcgqf36nkqfyUVXuqSsDzCTRvg7`
- execution marker `/account/BP6buTDdFThPYWU3NFGSsD8k3ymewStQo4hj86tAwq1a`

Each will show `Program Owner: 2JFHW18V15yv6C9Xs4AMERE92Q8FrRfRULSSMkQszA45`,
which base58-decodes to the verifier's ImageID
`1346b65293ac9b11d4b1029a0d02559462238582124062925a3ad24298ff4e1e` — exactly
what `spel program-id artifacts/programs/multisig_verifier.bin` prints for the
binary committed here.

**Measured 2026-08-24, they do not show that yet**, because the explorer's index
was still at block 20769 and these accounts were created in block 20857 onwards;
it answers `Program Owner: 11111111111111111111111111111111` and `Data: 0 bytes`
for all four. That is the indexer, not the chain — `getAccount` on the same
addresses returns the owner and the full record right now, and
`verify-onchain.sh` does exactly that. The superseded run's accounts, listed
below, are old enough to be indexed and do render, which is how the difference
was told apart from a broken address. Re-check the lag with the two commands in
[What the explorer shows](#what-the-explorer-shows-measured-rather-than-asserted)
before concluding anything from a blank page.

**Do not expect to find the approval transactions on the block explorer**, for
two separate reasons that are worth keeping apart.

The first is the indexer gap above, which applies to the public transactions
too and is nothing to do with this design.

The second is specific to approvals and is the point of the submission: a
privacy-preserving transaction publishes commitments and nullifiers and carries
neither `program_id` nor `instruction_data`, so there is nothing for an indexer
to attribute even once the explorer catches up. An approval is *supposed* to be
unattributable. That is why reading the accounts is the verification path here:
it proves two distinct members approved without ever making them findable.

## Invocation reference

The privacy path is selected by the `Private/` prefix on the approver, and the
chained membership program is registered with `--bin-membership`:

```bash
spel --idl idl/multisig_verifier.idl.json \
     --program artifacts/programs/multisig_verifier.bin \
     --bin-membership artifacts/programs/membership_lez.bin \
     -- approve --approver Private/<account-id> $(cat approve_0.args)
```

`create_multisig`, `create_proposal` and `execute` take a `Public/` authority and
need no `--bin-` registration: they declare no chained call.

`execute` takes its approval marker accounts as a **single comma-separated
flag**:

```bash
spel --idl idl/multisig_verifier.idl.json \
     --program artifacts/programs/multisig_verifier.bin \
     -- execute --executor Public/<signer> \
        --approvals <markerA>,<markerB> $(cat exec.args)
```

Repeating `--approvals` does **not** work and fails in a way that looks like a
program bug: `spel-cli/src/tx.rs` resolves variadic accounts with `last_value()`,
so only the final flag survives. The program then sees fewer accounts than
nullifiers and correctly rejects with `E_APPROVAL_COUNT_MISMATCH` (5009).

Argument encoding, since it is easy to get wrong:

- fixed `[u8; 32]` arguments take **hex**
- `Vec<u32>` (`--witness-words`) takes **comma-separated decimals**
- `Vec<[u8; 32]>` (`--approval-nullifiers`) takes **comma-separated hex**
- variadic accounts (`--approvals`) take **one comma-separated flag**

## Redeploying: what has to be re-run and what has to be renumbered

This is the runbook that produced the deployment at the top of this file, and it
is kept because the testnet gets reset. When it is re-run, this is the order, and
this is what goes stale.

**Re-run, in this order.**

1. `./scripts/build-programs.sh` — rebuild both guests, re-check the membership
   pin, regenerate the IDL and merge the error codes back into it. Confirm the
   two ImageIDs it prints match the "Built artifacts" table above; if they do
   not, everything below is derived from the wrong binary. The membership id
   must still be `56f784d6b37f5cbac85d2eca3e28f56346e8739e6c22cb15a1b7165616758e31`;
   if it is not, something outside the `records` feature changed in
   `multisig-core` and the membership program needs redeploying as well.
2. Provision a recipient — `wallet account new public`, then
   `wallet auth-transfer init`, then confirm a non-zero `program_owner`.
3. Fund the signer if needed — `wallet vault claim --amount <n>`. The lifecycle
   needs at least `FUND` (default 500) on top of whatever the deploys cost.
4. `SIGNER=… APPROVERS=… RECIPIENT=… ./scripts/deploy-and-run.sh` — deploy, create,
   fund, propose, approve to threshold, execute. It fails if the balances do not
   move by exactly `AMOUNT`, so a run that reports success moved the money.
5. `./scripts/verify-onchain.sh .testnet <proposal-id>` — reads all six accounts,
   confirms the verifier owns them, and decodes each record.

**Renumber afterwards.**

| Where | What |
|---|---|
| this file, "The lifecycle, on chain" | eight transaction hashes and their block numbers |
| this file, "The accounts" | six addresses, the treasury's balance before and after, and the recipient's |
| this file, multisig id / member root / config hash / proposal id | all four change: the id is random per run |
| this file, "Superseded addresses" | append the run being replaced, do not overwrite it — the list is cumulative |
| `README.md`, "On the public LEZ testnet" | the same eight hashes and the same six addresses |
| `README.md`, "Programs" | the verifier's ImageID, if it was not already updated by the rebuild. The membership id does not move unless `crates/membership-circuit/` or the non-`records` half of `multisig-core` does |
| `artifacts/testnet/` | replace its contents with the new run's `multisig.json`, `proposals/<id>.json` and `proposal_id` |
| `scripts/verify-onchain-lifecycle.sh` | the eight hashes and the expected variant of each |
| `app/README.md` | the `.lgx` SHA-256, if the package was rebuilt — `scripts/preflight.sh` recomputes every digest quoted beside a path and fails on a stale one, so this row is enforced rather than remembered |
| `docs/cu-costs.md` | the wall-clock rows, if the run is timed — the cycle table is current for as long as the ImageID is |
| `.testnet/` | delete `pda_addresses.txt`, `msig_id` and `prop_id` if a run leaves them behind. They are pre-`lifecycle.tsv` scratch files that no script writes any more, and a manifest that disagrees with the chain is worse than no manifest |

**What does not change.** The deployment hash of a program is
`SHA256(borsh(bytecode))`, so re-deploying a byte-identical binary reproduces an
identical hash and the deploy step is idempotent. The two deploy hashes in a
re-run are therefore a function of the committed binaries alone, and can be
computed before deploying anything:

```bash
python3 -c "
import hashlib,struct,sys
b=open(sys.argv[1],'rb').read()
print(hashlib.sha256(struct.pack('<I',len(b))+b).hexdigest())" \
  artifacts/programs/multisig_verifier.bin
```

For the binary committed here that is
`2d6f720e3c6dd8d876c8617eada5ddcd3c13a978b2edcb1921a3de73231e82e2`, and
`getTransaction` on it now resolves in block 20856. That equality — a hash
computed from a file in this repository, answered by the public sequencer — is
what ties the deployed program to the committed source, and
`scripts/verify-onchain-lifecycle.sh` recomputes it rather than reading it back
out of this table. `membership_lez` keeps
`fb8eb10f7f394286c109cb6502a1c95294180523f30d06f707fc087a589bea98` from block
4459 — its bytecode did not change, so its deployment could not.

## Superseded addresses

The previous deployment is on chain forever. Its accounts are still reachable,
they are **empty**, and anyone who follows a stale link from an older commit, an
older README or a mirror will land on exactly the account state this revision
exists to fix. They are listed here rather than deleted, because a reader who
finds a blank account and no explanation has to choose between "the indexer is
behind", "the submission is lying" and "this is an address the submission itself
retired" — and only one of those is true.

These belong to verifier ImageID
`5bb4008273ddc31d1c2b5bad8835daaf4c567e029dbb059c20c7e83ba5966f82`
(`7AyJ7x4DuAa58ALGqLXYwqdhEvQLCz5A2GFdcDrwyzUZ` in base58), which this repository
no longer contains. On LEZ a program's identity **is** its ImageID and every PDA
is derived from it, so these five addresses are not derivable from the binary
committed here at all — `scripts/verify-onchain.sh` derives from that binary and
therefore never looks at them.

That they are the old program's is checkable: `getAccount` on any of the five
returns
`program_owner = 8200b45b,1dc3dd73,ad5b2b1c,afda3588,027e564c,9c05bb9d,3be8c720,826f96a5`,
which is `5bb40082…` read as little-endian words, `balance 0` and `data 0 bytes`.
Measured 2026-08-24, all five.

| Account | Superseded address | State |
|---|---|---|
| multisig | `4wqJXoEhqqqYknt1s7gHcgBL6pkfwNJDfhbVVeAqwtnX` | claimed, no data |
| proposal | `E11Awng7j59dVft83VVrwftXp41roJPKY5QRMb45Zcoe` | claimed, no data |
| approval marker A | `DaG2Qan1ie5YhEpcti2LMCsvbkYi7WjWxnNKvxiqxi7B` | claimed, no data |
| approval marker B | `FMj5yL8cpcrQzN7xhENHC2vysTrNwbtokPbTYjr98rPt` | claimed, no data |
| execution marker | `CpiuicNDii6uCeMXtjd1W6hek6Vq35HJ7k3mz1Q82Fui` | claimed, no data |

"No data" is not a fault in the chain and not an outage: that verifier claimed
addresses and wrote nothing behind them, and there was no treasury, so nothing
moved. Removing that defect is what changed the guest and therefore the ImageID
and therefore every address above.

Its seven transactions are still live and still valid — they simply are not this
program's, having been signed against the other ImageID:

| Step | Superseded transaction | Block |
|---|---|---|
| deploy `multisig_verifier` | `517efe12a0b592abe4d21a03246866b95c4379483e87af62fd9f26f7b8fe45ff` | 4469 |
| `create_multisig` | `2930c1db4521b7c0b912278f4025e430704cfb9a7ebfcb5d22c374fd7ce85b70` | 4476 |
| `create_proposal` | `68d5127e1e5570936f8d78e9a2da4d485562566cd8b7487a59322bf059406978` | 4477 |
| `approve` (member A, privacy tx) | `41f5bb99346a0bef6aa0c69243473a554b84f0f0ad65e460bbb6890b11644942` | 4484 |
| `approve` (member B, privacy tx) | `ae006465f5f945b8ba2666f28a5357d0a2aab4af05508c9c2811e0101d0ac649` | 4492 |
| `execute` | `b43e46505f571e31d6051f7da43563db605b6a74b90c670da2d3582d53412ecd` | 4493 |

The seventh is the `membership_lez` deployment at block 4459, which is *not*
superseded: it is the live deployment of the binary committed here, and it
appears in the current table at the top of this file for that reason.

Its derived values, so a stale quote elsewhere can be recognised for what it is:
multisig id `df2c8c3d0a036414cd819aa04c023c489f4a5ca2c0e7e99cca80363d14ab8472`,
member root `2e6fa5feaacec254fe7a2124cf6a2e62f7e5be8f0e14b37a0e4b42767ccc5a7d`,
config hash `92100d32ab976481e74fcaf28d1ab99f5f1be27421e190bb07ba09185a305475`,
proposal id `68cea120676e6b36df3f4bb0f6b851f4ef15a9ee8ea96465a291b679e62c5447`,
action `transfer 100 LEZ to the grants treasury` — an untyped action string, which
is the schema `msig` carried before a proposal named a recipient and an amount.
