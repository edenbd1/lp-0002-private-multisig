# Compute cost

Measured by replaying each instruction through the **sequencer's own executor**
— the same executor, input order and 32M-cycle session limit the chain applies
(`lee/state_machine/src/program/mod.rs:55-110`).

Reproduce with:

```bash
cargo test -p multisig-verifier-tests --test verifier_rejects -- --ignored --nocapture
```

## Results

Verifier ImageID `a8a87f8b456299144236f42f194f1b85c11265763a976c055a7f471b61500750`.

| Instruction | Segments | User cycles | Proving cycles | Share of the public budget |
|---|---:|---:|---:|---:|
| `create_multisig` | 1 | 209,349 | 524,288 | 1.56 % |
| `fund_treasury` | 1 | 298,376 | 524,288 | 1.56 % |
| `create_proposal` | 1 | 338,828 | 524,288 | 1.56 % |
| `approve` | 1 | 541,207 | 1,048,576 | 3.12 % |
| `execute` (M=1) | 1 | 623,636 | 1,048,576 | 3.12 % |
| `execute` (M=3) | 1 | 786,596 | 1,048,576 | 3.12 % |
| `execute` (M=5) | **2** | 948,651 | 1,114,112 | 3.32 % |
| `execute` (M=7) | 2 | 1,109,909 | 1,310,720 | 3.91 % |
| `execute` (M=2, tiered) | 1 | 710,702 | 1,048,576 | 3.12 % |
| `rotate_config` (M=3) | 1 | 750,617 | 1,048,576 | 3.12 % |

**Spending tiers and rotation made every instruction more expensive, and one row
changed shape.** Against the same table before that work — `approve` 495,788,
`execute` 571,589 / 735,024 / 896,983 / 1,058,884 — each row is between 45,000
and 52,000 user cycles dearer, and **`execute` at M=5 crossed from one segment to
two**. That crossing is the only figure here that changes what a reader should
plan for: an operator sizing a 5-of-N multisig now pays for a second segment.

The tier itself is not what costs. `execute` (M=2, tiered) at 710,702 sits about
6,000 cycles above where an untiered M=2 would fall on the slope below, so
decoding a table and resolving an amount against it is nearly free. The 45–52k is
paid on *every* instruction, tiered or not, because `config_hash` now commits to
a tiers hash and each instruction recomputes that commitment before it trusts the
address it was handed. That is the price of the anchoring, not of the feature —
a multisig with no tiers pays it too, and pays it so that a multisig with tiers
cannot be read under someone else's table.

`rotate_config` at M=3 costs 750,617 against `execute` (M=3) at 786,596: slightly
less, because it counts the same approvals and writes two records but moves no
value, so it never touches the treasury or the recipient.

`create_multisig`, `fund_treasury` and `create_proposal` were once argued about
here rather than measured — "strictly less work than `execute` (M=1), and bounded
by it". True, and now unnecessary: they are in the table, from the same harness
and the same command as every other row. The criterion says *each* on-chain
operation, and an argument is not a number. They land in the smaller power-of-two
segment, which is why all three read 1.56 % against `approve`'s 3.12 %.

Every row is a real accepted execution against a fixture of that size, which is
why the M=7 row needs a 7-member set rather than a 5-member one with the
threshold overwritten: the threshold is inside `config_hash`, which is inside
every PDA address, so a fixture edited in place resolves to accounts that do not
exist and the table would be measuring rejections.

## These numbers went up, and by how much

The previous revision — ImageID `5bb40082…`, deployed before the 2026-08-24
redeploy and now superseded — measured `approve` at **337,105** and `execute`
(M=1) at **267,055**. Both were
re-measured from that exact committed binary rather than quoted from memory, so
the comparison is between two runs of the same command.

Persisting state and paying a treasury cost roughly **159,000 cycles on
`approve`** and **305,000 on `execute` (M=1)**, and raised the per-approval slope
from ~48,600 to the **81,046** the table above now measures. (That revision's
text said ~81,700; recomputing the slope from its own rows gives 81,216, so the
figure was carried rather than derived. It is derived here.)

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

**`execute` scales linearly in M**, at **81,046 user cycles per additional
approval** — the slope over the M=1 to M=7 rows above, recomputed from them
rather than carried forward. That is one SHA-256 for the marker seed, one for the PDA
derivation, the marker account's own 65 bytes in and out, plus the pairwise
distinctness comparisons. The distinctness check is quadratic in M by choice: M
is a multisig threshold, a small number, and a sort would cost more cycles than
it saves at these sizes.

**M=5 is now the first row that spans two segments**, at 3.32 % of the budget,
and that is a change: before spending tiers, M=5 fitted in one. The 45–52k cycles
the tier commitment adds to every instruction were enough to push that row over a
power-of-two boundary, which is the one place in this table where the feature
changes what an operator should plan for rather than only what they pay.

Both rows are measured rather than extrapolated, which is why they are in the
table: an earlier revision's comment claimed the measurements covered M up to 7
while the loop generating them stopped at 5. Extrapolating from the slope, a
10-of-N execution lands near 1.35M user cycles — still two segments, and still
under 4.5 % of the 32M-cycle public budget.

**What is excluded.** These are the guest's own cycles. They do not include
LEZ's privacy circuit recursively verifying the chained membership call — that
is the platform's cost, not this program's, and it is identical for any program
that composes a chained call.

**Wall-clock is a different number entirely.** Executing `approve` takes
milliseconds; *proving* it with `RISC0_DEV_MODE=0` is what costs time. Most of
the wall-clock figures below were measured against the **previous** verifier,
whose `approve` was about a third cheaper in cycles. Two were not — the
2026-08-24 local run (482 s) and the 2026-08-24 public-testnet redeploy
(614 s / 609 s) are this binary — and each figure is labelled with the run it
came from rather than quietly carried forward.

## The whole lifecycle, against a real sequencer, on this binary

Measured 2026-08-24 with `./scripts/e2e-local-sequencer.sh`, `MEMBERS=2
THRESHOLD=1`, an actual `sequencer_service` in standalone mode on localhost,
`RISC0_DEV_MODE=0`, Apple-silicon laptop:

| Step | Result |
|---|---|
| deploy `multisig_verifier` | `268834b601f78b59090e90f8f10fd8ce3b526528e1224983edba95224be31aa3` |
| `create_multisig` | multisig and treasury created, both decoding |
| `fund_treasury` | treasury balance **0 → 500**, via the chained call |
| `create_proposal` | recipient, amount and memo persisted and re-derivable |
| `approve` (privacy tx, one real proof) | **482 s** wall clock |
| `execute` | treasury **500 → 250**, recipient **0 → 250** |

The last row is the criterion. The script exits non-zero unless both balances
move by exactly the proposed amount, so a run that reports success has checked
rather than claimed — and `scripts/verify-onchain.sh` then read all six accounts
back off that chain and decoded every one of them at the offsets
[`account-layout.md`](account-layout.md) publishes.

482 s against 437 s for the same shape on the previous binary, on the same
laptop: about a tenth more for a guest segment that doubled in size, because the
guest's own segment is a small part of what the privacy circuit proves. One
measurement of each, so read it as "no material change", not as a ratio.

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

**The same 2-of-3, on a GitHub `ubuntu-latest` runner** (run `31885000503`):
**3752 s** and **3746 s**, 138 minutes end to end. That is roughly 6.5x the
laptop, and it is the honest figure for a 4-vCPU shared runner with no GPU —
worth stating because "about twenty minutes" is a laptop number, not a
universal one. It is also the measurement behind the 180-minute job budget in
`.github/workflows/e2e-local-sequencer.yml`. Run `32619018169`, eight days
later, took **139m 52s** end to end — the same shape on the same runner image,
agreeing with the first to within two minutes, which is why that budget has not
moved.

**Ids rather than links, and the reason is worth more than the convenience.**
Both runs are real and the GitHub API still answers for them, but each ran on a
commit that no longer exists: this branch's history was rewritten to remove
references to work outside this repository, which gave every commit a new hash.
`scripts/check-run-citations.py` requires a *linked* run to sit on a commit this
branch contains and merely reports a bare id, and it is what caught these two
after the rewrite. Linking them would send a reader to a commit this repository
cannot show.

**Against the public testnet, measured too.** The 2026-08-03 redeploy recorded
**360 s** and **179 s** for its two approvals. The gap between them is not the
chain: the first ran while an unrelated Risc0 proof was saturating the same
laptop, and the second did not. Take 179 s as the testnet figure and 360 s as
what contention costs — both are in that run's `lifecycle.tsv`, and neither is
rounded in this document's favour.

**Re-measured on 2026-08-12, after the migration to LEZ v0.2.4**: **440 s** and
**469 s** on the same laptop against the public testnet.

**And again on 2026-08-25, which is the run the current deployment came from**:
**484 s** and **550 s** for the two approvals of the 3-of-3 with a spending tier,
against the public testnet on the same laptop. (The 2026-08-24 run that this one
supersedes measured **614 s** and **609 s** for a 2-of-3.) The second approval
here is the slower of the pair because it was resubmitted: the account that first
carried it had never been initialised under the transfer program, so the privacy
circuit refused it client-side before any proof was wasted, and the retry ran
against a freshly provisioned approver. Both figures are laptop wall-clock from that
run's log; the run's on-chain evidence is the two transfer approvals recorded in
`docs/DEPLOYMENT.md` (`1a5e529d…` and `28a07e8d…`), which resolve on the explorer.
The per-approval timing itself lives in `.testnet/lifecycle.tsv`, which is build
output rather than a committed file, so treat the seconds as a laptop measurement
rather than something a clone can re-derive. Two approvals within 5 s of
each other is what an uncontended machine looks like; compare the 440/469 pair
above and the 444/704 pair below, both taken under load.

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
| Apple silicon laptop, public testnet, LEZ v0.2.4, the 2026-08-24 run | **609-614 s** |
| Apple silicon laptop, public testnet, LEZ v0.2.4, the deployed run | **484-550 s** |
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
