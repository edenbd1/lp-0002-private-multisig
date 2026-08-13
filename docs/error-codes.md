# Error codes

Every rejection the on-chain verifier can produce, with the attack it stops and
the test that proves it does. The codes are stable: an integration may branch on
them.

On chain they surface as `Program error [11NNN]: Program error 5NNN: <message>`,
where the `11NNN` form is SPEL's envelope around the program's own `5NNN`.

| Code | Meaning | Raised by | Stops |
|---|---|---|---|
| `5001` | `witness_words` did not decode as an `ApproveWitness` | `approve` | Malformed or truncated witness data |
| `5002` | `config_hash` is not `H(member_root ‖ threshold)` | `approve`, `execute` | Presenting a config hash inconsistent with the pair it claims to commit to |
| `5003` | No multisig is committed at this `(multisig_id, config_hash)` | `create_proposal`, `approve`, `execute` | **The invented member set, and the lowered threshold.** Both resolve to a PDA nobody initialised |
| `5004` | No proposal is committed at this `proposal_ref` | `approve`, `execute` | Approving or executing a proposal that was never published |
| `5005` | The nullifier is not the one the witness yields | `approve` | Proving a real membership while occupying another member's marker |
| `5006` | A marker seed does not commit to what it claims | `approve`, `execute` | Landing an approval or execution at an address that misrepresents it |
| `5007` | `proposal_ref` is not `H(multisig_id ‖ proposal_id ‖ action_hash)` | `create_proposal` | Publishing a proposal whose reference does not bind its action |
| `5008` | Threshold is zero | `create_multisig` | A 0-of-N multisig, which anyone could execute |
| `5009` | Approval accounts and nullifiers differ in count | `execute` | Unpairing accounts from the nullifiers they are checked against |
| `5010` | Fewer approvals than the anchored threshold | `execute` | Executing under-approved |
| `5011` | The same nullifier appeared twice | `execute` | **The replay attack**: one member's approval presented M times |
| `5012` | An approval account is not the marker PDA for its nullifier on this proposal | `execute` | Counting a marker earned on a different proposal, or an unrelated account |
| `5013` | An approval marker was never claimed by this program | `execute` | Pre-creating an account at a marker address without ever proving membership |

## The second layer

Before any of the above runs, SPEL's own account validation rejects an account
that is not at the PDA the instruction arguments derive:

```
account validation failed: PdaMismatch { account_name: "multisig", expected: …, actual: … }
```

This is defence in depth, not redundancy. The program's checks establish that the
arguments are internally consistent; the framework's check establishes that the
accounts presented are the ones those arguments name. An attacker has to get past
both, and `the_framework_rejects_a_multisig_at_the_wrong_address` documents it
as a test rather than leaving it as folklore.

## Coverage

Each code above is exercised against the built binary through the sequencer's own
executor:

```
cargo test -p multisig-verifier-tests
```

30 tests: five honest controls and the rest attacks, each required to be
rejected with the code documented above. Every one of the thirteen codes is
exercised against the built binary, and every instruction — including
`create_multisig` and `create_proposal` — has coverage there.

The circuit-side bindings are covered separately by 22 tests in
`cargo test -p multisig-core`, and the pin between the verifier and the
membership binary by 2 more in `--test program_id_pin`. 53 in total.
