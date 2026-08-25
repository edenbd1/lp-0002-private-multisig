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
| `5007` | `proposal_ref` is not `H(multisig_id ‖ config_hash ‖ proposal_id ‖ action_hash)` | `create_proposal` | Publishing a proposal whose reference does not bind its action |
| `5008` | Threshold is zero | `create_multisig` | A 0-of-N multisig, which anyone could execute |
| `5009` | Approval accounts and nullifiers differ in count | `execute` | Unpairing accounts from the nullifiers they are checked against |
| `5010` | Fewer approvals than the anchored threshold | `execute` | Executing under-approved |
| `5011` | The same nullifier appeared twice | `execute` | **The replay attack**: one member's approval presented M times |
| `5012` | An approval account is not the marker PDA for its nullifier on this proposal | `execute` | Counting a marker earned on a different proposal, or an unrelated account |
| `5013` | An approval marker was never claimed by this program | `execute` | Pre-creating an account at a marker address without ever proving membership |
| `5014` | The treasury is not this program's, or not the one the multisig names | `fund_treasury`, `execute` | Paying out of, or into, an account this program has no right to move |
| `5015` | A balance cannot cover the move, or would overflow | `fund_treasury`, `execute` | A partial payment: the transfer is refused whole rather than truncated |
| `5016` | The funder is not held by the native transfer program | `fund_treasury` | **Chaining into a program the caller wrote**, which could report a funding it never performed |
| `5017` | A zero amount | `create_proposal`, `fund_treasury` | A proposal the threshold would gate for nothing |
| `5018` | The action fields do not re-derive the address they live at | `create_proposal`, `execute` | **The bait-and-switch, at the record layer**: paying a recipient or an amount the approvals were never bound to |
| `5019` | The recipient presented is not the one the proposal names | `execute` | An executor redirecting the payment to themselves |
| `5020` | The recipient cannot spend what it receives, or is the treasury | `create_proposal`, `execute` | Burning the treasury into an account nobody controls |
| `5021` | The proposal is already marked executed | `execute` | Paying twice, if the marker's `init` guard were ever bypassed |
| `5022` | An account this program owns holds no readable record | `create_proposal`, `fund_treasury`, `execute` | Computing a payment from bytes that did not parse |
| `5023` | The tier table is not monotone | `create_multisig`, `create_proposal`, `approve`, `execute`, `rotate_config` | A table where a larger transfer needs fewer approvals than a smaller one |
| `5024` | The tier table supplied is not the one this configuration anchors | `execute` | Substituting a cheaper table at the moment the approvals are counted |
| `5025` | This configuration has been superseded by a rotation | `create_proposal`, `approve`, `execute`, `rotate_config` | A retired member set still proposing, approving or paying |
| `5026` | A rotation that changes nothing | `create_proposal`, `rotate_config` | Spending a threshold of approvals on a no-op, and colliding with an account that already exists |
| `5027` | A proposal spent by the instruction for the other action shape | `execute`, `rotate_config` | Executing a rotation proposal as a transfer, or the reverse. `rotate_to` says which shape a proposal is, and until this existed `execute` never read it: a rotation proposal reached the transfer path and was stopped only by the recipient check, because a rotation's stored recipient is the zero account id and no usable account has it. That is a fact about which account ids exist, not a rule this program enforces — and it reported the refusal as a recipient mismatch, naming the wrong cause |

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

**84 tests there**, in five files:

| File | Tests | What it establishes |
|---|---:|---|
| `verifier_rejects.rs` | 30 | The gate cannot be forced: 25 forgeries, 5 honest controls |
| `state_and_transfer.rs` | 22 | Passing the gate moves real balances, and every account it claims comes out readable |
| `idl_contract.rs` | 6 | The IDL, this document and `scripts/pda.py` still agree with the guest source |
| `tiers_and_rotation.rs` | 23 | Spending tiers may only ever lower the bar, and a rotation replaces a configuration rather than adding one |
| `program_id_pin.rs` | 3 | The verifier chains only to the exact membership binary committed here |

Every one of the twenty-seven codes is named by at least one test, and the three
files divide cleanly: `verifier_rejects.rs` covers `5001`–`5013`,
`state_and_transfer.rs` covers `5014`–`5022`, and `tiers_and_rotation.rs` covers
`5023`–`5027` while revisiting `5002`, `5003`, `5006`, `5008` and `5010` under a
tier table or a rotation. A further 6 rejections in `verifier_rejects.rs` are caught one
layer earlier than the program body — by SPEL's address validation or LEZ's init
guard — so they assert that rejection rather than a code:
`the_framework_rejects_a_multisig_at_the_wrong_address`,
`executing_the_same_proposal_twice_is_rejected`, and the four foreign-multisig
and second-config cases.

The claim that *every* code is covered is itself checked, rather than asserted:
`the_error_code_document_covers_every_code_the_guest_declares` compares this
table against the `const E_*` block in the guest source, and
`the_idl_carries_every_error_code_the_guest_declares` compares the IDL against
the same source. Adding a code without documenting it fails `cargo test`. That
matters because this document once claimed full coverage at a moment when three
codes had none.

The circuit-side bindings are covered separately by 25 tests in
`cargo test -p multisig-core`, plus 16 for the account layouts and the tier wire
format, and the client surface by 2 in `multisig-sdk` (one of them a doctest) and
2 in `multisig-cli`.

**131 in total.** `cargo test --workspace` runs all of them.
