# Deployment

> ## Redeployed 2026-08-03, after a bypass found by cross-review
>
> The verifier that was live until 2026-08-03 carried a **threshold and
> membership bypass**: `config_hash` appeared in neither `proposal_ref` nor the
> proposal account's address, and `create_multisig` places no ownership
> constraint on `multisig_id` — by design, anyone may create a multisig. An
> outsider could therefore create a 1-of-1 config **under a victim's multisig
> id**, approve the victim's proposal against their own member set, and execute
> a 3-of-5 on that single approval.
>
> Everything on this page is the **fixed** build: verifier ImageID
> `00286a88…`, redeployed, and the whole lifecycle re-run against it. The
> previous deployment is gone from this page on purpose — it was evidence for a
> program that could be bypassed. The finding, why the earlier fix missed this
> variant, and the two regression tests that reproduce it are in
> [`security.md`](security.md).

## Status

**Deployed, and the full lifecycle has been run on the public LEZ testnet.**
Every hash below is live; re-check any of them with `getTransaction` against
`https://testnet.lez.logos.co`.

## The lifecycle, on chain

A **2-of-3** multisig: created, a proposal published, two approvals gathered on
the privacy-preserving path, and executed.

| Step | Transaction | On the explorer |
|---|---|---|
| deploy `membership_lez` | `64098974b7d28f4facf1218e771d27c6163f7fb7ce3bd4f218df6db42ace6dde` | [link](https://explorer.testnet.lez.logos.co/transaction/64098974b7d28f4facf1218e771d27c6163f7fb7ce3bd4f218df6db42ace6dde) |
| deploy `multisig_verifier` | `78a68aa2b0bdcc5c99778fdfa52469f6b367c36f28489f4a40418873dd9ab1ca` | [link](https://explorer.testnet.lez.logos.co/transaction/78a68aa2b0bdcc5c99778fdfa52469f6b367c36f28489f4a40418873dd9ab1ca) |
| `create_multisig` | `09e8245d8673dd2de0621f3e843822cba30f5297c87a7224da950c093f5fd60e` | not indexed |
| `create_proposal` | `88698af7fd639229563b22f59ca00d3d5e5f30baea20677ca82e611add7c17f5` | not indexed |
| `approve` (member A, **privacy tx**) | `0b0a13dd39b57222d0d641863061db81e925b7f21c635701139f73f7d0a356a7` | not indexed |
| `approve` (member B, **privacy tx**) | `ff22f4c33bce23f6454be09872f14fae8672ea334d6cfda66270d4d8a09259b7` | not indexed |
| `execute` | `18d428af895cd992899cbbec0ad5eb326e7cf0388f985c609ac510f05cba3e98` | not indexed |

### Why five of them say "not indexed"

**All seven are live.** `getTransaction` returns every one of them; the last
column is about the explorer's indexer, not about the chain.

What was measured, rather than assumed: the two deployment URLs return the full
transaction with the bytecode inline, and the other five return exactly the same
2416-byte page shell that a hash which *cannot exist* returns. Identical byte
counts, so the explorer is not saying "this transaction is empty" — it is saying
it has no record of the hash at all, while `getTransaction` returns it.

The indexer's coverage is uneven rather than absent, and that is checkable
here rather than asserted: other public program invocations on this testnet do
render, and one of the two deployments above was submitted *after* the five
lifecycle transactions and still renders. So this is not an indexing lag and it
is not specific to this submission — re-submitting the same transactions would
produce the same result.

So do not judge these by clicking. Check any hash directly:

```bash
curl -s -X POST https://testnet.lez.logos.co -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getTransaction","params":["<hash>"]}'
```

A `"result"` that is a base64 string means the transaction is on chain; a
`null` means it is not. `./scripts/verify-onchain.sh` does the stronger check —
it reads the five accounts the lifecycle produced and confirms the verifier
program owns them, which no amount of transaction-fetching can fake.

If the testnet is reset again and the hashes go cold, `./scripts/deploy-and-run.sh`
re-runs the whole lifecycle. The two deployment hashes come back identical,
because a deployment hash is `SHA256(borsh(bytecode))` and the binaries are
committed; the five lifecycle hashes are signed with a nonce and will be new.

Multisig id `e0ea615c1179270ea9fbc0c7c7978bac79996f7e154501b8c8683e092646ca2e`,
member root `1ae4233b5c8c256948196373b0c0169efce584a8cab1236e45f3c140caf0428e`,
config hash `1ae8ce6b1135b9e01f89cd1d200ef6bd6e9e8e29fe44d115cb86ea566991269f`
(which is what anchors the root *and* the threshold in the multisig's address).
The action was `transfer 100 LEZ to the grants treasury`.

## The accounts, and what they prove

All five are owned by the verifier program
(the verifier ProgramId).
Read them yourself with `./scripts/verify-onchain.sh`, or derive the addresses
with `scripts/pda.py` and query `getAccount`.

| Account | Address |
|---|---|
| multisig | `9GwWLZThKirW56FPWTCpXPhZ1gTrEy7HYuk5jerQvUTR` |
| proposal | `2G8eAKAgTR7JjkDrzioPc4nJmHDx3MgK5Kk9xDaH6LHN` |
| approval marker A | `yyNX12APVKBCh9yJyR98rG7qyM8mrfBda8m34AC115N` |
| approval marker B | `BWGfzw5EescKdS53nkawCQdkhu5koW2LfXPCFnS45Vrn` |
| execution marker | `DXGgrR6R52Et4wjNDQyLeSxtvStpj69xGNC2ZKv4kuWc` |

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
| `membership_lez` | `a48ecc5289404ad01fd6d6fd1d79eaebb8d2f0fe4f2dc2ebbc85003ee82af3d6` | `52cc8ea4,d04a4089,fdd6d61f,ebea791d,fef0d2b8,ebc22d4f,3e0085bc,d6f32ae8` |
| `multisig_verifier` | `00286a889dabd8c3d7fdfb058c5935f5d46172946249c98da73953e3f136ed5d` | the verifier ProgramId |

Verify for yourself:

```bash
spel program-id artifacts/programs/membership_lez.bin
spel program-id artifacts/programs/multisig_verifier.bin
```

**The build is reproducible.** Rebuilding the verifier guest from a clean
`cargo risczero build` reproduces ImageID
`00286a889dabd8c3d7fdfb058c5935f5d46172946249c98da73953e3f136ed5d` exactly —
checked, not assumed. That matters because the ImageID *is* the program's
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
| `wallet` | LEZ `v0.2.0` | Wallet home is `LEE_WALLET_HOME_DIR`, default `~/.lee/wallet`. A pre-v0.2.0 wallet looks in `~/.nssa/wallet` and will not work |
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

./scripts/deploy-and-run.sh
```

It deploys both programs, creates a multisig, publishes a proposal, gathers
`THRESHOLD` approvals on the privacy-preserving path, and executes. Every
transaction hash is appended to `.testnet/lifecycle.tsv`.

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

This reads the multisig PDA, the proposal PDA, each approval marker, and the
execution marker over JSON-RPC, and reports each one's owner.

**Do not expect to find approvals on the block explorer**, for two separate
reasons that are worth keeping apart.

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
