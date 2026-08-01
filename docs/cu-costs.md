# Compute cost

Measured by replaying each instruction through the **sequencer's own executor**
— the same executor, input order and 32M-cycle session limit the chain applies
(`lee/state_machine/src/program.rs:55-110`).

Reproduce with:

```bash
cargo test -p multisig-verifier-tests -- --ignored --nocapture
```

## Results

Verifier ImageID `cf5724b0e8dabd4a1519f8d9ea7371d69e1d7e2f6d8c931f1e4b3110150d7982`.

| Instruction | Segments | User cycles | Proving cycles | Share of the public budget |
|---|---:|---:|---:|---:|
| `approve` | 1 | 336,790 | 524,288 | 1.56 % |
| `execute` (M=1) | 1 | 266,907 | 524,288 | 1.56 % |
| `execute` (M=3) | 1 | 363,002 | 524,288 | 1.56 % |
| `execute` (M=5) | 1 | 460,781 | 524,288 | 1.56 % |

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
milliseconds; *proving* it with `RISC0_DEV_MODE=0` takes upwards of ten minutes
on a laptop. Budget the lifecycle accordingly: a 3-of-5 run against the public
testnet is a couple of hours, dominated entirely by proving.

## A note on the revision

These numbers are for the verifier *after* the proposal PDA was reseeded to
`[multisig_id, proposal_ref]` (see `docs/security.md`). The extra seed costs one
more SHA-256 in the address derivation SPEL performs before the body runs, which
is the ~1,100-cycle difference from the pre-fix measurements. It changes nothing
material: every instruction still fits one segment at 1.56 % of the budget.

## Method note

The measurement executes rather than proves, deliberately. Proving would take
minutes per instruction and would not change the cycle counts — the cycle count
*is* the input to proving. Executing gives the same numbers in seconds and keeps
the measurement runnable in CI.
