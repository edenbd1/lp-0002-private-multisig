//! Spending tiers and configuration rotation, against the committed binary.
//!
//! Both features widen what the program accepts, so both are tested the way the
//! rest of this suite tests widening: the honest case as a control, then every
//! way the widening could be turned into an attack. A tier that could raise the
//! bar, a tier table substituted at call time, a rotation cheaper than the
//! threshold it rewrites, an old configuration still able to spend — each has a
//! test here, and each of those tests fails if the corresponding check is
//! removed.
//!
//! These run through the sequencer's own execution path and do not prove: a
//! rejection here is the rejection the chain performs.
//!
//! Run with: `cargo test -p multisig-verifier-tests --test tiers_and_rotation`

use multisig_core::state::*;
use multisig_core::*;
use multisig_verifier_tests::*;

/// A 3-of-5 whose first 300 units need only two approvals.
///
/// `FIXTURE_AMOUNT` is 250, so the fixture's own proposal sits under the cap and
/// the default proposal is the tiered one.
const SMALL_SPEND_TIER: [(u128, u32); 1] = [(300, 2)];

// ---------------------------------------------------------------------------
// Tiers — the honest cases
// ---------------------------------------------------------------------------

/// The control. Without a tier, two approvals are one short of 3-of-5, so
/// everything below that *accepts* two approvals is accepting them because of
/// the tier and not for some other reason.
#[test]
fn without_a_tier_two_approvals_are_one_short() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let err = run(
        &elf,
        &pid,
        f.execute_accounts(&[0, 2]),
        &f.execute_ix(&[0, 2]),
    )
    .expect_err("two approvals cannot satisfy a 3-of-5 with no tiers");
    assert_rejected(err, 5010, "threshold");
}

#[test]
fn a_tier_lets_a_small_transfer_execute_below_the_default_threshold() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid).with_tiers(&SMALL_SPEND_TIER);
    assert!(
        f.amount <= SMALL_SPEND_TIER[0].0,
        "this test is only meaningful if the proposal is under the cap"
    );

    let out = output(
        &elf,
        &pid,
        f.execute_accounts(&[0, 2]),
        &f.execute_ix(&[0, 2]),
    )
    .expect("two approvals must satisfy a tier that asks for two");

    let (pre_r, post_r) = state_of(&out, &f.recipient_addr());
    assert_eq!(pre_r.balance, 0);
    assert_eq!(
        post_r.balance, f.amount,
        "the transfer the tier authorised must actually have moved"
    );
}

/// The same tier, the same two approvals, an amount over the cap. This is the
/// property that makes a tier table safe to anchor: it prices *small* spends,
/// and says nothing about large ones.
#[test]
fn the_same_two_approvals_do_not_carry_a_transfer_above_the_cap() {
    let elf = elf();
    let pid = program_id(&elf);
    let over = SMALL_SPEND_TIER[0].0 + 1;
    let f = Fixture::new(&pid)
        .with_tiers(&SMALL_SPEND_TIER)
        .with_amount(over);
    assert!(
        over <= f.treasury_balance,
        "the refusal must be about the threshold, not about an empty treasury"
    );

    let err = run(
        &elf,
        &pid,
        f.execute_accounts(&[0, 2]),
        &f.execute_ix(&[0, 2]),
    )
    .expect_err("above the cap, the default threshold applies again");
    assert_rejected(err, 5010, "threshold");

    // And the control on the control: three approvals do carry it, so the
    // refusal above is the cap and not something broken about this fixture.
    run(
        &elf,
        &pid,
        f.execute_accounts(&[0, 2, 4]),
        &f.execute_ix(&[0, 2, 4]),
    )
    .expect("three approvals satisfy the default threshold at any amount");
}

// ---------------------------------------------------------------------------
// Tiers — the tables that must not be anchorable
// ---------------------------------------------------------------------------

/// A tier may only ever *lower* the bar. One that asks for more than the
/// default would let a member set write a rule the threshold does not
/// authorise, and it is refused before it can be anchored.
#[test]
fn a_tier_may_not_ask_for_more_than_the_default_threshold() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid).with_tiers(&[(300, 4)]);
    assert_eq!(f.threshold, 3, "the fixture default this tier exceeds");

    let err = run(
        &elf,
        &pid,
        f.create_multisig_accounts(f.config_hash),
        &f.create_multisig_ix(),
    )
    .expect_err("a tier above the default threshold must not be anchorable");
    assert_rejected(err, 5023, "tier");
}

/// Caps must strictly increase. Two tiers at the same cap, or in descending
/// order, make "which tier applies" depend on iteration order rather than on
/// the amount — so the table is refused rather than resolved.
#[test]
fn tier_caps_that_do_not_strictly_increase_are_refused() {
    let elf = elf();
    let pid = program_id(&elf);
    for table in [
        vec![(300u128, 2u32), (200, 3)],
        vec![(300u128, 2u32), (300, 3)],
    ] {
        let f = Fixture::new(&pid).with_tiers(&table);
        let err = run(
            &elf,
            &pid,
            f.create_multisig_accounts(f.config_hash),
            &f.create_multisig_ix(),
        )
        .expect_err("caps must strictly increase");
        assert_rejected(err, 5023, "tier");
    }
}

/// Thresholds must not decrease as the amount grows: a table where a larger
/// transfer is cheaper than a smaller one inverts the whole point.
#[test]
fn a_tier_that_makes_a_larger_transfer_cheaper_is_refused() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid).with_tiers(&[(100, 2), (300, 1)]);

    let err = run(
        &elf,
        &pid,
        f.create_multisig_accounts(f.config_hash),
        &f.create_multisig_ix(),
    )
    .expect_err("a bigger spend must never need fewer approvals than a smaller one");
    assert_rejected(err, 5023, "tier");
}

/// A zero-approval tier would let anyone spend. Refused.
#[test]
fn a_tier_asking_for_zero_approvals_is_refused() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid).with_tiers(&[(300, 0)]);

    let err = run(
        &elf,
        &pid,
        f.create_multisig_accounts(f.config_hash),
        &f.create_multisig_ix(),
    )
    .expect_err("a tier requiring no approvals must not be anchorable");
    assert_rejected(err, 5023, "tier");
}

// ---------------------------------------------------------------------------
// Tiers — substitution at call time
// ---------------------------------------------------------------------------

/// Handing `execute` a generous tier table it was not anchored with does not
/// work, and the reason is worth stating: `tiers_hash` is folded into
/// `config_hash`, so a table the caller invents no longer matches the
/// `config_hash` they must also present.
#[test]
fn a_tier_table_invented_at_execute_time_does_not_match_the_config_hash() {
    let elf = elf();
    let pid = program_id(&elf);
    let honest = Fixture::new(&pid);

    // The honest accounts and the honest config_hash, with a table that would
    // let a single approval spend.
    let mut ix = honest.execute_ix(&[0]);
    if let VerifierInstruction::Execute { ref mut tiers, .. } = ix {
        *tiers = encode_tier_table(&[TierPolicy {
            max_amount: 300,
            threshold: 1,
        }]);
    }

    let err = run(&elf, &pid, honest.execute_accounts(&[0]), &ix)
        .expect_err("a tier table outside the anchored config must be refused");
    assert_rejected(err, 5002, "config");
}

/// So the caller computes a `config_hash` that *does* commit to the generous
/// table. Now the arithmetic is consistent — and the PDA moves, so the
/// instruction reads an account nobody ever created. This is the same defence
/// the member set and the threshold already had, extended to tiers for free.
#[test]
fn a_config_hash_that_commits_to_invented_tiers_resolves_to_an_account_nobody_created() {
    let elf = elf();
    let pid = program_id(&elf);
    let forged = Fixture::new(&pid).with_tiers(&[(300, 1)]);
    let honest = Fixture::new(&pid);

    assert_ne!(
        forged.multisig_id_addr(),
        honest.multisig_id_addr(),
        "if the tiers did not move the address, this test proves nothing"
    );

    // Everything the forger controls is internally consistent. What they cannot
    // do is make the account at the forged address exist.
    let mut accounts = forged.execute_accounts(&[0]);
    accounts[1] = forged.multisig_account(false);

    let err = run(&elf, &pid, accounts, &forged.execute_ix(&[0]))
        .expect_err("no multisig was ever created at the forged configuration");
    assert_rejected(err, 5003, "anchor");
}

/// The other half of the same guarantee, from the record's side: an account
/// whose stored `tiers_hash` disagrees with the table presented is refused even
/// though it is anchored and decodes.
#[test]
fn an_anchored_record_whose_tiers_hash_disagrees_is_refused() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid).with_tiers(&SMALL_SPEND_TIER);

    // The record at the tiered address, but carrying the empty-table hash.
    let mut record = f.multisig_record();
    record.tiers_hash = no_tiers_hash();
    let mut accounts = f.execute_accounts(&[0, 2]);
    accounts[1] = owned_with(pid, f.multisig_id_addr(), 0, encode_multisig(&record));

    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2]))
        .expect_err("the stored tiers_hash must match the table being applied");
    assert_rejected(err, 5024, "tier");
}

// ---------------------------------------------------------------------------
// Rotation — the honest case
// ---------------------------------------------------------------------------

#[test]
fn a_rotation_anchors_the_new_configuration_and_retires_the_old() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let r = f.rotation(4, &[]);

    let out = output(
        &elf,
        &pid,
        f.rotate_accounts(&r, &[0, 2, 4]),
        &f.rotate_ix(&r, &[0, 2, 4]),
    )
    .expect("three approvals satisfy the 3-of-5 in force, which is what a rotation costs");

    // The new configuration exists, at its own address. Ownership of an
    // `init` account is assigned by the sequencer after the program returns, so
    // it is not in this output and asserting it here would assert nothing; what
    // the program is responsible for is the account going from empty to
    // carrying the configuration, and that is what is checked.
    let (pre_new, post_new) = state_of(&out, &f.rotated_multisig_addr(&r));
    assert!(
        pre_new.data.is_empty(),
        "the new configuration must not have existed before"
    );
    let installed = decode_multisig(&post_new.data).expect("the new record decodes");
    assert_eq!(installed.threshold, 4);
    assert_eq!(installed.member_root, r.new_member_root);
    assert_eq!(
        installed.superseded_by, [0u8; 32],
        "the configuration just installed is the live one"
    );

    // The old one is still there, still readable, and now points at its
    // successor. A rotation does not delete history.
    let (_, post_old) = state_of(&out, &f.multisig_id_addr());
    let retired = decode_multisig(&post_old.data).expect("the old record still decodes");
    assert_eq!(
        retired.threshold, f.threshold,
        "the old record is not rewritten"
    );
    assert_eq!(
        retired.superseded_by, r.new_config_hash,
        "the old record must name the configuration that replaced it"
    );
}

/// A rotation moves no value. The new treasury is a second, empty account — the
/// funds stay where they are until someone proposes to move them under the new
/// rules.
#[test]
fn a_rotation_does_not_move_the_treasury() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let r = f.rotation(4, &[]);

    let out = output(
        &elf,
        &pid,
        f.rotate_accounts(&r, &[0, 2, 4]),
        &f.rotate_ix(&r, &[0, 2, 4]),
    )
    .expect("the honest rotation");

    let (_, post_new_treasury) = state_of(&out, &f.rotated_treasury_addr(&r));
    assert_eq!(
        post_new_treasury.balance, 0,
        "a rotation must not conjure a balance"
    );
    let t = decode_treasury(&post_new_treasury.data).expect("the new treasury record decodes");
    assert_eq!(
        t.config_hash, r.new_config_hash,
        "the new treasury must belong to the configuration that was just installed"
    );

    // The treasury being left keeps every unit it had, and the reason is
    // stronger than a balance comparison: `rotate_config` never names that
    // account, so it does not appear in the output at all. An instruction
    // cannot move value held by an account it does not touch.
    assert!(
        out.pre_states
            .iter()
            .all(|p| p.account_id != f.treasury_addr()),
        "the old treasury is not among the accounts a rotation touches"
    );
}

// ---------------------------------------------------------------------------
// Rotation — what it must refuse
// ---------------------------------------------------------------------------

/// The property that makes tiers and rotation safe *together*. A tier can make
/// a small transfer cheap; it must never make rewriting the member set cheap,
/// or the cheapest action available would be the one that hands the multisig to
/// someone else.
#[test]
fn a_tier_does_not_lower_what_a_rotation_costs() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid).with_tiers(&SMALL_SPEND_TIER);
    let r = f.rotation(4, &[]);

    // Two approvals: enough for a 250-unit transfer under this very tier table.
    let err = run(
        &elf,
        &pid,
        f.rotate_accounts(&r, &[0, 2]),
        &f.rotate_ix(&r, &[0, 2]),
    )
    .expect_err("a rotation costs the default threshold, whatever the tiers say");
    assert_rejected(err, 5010, "threshold");

    // The control: at the default threshold the same rotation goes through, so
    // the refusal above is the tier being ignored and nothing else.
    run(
        &elf,
        &pid,
        f.rotate_accounts(&r, &[0, 2, 4]),
        &f.rotate_ix(&r, &[0, 2, 4]),
    )
    .expect("three approvals are the default threshold");
}

#[test]
fn a_rotation_to_the_configuration_already_in_force_is_refused() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let r = f.rotation(f.threshold, &[]);
    assert_eq!(
        r.new_config_hash, f.config_hash,
        "same members, same threshold, same tiers is the same configuration"
    );

    let err = run(
        &elf,
        &pid,
        f.rotate_accounts(&r, &[0, 2, 4]),
        &f.rotate_ix(&r, &[0, 2, 4]),
    )
    .expect_err("rotating to the configuration in force changes nothing");
    assert_rejected(err, 5026, "rotation");
}

/// The new configuration must be a configuration the program would have
/// accepted at creation. Anything else would let a rotation install a state
/// `create_multisig` refuses.
#[test]
fn a_rotation_into_an_invalid_tier_table_is_refused() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let r = f.rotation(3, &[(300, 4)]);

    let err = run(
        &elf,
        &pid,
        f.rotate_accounts(&r, &[0, 2, 4]),
        &f.rotate_ix(&r, &[0, 2, 4]),
    )
    .expect_err("a rotation must not install a table creation would refuse");
    assert_rejected(err, 5023, "tier");
}

#[test]
fn a_rotation_to_a_zero_threshold_is_refused() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let r = f.rotation(0, &[]);

    let err = run(
        &elf,
        &pid,
        f.rotate_accounts(&r, &[0, 2, 4]),
        &f.rotate_ix(&r, &[0, 2, 4]),
    )
    .expect_err("a 0-of-N multisig is not a multisig");
    assert_rejected(err, 5008, "threshold");
}

/// The approvals on a rotation proposal are approvals of *that* rotation. A
/// proposal to rotate somewhere else does not carry them.
#[test]
fn approvals_for_one_rotation_do_not_carry_another() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let approved = f.rotation(4, &[]);
    let substituted = f.rotation(2, &[]);

    // The proposal the members actually approved, with the rotation the caller
    // would rather perform.
    let mut ix = f.rotate_ix(&substituted, &[0, 2, 4]);
    if let VerifierInstruction::RotateConfig {
        ref mut proposal_ref,
        ref mut execution_marker_seed,
        ..
    } = ix
    {
        *proposal_ref = approved.proposal_ref;
        *execution_marker_seed = compute_execution_marker(&approved.proposal_ref);
    }
    let mut accounts = f.rotate_accounts(&substituted, &[0, 2, 4]);
    accounts[0] = uninitialised(public_pda(
        &pid,
        &[compute_execution_marker(&approved.proposal_ref)],
    ));
    accounts[4] = f.rotate_proposal_account(&approved, true);
    for (i, &m) in [0usize, 2, 4].iter().enumerate() {
        accounts[6 + i] = f.rotate_marker_account(&approved, m, true);
    }

    let err =
        run(&elf, &pid, accounts, &ix).expect_err("these approvals are for a different rotation");
    assert_rejected(err, 5006, "different");
}

// ---------------------------------------------------------------------------
// The two action shapes, and the instruction each belongs to
// ---------------------------------------------------------------------------

/// A proposal is a transfer or a rotation, and the instruction that spends it
/// must be the matching one.
///
/// This test exists because of what it found. `execute` used to reach the
/// transfer path with a rotation proposal in hand: it never read `rotate_to`.
/// It got no further — a rotation's stored recipient is the zero account id,
/// and the next check requires the recipient account passed in to *be* that id
/// and to be owned by the transfer program, which no usable account is. So the
/// path was closed, but closed by a fact about which account ids exist rather
/// than by any rule this program states, and the refusal came back as a
/// recipient mismatch, naming the wrong cause. The guard says it instead.
#[test]
fn execute_refuses_a_rotation_proposal() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let r = f.rotation(4, &[]);

    // The rotation proposal, presented to `execute` with its own approvals.
    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[2] = f.rotate_proposal_account(&r, true);
    let mut ix = f.execute_ix(&[0, 2, 4]);
    if let VerifierInstruction::Execute {
        ref mut proposal_ref,
        ref mut execution_marker_seed,
        ..
    } = ix
    {
        *proposal_ref = r.proposal_ref;
        *execution_marker_seed = compute_execution_marker(&r.proposal_ref);
    }
    accounts[0] = uninitialised(public_pda(
        &pid,
        &[compute_execution_marker(&r.proposal_ref)],
    ));
    for (i, &m) in [0usize, 2, 4].iter().enumerate() {
        accounts[EXEC_FIRST_APPROVAL + i] = f.rotate_marker_account(&r, m, true);
    }

    let err = run(&elf, &pid, accounts, &ix)
        .expect_err("a rotation is not executed by the transfer instruction");
    assert_rejected(err, 5027, "rotation");
}

/// The mirror image, and the reason it is worth its own test: this direction
/// was already refused, but as an `action_hash` mismatch — true, and two steps
/// removed from the cause.
#[test]
fn rotate_config_refuses_a_transfer_proposal() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let r = f.rotation(4, &[]);

    // The transfer proposal the fixture already anchors, presented to
    // `rotate_config`.
    let mut accounts = f.rotate_accounts(&r, &[0, 2, 4]);
    accounts[4] = f.proposal_account(true);
    let mut ix = f.rotate_ix(&r, &[0, 2, 4]);
    if let VerifierInstruction::RotateConfig {
        ref mut proposal_ref,
        ref mut execution_marker_seed,
        ..
    } = ix
    {
        *proposal_ref = f.proposal_ref;
        *execution_marker_seed = compute_execution_marker(&f.proposal_ref);
    }
    accounts[0] = uninitialised(public_pda(
        &pid,
        &[compute_execution_marker(&f.proposal_ref)],
    ));
    for (i, &m) in [0usize, 2, 4].iter().enumerate() {
        accounts[6 + i] = f.marker_account(m, true);
    }

    let err = run(&elf, &pid, accounts, &ix)
        .expect_err("a transfer is not executed by the rotation instruction");
    assert_rejected(err, 5027, "transfer");
}

// ---------------------------------------------------------------------------
// After a rotation: the retired configuration
// ---------------------------------------------------------------------------

/// Once superseded, the old configuration cannot spend. Without this, a
/// rotation would add a member set rather than replace one, and every past
/// member set would keep its keys to the treasury forever.
#[test]
fn a_superseded_configuration_cannot_execute() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let r = f.rotation(4, &[]);

    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[1] = f.multisig_account_superseded_by(&r);

    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2, 4]))
        .expect_err("a retired configuration must not move the treasury");
    assert_rejected(err, 5025, "superseded");
}

#[test]
fn a_superseded_configuration_cannot_approve() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let r = f.rotation(4, &[]);

    let mut accounts = f.approve_accounts(0, true, true);
    accounts[1] = f.multisig_account_superseded_by(&r);

    let err = run(&elf, &pid, accounts, &f.approve_ix(0))
        .expect_err("a retired configuration must not gather new approvals");
    assert_rejected(err, 5025, "superseded");
}

#[test]
fn a_superseded_configuration_cannot_be_rotated_again() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let first = f.rotation(4, &[]);
    let second = f.rotation(2, &[]);

    let mut accounts = f.rotate_accounts(&second, &[0, 2, 4]);
    accounts[1] = f.multisig_account_superseded_by(&first);

    let err = run(&elf, &pid, accounts, &f.rotate_ix(&second, &[0, 2, 4]))
        .expect_err("a retired configuration must not fork the chain of rotations");
    assert_rejected(err, 5025, "superseded");
}

/// No `StaleProposal` check exists in this program, and none is needed. A
/// proposal's address is derived through `config_hash`, so proposals raised
/// under the old configuration live at addresses the new one never reads. This
/// test is what makes that claim checkable rather than an assertion in a
/// comment.
#[test]
fn proposals_do_not_survive_a_rotation_because_their_addresses_do_not() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let r = f.rotation(4, &[]);

    // The same proposal_id and the same action, read under the new
    // configuration: a different proposal_ref, and therefore a different PDA.
    let after = compute_proposal_ref(
        &f.multisig_id,
        &r.new_config_hash,
        &f.proposal_id,
        &f.action_hash,
    );
    assert_ne!(
        after, f.proposal_ref,
        "proposal_ref must be scoped to the configuration that raised it"
    );
    assert_ne!(
        public_pda(&pid, &[f.multisig_id, r.new_config_hash, after]),
        f.proposal_addr(),
        "the new configuration must not inherit the old one's proposal accounts"
    );
}
