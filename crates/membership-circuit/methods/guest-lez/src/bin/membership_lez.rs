// LP-0002 member-set membership proof as a native LEZ program.
//
// WHY THIS EXISTS
//
// The one path on LEZ v0.2.0 where a proof is genuinely verified on chain is the
// privacy-preserving transaction: the client proves locally, and LEZ's privacy
// circuit composes each chained call with a real `env::verify` over the callee's
// `ProgramOutput` (`lee/privacy_preserving_circuit/src/execution_state.rs:149`),
// which the sequencer then checks against the pinned
// `PRIVACY_PRESERVING_CIRCUIT_ID`. To take part in that composition, the
// membership proof has to *be* a LEZ program: read via `read_lee_inputs`, emit a
// `ProgramOutput`. That is this file.
//
// A LEZ *public* transaction would not do: the sequencer merely re-executes the
// program host-side (`lee/state_machine/src/program.rs:73-77`), so nothing is
// proved and nothing is verified. A multisig built on the public path would be a
// signature check wearing a zero-knowledge costume.
//
// PRIVACY
//
// The witness travels in the instruction. That is safe only on the privacy path:
// a privacy `Message` publishes commitments and nullifiers but carries neither
// `program_id` nor `instruction_data`
// (`lee/state_machine/privacy_preserving_transaction/message.rs:14-24`). A public
// `Message` publishes `instruction_data` verbatim, so this program must never be
// invoked on the public path. It is not: the verifier reaches it through a
// ChainedCall inside a privacy transaction.
//
// This is what delivers the criterion that an approval is hidden *from the other
// members* as well as from outside observers. Every member sees the same thing
// an outsider sees: a marker PDA at an address derived from a nullifier they
// cannot invert.
//
// WHAT IT PROVES
//
// Via the shared `multisig_core::approve`:
//   1. npk        = SHA256(NPK_DERIVE_PREFIX || msk)
//   2. account_id = SHA256(PRIVATE_ACCOUNT_ID_PREFIX || npk || identifier_LE)
//   3. leaf       = SHA256(MEMBER_LEAF_PREFIX || account_id || salt)
//   4. leaf folds via merkle_path to statement.member_root
//   5. nullifier  = SHA256(APPROVAL_NULLIFIER_PREFIX || proposal_ref || msk)
//
// A false approval makes `approve` return an error, which this program turns
// into a panic. A panic aborts the guest, so no proof exists for a non-member,
// and no transaction can be built. The verifier program then checks, on chain,
// that `statement.member_root` is the root anchored in the multisig PDA address,
// which is what makes the anchored-set guarantee real.

#![no_main]

risc0_zkvm::guest::entry!(main);

use lee_core::program::{read_lee_inputs, AccountPostState, ProgramInput, ProgramOutput};
use multisig_core::{approve, ApproveInstruction};

fn main() {
    let (
        ProgramInput {
            self_program_id,
            caller_program_id,
            pre_states,
            instruction,
        },
        instruction_words,
    ) = read_lee_inputs::<ApproveInstruction>();

    // The whole zero-knowledge statement. Returns an error, escalated here to a
    // panic, if the approver is not a member of the named set or the statement
    // is internally inconsistent — so no proof can be produced for it.
    let leaf = approve(&instruction.witness, &instruction.statement)
        .unwrap_or_else(|e| panic!("approval is not valid: {e:?}"));

    // A committed member leaf is never the zero hash, and the nullifier the
    // caller pins is proved equal to the one this witness yields (checked inside
    // `approve`), so a caller cannot occupy a marker for one nullifier while
    // having proved another.
    assert!(leaf != [0u8; 32], "member leaf must be non-zero");
    assert!(
        instruction.statement.nullifier != [0u8; 32],
        "nullifier must be non-zero for the caller's PDA seed",
    );

    // An approval moves no balances and mutates no state: claiming the marker
    // PDA is the verifier program's job on the post-state. Every post-state is
    // its pre-state unchanged, which satisfies every rule in
    // `validate_execution`.
    let post_states: Vec<AccountPostState> = pre_states
        .iter()
        .map(|pre| AccountPostState::new(pre.account.clone()))
        .collect();

    ProgramOutput::new(
        self_program_id,
        caller_program_id,
        instruction_words,
        pre_states,
        post_states,
    )
    .write();
}
