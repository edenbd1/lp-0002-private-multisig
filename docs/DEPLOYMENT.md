# Deployment

## Status

**Deployed, and the full lifecycle has been run on the public LEZ testnet.**
Every hash below is live; re-check any of them with `getTransaction` against
`https://testnet.lez.logos.co`.

## The lifecycle, on chain

A **2-of-3** multisig: created, a proposal published, two approvals gathered on
the privacy-preserving path, and executed.

| Step | Transaction | On the explorer |
|---|---|---|
| deploy `membership_lez` | [`fb8eb10f7f394286c109cb6502a1c95294180523f30d06f707fc087a589bea98`](https://explorer.testnet.lez.logos.co/transaction/fb8eb10f7f394286c109cb6502a1c95294180523f30d06f707fc087a589bea98) | renders |
| deploy `multisig_verifier` | [`517efe12a0b592abe4d21a03246866b95c4379483e87af62fd9f26f7b8fe45ff`](https://explorer.testnet.lez.logos.co/transaction/517efe12a0b592abe4d21a03246866b95c4379483e87af62fd9f26f7b8fe45ff) | renders |
| `create_multisig` | [`2930c1db4521b7c0b912278f4025e430704cfb9a7ebfcb5d22c374fd7ce85b70`](https://explorer.testnet.lez.logos.co/transaction/2930c1db4521b7c0b912278f4025e430704cfb9a7ebfcb5d22c374fd7ce85b70) | renders |
| `create_proposal` | [`68d5127e1e5570936f8d78e9a2da4d485562566cd8b7487a59322bf059406978`](https://explorer.testnet.lez.logos.co/transaction/68d5127e1e5570936f8d78e9a2da4d485562566cd8b7487a59322bf059406978) | renders |
| `approve` (member A, **privacy tx**) | [`41f5bb99346a0bef6aa0c69243473a554b84f0f0ad65e460bbb6890b11644942`](https://explorer.testnet.lez.logos.co/transaction/41f5bb99346a0bef6aa0c69243473a554b84f0f0ad65e460bbb6890b11644942) | renders, `Privacy-Preserving Transaction` |
| `approve` (member B, **privacy tx**) | [`ae006465f5f945b8ba2666f28a5357d0a2aab4af05508c9c2811e0101d0ac649`](https://explorer.testnet.lez.logos.co/transaction/ae006465f5f945b8ba2666f28a5357d0a2aab4af05508c9c2811e0101d0ac649) | renders, `Privacy-Preserving Transaction` |
| `execute` | [`b43e46505f571e31d6051f7da43563db605b6a74b90c670da2d3582d53412ecd`](https://explorer.testnet.lez.logos.co/transaction/b43e46505f571e31d6051f7da43563db605b6a74b90c670da2d3582d53412ecd) | renders |

### How the explorer column was measured

All seven are live, and all seven render. `getTransaction` returns every one of
them over RPC, and clicking any hash above shows its page.

**A correction, because this document said the opposite.** It used to state that
the explorer was a WASM application serving the same 2416-byte shell for every
`/transaction/<hash>` URL and rendering client-side, so that `curl` could not
distinguish an indexed transaction from an impossible one. That was true when
`scripts/check-explorer.py` was written — it is why the script drives a browser
at all — and it is not true now. Re-measured **2026-08-15**, the explorer
server-side renders, so a one-line `curl` does separate the two cases:

```bash
# a real transaction: ~366 kB, and the body carries its type and proof size
curl -s https://explorer.testnet.lez.logos.co/transaction/41f5bb99346a0bef6aa0c69243473a554b84f0f0ad65e460bbb6890b11644942 | wc -c

# a hash that cannot exist: 2416 bytes, and the body says why
curl -s "https://explorer.testnet.lez.logos.co/transaction/$(python3 -c 'print("ff"*32)')" | wc -c
```

The second returns `Failed to load transaction: error running server function:
Transaction not found`. Compare the bodies rather than only the sizes — a size
is a weaker signal that happens to work today.

`scripts/check-explorer.py` remains the stronger check and is worth running as a
second opinion: it renders each page headless and compares it against the same
impossible hash as a control, so it reads the DOM a reviewer actually sees, and
it keeps working if the explorer returns to client-side rendering. If that
control ever renders as a *found* transaction the script aborts rather than
report anything, because the baseline every verdict rests on would be invalid.

**The explorer is still a separate index, and it lags the sequencer.** A hash
submitted minutes ago can read `Transaction not found` there while
`getTransaction` already returns it. That is an indexing delay, not a gap, and
it affects anything recent by anyone — it is not specific to this submission and
not a property of privacy transactions. The script prints the explorer's most
recent indexed block so the current lag can be read off against `getLastBlockId`
rather than quoted from here, where it would go stale.

So do not judge these by clicking alone, in either direction: a link that does
not render is evidence about the indexer, and a link that does render does not
prove ownership of anything. Check the chain directly as well:

```bash
curl -s -X POST https://testnet.lez.logos.co -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getTransaction","params":["<hash>"]}'
```

A non-`null` `"result"` means the transaction is on chain — on LEZ v0.2.4 that
result is a decoded object, not a string, so test whether it is null rather than
what type it is. `./scripts/verify-onchain.sh` is the stronger check: it reads
the five accounts the lifecycle produced and confirms the verifier program owns
them, which no amount of transaction-fetching can fake.

If the testnet is reset again and the hashes go cold, `./scripts/deploy-and-run.sh`
re-runs the whole lifecycle. The two deployment hashes come back identical,
because a deployment hash is `SHA256(borsh(bytecode))` and the binaries are
committed; the five lifecycle hashes are signed with a nonce and will be new.

Multisig id `df2c8c3d0a036414cd819aa04c023c489f4a5ca2c0e7e99cca80363d14ab8472`,
member root `2e6fa5feaacec254fe7a2124cf6a2e62f7e5be8f0e14b37a0e4b42767ccc5a7d`,
config hash `92100d32ab976481e74fcaf28d1ab99f5f1be27421e190bb07ba09185a305475`
(which is what anchors the root *and* the threshold in the multisig's address).
The action was `transfer 100 LEZ to the grants treasury`.

## The accounts, and what they prove

All five are owned by the verifier program
(ProgramId `8200b45b,1dc3dd73,ad5b2b1c,afda3588,027e564c,9c05bb9d,3be8c720,826f96a5`).
Read them yourself with `./scripts/verify-onchain.sh`, or derive the addresses
with `scripts/pda.py` and query `getAccount`.

| Account | Address |
|---|---|
| multisig | `4wqJXoEhqqqYknt1s7gHcgBL6pkfwNJDfhbVVeAqwtnX` |
| proposal | `E11Awng7j59dVft83VVrwftXp41roJPKY5QRMb45Zcoe` |
| approval marker A | `DaG2Qan1ie5YhEpcti2LMCsvbkYi7WjWxnNKvxiqxi7B` |
| approval marker B | `FMj5yL8cpcrQzN7xhENHC2vysTrNwbtokPbTYjr98rPt` |
| execution marker | `CpiuicNDii6uCeMXtjd1W6hek6Vq35HJ7k3mz1Q82Fui` |

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
| `multisig_verifier` | `5bb4008273ddc31d1c2b5bad8835daaf4c567e029dbb059c20c7e83ba5966f82` | `8200b45b,1dc3dd73,ad5b2b1c,afda3588,027e564c,9c05bb9d,3be8c720,826f96a5` |

Verify for yourself:

```bash
spel program-id artifacts/programs/membership_lez.bin
spel program-id artifacts/programs/multisig_verifier.bin
```

**The build is reproducible.** Rebuilding the verifier guest from a clean
`cargo risczero build` reproduces ImageID
`5bb4008273ddc31d1c2b5bad8835daaf4c567e029dbb059c20c7e83ba5966f82` exactly —
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

**The accounts are on the block explorer even though the approvals are not**,
and the distinction matters because a reader who takes the warning below as
"the explorer will not help" stops looking one click too early. The explorer has
an account view, and it renders all three PDAs with their owner:

- multisig `/account/4wqJXoEhqqqYknt1s7gHcgBL6pkfwNJDfhbVVeAqwtnX`
- proposal `/account/E11Awng7j59dVft83VVrwftXp41roJPKY5QRMb45Zcoe`
- execution marker `/account/CpiuicNDii6uCeMXtjd1W6hek6Vq35HJ7k3mz1Q82Fui`

Each shows `Program Owner: 7AyJ7x4DuAa58ALGqLXYwqdhEvQLCz5A2GFdcDrwyzUZ`, which
base58-decodes to `5bb4008273ddc31d1c2b5bad8835daaf4c567e029dbb059c20c7e83ba5966f82`
— exactly what `spel program-id artifacts/programs/multisig_verifier.bin` prints
for the binary committed here. That is the whole claim, checkable in a browser.

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
