// LP-0002 on-chain private M-of-N multisig verifier.
//
// WHAT MAKES THE THRESHOLD REAL ON CHAIN
//
// A LEZ public transaction re-executes rather than proves
// (`lee/state_machine/src/program/mod.rs:73-77`), so no program on the public
// path can verify a membership proof. This program targets the privacy-preserving
// path, where LEZ's circuit composes each chained call with a real `env::verify`
// over the callee's `ProgramOutput`
// (`lee/privacy_preserving_circuit/src/execution_state.rs:149-155`) and the
// sequencer checks the receipt against the pinned
// `PRIVACY_PRESERVING_CIRCUIT_ID`. The `approve` instruction declares a
// ChainedCall to the LEZ-native membership program, so membership is genuinely
// verified on chain as a precondition of the transaction's acceptance.
//
// WHAT THE THRESHOLD BUYS, ONCE IT IS REACHED
//
// It moves money. Each multisig owns a **treasury PDA** seeded by
// `[multisig_id, config_hash, "treasury"]`. `fund_treasury` fills it by chaining
// into the native transfer program; `execute` empties the proposed amount out of
// it, directly, into the account the proposal named. That works without any
// transfer authority over a third party because LEZ rule 5 forbids a program
// from *decreasing* a balance it does not own and says nothing about increasing
// one (`lee/state_machine/core/src/program/mod.rs:706-717`), and rule 8 only
// requires the debit and the credit to balance within the transaction
// (ibid., 741-760). The treasury is this program's own account, so debiting it
// is this program debiting itself, and crediting the recipient is the permitted
// direction — no signature from either side.
//
// WHY THE MEMBER SET AND THE THRESHOLD ARE ANCHORED BY ADDRESS
//
// A membership proof establishes membership against whatever root the statement
// names. On its own that is not enough: an attacker could invent a one-leaf tree
// holding themselves. The multisig account is a PDA whose address derives from
// `[multisig_id, config_hash]`, and `config_hash = H(member_root || threshold)`.
// `create_multisig` initialises exactly that PDA, so it becomes owned by this
// program. Every other instruction references the multisig as the PDA for the
// `(multisig_id, config_hash)` it was handed, and requires it to be owned by
// this program. An invented root, or a lowered threshold, gives a different
// config hash, hence a different PDA address that was never initialised, whose
// owner is the default — and the instruction is rejected.
//
// The same trick anchors the proposal: its PDA address is seeded by
// `[multisig_id, config_hash, proposal_ref]`, and `proposal_ref` commits to
// `(multisig_id, config_hash, proposal_id, action_hash)`. Approvals are
// therefore bound to the exact action, and a bait-and-switch under the same
// proposal id lands on a different, unapproved address.
//
// WHY THE STATE IS WRITTEN AS WELL AS ADDRESSED
//
// Anchoring by address makes a value unforgeable; it does not make it readable.
// A third party holding a proposal's address could confirm a `(root, threshold,
// action)` guess but could not *discover* it. So every account this program
// claims also carries a borsh record of what it stands for — the layouts and
// their byte offsets are in `docs/account-layout.md`, and `multisig-core`'s
// `state` module decodes them from those offsets alone.
//
// None of it is trusted where the address already decides. `execute` re-derives
// `action_hash` and `proposal_ref` from the stored action bytes and requires
// them to equal the address it was handed, so the record is checked against the
// commitment on every execution rather than believed.
//
// PRIVACY AND UNLINKABILITY
//
// A privacy `Message` publishes neither `program_id` nor `instruction_data`
// (`privacy_preserving_transaction/message.rs:14-27`). The only public trace an
// approval leaves is the marker PDA, seeded by
// `SHA256(APPROVAL_MARKER_PREFIX || proposal_ref || nullifier)`. The nullifier is
// `SHA256(APPROVAL_NULLIFIER_PREFIX || proposal_ref || msk)`, a function of the
// member's secret, so an observer who knows every member of the set — including
// the other members — still cannot link a marker to one of them. Claiming the
// marker requires it to be uninitialised, which is the double-approval guard: a
// second approval by the same member on the same proposal targets the same
// address and fails.
//
// The marker's *data* records the proposal and the nullifier, which are the two
// values already implied by its address. It records nothing else, and in
// particular nothing derived from the witness.
//
// The execution is likewise unlinkable: it consumes marker addresses, never
// member identities, and the executor need not be a member at all.

#![no_main]

use spel_framework::prelude::*;

risc0_zkvm::guest::entry!(main);

// ---------------------------------------------------------------------------
// Error codes. Deterministic and stable: an integration may branch on these.
// Documented in docs/error-codes.md, and mirrored into the IDL by
// scripts/idl-errors.py, which reads this very block.
// ---------------------------------------------------------------------------

/// The witness bytes did not decode as an `ApproveWitness`.
const E_BAD_WITNESS: u32 = 5001;
/// `config_hash` is not `H(member_root || threshold)` for the supplied pair.
const E_CONFIG_MISMATCH: u32 = 5002;
/// No multisig is committed at this `(multisig_id, config_hash)`: the member set
/// or the threshold is not anchored.
const E_MULTISIG_NOT_ANCHORED: u32 = 5003;
/// No proposal is committed at this `proposal_ref`.
const E_PROPOSAL_NOT_ANCHORED: u32 = 5004;
/// The supplied nullifier is not the one the witness yields.
const E_NULLIFIER_MISMATCH: u32 = 5005;
/// A marker seed does not commit to the proposal and nullifier it claims to.
const E_MARKER_SEED_MISMATCH: u32 = 5006;
/// `proposal_ref` is not `H(multisig_id || config_hash || proposal_id || action_hash)`.
const E_PROPOSAL_REF_MISMATCH: u32 = 5007;
/// A threshold of zero would make the multisig meaningless.
const E_BAD_THRESHOLD: u32 = 5008;
/// Fewer approval accounts than nullifiers, or vice versa.
const E_APPROVAL_COUNT_MISMATCH: u32 = 5009;
/// Fewer distinct approvals than the anchored threshold.
const E_THRESHOLD_NOT_MET: u32 = 5010;
/// The same nullifier was presented twice in one execution.
const E_DUPLICATE_APPROVAL: u32 = 5011;
/// An approval account is not the marker PDA for the nullifier it was paired
/// with, or not for this proposal.
const E_APPROVAL_NOT_FOR_PROPOSAL: u32 = 5012;
/// An approval marker exists at the right address but was never claimed by this
/// program, so no membership proof was ever verified for it.
const E_APPROVAL_NOT_ANCHORED: u32 = 5013;
/// The treasury account is not owned by this program, so this program may not
/// debit it and must not pretend it can.
const E_TREASURY_UNOWNED: u32 = 5014;
/// The treasury cannot cover the proposed amount, or a balance would overflow.
const E_TREASURY_SHORT: u32 = 5015;
/// The funding account is not held by the native transfer program, so chaining
/// into that program with it would reach an account it does not own.
const E_FUNDER_NOT_TRANSFERABLE: u32 = 5016;
/// A zero amount: a proposal that moves nothing, or a funding of nothing.
const E_BAD_AMOUNT: u32 = 5017;
/// The action fields do not hash to the `action_hash`, or `action_hash` and
/// `proposal_id` do not re-derive the `proposal_ref` the address commits to.
const E_ACTION_MISMATCH: u32 = 5018;
/// The recipient account presented is not the one the proposal names.
const E_RECIPIENT_MISMATCH: u32 = 5019;
/// The recipient is not held by the native transfer program, so it could never
/// spend what it received; or it is the treasury itself.
const E_RECIPIENT_UNUSABLE: u32 = 5020;
/// The proposal is already marked executed.
const E_ALREADY_EXECUTED: u32 = 5021;
/// An account this program owns does not hold the record it should.
const E_STATE_DECODE: u32 = 5022;

/// ProgramId of the LEZ-native membership program (`membership_lez.bin`).
/// The deployment is content-addressed, so this pins exactly the audited binary.
///
/// Verify with:
///   spel program-id artifacts/programs/membership_lez.bin
///
/// Regenerated by `scripts/build-programs.sh`, which fails if this constant and
/// the built binary disagree.
/// ImageID `56f784d6b37f5cbac85d2eca3e28f56346e8739e6c22cb15a1b7165616758e31` — the
/// one already on chain. `multisig-core`'s account layouts are behind a feature
/// this guest does not enable, so the membership binary is byte-identical to the
/// deployed one and needs no redeploy.
pub const MEMBERSHIP_LEZ_PROGRAM_ID: nssa_core::program::ProgramId = [
    3599038294,3126624179,3392036296,1677011006,2658396230,365634156,1444329377,831419670,
];

/// ProgramId of the LEZ-native `authenticated_transfer` program, **pinned**
/// rather than read off whatever account the caller handed us.
///
/// This is a security boundary, not a convenience. LEZ deployment is
/// permissionless, so anyone may deploy a program and own accounts with it. If
/// `fund_treasury` chained into `funder.account.program_owner`, a caller could
/// pass an account owned by a program they wrote, and this program would
/// obediently chain into it — which could decline to move anything while this
/// program reported the treasury funded.
///
/// Pinning the id closes that: the program invoked is the one whose bytecode
/// hashes to this value, and the check below refuses any funder the real
/// transfer program does not own. Verified against
/// `_external/lez/artifacts/lez/programs/authenticated_transfer.bin` at tag
/// v0.2.4 — ImageID
/// `fe96c4228babbe8bc578e3e25b884cacb07f8c86541f27ed676789875eef875a`.
/// Reproduce with `spel program-id authenticated_transfer.bin`.
pub const AUTH_TRANSFER_PROGRAM_ID: nssa_core::program::ProgramId = [
    583309054,2344528779,3806558405,2890696795,2257354672,3978764116,2273929063,1518858078,
];

/// The native transfer program's instruction, mirrored rather than imported:
/// that crate is `edition = "2024"`, which the pinned risc0 guest toolchain does
/// not build. The wire format is a risc0 `serde` enum — variant index first — so
/// the variant ORDER here is the ABI and must not be reordered. `Initialize` is
/// never constructed here; it exists so `Transfer` keeps index 0.
#[derive(serde::Serialize)]
enum AuthTransfer {
    /// Move `amount` of native balance. Accounts: `[sender, recipient]`.
    Transfer { amount: u128 },
    #[allow(dead_code)]
    Initialize,
}

// ---------------------------------------------------------------------------
// On-chain records
//
// Every field is fixed-width except the execution marker's nullifier list, and
// borsh writes them back to back with no padding — which is what lets
// `docs/account-layout.md` give a byte offset per field, and
// `multisig_core::state` decode from those offsets with no borsh at all.
//
// `#[account_type]` is what puts these into the generated IDL, so a client sees
// the layouts without reading this file.
// ---------------------------------------------------------------------------

/// The layout version every record carries at offset 0.
const STATE_FORMAT_V1: u8 = 1;
/// A proposal that has not executed.
const STATUS_OPEN: u8 = 0;
/// A proposal whose threshold was met and whose action was carried out.
const STATUS_EXECUTED: u8 = 1;

/// The multisig account's record, at the PDA seeded by
/// `[multisig_id, config_hash]`. 133 bytes.
#[account_type]
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct MultisigRecord {
    pub format: u8,
    pub multisig_id: [u8; 32],
    pub member_root: [u8; 32],
    pub threshold: u32,
    /// The treasury PDA's own address, recorded at creation so no later
    /// instruction can name a different source of funds.
    pub treasury: [u8; 32],
    /// Who created it. Recorded, never trusted: creation is permissionless and
    /// being named here confers nothing.
    pub authority: [u8; 32],
}

/// The treasury account's record, at the PDA seeded by
/// `[multisig_id, config_hash, "treasury"]`. 65 bytes.
#[account_type]
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct TreasuryRecord {
    pub format: u8,
    pub multisig_id: [u8; 32],
    pub config_hash: [u8; 32],
}

/// The proposal account's record, at the PDA seeded by
/// `[multisig_id, config_hash, proposal_ref]`. 210 bytes.
#[account_type]
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct ProposalRecord {
    pub format: u8,
    pub multisig_id: [u8; 32],
    pub config_hash: [u8; 32],
    pub proposal_id: [u8; 32],
    pub action_hash: [u8; 32],
    /// The account the treasury pays.
    pub recipient: [u8; 32],
    /// How much it pays.
    pub amount: u128,
    /// Commitment to the human-readable memo the members approved.
    pub memo_hash: [u8; 32],
    /// `STATUS_OPEN` or `STATUS_EXECUTED`. One-way: nothing lowers it.
    pub status: u8,
}

/// An approval marker's record. 65 bytes.
#[account_type]
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct ApprovalMarkerRecord {
    pub format: u8,
    pub proposal_ref: [u8; 32],
    pub nullifier: [u8; 32],
}

/// The execution marker's record: the audit trail of one completed execution.
/// 86 bytes plus 32 per nullifier.
#[account_type]
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct ExecutionMarkerRecord {
    pub format: u8,
    pub proposal_ref: [u8; 32],
    pub recipient: [u8; 32],
    pub amount: u128,
    pub status: u8,
    /// The nullifiers this execution consumed, in the order presented.
    pub nullifiers: Vec<[u8; 32]>,
}

#[lez_program]
mod multisig_verifier {
    #[allow(unused_imports)]
    use super::*;

    /// Write a record into an account this program owns or has just claimed.
    ///
    /// A serialisation failure is surfaced as an error rather than a panic, so a
    /// caller sees a documented code instead of a guest abort.
    fn write(account: &mut Account, state: &impl BorshSerialize) -> Result<(), SpelError> {
        let bytes = borsh::to_vec(state)
            .map_err(|_| SpelError::custom(E_STATE_DECODE, "record failed to serialize"))?;
        account.data = bytes
            .try_into()
            .map_err(|_| SpelError::custom(E_STATE_DECODE, "record does not fit the account"))?;
        Ok(())
    }

    /// Re-derive the anchored commitments from a stored proposal record and
    /// require them to equal the address the caller named.
    ///
    /// This is what stops the persisted action from being merely *believed*. The
    /// approvals are bound to `proposal_ref`; `proposal_ref` is bound to
    /// `action_hash`; `action_hash` is bound to `(recipient, amount, memo_hash)`.
    /// Recomputing the chain from the bytes on disk closes it back on itself, so
    /// a record that does not describe the action the members approved cannot be
    /// paid out — whatever wrote it.
    fn check_action_binds(
        record: &ProposalRecord,
        multisig_id: &[u8; 32],
        config_hash: &[u8; 32],
        proposal_ref: &[u8; 32],
    ) -> Result<(), SpelError> {
        let expected_action = multisig_core::compute_transfer_action_hash(
            multisig_id,
            &record.recipient,
            record.amount,
            &record.memo_hash,
        );
        if expected_action != record.action_hash {
            return Err(SpelError::custom(
                E_ACTION_MISMATCH,
                "the stored action does not hash to this proposal's action_hash",
            ));
        }
        let expected_ref = multisig_core::compute_proposal_ref(
            multisig_id,
            config_hash,
            &record.proposal_id,
            &record.action_hash,
        );
        if expected_ref != *proposal_ref {
            return Err(SpelError::custom(
                E_ACTION_MISMATCH,
                "the stored action does not re-derive this proposal_ref",
            ));
        }
        Ok(())
    }

    /// Publish a multisig: commit to a member set and a threshold, and open the
    /// treasury it will pay from.
    ///
    /// Anyone can create a multisig; it is theirs, funded by them, and
    /// independent of every other. What matters is that a given
    /// `(multisig_id, config_hash)` maps to exactly one on-chain PDA, which
    /// `init` guarantees by refusing to overwrite an existing account.
    ///
    /// Accounts:
    /// - `multisig` (init, PDA seeded by `[multisig_id, config_hash]`): the
    ///   on-chain commitment. Its address encodes the member root *and* the
    ///   threshold, so neither can be altered later; the record it now carries
    ///   makes both readable rather than merely checkable.
    /// - `treasury` (init, PDA seeded by `[multisig_id, config_hash, "treasury"]`):
    ///   created empty here and filled by `fund_treasury`, in a second
    ///   transaction. Not a style choice: an account cannot be initialised and
    ///   paid into at once, because the chained transfer reads a pre-state the
    ///   initialisation has not written yet.
    /// - `authority` (signer): the creator.
    #[instruction]
    pub fn create_multisig(
        #[account(init, pda = [arg("multisig_id"), arg("config_hash")])]
        mut multisig: AccountWithMetadata,
        #[account(init, pda = [arg("multisig_id"), arg("config_hash"), literal("treasury")])]
        mut treasury: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        member_root: [u8; 32],
        threshold: u32,
    ) -> SpelResult {
        // A 0-of-N multisig would let anyone execute. Reject it at creation so
        // no such instance can exist on chain.
        if threshold == 0 {
            return Err(SpelError::custom(
                E_BAD_THRESHOLD,
                "threshold must be at least 1",
            ));
        }

        // Re-derive the commitment the PDA address encodes. The macro already
        // constrains `multisig` to be the PDA for this `(id, config_hash)`; this
        // check is what gives `config_hash` its meaning, tying the address to a
        // specific member root and threshold rather than to opaque bytes.
        let expected = multisig_core::compute_config_hash(&member_root, threshold);
        if expected != config_hash {
            return Err(SpelError::custom(
                E_CONFIG_MISMATCH,
                "config_hash does not commit to this (member_root, threshold)",
            ));
        }

        write(
            &mut multisig.account,
            &MultisigRecord {
                format: STATE_FORMAT_V1,
                multisig_id,
                member_root,
                threshold,
                treasury: *treasury.account_id.value(),
                authority: *authority.account_id.value(),
            },
        )?;
        write(
            &mut treasury.account,
            &TreasuryRecord {
                format: STATE_FORMAT_V1,
                multisig_id,
                config_hash,
            },
        )?;

        Ok(SpelOutput::execute(
            vec![multisig, treasury, authority],
            vec![],
        ))
    }

    /// Move `amount` from the funder into this multisig's treasury.
    ///
    /// The funder is not this program's account to debit, so the decrease is
    /// declared as a chained call into the program that owns their balance —
    /// they signed this transaction, which is what authorises it. The increase
    /// on the treasury needs no authority: any program may raise any balance.
    ///
    /// Deliberately unrestricted as to who may fund: a donation to a multisig's
    /// treasury takes nothing from anyone, and requiring the creator would only
    /// mean a treasury cannot be topped up by the people it serves.
    ///
    /// Accounts:
    /// - `multisig` (PDA): required owned, and required to be the multisig this
    ///   treasury belongs to.
    /// - `treasury` (mut, PDA): the destination.
    /// - `funder` (mut, signer): pays.
    #[instruction]
    pub fn fund_treasury(
        ctx: ProgramContext,
        #[account(pda = [arg("multisig_id"), arg("config_hash")])]
        multisig: AccountWithMetadata,
        #[account(mut, pda = [arg("multisig_id"), arg("config_hash"), literal("treasury")])]
        treasury: AccountWithMetadata,
        #[account(mut, signer)] funder: AccountWithMetadata,
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        amount: u128,
    ) -> SpelResult {
        let _ = (multisig_id, config_hash);

        if multisig.account.program_owner != ctx.self_program_id {
            return Err(SpelError::custom(
                E_MULTISIG_NOT_ANCHORED,
                "no multisig is committed for this (id, config): it is not anchored",
            ));
        }
        // A treasury this program does not own is one it could never spend from,
        // so filling it would be a donation to nobody.
        if treasury.account.program_owner != ctx.self_program_id {
            return Err(SpelError::custom(
                E_TREASURY_UNOWNED,
                "the treasury account is not owned by this program",
            ));
        }
        let record = MultisigRecord::try_from_slice(&multisig.account.data)
            .map_err(|_| SpelError::custom(E_STATE_DECODE, "multisig record failed to decode"))?;
        if &record.treasury != treasury.account_id.value() {
            return Err(SpelError::custom(
                E_TREASURY_UNOWNED,
                "this treasury is not the one the multisig was created with",
            ));
        }
        if amount == 0 {
            return Err(SpelError::custom(E_BAD_AMOUNT, "zero funding amount"));
        }
        // Not "is it owned by something" but "is it owned by *the* transfer
        // program". LEZ deployment is permissionless, so a caller could hand us
        // an account owned by a program they wrote and we would chain into it.
        if funder.account.program_owner != AUTH_TRANSFER_PROGRAM_ID {
            return Err(SpelError::custom(
                E_FUNDER_NOT_TRANSFERABLE,
                "the funding account is not held by the native transfer program",
            ));
        }
        if funder.account.balance < amount {
            return Err(SpelError::custom(
                E_TREASURY_SHORT,
                "the funder cannot cover this funding",
            ));
        }

        let funding = nssa_core::program::ChainedCall::new(
            AUTH_TRANSFER_PROGRAM_ID,
            vec![funder.clone(), treasury.clone()],
            &AuthTransfer::Transfer { amount },
        );
        Ok(SpelOutput::execute(
            vec![multisig, treasury, funder],
            vec![funding],
        ))
    }

    /// Publish a proposal against an existing multisig.
    ///
    /// The action is not opaque any more: the instruction carries the fields the
    /// execution will pay out, and this instruction refuses them unless they
    /// hash to the `action_hash` the proposal's own address commits to. So the
    /// members who approve `proposal_ref` are approving exactly this recipient
    /// and exactly this amount, and the stored copy is verified at the moment it
    /// is written rather than trusted at the moment it is spent.
    ///
    /// Accounts:
    /// - `proposal` (init, PDA seeded by `[multisig_id, config_hash, proposal_ref]`).
    /// - `multisig` (PDA seeded by `[multisig_id, config_hash]`): required to be
    ///   owned by this program, so a proposal cannot be attached to a multisig
    ///   that was never created.
    /// - `authority` (signer): the proposer. Deliberately unrestricted — a
    ///   proposal on its own grants nothing; only M approvals do.
    #[instruction]
    pub fn create_proposal(
        #[account(init, pda = [arg("multisig_id"), arg("config_hash"), arg("proposal_ref")])]
        mut proposal: AccountWithMetadata,
        #[account(pda = [arg("multisig_id"), arg("config_hash")])]
        multisig: AccountWithMetadata,
        #[account(signer)] authority: AccountWithMetadata,
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        proposal_id: [u8; 32],
        action_hash: [u8; 32],
        proposal_ref: [u8; 32],
        recipient: [u8; 32],
        amount: u128,
        memo_hash: [u8; 32],
    ) -> SpelResult {
        if multisig.account.program_owner == nssa_core::program::DEFAULT_PROGRAM_ID {
            return Err(SpelError::custom(
                E_MULTISIG_NOT_ANCHORED,
                "no multisig is committed for this (id, config): it is not anchored",
            ));
        }

        let expected =
            multisig_core::compute_proposal_ref(&multisig_id, &config_hash, &proposal_id, &action_hash);
        if expected != proposal_ref {
            return Err(SpelError::custom(
                E_PROPOSAL_REF_MISMATCH,
                "proposal_ref does not commit to this (multisig, proposal, action)",
            ));
        }

        // A proposal that moves nothing would give the threshold nothing to
        // gate, which is the gap this program used to have.
        if amount == 0 {
            return Err(SpelError::custom(
                E_BAD_AMOUNT,
                "a proposal must move a non-zero amount",
            ));
        }

        let record = ProposalRecord {
            format: STATE_FORMAT_V1,
            multisig_id,
            config_hash,
            proposal_id,
            action_hash,
            recipient,
            amount,
            memo_hash,
            status: STATUS_OPEN,
        };
        // The action fields must be the ones `action_hash` — and therefore
        // `proposal_ref`, and therefore every approval — commits to.
        check_action_binds(&record, &multisig_id, &config_hash, &proposal_ref)?;

        // Paying the treasury out to itself would put the same account id twice
        // in one transaction, which LEZ refuses with an error naming neither the
        // proposal nor the recipient. Refused here, where the message can.
        let multisig_record = MultisigRecord::try_from_slice(&multisig.account.data)
            .map_err(|_| SpelError::custom(E_STATE_DECODE, "multisig record failed to decode"))?;
        if multisig_record.treasury == recipient {
            return Err(SpelError::custom(
                E_RECIPIENT_UNUSABLE,
                "a proposal cannot pay the treasury into itself",
            ));
        }

        write(&mut proposal.account, &record)?;

        Ok(SpelOutput::execute(
            vec![proposal, multisig, authority],
            vec![],
        ))
    }

    /// Approve a proposal by proving membership in the committed member set,
    /// without revealing which member.
    ///
    /// Accounts:
    /// - `approval_marker` (init, PDA seeded by `approval_marker_seed`): the
    ///   public, replay-guarded record that *some* member approved. Its address
    ///   reveals nothing about who, and neither does the record it carries.
    /// - `multisig` (PDA seeded by `[multisig_id, config_hash]`): the anchored
    ///   member set and threshold. Required to be owned by this program, so an
    ///   invented root, whose PDA was never initialised, is rejected.
    /// - `proposal` (PDA seeded by `[multisig_id, config_hash, proposal_ref]`):
    ///   the anchored action.
    /// - `approver` (signer): the account submitting the approval. Note this is
    ///   *not* the member's identity — a member may submit from any account, and
    ///   the proof binds the approval to the member's secret, not to this signer.
    ///
    /// Args:
    /// - `witness_words`: the approval witness, risc0-serde encoded. Carried to
    ///   the chained call. Safe only because a privacy transaction publishes no
    ///   instruction data; this must never be invoked on the public path.
    /// - the rest: the public statement the proof establishes.
    #[instruction]
    pub fn approve(
        #[account(init, pda = arg("approval_marker_seed"))]
        mut approval_marker: AccountWithMetadata,
        #[account(pda = [arg("multisig_id"), arg("config_hash")])]
        multisig: AccountWithMetadata,
        #[account(pda = [arg("multisig_id"), arg("config_hash"), arg("proposal_ref")])]
        proposal: AccountWithMetadata,
        #[account(signer)] approver: AccountWithMetadata,
        witness_words: Vec<u32>,
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        member_root: [u8; 32],
        threshold: u32,
        proposal_ref: [u8; 32],
        nullifier: [u8; 32],
        approval_marker_seed: [u8; 32],
    ) -> SpelResult {
        // 1. Decode the witness and rebuild the statement the proof establishes.
        let witness: multisig_core::ApproveWitness = risc0_zkvm::serde::from_slice(&witness_words)
            .map_err(|_| SpelError::custom(E_BAD_WITNESS, "witness_words did not decode"))?;

        // 2. Tie `config_hash` to the member root the proof will be checked
        //    against, and to the threshold the execution will be checked against.
        let expected_config = multisig_core::compute_config_hash(&member_root, threshold);
        if expected_config != config_hash {
            return Err(SpelError::custom(
                E_CONFIG_MISMATCH,
                "config_hash does not commit to this (member_root, threshold)",
            ));
        }

        // 3. Anchor the member set. The macro's
        //    `pda = [multisig_id, config_hash]` constraint already guarantees
        //    `multisig` is the PDA for exactly this config. Requiring it to be
        //    owned by this program rejects an invented member set: only
        //    `create_multisig` initialises these PDAs, so a fabricated root
        //    lands on an uninitialised address whose owner is the default.
        if multisig.account.program_owner == nssa_core::program::DEFAULT_PROGRAM_ID {
            return Err(SpelError::custom(
                E_MULTISIG_NOT_ANCHORED,
                "no multisig is committed for this (id, config): the member set is not anchored",
            ));
        }

        // 4. Anchor the action. Approving requires naming the true
        //    `proposal_ref`, which commits to the exact action bytes.
        if proposal.account.program_owner == nssa_core::program::DEFAULT_PROGRAM_ID {
            return Err(SpelError::custom(
                E_PROPOSAL_NOT_ANCHORED,
                "no proposal is committed at this proposal_ref",
            ));
        }

        // 5. Re-derive the nullifier from the witness secret and the proposal,
        //    and require it to equal the pinned one. Without this a caller could
        //    prove one approval while occupying another member's marker.
        let derived = multisig_core::compute_approval_nullifier(&proposal_ref, &witness.msk);
        if derived != nullifier {
            return Err(SpelError::custom(
                E_NULLIFIER_MISMATCH,
                "nullifier does not match the supplied witness",
            ));
        }

        // 6. Bind the marker address to the proposal and the nullifier, so the
        //    public trace records which proposal was approved and `execute` can
        //    re-derive exactly this address from the nullifier it is handed.
        let expected_seed = multisig_core::compute_approval_marker(&proposal_ref, &nullifier);
        if expected_seed != approval_marker_seed {
            return Err(SpelError::custom(
                E_MARKER_SEED_MISMATCH,
                "approval_marker_seed does not commit to the proposal and nullifier",
            ));
        }

        // 7. Declare the chained call. The privacy circuit executes and proves
        //    the membership program, then discharges the assumption with
        //    env::verify over its ProgramOutput. Merkle membership against
        //    `member_root` and the nullifier derivation are proved there, in
        //    zero knowledge.
        let instruction = multisig_core::ApproveInstruction {
            witness,
            statement: multisig_core::ApproveStatement {
                member_root,
                proposal_ref,
                nullifier,
            },
        };
        let chained = vec![nssa_core::program::ChainedCall::new(
            MEMBERSHIP_LEZ_PROGRAM_ID,
            Vec::new(),
            &instruction,
        )];

        // 8. Claim the marker PDA and record what it stands for. Both values are
        //    already implied by the address; writing them makes the marker
        //    self-describing to a reader who has only found it.
        write(
            &mut approval_marker.account,
            &ApprovalMarkerRecord {
                format: STATE_FORMAT_V1,
                proposal_ref,
                nullifier,
            },
        )?;

        Ok(SpelOutput::execute(
            vec![approval_marker, multisig, proposal, approver],
            chained,
        ))
    }

    /// Execute a proposal once the anchored threshold of distinct approvals
    /// exists — and pay it.
    ///
    /// This is the instruction that makes the multisig an M-of-N gate rather
    /// than a collection of independent approvals. It counts, on chain, and it
    /// counts *distinct members* — see check 6 below. Then it moves the money:
    /// the treasury this program owns is debited and the recipient the proposal
    /// named is credited, in the same transaction, so LEZ's balance-preservation
    /// rule is satisfied by construction.
    ///
    /// Anyone may execute; the executor need not be a member. That is
    /// deliberate, and it is what makes the completed execution unlinkable to
    /// any member's account: the transaction that lands carries the executor's
    /// signature, and the executor can be a disinterested relayer. It is safe
    /// precisely because the executor chooses nothing — the recipient and the
    /// amount come from the proposal record, which is checked against the
    /// address the approvals were bound to.
    ///
    /// Accounts:
    /// - `execution_marker` (init, PDA seeded by `execution_marker_seed`): the
    ///   proof-of-execution a downstream integration consumes, carrying the
    ///   nullifiers it consumed. `init` refuses to overwrite, so a proposal
    ///   executes at most once.
    /// - `multisig`, `proposal`: the anchored config and action.
    /// - `treasury` (mut, PDA): debited.
    /// - `recipient` (mut): credited. Not a PDA and not a signer — crediting
    ///   needs no authority, and requiring the payee to sign would make a
    ///   treasury payment need the payee's cooperation.
    /// - `executor` (signer): pays and authors.
    /// - `approvals` (rest): the M approval markers being counted. The macro
    ///   does not constrain rest-account addresses, so each is re-derived and
    ///   checked explicitly below.
    #[instruction]
    pub fn execute(
        ctx: ProgramContext,
        #[account(init, pda = arg("execution_marker_seed"))]
        mut execution_marker: AccountWithMetadata,
        #[account(pda = [arg("multisig_id"), arg("config_hash")])]
        multisig: AccountWithMetadata,
        #[account(mut, pda = [arg("multisig_id"), arg("config_hash"), arg("proposal_ref")])]
        mut proposal: AccountWithMetadata,
        #[account(mut, pda = [arg("multisig_id"), arg("config_hash"), literal("treasury")])]
        mut treasury: AccountWithMetadata,
        #[account(mut)] mut recipient: AccountWithMetadata,
        #[account(signer)] executor: AccountWithMetadata,
        approvals: Vec<AccountWithMetadata>,
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        member_root: [u8; 32],
        threshold: u32,
        proposal_ref: [u8; 32],
        approval_nullifiers: Vec<[u8; 32]>,
        execution_marker_seed: [u8; 32],
    ) -> SpelResult {
        // 1. Tie config_hash to the pair it commits to. This is what stops an
        //    executor from supplying `threshold = 1` against a 3-of-5 set: a
        //    different threshold yields a different config hash, hence a
        //    different multisig PDA, which check 2 then finds uninitialised.
        let expected_config = multisig_core::compute_config_hash(&member_root, threshold);
        if expected_config != config_hash {
            return Err(SpelError::custom(
                E_CONFIG_MISMATCH,
                "config_hash does not commit to this (member_root, threshold)",
            ));
        }

        // 2. The anchored multisig. Its address encodes the threshold enforced
        //    in check 5.
        if multisig.account.program_owner == nssa_core::program::DEFAULT_PROGRAM_ID {
            return Err(SpelError::custom(
                E_MULTISIG_NOT_ANCHORED,
                "no multisig is committed for this (id, config): threshold is not anchored",
            ));
        }

        // 3. The anchored proposal, hence the exact action being executed.
        if proposal.account.program_owner == nssa_core::program::DEFAULT_PROGRAM_ID {
            return Err(SpelError::custom(
                E_PROPOSAL_NOT_ANCHORED,
                "no proposal is committed at this proposal_ref",
            ));
        }

        // 4. The execution marker is scoped to this proposal.
        let expected_exec = multisig_core::compute_execution_marker(&proposal_ref);
        if expected_exec != execution_marker_seed {
            return Err(SpelError::custom(
                E_MARKER_SEED_MISMATCH,
                "execution_marker_seed does not commit to this proposal",
            ));
        }

        // 5. Enough approvals, against the anchored threshold.
        if approvals.len() != approval_nullifiers.len() {
            return Err(SpelError::custom(
                E_APPROVAL_COUNT_MISMATCH,
                "each approval account must be paired with its nullifier",
            ));
        }
        if approvals.len() < threshold as usize {
            return Err(SpelError::custom(
                E_THRESHOLD_NOT_MET,
                "fewer approvals than the anchored threshold",
            ));
        }

        // 6. Distinctness, checked pairwise. This is the step that turns "M
        //    approval accounts" into "M distinct members": each marker address
        //    is a function of a nullifier, and each nullifier is a function of a
        //    member's secret, so two different addresses imply two different
        //    secrets. Presenting the same marker M times is caught here.
        //
        //    Quadratic in M on purpose: M is a multisig threshold, a small
        //    number, and a sort would cost more cycles than it saves. The CU
        //    measurements in docs/cu-costs.md cover M up to 7.
        for i in 0..approval_nullifiers.len() {
            for j in (i + 1)..approval_nullifiers.len() {
                if approval_nullifiers[i] == approval_nullifiers[j] {
                    return Err(SpelError::custom(
                        E_DUPLICATE_APPROVAL,
                        "the same approval was presented more than once",
                    ));
                }
            }
        }

        // 7. Every account presented is genuinely the approval marker for its
        //    nullifier *on this proposal*, and was genuinely claimed by this
        //    program — which is the only way it could have come into existence,
        //    and only ever after a membership proof was verified on chain.
        for (account, nullifier) in approvals.iter().zip(approval_nullifiers.iter()) {
            let seed = multisig_core::compute_approval_marker(&proposal_ref, nullifier);
            let expected_id = compute_pda(&ctx.self_program_id, &[&seed]);
            if account.account_id != expected_id {
                return Err(SpelError::custom(
                    E_APPROVAL_NOT_FOR_PROPOSAL,
                    "an approval account is not the marker PDA for its nullifier on this proposal",
                ));
            }
            if account.account.program_owner != ctx.self_program_id {
                return Err(SpelError::custom(
                    E_APPROVAL_NOT_ANCHORED,
                    "an approval marker was never claimed by this program",
                ));
            }
        }

        // 8. Read the action back and prove it is the action the approvals were
        //    gathered for. Everything after this point spends money, and this is
        //    the check that decides what "this action" means.
        let mut record = ProposalRecord::try_from_slice(&proposal.account.data)
            .map_err(|_| SpelError::custom(E_STATE_DECODE, "proposal record failed to decode"))?;
        check_action_binds(&record, &multisig_id, &config_hash, &proposal_ref)?;
        if record.status != STATUS_OPEN {
            return Err(SpelError::custom(
                E_ALREADY_EXECUTED,
                "this proposal is already marked executed",
            ));
        }

        // 9. The treasury must be ours to debit, and must be the one this
        //    multisig was created with. The macro already constrains its
        //    address; this pins it to the multisig's own record as well, so the
        //    two independent bindings would both have to fail together.
        if treasury.account.program_owner != ctx.self_program_id {
            return Err(SpelError::custom(
                E_TREASURY_UNOWNED,
                "the treasury account is not owned by this program",
            ));
        }
        let multisig_record = MultisigRecord::try_from_slice(&multisig.account.data)
            .map_err(|_| SpelError::custom(E_STATE_DECODE, "multisig record failed to decode"))?;
        if &multisig_record.treasury != treasury.account_id.value() {
            return Err(SpelError::custom(
                E_TREASURY_UNOWNED,
                "this treasury is not the one the multisig was created with",
            ));
        }

        // 10. The recipient is the one the proposal named, and is an account
        //     that can actually spend what it receives. Paying into an account
        //     the native transfer program does not own would move the balance
        //     somewhere nobody can move it out of again, which is a burn
        //     wearing a payment's clothes.
        if record.recipient != *recipient.account_id.value() {
            return Err(SpelError::custom(
                E_RECIPIENT_MISMATCH,
                "the recipient account presented is not the one this proposal names",
            ));
        }
        if recipient.account.program_owner != AUTH_TRANSFER_PROGRAM_ID {
            return Err(SpelError::custom(
                E_RECIPIENT_UNUSABLE,
                "the recipient is not held by the native transfer program",
            ));
        }

        // 11. Debit ours, credit theirs. Checked rather than saturating: an
        //     underflow here would mean the treasury promised more than it ever
        //     received, which is a bug to surface, not to absorb. The two moves
        //     are equal and both accounts are in this instruction's post-states,
        //     which is what satisfies LEZ's balance-preservation rule.
        treasury.account.balance = treasury
            .account
            .balance
            .checked_sub(record.amount)
            .ok_or_else(|| {
                SpelError::custom(E_TREASURY_SHORT, "the treasury cannot cover this proposal")
            })?;
        recipient.account.balance = recipient
            .account
            .balance
            .checked_add(record.amount)
            .ok_or_else(|| {
                SpelError::custom(E_TREASURY_SHORT, "the recipient's balance would overflow")
            })?;

        // 12. Record the outcome in both places a reader might look: the
        //     proposal flips to executed, and the marker carries the full audit
        //     trail. The marker's *existence* remains the authoritative replay
        //     guard — `init` refuses to overwrite — and the flag is its readable
        //     mirror, not a second gate that could disagree with it.
        record.status = STATUS_EXECUTED;
        write(&mut proposal.account, &record)?;
        write(
            &mut execution_marker.account,
            &ExecutionMarkerRecord {
                format: STATE_FORMAT_V1,
                proposal_ref,
                recipient: record.recipient,
                amount: record.amount,
                status: STATUS_EXECUTED,
                nullifiers: approval_nullifiers.clone(),
            },
        )?;

        let mut accounts = vec![
            execution_marker,
            multisig,
            proposal,
            treasury,
            recipient,
            executor,
        ];
        accounts.extend(approvals);
        Ok(SpelOutput::execute(accounts, vec![]))
    }
}
