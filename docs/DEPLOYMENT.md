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
| deploy `multisig_verifier` | `e24f5367521616f235acd26e3ee8937e8fc071335f187fab3ad282b6a691192f` |
| `create_multisig` | `de22a8c917774643969fec0b566082f701867b1359f18574fc8ee390badb3cdd` |
| `create_proposal` | `b7a4f74534cf9efc5da734c50b3c3ace7d1e7aa35aeaf86ca8f736dd566ea832` |
| `approve` (member A, **privacy tx**) | `6e035e4e702bcd241faa2ac304bc32178193bef25d66036a8bf7b0915d716347` |
| `approve` (member B, **privacy tx**) | `968c5d1ba1b828f93ee44a037b313f1c8c2b3ea30afb9302857b18fd8619dc55` |
| `execute` | `5817c49ce6ab86b5349ca2d55b95662f4cf7192b89be924d1f72c74f5d0e8b74` |

Multisig id `ccd1a480cdf52d79c6d720543e5f88a73027097397d626de8d4ec8ac0efe1ffe`,
member root `04a021a4d53a635a02eeebc193f2e6a3bad302cb3b5500a066038d0753db6fc2`,
config hash `af8baaf9198c0e717331834038f045c1a91ca6c80fb25159299e8bd93209f3c8`
(which is what anchors the root *and* the threshold in the multisig's address).
The action was `transfer 100 LEZ to the grants treasury`.

## The accounts, and what they prove

All five are owned by the verifier program
(the verifier ProgramId).
Read them yourself with `./scripts/verify-onchain.sh`, or derive the addresses
with `scripts/pda.py` and query `getAccount`.

| Account | Address |
|---|---|
| multisig | `chmP8jUqSHh2irhKVxBkM6GaGfLHmSq3TgCABziET3R` |
| proposal | `GdGHweUajfx7ocZNSC87WQNeYUP3Zm4EsNUiBqA4u3Kc` |
| approval marker A | `9q31RPufMoRe6pXcxrcuwFEJQN2Wnr2qV4HhXnV8a42r` |
| approval marker B | `GMgP7TMKoFVimMxX7PmtbeYG1dhGTGHUDh4F1yJmc8pv` |
| execution marker | `EsV6LpVUfR1iunep8g4etg1qTGQfzxbA1J7PjDBsFV5b` |

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
| `multisig_verifier` | `cf5724b0e8dabd4a1519f8d9ea7371d69e1d7e2f6d8c931f1e4b3110150d7982` | the verifier ProgramId |

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
