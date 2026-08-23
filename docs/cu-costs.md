# Compute cost

Measured by replaying each instruction through the **sequencer's own executor**
— the same executor, input order and 32M-cycle session limit the chain applies
(`lee/state_machine/src/program/mod.rs:55-110`).

Reproduce with:

```bash
cargo test -p multisig-verifier-tests --test verifier_rejects -- --ignored --nocapture
```

## Results

Verifier ImageID `1346b65293ac9b11d4b1029a0d02559462238582124062925a3ad24298ff4e1e`.

| Instruction | Segments | User cycles | Proving cycles | Share of the public budget |
|---|---:|---:|---:|---:|
| `approve` | 1 | 495,788 | 1,048,576 | 3.12 % |
| `execute` (M=1) | 1 | 571,589 | 1,048,576 | 3.12 % |
| `execute` (M=3) | 1 | 735,024 | 1,048,576 | 3.12 % |
| `execute` (M=5) | 1 | 896,983 | 1,048,576 | 3.12 % |
| `execute` (M=7) | 2 | 1,058,884 | 1,310,720 | 3.91 % |

`create_multisig`, `fund_treasury` and `create_proposal` do strictly less work
than `execute` (M=1) — a few hashes, a comparison, and a PDA claim — and are
bounded by it.

Every row is a real accepted execution against a fixture of that size, which is
why the M=7 row needs a 7-member set rather than a 5-member one with the
threshold overwritten: the threshold is inside `config_hash`, which is inside
every PDA address, so a fixture edited in place resolves to accounts that do not
exist and the table would be measuring rejections.

## These numbers went up, and by how much

The previous revision — ImageID `5bb40082…`, the one currently on chain —
measured `approve` at **337,105** and `execute` (M=1) at **267,055**. Both were
re-measured from that exact committed binary rather than quoted from memory, so
the comparison is between two runs of the same command.

Persisting state and paying a treasury cost roughly **159,000 cycles on
`approve`** and **305,000 on `execute` (M=1)**, and raised the per-approval slope
from ~48,600 to ~81,700.

Where it went, in rough order of size:

* **Account data crosses the boundary twice.** Every account's `data` is read in
  as a pre-state and written back as a post-state. The five records add 133, 65,
  210, 65 and 86+32M bytes to instructions that previously moved none — and on
  `execute` the marker records are paid for once per approval, which is most of
  the slope increase.
* **Borsh, in both directions**, on the multisig and proposal records.
* **Two more SHA-256 per execution**, re-deriving `action_hash` and
  `proposal_ref` from the stored action and requiring them to equal the address
  the approvals were bound to.

That is the price of the accounts being readable and the threshold being worth
something, and it is stated rather than absorbed.

## Reading the numbers

**User cycles** is the guest's real work. **Proving cycles** is the padded
power-of-two segment the prover actually commits to, which is why the first four
rows show the same 1,048,576: they fit inside one segment, and a segment is
billed whole. That is also where the previous revision sat one power of two
lower, at 524,288 — crossing that boundary is what doubled the *billed* figure
while the real work rose by about half.

**`execute` scales linearly in M**, at roughly **81,700 user cycles per
additional approval**. That is one SHA-256 for the marker seed, one for the PDA
derivation, the marker account's own 65 bytes in and out, plus the pairwise
distinctness comparisons. The distinctness check is quadratic in M by choice: M
is a multisig threshold, a small number, and a sort would cost more cycles than
it saves at these sizes.

**M=7 is the first row that spans two segments**, at 3.91 % of the budget. It is
measured rather than extrapolated, which is why it is in the table: the previous
revision's comment claimed the measurements covered M up to 7 while the loop
generating them stopped at 5. Extrapolating from the slope, a 10-of-N execution
lands near 1.3M user cycles — still two segments, and still under 4 % of the
32M-cycle public budget.

**What is excluded.** These are the guest's own cycles. They do not include
LEZ's privacy circuit recursively verifying the chained membership call — that
is the platform's cost, not this program's, and it is identical for any program
that composes a chained call.

**Wall-clock is a different number entirely.** Executing `approve` takes
milliseconds; *proving* it with `RISC0_DEV_MODE=0` is what costs time. The
wall-clock figures below were measured against the **previous** verifier, whose
`approve` was about a third cheaper in cycles; they have not been re-measured
against this one, and each is labelled with the run it came from rather than
quietly carried forward.

## Proof generation time, measured

Not estimated. `scripts/deploy-and-run.sh` times each approval and writes the
figure into `lifecycle.tsv` next to the transaction hash, so the number comes
out of the same run that produced the on-chain evidence.

A 2-of-3 lifecycle against a real standalone sequencer on localhost, Apple
silicon laptop, `RISC0_DEV_MODE=0`, **on LEZ v0.2.0**:

| Approval | Wall clock |
|---|---:|
| member 0 | **149 s** |
| member 1 | **154 s** |

The version matters: the same run on **v0.2.4** takes **437 s** on the same idle
machine. Both are kept, because the older one is what the earlier figures in
this document were measured against — see the table further down for all of them
side by side.

That interval covers everything the member waits for: building and locally
re-verifying the Merkle witness, proving, submitting on the privacy path, and
the sequencer confirming the transaction in a block. It is not proving in
isolation — it is the number that matters to whoever is sitting there.

A 1-of-2 run of the same script measured 151 s for its single approval, so the
cost is per-approval and does not grow with the member set: the tree depth
changes the witness by a few words and nothing else.

Reproduce it with:

```bash
./scripts/e2e-local-sequencer.sh          # 2-of-3, the default and the CI shape
```

**Re-measured on 2026-08-15, 2-of-3 against a local standalone sequencer on
v0.2.4**, same laptop, nothing else proving: **444 s** and **704 s** for the two
approvals, **1333 s** end to end including deploy, create, propose, execute and
the on-chain read-back. The two approvals differ by more than half again on an
otherwise idle machine, which is the variance to expect rather than a figure to
average away. "About eight minutes" appeared here and in the README until this
run; that was the v0.2.0 number and it no longer holds.

**The same 2-of-3, on a GitHub `ubuntu-latest` runner** (run 31885000503):
**3752 s** and **3746 s**, 138 minutes end to end. That is roughly 6.5x the
laptop, and it is the honest figure for a 4-vCPU shared runner with no GPU —
worth stating because "about twenty minutes" is a laptop number, not a
universal one. It is also the measurement behind the 180-minute job budget in
`.github/workflows/e2e-local-sequencer.yml`.

**Against the public testnet, measured too.** The 2026-08-03 redeploy recorded
**360 s** and **179 s** for its two approvals. The gap between them is not the
chain: the first ran while an unrelated Risc0 proof was saturating the same
laptop, and the second did not. Take 179 s as the testnet figure and 360 s as
what contention costs — both are in that run's `lifecycle.tsv`, and neither is
rounded in this document's favour.

**Re-measured on 2026-08-12, after the migration to LEZ v0.2.4**: **440 s** and
**469 s** on the same laptop against the public testnet. It is quoted because it
is the run the current deployment came from, and it is left as measured.

**Contention matters more than it looks, so it was controlled for.** The same
`MEMBERS=2 THRESHOLD=1` local-sequencer run measured **935 s** while a second,
unrelated Risc0 proof held roughly half the cores, and **437 s** on the same
machine with nothing else proving. Anyone reproducing these should check
`pgrep -fl r0vm` before timing anything.

**With contention ruled out, v0.2.4 really is about three times more expensive.**
That idle 437 s is against 149-154 s for the same 1-approval run under v0.2.0 —
and the CI runner shows the same ratio independently (1264 s → 4033 s). Two
machines with nothing in common but the version change is enough to stop calling
it variance. The most likely cause is that v0.2.4 folds ML-KEM 768 material into
the private account model, which the privacy circuit then has to carry; this
document does not claim to have isolated it further, because that would need the
two versions timed against the same circuit and that has not been done.

These timings are also the check that a lifecycle was *real*. A run whose
approvals complete in seconds did not prove anything: on 2026-08-12 an unpatched
`spel` failed every instruction while the script still reported success, and the
1 s and 32 s approval timings were the only visible tell.

**And on a CI runner, far more.** The scheduled e2e workflow measured
**1264 s** for its single approval on a standard GitHub `ubuntu-latest` runner
under LEZ v0.2.0 — roughly eight times the laptop figure, for the same proof of
the same circuit.

**On LEZ v0.2.4 the same job measured 4033 s**, about three times that — the
same ratio the laptop shows between 149-154 s and 437 s, on hardware with
nothing else in common. Both numbers were measured with `RISC0_DEV_MODE=0`, and
the older, more flattering figure is kept beside the newer one rather than
replaced by it.

Proving is CPU-bound and does not parallelise past the cores it is given, so the
honest way to read every number here is *per machine*:

| Where | One approval |
|---|---:|
| Apple silicon laptop, local sequencer, idle, LEZ v0.2.0 | **149-154 s** |
| Apple silicon laptop, local sequencer, idle, LEZ v0.2.4 | **437 s** |
| Apple silicon laptop, local sequencer, LEZ v0.2.4, one other proof running | **935 s** |
| Apple silicon laptop, public testnet | **179 s** (360 s under CPU contention) |
| Apple silicon laptop, public testnet, LEZ v0.2.4 | **440-469 s** (under contention) |
| GitHub `ubuntu-latest`, local sequencer, LEZ v0.2.0 | **1264 s** |
| GitHub `ubuntu-latest`, local sequencer, LEZ v0.2.4 | **4033 s** |

Quoting one of these as "the" proving time would be picking the flattering one.
On LEZ v0.2.4 a member on an idle laptop waits about seven minutes against a
local sequencer; the same proof on a shared CI runner took sixty-seven. The
spread is the point: this is a cost that belongs to the machine holding the
member's key, and any single number quoted without one is marketing.

What testnet adds on top of proving is block time and network latency per
submission, plus a wallet polling window that can expire before a privacy
transaction lands even though the transaction is fine.

## A note on the revision

These numbers are for the verifier *after* the proposal PDA was reseeded to
`[multisig_id, config_hash, proposal_ref]` and `config_hash` was folded into
`proposal_ref` (see [`security.md`](security.md)), and *after* the accounts were
given records to carry and the threshold a treasury to spend (see
[`account-layout.md`](account-layout.md)). The extra PDA seed costs one more
SHA-256 in the address derivation SPEL performs before the body runs — about
150-650 cycles depending on the instruction, and material to nothing next to the
record encoding above.

## Method note

The measurement executes rather than proves, deliberately. Proving would take
minutes per instruction and would not change the cycle counts — the cycle count
*is* the input to proving. Executing gives the same numbers in seconds and keeps
the measurement runnable in CI.
