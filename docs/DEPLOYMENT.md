# Deployment

## Status

**Deployed, and the full lifecycle has been run on the public LEZ testnet.**
Every hash below is live; re-check any of them with `getTransaction` against
`https://testnet.lez.logos.co`.

## The lifecycle, on chain

A **2-of-3** multisig: created, a proposal published, two approvals gathered on
the privacy-preserving path, and executed.

| Step | Transaction |
|---|---|
| deploy `membership_lez` | `64098974b7d28f4facf1218e771d27c6163f7fb7ce3bd4f218df6db42ace6dde` |
| deploy `multisig_verifier` | `dd8d0dc2206712468c163a018d40323fc953284401670ed43f9bad87f28bba69` |
| `create_multisig` | `109d8b42922e435268c31f8ba3f474c77b6b143488ac1137a90bca122e733791` |
| `create_proposal` | `4e0524446e074004b4f703c9d4c122c952c03a8b1cfbecbf1b987ffb18bf8fb6` |
| `approve` (member A, **privacy tx**) | `ead4460536488d41c0f23ebc0d1b8c3074142ebd425ca085f26e91e294094486` |
| `approve` (member B, **privacy tx**) | `c2fddecd9624fa9dd7c3734895eaf6027adbf44c49e7f8457bb0563fc9ae10b9` |
| `execute` | `f1ccbe5145b804ec43f0579a3dc3fd482eb30029992ca2b35e460caa34b6371f` |

Multisig id `ebbb2ec23288ffbe6c8fdd2c35eac05e621a1081b07caef0265976606b3bb0f5`,
member root `ed7da38f7f730d34dfdaaf28b08d5c16ad53b28b93367bfa732b62e21a20634c`,
config hash `763f849ca5067a6b3dc0e163b04adad8ef413d166fc0b9c124652dc99699d34d`
(which is what anchors the root *and* the threshold in the multisig's address).
The action was `transfer 100 LEZ to the grants treasury`.

## The accounts, and what they prove

All five are owned by the verifier program
(`2878411f,0c38e26e,efaa31b0,f73b079e,3be3ccbd,6d71006e,68f5bcf5,24ee3062`).
Read them yourself with `./scripts/verify-onchain.sh`, or derive the addresses
with `scripts/pda.py` and query `getAccount`.

| Account | Address |
|---|---|
| multisig | `4nf2HZtLRKCJh6eJHcsntgCntRXktMXxUw1BCwKhFvdR` |
| proposal | `4eWzzaDj668TCMyMC7BsWhZjVUKKm2SJ4kByThn2G9AZ` |
| approval marker A | `3SuRQe4gpDBMic5evbtsdXTHaSCCYhqT4GuLr7TFboF2` |
| approval marker B | `ECfwebcZyv2Vju3Fu2Q1mceu6E8yBtjLkXaGSNDS581r` |
| execution marker | `HaHrL1NPcjy2e4HBHP4Nq7gp7BXJd57jfSvUhi3y9fjo` |

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
| `multisig_verifier` | `1f4178286ee2380cb031aaef9e073bf7bdcce33b6e00716df5bcf5686230ee24` | `2878411f,0c38e26e,efaa31b0,f73b079e,3be3ccbd,6d71006e,68f5bcf5,24ee3062` |

Verify for yourself:

```bash
spel program-id artifacts/programs/membership_lez.bin
spel program-id artifacts/programs/multisig_verifier.bin
```

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
upwards of ten minutes; the run is dominated entirely by proving. Set `MEMBERS`
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

**Do not expect to find approvals on the block explorer.** A privacy-preserving
transaction publishes commitments and nullifiers and carries neither `program_id`
nor `instruction_data`, so the explorer's indexer has nothing to attribute. That
is the privacy property working exactly as designed — and it means reading the
accounts is the verification path, not searching the explorer.

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
