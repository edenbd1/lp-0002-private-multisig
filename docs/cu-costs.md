# Compute cost

Measured by replaying each instruction through the **sequencer's own executor**
— the same executor, input order and 32M-cycle session limit the chain applies
(`lee/state_machine/src/program.rs:55-110`).

Reproduce with:

```bash
cargo test -p multisig-verifier-tests -- --ignored --nocapture
```

## Results

Verifier ImageID `00286a889dabd8c3d7fdfb058c5935f5d46172946249c98da73953e3f136ed5d`.

| Instruction | Segments | User cycles | Proving cycles | Share of the public budget |
|---|---:|---:|---:|---:|
| `approve` | 1 | 336,937 | 524,288 | 1.56 % |
| `execute` (M=1) | 1 | 267,306 | 524,288 | 1.56 % |
| `execute` (M=3) | 1 | 363,354 | 524,288 | 1.56 % |
| `execute` (M=5) | 1 | 461,446 | 524,288 | 1.56 % |

`create_multisig` and `create_proposal` do strictly less work than `execute`
(M=1) — a hash, a comparison, and a PDA claim — and are bounded by it.

## Reading the numbers

**User cycles** is the guest's real work. **Proving cycles** is the padded
power-of-two segment the prover actually commits to, which is why all four rows
show the same 524,288: every instruction fits comfortably inside one segment, and
a segment is billed whole. The practical headroom is therefore large — the
budget share stays at 1.56 % until an instruction crosses 524,288 user cycles.

**`execute` scales linearly in M**, at roughly **48,500 user cycles per
additional approval**. That is one SHA-256 for the marker seed, one for the PDA
derivation, plus the pairwise distinctness comparisons. The distinctness check is
quadratic in M by choice: M is a multisig threshold, a small number, and a sort
would cost more cycles than it saves at these sizes. Extrapolating, a 10-of-N
execution lands near 700k user cycles and still fits one segment.

**What is excluded.** These are the guest's own cycles. They do not include
LEZ's privacy circuit recursively verifying the chained membership call — that
is the platform's cost, not this program's, and it is identical for any program
that composes a chained call.

**Wall-clock is a different number entirely.** Executing `approve` takes
milliseconds; *proving* it with `RISC0_DEV_MODE=0` is what costs time.

## Proof generation time, measured

Not estimated. `scripts/deploy-and-run.sh` times each approval and writes the
figure into `lifecycle.tsv` next to the transaction hash, so the number comes
out of the same run that produced the on-chain evidence.

A 2-of-3 lifecycle against a real standalone sequencer on localhost, Apple
silicon laptop, `RISC0_DEV_MODE=0`:

| Approval | Wall clock |
|---|---:|
| member 0 | **149 s** |
| member 1 | **154 s** |

That interval covers everything the member waits for: building and locally
re-verifying the Merkle witness, proving, submitting on the privacy path, and
the sequencer confirming the transaction in a block. It is not proving in
isolation — it is the number that matters to whoever is sitting there.

A 1-of-2 run of the same script measured 151 s for its single approval, so the
cost is per-approval and does not grow with the member set: the tree depth
changes the witness by a few words and nothing else.

Reproduce it with:

```bash
./scripts/e2e-local-sequencer.sh          # 2-of-3, about eight minutes total
```

**Against the public testnet, measured too.** The 2026-08-03 redeploy recorded
**360 s** and **179 s** for its two approvals. The gap between them is not the
chain: the first ran while an unrelated Risc0 proof was saturating the same
laptop, and the second did not. Take 179 s as the testnet figure and 360 s as
what contention costs — both are in that run's `lifecycle.tsv`, and neither is
rounded in this document's favour.

Proving itself is identical everywhere; what testnet adds is block time and
network latency per submission, and a wallet polling window that can expire
before a privacy transaction lands even though the transaction is fine.

## A note on the revision

These numbers are for the verifier *after* the proposal PDA was reseeded to
`[multisig_id, config_hash, proposal_ref]` and `config_hash` was folded into
`proposal_ref` (see [`security.md`](security.md)). The extra seed costs one more
SHA-256 in the address derivation SPEL performs before the body runs — about
150-650 cycles depending on the instruction, visible in the numbers above and
material to nothing: every instruction still fits one segment at 1.56 % of the
budget.

## Method note

The measurement executes rather than proves, deliberately. Proving would take
minutes per instruction and would not change the cycle counts — the cycle count
*is* the input to proving. Executing gives the same numbers in seconds and keeps
the measurement runnable in CI.
