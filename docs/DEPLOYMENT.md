# Deployment

## Status

**The programs are built and their identities are pinned below. The public-testnet
lifecycle has not yet been run.** This page will carry the transaction hashes when
it has; nothing here claims an on-chain fact that is not yet true.

Everything that does not need a funded account is done and reproducible today:
both guests build, the verifier is exercised against the sequencer's own executor
by 15 adversarial tests, and `scripts/demo.sh` runs the full lifecycle end to end
from a clean clone.

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
export APPROVER=<authorized Private account id>

./scripts/deploy-and-run.sh
```

It deploys both programs, creates a 3-of-5 multisig, publishes a proposal,
gathers three approvals on the privacy-preserving path, and executes. Every
transaction hash is appended to `.testnet/lifecycle.tsv`.

**Budget several hours.** Proving one approval with `RISC0_DEV_MODE=0` takes
upwards of ten minutes; the run is dominated entirely by proving. Set `MEMBERS`
and `THRESHOLD` to shrink it.

> The wallet may print `Transaction NOT confirmed` for a privacy-preserving
> transaction whose proving outruns its polling window. The transaction lands
> anyway. The script polls `getTransaction` rather than trusting the CLI's
> verdict, and so should you.

> A privacy transaction spends the signer's commitment, so the approver's private
> account must be re-synced (`wallet account sync-private`) before each approval,
> or its membership proof is stale and the sequencer drops the transaction. The
> script does this between approvals.

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

Argument encoding, since it is easy to get wrong:

- fixed `[u8; 32]` arguments take **hex**
- `Vec<u32>` (`--witness-words`) takes **comma-separated decimals**
- `Vec<[u8; 32]>` (`--approval-nullifiers`) takes **comma-separated hex**
