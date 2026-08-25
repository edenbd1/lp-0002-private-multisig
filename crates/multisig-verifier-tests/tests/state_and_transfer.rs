//! What the threshold *does*, and what a third party can read afterwards.
//!
//! The tests in `verifier_rejects.rs` establish that the gate cannot be forced.
//! These establish that passing through it has an effect: real balances move,
//! and every account the program claims comes out carrying a record that decodes
//! at the offsets `docs/account-layout.md` publishes.
//!
//! They read the guest's committed `ProgramOutput` rather than only its exit
//! status. That journal is exactly what the sequencer applies to the chain, so
//! an assertion on a post-state here is an assertion about what `getAccount`
//! would return — the same claim, one layer earlier and without a testnet.
//!
//! Every decode goes through `multisig_core::state`, which reads by byte offset
//! and knows nothing about borsh. If the program's encoding and the published
//! table ever disagree, these fail.
//!
//! Run with: `cargo test -p multisig-verifier-tests --test state_and_transfer`

use lee_core::account::AccountId;
use lee_core::program::ProgramId;
use multisig_core::state::*;
use multisig_core::*;
use multisig_verifier_tests::*;

/// Sum of every balance in a list of accounts, as `u128` cannot overflow across
/// the handful of accounts one instruction touches.
fn total(balances: impl IntoIterator<Item = u128>) -> u128 {
    balances.into_iter().sum()
}

// ---------------------------------------------------------------------------
// create_multisig — the configuration becomes readable, not merely checkable
// ---------------------------------------------------------------------------

/// The multisig account used to come back from `getAccount` with `data: []`.
/// Anyone holding the address could confirm a guess at `(root, threshold)` by
/// re-deriving the PDA, and could discover neither. Now the account says so.
#[test]
fn create_multisig_writes_a_record_a_stranger_can_read() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let out = output(
        &elf,
        &pid,
        f.create_multisig_accounts(f.config_hash),
        &f.create_multisig_ix(),
    )
    .expect("an honest multisig must be creatable");

    let (_, multisig) = state_of(&out, &f.multisig_id_addr());
    assert!(
        !multisig.data.is_empty(),
        "the multisig account must not come back empty — that was the whole defect"
    );
    let record = decode_multisig(&multisig.data).expect("the multisig record must decode");
    assert_eq!(record.format, STATE_FORMAT_V1);
    assert_eq!(record.multisig_id, f.multisig_id);
    assert_eq!(record.member_root, f.member_root);
    assert_eq!(record.threshold, f.threshold);
    assert_eq!(
        record.treasury,
        *f.treasury_addr().value(),
        "the record must name the treasury PDA the same instruction created"
    );
    assert_eq!(record.authority, FIXTURE_AUTHORITY);
    assert_eq!(
        multisig.data.len(),
        MULTISIG_LEN,
        "the published length must be the length actually written"
    );

    // And the treasury, which exists so the threshold has something to spend.
    let (_, treasury) = state_of(&out, &f.treasury_addr());
    let t = decode_treasury(&treasury.data).expect("the treasury record must decode");
    assert_eq!(t.multisig_id, f.multisig_id);
    assert_eq!(t.config_hash, f.config_hash);
    assert_eq!(treasury.data.len(), TREASURY_LEN);
    assert_eq!(
        treasury.balance, 0,
        "a treasury is created empty; funding it is a separate transaction, \
         because a chained transfer reads a pre-state the initialisation has not written"
    );
}

// ---------------------------------------------------------------------------
// create_proposal — the action is stored, and checked against its own address
// ---------------------------------------------------------------------------

#[test]
fn create_proposal_persists_the_action_the_members_will_approve() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let out = output(
        &elf,
        &pid,
        f.create_proposal_accounts(f.proposal_ref, true),
        &f.create_proposal_ix(),
    )
    .expect("an honest proposal must be creatable");

    let (_, proposal) = state_of(&out, &f.proposal_addr());
    let record = decode_proposal(&proposal.data).expect("the proposal record must decode");
    assert_eq!(proposal.data.len(), PROPOSAL_LEN);
    assert_eq!(record.proposal_id, f.proposal_id);
    assert_eq!(record.action_hash, f.action_hash);
    assert_eq!(record.recipient, f.recipient);
    assert_eq!(record.amount, f.amount);
    assert_eq!(record.memo_hash, f.memo_hash);
    assert_eq!(
        record.status, STATUS_OPEN,
        "a freshly published proposal has not executed"
    );

    // The record is not merely stored: re-deriving the commitment chain from it
    // reproduces the address it lives at. That is what `execute` re-checks.
    let redone = compute_transfer_action_hash(
        &record.multisig_id,
        &record.recipient,
        record.amount,
        &record.memo_hash,
    );
    assert_eq!(redone, record.action_hash);
    assert_eq!(
        compute_proposal_ref(
            &record.multisig_id,
            &record.config_hash,
            &record.proposal_id,
            &record.action_hash,
        ),
        f.proposal_ref
    );
}

/// A proposal that moves nothing gives the threshold nothing to gate, which is
/// exactly the gap this program used to have.
#[test]
fn creating_a_proposal_that_moves_nothing_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    // A consistent zero-amount proposal: the hashes agree with each other, so
    // it is the amount check and nothing else that trips.
    let action_hash = compute_transfer_action_hash(&f.multisig_id, &f.recipient, 0, &f.memo_hash);
    let proposal_ref =
        compute_proposal_ref(&f.multisig_id, &f.config_hash, &f.proposal_id, &action_hash);
    let ix = VerifierInstruction::CreateProposal {
        multisig_id: f.multisig_id,
        config_hash: f.config_hash,
        proposal_id: f.proposal_id,
        action_hash,
        proposal_ref,
        recipient: f.recipient,
        amount: 0,
        memo_hash: f.memo_hash,
        rotate_to: [0u8; 32],
    };
    let err = run(
        &elf,
        &pid,
        f.create_proposal_accounts(proposal_ref, true),
        &ix,
    )
    .expect_err("a zero-amount proposal must be rejected");
    assert_rejected(err, 5017, "non-zero");
}

/// The bait-and-switch, moved down a layer. `action_hash` is inside
/// `proposal_ref`, which is the proposal's address, so a proposer who publishes
/// one recipient in the hash and a different one in the fields is caught here —
/// before any member has approved anything.
#[test]
fn creating_a_proposal_whose_fields_contradict_its_action_hash_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let attacker = [0x66; 32];
    let mut ix = f.create_proposal_ix();
    if let VerifierInstruction::CreateProposal { recipient, .. } = &mut ix {
        // The hash and the ref still describe the honest recipient.
        *recipient = attacker;
    }
    let err = run(
        &elf,
        &pid,
        f.create_proposal_accounts(f.proposal_ref, true),
        &ix,
    )
    .expect_err("fields that contradict the action hash must be rejected");
    assert_rejected(err, 5018, "does not hash");
}

/// Paying the treasury into itself would put one account id twice in a
/// transaction, which LEZ refuses with an error naming neither the proposal nor
/// the recipient. Refused at publication, where the message can say why.
#[test]
fn creating_a_proposal_that_pays_the_treasury_into_itself_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let treasury = *f.treasury_addr().value();
    let action_hash =
        compute_transfer_action_hash(&f.multisig_id, &treasury, f.amount, &f.memo_hash);
    let proposal_ref =
        compute_proposal_ref(&f.multisig_id, &f.config_hash, &f.proposal_id, &action_hash);
    let ix = VerifierInstruction::CreateProposal {
        multisig_id: f.multisig_id,
        config_hash: f.config_hash,
        proposal_id: f.proposal_id,
        action_hash,
        proposal_ref,
        recipient: treasury,
        amount: f.amount,
        memo_hash: f.memo_hash,
        rotate_to: [0u8; 32],
    };
    let err = run(
        &elf,
        &pid,
        f.create_proposal_accounts(proposal_ref, true),
        &ix,
    )
    .expect_err("a proposal paying the treasury into itself must be rejected");
    assert_rejected(err, 5020, "into itself");
}

// ---------------------------------------------------------------------------
// approve — the marker says what it is
// ---------------------------------------------------------------------------

/// The marker's address already implies the proposal and the nullifier. Writing
/// them makes it self-describing to a reader who has only found the account —
/// and it must contain nothing else, because anything derived from the witness
/// would be a privacy leak in a public account.
#[test]
fn approve_records_the_proposal_and_the_nullifier_it_spent() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let out = output(
        &elf,
        &pid,
        f.approve_accounts(0, true, true),
        &f.approve_ix(0),
    )
    .expect("a member of an anchored set must be able to approve");

    let marker_addr = public_pda(&pid, &[f.marker_seed(0)]);
    let (_, marker) = state_of(&out, &marker_addr);
    let record = decode_approval_marker(&marker.data).expect("the marker record must decode");
    assert_eq!(marker.data.len(), APPROVAL_MARKER_LEN);
    assert_eq!(record.proposal_ref, f.proposal_ref);
    assert_eq!(record.nullifier, f.nullifier(0));

    // The record is exactly the two values the address already commits to, and
    // no more: 1 + 32 + 32. A member's secret, identifier, salt or leaf index
    // appearing here would be readable by everyone.
    assert_eq!(
        APPROVAL_MARKER_LEN,
        1 + 32 + 32,
        "the marker must carry the proposal and the nullifier, and nothing else"
    );

    // The chained call to the membership program is still declared: persistence
    // must not have displaced the thing that makes the approval a proof.
    assert_eq!(
        out.chained_calls.len(),
        1,
        "approve must still chain into the membership program"
    );
}

// ---------------------------------------------------------------------------
// fund_treasury — value gets in
// ---------------------------------------------------------------------------

/// The funder is not this program's account to debit, so the decrease is
/// delegated to the program that owns their balance — pinned by id, not read off
/// the account, so a caller cannot point us at a program they wrote.
#[test]
fn fund_treasury_chains_into_the_pinned_transfer_program() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let funder = funding_signer([0xF0; 32], 1_000);

    let out = output(&elf, &pid, f.fund_accounts(funder.clone()), &f.fund_ix(400))
        .expect("an honest funding must be accepted");

    assert_eq!(out.chained_calls.len(), 1, "funding is one chained call");
    let call = &out.chained_calls[0];
    assert_eq!(
        call.program_id, AUTH_TRANSFER_PROGRAM_ID,
        "the callee must be the pinned native transfer program"
    );
    let ids: Vec<AccountId> = call.pre_states.iter().map(|p| p.account_id).collect();
    assert_eq!(
        ids,
        vec![funder.account_id, f.treasury_addr()],
        "the transfer program takes [sender, recipient], in that order"
    );

    // This instruction must not also move the balance itself. Two competing
    // post-states for one account are refused by the runtime, silently, and the
    // shape that survives is: chain the call, touch nothing.
    let (pre_t, post_t) = state_of(&out, &f.treasury_addr());
    assert_eq!(
        pre_t.balance, post_t.balance,
        "the chained call moves the balance; this instruction must not"
    );
    let (pre_f, post_f) = state_of(&out, &funder.account_id);
    assert_eq!(pre_f.balance, post_f.balance);
}

/// The security boundary the pin exists for: an account owned by a program the
/// caller wrote must not be accepted as a funder.
#[test]
fn fund_treasury_refuses_a_funder_the_transfer_program_does_not_own() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let attacker_program: ProgramId = [7; 8];
    let mut funder = owned_with(
        attacker_program,
        AccountId::new([0xF1; 32]),
        1_000,
        Vec::new(),
    );
    funder.is_authorized = true;

    let err = run(&elf, &pid, f.fund_accounts(funder), &f.fund_ix(400))
        .expect_err("a funder held by a foreign program must be refused");
    assert_rejected(err, 5016, "native transfer program");
}

#[test]
fn fund_treasury_refuses_a_funder_that_cannot_cover_it() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let err = run(
        &elf,
        &pid,
        f.fund_accounts(funding_signer([0xF2; 32], 10)),
        &f.fund_ix(400),
    )
    .expect_err("a funder short of the amount must be refused");
    assert_rejected(err, 5015, "cannot cover");
}

#[test]
fn fund_treasury_refuses_a_zero_amount() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let err = run(
        &elf,
        &pid,
        f.fund_accounts(funding_signer([0xF3; 32], 1_000)),
        &f.fund_ix(0),
    )
    .expect_err("funding nothing must be refused");
    assert_rejected(err, 5017, "zero funding");
}

// ---------------------------------------------------------------------------
// execute — value gets out, and only through the gate
// ---------------------------------------------------------------------------

/// The criterion in one test: a threshold of private approvals produces a real
/// debit and a real credit.
#[test]
fn execute_moves_the_treasury_balance_to_the_recipient() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let out = output(
        &elf,
        &pid,
        f.execute_accounts(&[0, 2, 4]),
        &f.execute_ix(&[0, 2, 4]),
    )
    .expect("three distinct approvals must satisfy a 3-of-5 threshold");

    let (pre_t, post_t) = state_of(&out, &f.treasury_addr());
    let (pre_r, post_r) = state_of(&out, &f.recipient_addr());

    assert_eq!(pre_t.balance, FIXTURE_TREASURY_BALANCE);
    assert_eq!(post_t.balance, FIXTURE_TREASURY_BALANCE - FIXTURE_AMOUNT);
    assert_eq!(pre_r.balance, 0);
    assert_eq!(post_r.balance, FIXTURE_AMOUNT);
}

/// LEZ rule 8 requires the debit and the credit to balance inside one
/// transaction (`lee/state_machine/core/src/program/mod.rs:741-760`). Asserted
/// here rather than left to the sequencer, because a program that fails it fails
/// *after* a proof has been generated.
#[test]
fn execute_preserves_the_total_balance() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let out = output(
        &elf,
        &pid,
        f.execute_accounts(&[0, 2, 4]),
        &f.execute_ix(&[0, 2, 4]),
    )
    .expect("the honest execution must be accepted");

    let before = total(out.pre_states.iter().map(|p| p.account.balance));
    let after = total(out.post_states.iter().map(|p| p.account().balance));
    assert_eq!(
        before, after,
        "total balance must be preserved across the instruction"
    );

    // Conservation is satisfied trivially by an instruction that moves nothing,
    // which is precisely the shape this revision replaced — so the test has to
    // also insist that something moved. Caught by mutation testing: with the
    // debit and credit deleted, the assertion above still passed.
    let (pre_t, post_t) = state_of(&out, &f.treasury_addr());
    assert_ne!(
        pre_t.balance, post_t.balance,
        "the treasury must actually have been debited"
    );
    let (pre_r, post_r) = state_of(&out, &f.recipient_addr());
    assert_eq!(
        post_t.balance + post_r.balance,
        pre_t.balance + pre_r.balance
    );
    assert_eq!(
        pre_t.balance - post_t.balance,
        post_r.balance - pre_r.balance
    );
}

/// The audit trail: which nullifiers were spent, on what, and for how much.
#[test]
fn execute_records_the_nullifiers_it_consumed_and_flags_the_proposal() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let members = [0usize, 2, 4];

    let out = output(
        &elf,
        &pid,
        f.execute_accounts(&members),
        &f.execute_ix(&members),
    )
    .expect("the honest execution must be accepted");

    let (_, marker) = state_of(&out, &f.execution_addr());
    let record = decode_execution_marker(&marker.data).expect("the execution record must decode");
    assert_eq!(
        marker.data.len(),
        EXECUTION_MARKER_HEADER_LEN + members.len() * 32
    );
    assert_eq!(record.proposal_ref, f.proposal_ref);
    assert_eq!(record.recipient, f.recipient);
    assert_eq!(record.amount, f.amount);
    assert_eq!(record.status, STATUS_EXECUTED);
    assert_eq!(
        record.nullifiers,
        members.iter().map(|&m| f.nullifier(m)).collect::<Vec<_>>(),
        "the marker must list the nullifiers, in the order they were presented"
    );

    // The proposal is flagged too, so a reader who has only its address learns
    // the outcome without having to derive the execution marker's.
    let (_, proposal) = state_of(&out, &f.proposal_addr());
    let p = decode_proposal(&proposal.data).expect("the proposal record must decode");
    assert_eq!(p.status, STATUS_EXECUTED);
    assert_eq!(
        p.action_hash, f.action_hash,
        "flipping the status must not disturb anything else in the record"
    );
}

/// A treasury short of the proposal is a refusal, not a partial payment.
#[test]
fn executing_a_proposal_the_treasury_cannot_cover_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let mut f = Fixture::new(&pid);
    f.treasury_balance = FIXTURE_AMOUNT - 1;

    let err = run(
        &elf,
        &pid,
        f.execute_accounts(&[0, 2, 4]),
        &f.execute_ix(&[0, 2, 4]),
    )
    .expect_err("a treasury short of the proposal must refuse to pay");
    assert_rejected(err, 5015, "cannot cover");
}

/// The executor need not be a member, and need not be trusted — because they
/// choose nothing. Substituting their own account as the payee is refused: the
/// recipient comes from the proposal record the approvals were bound to.
#[test]
fn an_executor_cannot_redirect_the_payment_to_themselves() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[4] = held_by_transfer([0xEE; 32], 0);

    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2, 4]))
        .expect_err("the executor must not be able to name the payee");
    assert_rejected(err, 5019, "not the one this proposal names");
}

/// Paying into an account the native transfer program does not own would move
/// the balance somewhere nobody can move it out of again — a burn wearing a
/// payment's clothes.
#[test]
fn executing_into_an_account_that_could_never_spend_it_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    // Right address, wrong custodian: a default-owned account at the recipient's
    // id. It passes the identity check and fails the usability one.
    accounts[4] = uninitialised(f.recipient_addr());

    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2, 4]))
        .expect_err("an unusable recipient must be refused");
    assert_rejected(err, 5020, "native transfer program");
}

/// The persisted action is checked against the address the approvals were bound
/// to, on every execution. Rewriting the amount in the stored record — which
/// only this program can do, and which a future bug might — cannot make the
/// treasury pay it.
#[test]
fn a_tampered_proposal_record_cannot_be_paid_out() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let mut tampered = f.proposal_record();
    tampered.amount = FIXTURE_TREASURY_BALANCE; // drain it instead

    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[2] = f.proposal_account_holding(true, &tampered);

    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2, 4]))
        .expect_err("a record that does not re-derive its own address must not be paid");
    assert_rejected(err, 5018, "does not hash");
}

/// The execution marker's `init` is the authoritative replay guard. The status
/// flag is its readable mirror — and it refuses a replay too, so the two cannot
/// disagree about whether a proposal has been paid.
#[test]
fn executing_a_proposal_already_flagged_executed_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let mut done = f.proposal_record();
    done.status = STATUS_EXECUTED;

    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[2] = f.proposal_account_holding(true, &done);

    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2, 4]))
        .expect_err("a proposal already marked executed must not pay twice");
    assert_rejected(err, 5021, "already marked executed");
}

/// The treasury is pinned twice: by its PDA address, and by the address the
/// multisig recorded at creation. Substituting a treasury the multisig does not
/// name would have to defeat both.
#[test]
fn executing_against_a_treasury_the_multisig_does_not_name_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let mut record = f.multisig_record();
    record.treasury = [0x99; 32];

    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[1] = owned_with(pid, f.multisig_id_addr(), 0, encode_multisig(&record));

    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2, 4]))
        .expect_err("a treasury the multisig does not name must be refused");
    assert_rejected(err, 5014, "not the one the multisig was created with");
}

// ---------------------------------------------------------------------------
// The document is the interface
// ---------------------------------------------------------------------------

/// `docs/account-layout.md` publishes a byte offset per field. This test is what
/// makes that a promise rather than a description: every record the program
/// writes is decoded by `multisig_core::state`, which reads those offsets and
/// has never heard of borsh. Encoding drift lands here.
#[test]
fn every_record_the_program_writes_decodes_at_the_published_offsets() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let created = output(
        &elf,
        &pid,
        f.create_multisig_accounts(f.config_hash),
        &f.create_multisig_ix(),
    )
    .expect("create_multisig");
    assert_eq!(
        state_of(&created, &f.multisig_id_addr()).1.data.len(),
        MULTISIG_LEN
    );
    assert_eq!(
        state_of(&created, &f.treasury_addr()).1.data.len(),
        TREASURY_LEN
    );

    let proposed = output(
        &elf,
        &pid,
        f.create_proposal_accounts(f.proposal_ref, true),
        &f.create_proposal_ix(),
    )
    .expect("create_proposal");
    assert_eq!(
        state_of(&proposed, &f.proposal_addr()).1.data.len(),
        PROPOSAL_LEN
    );

    let approved = output(
        &elf,
        &pid,
        f.approve_accounts(0, true, true),
        &f.approve_ix(0),
    )
    .expect("approve");
    assert_eq!(
        state_of(&approved, &public_pda(&pid, &[f.marker_seed(0)]))
            .1
            .data
            .len(),
        APPROVAL_MARKER_LEN
    );

    let executed = output(
        &elf,
        &pid,
        f.execute_accounts(&[0, 2, 4]),
        &f.execute_ix(&[0, 2, 4]),
    )
    .expect("execute");
    assert_eq!(
        state_of(&executed, &f.execution_addr()).1.data.len(),
        EXECUTION_MARKER_HEADER_LEN + 3 * 32
    );

    // And the bytes the fixture builds host-side are byte-identical to the ones
    // the guest writes, which is what lets every other test in this file build
    // realistic pre-states rather than approximate ones.
    assert_eq!(
        state_of(&created, &f.multisig_id_addr()).1.data.as_ref(),
        encode_multisig(&f.multisig_record()).as_slice(),
        "the host encoder and the guest must agree byte for byte"
    );
    assert_eq!(
        state_of(&proposed, &f.proposal_addr()).1.data.as_ref(),
        encode_proposal(&f.proposal_record()).as_slice()
    );
}

// ---------------------------------------------------------------------------
// The records this program owns must be readable, or nothing proceeds
// ---------------------------------------------------------------------------

/// An account this program owns whose bytes it cannot read is a bug, a partial
/// write, or a layout migration nobody finished. Whichever it is, the answer is
/// a documented refusal rather than a guest panic with no code attached — and
/// certainly not a payment computed from bytes that did not parse.
#[test]
fn a_multisig_record_that_does_not_decode_stops_a_proposal() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let mut accounts = f.create_proposal_accounts(f.proposal_ref, true);
    accounts[1] = owned_with(pid, f.multisig_id_addr(), 0, vec![0xFF; 7]);

    let err = run(&elf, &pid, accounts, &f.create_proposal_ix())
        .expect_err("an unreadable multisig record must stop the proposal");
    assert_rejected(err, 5022, "failed to decode");
}

/// The same, on the path that spends money.
#[test]
fn a_proposal_record_that_does_not_decode_stops_the_payment() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[2] = owned_with(pid, f.proposal_addr(), 0, vec![0x01, 0x02, 0x03]);

    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2, 4]))
        .expect_err("an unreadable proposal record must stop the payment");
    assert_rejected(err, 5022, "failed to decode");
}
