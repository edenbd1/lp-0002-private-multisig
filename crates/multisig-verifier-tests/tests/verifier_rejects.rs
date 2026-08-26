//! Adversarial tests against the deployed LP-0002 multisig verifier.
//!
//! These tests run the committed `multisig_verifier.bin` through the
//! *sequencer's own* execution path — same input order, same 32M session limit,
//! same executor (`lee/state_machine/src/program/mod.rs:55-110`) — so a rejection
//! here is the same rejection the chain performs. They deliberately do not
//! prove: proving costs minutes and would establish nothing extra about which
//! inputs are accepted.
//!
//! Each test constructs a specific way to steal an execution, or to approve
//! without being a member, and requires the deployed binary to reject it with
//! the matching error code. Honest calls are the controls.
//!
//! Run with: `cargo test -p multisig-verifier-tests --test verifier_rejects`

use multisig_core::*;
use multisig_verifier_tests::*;

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

/// If an honest approval is rejected, every rejection below is meaningless.
#[test]
fn an_honest_approval_is_accepted() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    run(
        &elf,
        &pid,
        f.approve_accounts(0, true, true),
        &f.approve_ix(0),
    )
    .expect("a member of an anchored set must be able to approve");
}

/// The other control: three distinct members reach the 3-of-5 threshold.
#[test]
fn an_honest_execution_at_threshold_is_accepted() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    run(
        &elf,
        &pid,
        f.execute_accounts(&[0, 2, 4]),
        &f.execute_ix(&[0, 2, 4]),
    )
    .expect("three distinct approvals must satisfy a 3-of-5 threshold");
}

// ---------------------------------------------------------------------------
// approve — the member set is anchored
// ---------------------------------------------------------------------------

/// The invented-set attack: an outsider builds a one-leaf tree holding
/// themselves and proves membership in it. No multisig was committed for that
/// root, so the multisig PDA is uninitialised and the approval is rejected.
#[test]
fn approving_against_an_unanchored_member_set_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let err = run(
        &elf,
        &pid,
        f.approve_accounts(0, false, true),
        &f.approve_ix(0),
    )
    .expect_err("an unanchored member set must be rejected");
    assert_rejected(err, 5003, "anchor");
}

/// A config hash that does not commit to the (root, threshold) pair supplied.
///
/// The accounts are made *consistent* with the forged hash — the multisig
/// account sits at the PDA the forged hash derives, and is owned by the verifier
/// — so that SPEL's address validation passes and the program's own
/// `E_CONFIG_MISMATCH` check is what trips. Without that setup the framework
/// rejects first; see `the_framework_rejects_a_multisig_at_the_wrong_address`.
#[test]
fn approving_with_an_inconsistent_config_hash_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let forged = [0xAB; 32];
    let mut ix = f.approve_ix(0);
    if let VerifierInstruction::Approve { config_hash, .. } = &mut ix {
        *config_hash = forged;
    }
    let accounts = vec![
        uninitialised(public_pda(&pid, &[f.marker_seed(0)])),
        owned_by(pid, public_pda(&pid, &[f.multisig_id, forged])),
        f.proposal_account(true),
        signer([0xC1; 32]),
    ];
    let err =
        run(&elf, &pid, accounts, &ix).expect_err("an inconsistent config hash must be rejected");
    assert_rejected(err, 5002, "config");
}

/// Defence in depth: SPEL's own account validation rejects a multisig account
/// that is not at the PDA the instruction arguments derive, before the program
/// body runs at all. Documented as a test because it is a real second layer —
/// an attacker must get past both.
#[test]
fn the_framework_rejects_a_multisig_at_the_wrong_address() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let mut ix = f.approve_ix(0);
    if let VerifierInstruction::Approve { config_hash, .. } = &mut ix {
        *config_hash = [0xAB; 32];
    }
    // Accounts left at the honest addresses, inconsistent with the forged arg.
    let err = run(&elf, &pid, f.approve_accounts(0, true, true), &ix)
        .expect_err("a multisig account at the wrong address must be rejected");
    let msg = format!("{err:#}").to_lowercase();
    assert!(
        msg.contains("pdamismatch") && msg.contains("multisig"),
        "expected the framework PDA mismatch, got: {msg}"
    );
}

/// Approving a proposal that was never created.
#[test]
fn approving_an_unanchored_proposal_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let err = run(
        &elf,
        &pid,
        f.approve_accounts(0, true, false),
        &f.approve_ix(0),
    )
    .expect_err("an unanchored proposal must be rejected");
    assert_rejected(err, 5004, "proposal");
}

/// A forged nullifier: the approver substitutes a nullifier of their choosing
/// while proving a real membership, to occupy someone else's marker.
#[test]
fn a_forged_nullifier_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let forged_nullifier = [0x00; 32];
    let forged_seed = compute_approval_marker(&f.proposal_ref, &forged_nullifier);
    let mut ix = f.approve_ix(0);
    if let VerifierInstruction::Approve {
        nullifier,
        approval_marker_seed,
        ..
    } = &mut ix
    {
        *nullifier = forged_nullifier;
        // Keep the marker seed consistent with the forged nullifier so it is the
        // nullifier check that trips, not the marker check.
        *approval_marker_seed = forged_seed;
    }
    // And place the marker account at the address the forged seed derives, so
    // SPEL's address validation passes and the program's own check is what
    // rejects. The witness still yields the honest nullifier, not this one.
    let accounts = vec![
        uninitialised(public_pda(&pid, &[forged_seed])),
        f.multisig_account(true),
        f.proposal_account(true),
        signer([0xC1; 32]),
    ];
    let err = run(&elf, &pid, accounts, &ix).expect_err("a forged nullifier must be rejected");
    assert_rejected(err, 5005, "nullifier");
}

/// A marker seed that does not commit to the proposal and nullifier: an attempt
/// to land the approval at an address that misrepresents what was approved.
#[test]
fn a_forged_marker_seed_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let forged_seed = [0xAB; 32];
    let mut ix = f.approve_ix(0);
    if let VerifierInstruction::Approve {
        approval_marker_seed,
        ..
    } = &mut ix
    {
        *approval_marker_seed = forged_seed;
    }
    let accounts = vec![
        uninitialised(public_pda(&pid, &[forged_seed])),
        f.multisig_account(true),
        f.proposal_account(true),
        signer([0xC1; 32]),
    ];
    let err = run(&elf, &pid, accounts, &ix).expect_err("a forged marker seed must be rejected");
    assert_rejected(err, 5006, "marker");
}

// ---------------------------------------------------------------------------
// execute — the threshold is real
// ---------------------------------------------------------------------------

/// Two approvals against a 3-of-5 threshold.
#[test]
fn executing_below_the_threshold_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let err = run(
        &elf,
        &pid,
        f.execute_accounts(&[0, 1]),
        &f.execute_ix(&[0, 1]),
    )
    .expect_err("two approvals must not satisfy a 3-of-5 threshold");
    assert_rejected(err, 5010, "threshold");
}

/// The replay attack: one member's approval presented three times. Without the
/// pairwise-distinctness check this would pass a 3-of-5 gate with one signature.
#[test]
fn presenting_the_same_approval_three_times_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let err = run(
        &elf,
        &pid,
        f.execute_accounts(&[0, 0, 0]),
        &f.execute_ix(&[0, 0, 0]),
    )
    .expect_err("one member cannot satisfy a 3-of-5 threshold alone");
    assert_rejected(err, 5011, "more than once");
}

/// A marker account at the right address but never claimed by the verifier — so
/// no membership proof was ever verified for it. This is the attack where a
/// third party pre-creates an account at the marker address.
#[test]
fn an_unclaimed_approval_marker_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[EXEC_FIRST_APPROVAL] = f.marker_account(0, false); // right address, default owner
    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2, 4]))
        .expect_err("a marker never claimed by this program must be rejected");
    assert_rejected(err, 5013, "never claimed");
}

/// An account that is owned by the verifier but is not the marker for the
/// nullifier it was paired with — for instance a marker from another proposal.
#[test]
fn an_approval_marker_from_another_proposal_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    // A second proposal under the same multisig, and member 0's marker on it.
    let other_action = compute_action_hash(&f.multisig_id, b"a different action entirely");
    let other_ref = compute_proposal_ref(
        &f.multisig_id,
        &f.config_hash,
        &f.proposal_id,
        &other_action,
    );
    let other_nullifier = compute_approval_nullifier(&other_ref, &f.members[0].0);
    let other_marker = compute_approval_marker(&other_ref, &other_nullifier);

    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[EXEC_FIRST_APPROVAL] = owned_by(pid, public_pda(&pid, &[other_marker]));
    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2, 4]))
        .expect_err("a marker earned on another proposal must not count here");
    assert_rejected(err, 5012, "not the marker");
}

/// The count-mismatch guard: more accounts than nullifiers.
#[test]
fn mismatched_approval_and_nullifier_counts_are_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let accounts = f.execute_accounts(&[0, 2, 4]);
    // Four marker accounts, three nullifiers.
    let mut accounts4 = accounts.clone();
    accounts4.push(f.marker_account(1, true));
    let err = run(&elf, &pid, accounts4, &f.execute_ix(&[0, 2, 4]))
        .expect_err("each approval account must be paired with its nullifier");
    assert_rejected(err, 5009, "paired");
}

/// The lowered-threshold attack: an executor supplies `threshold = 1` so that a
/// single approval suffices. A different threshold is a different config hash,
/// hence a different multisig PDA — which was never created.
#[test]
fn lowering_the_threshold_at_execution_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let forged_threshold = 1u32;
    let forged_config = compute_config_hash(&f.member_root, forged_threshold, &no_tiers_hash());
    let ix = VerifierInstruction::Execute {
        multisig_id: f.multisig_id,
        config_hash: forged_config,
        member_root: f.member_root,
        threshold: forged_threshold,
        proposal_ref: f.proposal_ref,
        approval_nullifiers: vec![f.nullifier(0)],
        execution_marker_seed: compute_execution_marker(&f.proposal_ref),
        tiers: encode_tier_table(&[]),
    };
    // The forged config resolves to a PDA nobody ever initialised.
    let accounts = vec![
        uninitialised(public_pda(
            &pid,
            &[compute_execution_marker(&f.proposal_ref)],
        )),
        uninitialised(public_pda(&pid, &[f.multisig_id, forged_config])),
        f.proposal_account(true),
        uninitialised(public_pda(
            &pid,
            &[f.multisig_id, forged_config, literal_seed("treasury")],
        )),
        f.recipient_account(),
        signer([0xE1; 32]),
        f.marker_account(0, true),
    ];
    let err = run(&elf, &pid, accounts, &ix)
        .expect_err("a lowered threshold must land on an uninitialised multisig PDA");
    assert_rejected(err, 5003, "anchor");
}

/// An execution marker seed that does not commit to this proposal.
#[test]
fn a_forged_execution_marker_seed_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let forged_seed = [0xAB; 32];
    let mut ix = f.execute_ix(&[0, 2, 4]);
    if let VerifierInstruction::Execute {
        execution_marker_seed,
        ..
    } = &mut ix
    {
        *execution_marker_seed = forged_seed;
    }
    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[0] = uninitialised(public_pda(&pid, &[forged_seed]));
    let err = run(&elf, &pid, accounts, &ix)
        .expect_err("a forged execution marker seed must be rejected");
    assert_rejected(err, 5006, "marker");
}

// ---------------------------------------------------------------------------
// Compute cost
// ---------------------------------------------------------------------------

/// Report the deployed verifier's on-chain compute cost per instruction, by
/// replaying execution through the sequencer's own executor. This is the
/// Performance criterion's number: the guest's own cycles, excluding the privacy
/// circuit's recursive verification of the chained call, which is LEZ's cost not
/// ours.
///
/// Run with:
///   cargo test -p multisig-verifier-tests --test verifier_rejects -- --ignored --nocapture
#[test]
#[ignore = "reports a measurement rather than asserting a property"]
fn report_the_cycle_costs() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    let report = |label: &str, s: risc0_zkvm::SessionInfo| {
        let user: u64 = s.segments.iter().map(|x| u64::from(x.cycles)).sum();
        let proving: u64 = s.segments.iter().map(|x| 1u64 << x.po2).sum();
        let pct = proving as f64 / MAX_NUM_CYCLES_PUBLIC_EXECUTION as f64 * 100.0;
        println!(
            "{label:<28} segments {:<3} user {user:<10} proving {proving:<10} budget {pct:.2} %",
            s.segments.len()
        );
    };

    println!("multisig verifier, guest execution only");
    // The three cheap instructions were once argued about rather than measured
    // — "they do strictly less work than execute (M=1) and are bounded by it".
    // The criterion says each on-chain operation, and an argument is not a
    // number, so they are measured here with the same harness as the rest.
    report(
        "create_multisig",
        session(
            &elf,
            &pid,
            f.create_multisig_accounts(f.config_hash),
            &f.create_multisig_ix(),
        )
        .expect("create_multisig executes"),
    );
    report(
        "create_proposal",
        session(
            &elf,
            &pid,
            f.create_proposal_accounts(f.proposal_ref, true),
            &f.create_proposal_ix(),
        )
        .expect("create_proposal executes"),
    );
    report(
        "fund_treasury",
        session(
            &elf,
            &pid,
            f.fund_accounts(funding_signer([0xF0; 32], 1_000)),
            &f.fund_ix(400),
        )
        .expect("fund_treasury executes"),
    );
    report(
        "approve",
        session(
            &elf,
            &pid,
            f.approve_accounts(0, true, true),
            &f.approve_ix(0),
        )
        .expect("approve executes"),
    );
    for m in [1usize, 3, 5, 7] {
        // A whole fixture per M, not a mutated one: the threshold is inside
        // `config_hash`, which is inside every PDA address and inside
        // `proposal_ref`, so changing it in place leaves a multisig whose
        // accounts no longer resolve. The table would then measure rejections.
        let fx = Fixture::with_members(&pid, m.max(5), m as u32);
        let members: Vec<usize> = (0..m).collect();
        report(
            &format!("execute (M={m})"),
            session(
                &elf,
                &pid,
                fx.execute_accounts(&members),
                &fx.execute_ix(&members),
            )
            .expect("execute executes"),
        );
    }

    // The two instructions the tier and rotation work added. `execute` under a
    // tier is measured separately from `execute` without one, because decoding
    // the table and resolving the amount against it is work the untiered path
    // does not do — and a reader comparing the two learns what a tier costs.
    let tiered = Fixture::new(&pid).with_tiers(&[(300, 2)]);
    report(
        "execute (tiered, M=2)",
        session(
            &elf,
            &pid,
            tiered.execute_accounts(&[0, 2]),
            &tiered.execute_ix(&[0, 2]),
        )
        .expect("a tiered execute executes"),
    );

    let rot = f.rotation(4, &[]);
    report(
        "rotate_config (M=3)",
        session(
            &elf,
            &pid,
            f.rotate_accounts(&rot, &[0, 2, 4]),
            &f.rotate_ix(&rot, &[0, 2, 4]),
        )
        .expect("rotate_config executes"),
    );
}

// ---------------------------------------------------------------------------
// create_multisig and create_proposal
//
// These two instructions had no coverage against the binary until an audit
// pass noticed that three documented error codes — 5001, 5007, 5008 — were
// never exercised, while docs/error-codes.md claimed every code was. The
// claim is only worth making if the tests exist, so here they are.
// ---------------------------------------------------------------------------

/// The control: an honest multisig creation is accepted.
#[test]
fn an_honest_create_multisig_is_accepted() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let ix = VerifierInstruction::CreateMultisig {
        multisig_id: f.multisig_id,
        config_hash: f.config_hash,
        member_root: f.member_root,
        threshold: f.threshold,
        tiers: encode_tier_table(&[]),
    };
    run(&elf, &pid, f.create_multisig_accounts(f.config_hash), &ix)
        .expect("a well-formed multisig must be creatable");
}

/// A 0-of-N multisig would let anyone execute, so creation must refuse it
/// outright rather than leave such an instance reachable on chain.
#[test]
fn creating_a_zero_threshold_multisig_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let config = compute_config_hash(&f.member_root, 0, &no_tiers_hash());
    let ix = VerifierInstruction::CreateMultisig {
        multisig_id: f.multisig_id,
        config_hash: config,
        member_root: f.member_root,
        threshold: 0,
        tiers: encode_tier_table(&[]),
    };
    let err = run(&elf, &pid, f.create_multisig_accounts(config), &ix)
        .expect_err("a zero threshold must be rejected");
    assert_rejected(err, 5008, "at least 1");
}

/// A config hash that does not commit to the (root, threshold) pair it is
/// presented with. Without this check `config_hash` would be opaque bytes and
/// the anchoring of the threshold would mean nothing.
#[test]
fn creating_a_multisig_with_an_inconsistent_config_hash_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let forged = [0xCD; 32];
    let ix = VerifierInstruction::CreateMultisig {
        multisig_id: f.multisig_id,
        config_hash: forged,
        member_root: f.member_root,
        threshold: f.threshold,
        tiers: encode_tier_table(&[]),
    };
    let err = run(&elf, &pid, f.create_multisig_accounts(forged), &ix)
        .expect_err("an inconsistent config hash must be rejected");
    assert_rejected(err, 5002, "config");
}

/// The control: an honest proposal against an anchored multisig is accepted.
#[test]
fn an_honest_create_proposal_is_accepted() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    run(
        &elf,
        &pid,
        f.create_proposal_accounts(f.proposal_ref, true),
        &f.create_proposal_ix(),
    )
    .expect("a proposal on an anchored multisig must be creatable");
}

/// A proposal reference that does not commit to `(multisig, proposal, action)`.
///
/// This is the check that makes binding 3 real: if `proposal_ref` could be
/// arbitrary bytes, approvals would no longer be tied to the action, and the
/// bait-and-switch the whole design exists to prevent would be back.
#[test]
fn creating_a_proposal_with_an_unbound_ref_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let forged_ref = [0xEF; 32];
    let mut ix = f.create_proposal_ix();
    if let VerifierInstruction::CreateProposal { proposal_ref, .. } = &mut ix {
        *proposal_ref = forged_ref;
    }
    let err = run(
        &elf,
        &pid,
        f.create_proposal_accounts(forged_ref, true),
        &ix,
    )
    .expect_err("a proposal_ref that binds nothing must be rejected");
    assert_rejected(err, 5007, "does not commit");
}

/// A proposal cannot be attached to a multisig that was never created.
#[test]
fn creating_a_proposal_on_an_unanchored_multisig_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let err = run(
        &elf,
        &pid,
        f.create_proposal_accounts(f.proposal_ref, false),
        &f.create_proposal_ix(),
    )
    .expect_err("a proposal on a multisig nobody created must be rejected");
    assert_rejected(err, 5003, "anchor");
}

/// Witness bytes that do not decode as an `ApproveWitness`.
///
/// The decode happens before any other check in `approve`, so a caller who
/// sends garbage must get a specific, documented rejection rather than an
/// opaque guest panic.
#[test]
fn an_undecodable_witness_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let mut ix = f.approve_ix(0);
    if let VerifierInstruction::Approve { witness_words, .. } = &mut ix {
        // Structurally valid u32s, but not a risc0-serde ApproveWitness.
        *witness_words = vec![0xDEAD_BEEF, 1, 2, 3];
    }
    let err = run(&elf, &pid, f.approve_accounts(0, true, true), &ix)
        .expect_err("an undecodable witness must be rejected");
    assert_rejected(err, 5001, "did not decode");
}

// ---------------------------------------------------------------------------
// Further attacks against the same bindings
// ---------------------------------------------------------------------------

/// Replay: executing a proposal a second time.
///
/// The guard is `init` on the execution marker — LEZ refuses to claim an
/// account that is no longer default-owned. Modelled here by presenting the
/// marker already owned by this program, which is the state the first
/// execution leaves it in.
#[test]
fn executing_the_same_proposal_twice_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    // The state after a first successful execution.
    accounts[0] = owned_by(
        pid,
        public_pda(&pid, &[compute_execution_marker(&f.proposal_ref)]),
    );
    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2, 4]))
        .expect_err("a proposal must not execute twice");
    let msg = format!("{err:#}").to_lowercase();
    assert!(
        msg.contains("claim") || msg.contains("init") || msg.contains("default"),
        "expected the init/claim guard to reject a replay, got: {msg}"
    );
}

/// An approval marker earned on a *different multisig* must not count here.
///
/// `proposal_ref` folds in `multisig_id`, so the marker for the same member on
/// the same proposal id under another multisig lands at a different address and
/// fails the marker-derivation check.
#[test]
fn an_approval_marker_from_another_multisig_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    // Same member, same proposal id, same action bytes — different multisig.
    let other_msig = [0xB0; 32];
    let other_action = compute_action_hash(&other_msig, b"transfer 100 to the treasury");
    let other_ref =
        compute_proposal_ref(&other_msig, &f.config_hash, &f.proposal_id, &other_action);
    let other_nullifier = compute_approval_nullifier(&other_ref, &f.members[0].0);
    let other_marker = compute_approval_marker(&other_ref, &other_nullifier);

    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[EXEC_FIRST_APPROVAL] = owned_by(pid, public_pda(&pid, &[other_marker]));
    let err = run(&elf, &pid, accounts, &f.execute_ix(&[0, 2, 4]))
        .expect_err("a marker from another multisig must not count");
    assert_rejected(err, 5012, "not the marker");
}

/// Presenting *more* than the threshold is allowed — the check is `>=`, not
/// `==`. Worth pinning as a test so a later tightening to `==` is a deliberate
/// choice rather than an accident.
#[test]
fn presenting_more_approvals_than_the_threshold_is_accepted() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    // Threshold is 3; present 4 distinct members.
    run(
        &elf,
        &pid,
        f.execute_accounts(&[0, 1, 2, 3]),
        &f.execute_ix(&[0, 1, 2, 3]),
    )
    .expect("more approvals than the threshold must still satisfy it");
}

/// A member of the set who was never in *this* multisig's committed root
/// cannot approve, even with a structurally valid witness — because the root
/// the statement names is checked against the anchored one by ownership.
#[test]
fn approving_with_a_witness_for_a_different_member_set_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    // Build a witness that is internally valid against an invented one-member
    // root, then present it against this multisig's anchored config.
    let outsider_msk = [0xCC; 32];
    let outsider_id = 99u128;
    let outsider_salt = [0xDD; 32];
    let aid = derive_account_id(&derive_npk(&outsider_msk), outsider_id);
    let leaf = compute_member_leaf(&aid, &outsider_salt);
    let (fake_root, fake_paths) = build_member_tree(&[leaf]);
    let nullifier = compute_approval_nullifier(&f.proposal_ref, &outsider_msk);

    let instruction = ApproveInstruction {
        witness: ApproveWitness {
            msk: outsider_msk,
            identifier: outsider_id,
            salt: outsider_salt,
            merkle_path: fake_paths[0].1.clone(),
            leaf_index: fake_paths[0].0,
        },
        statement: ApproveStatement {
            member_root: fake_root,
            proposal_ref: f.proposal_ref,
            nullifier,
        },
    };
    let marker_seed = compute_approval_marker(&f.proposal_ref, &nullifier);
    let forged_config = compute_config_hash(&fake_root, f.threshold, &no_tiers_hash());
    let ix = VerifierInstruction::Approve {
        witness_words: risc0_zkvm::serde::to_vec(&instruction).expect("encode"),
        multisig_id: f.multisig_id,
        config_hash: forged_config,
        member_root: fake_root,
        threshold: f.threshold,
        proposal_ref: f.proposal_ref,
        nullifier,
        approval_marker_seed: marker_seed,
        tiers: encode_tier_table(&[]),
    };
    // The invented root resolves to a multisig PDA nobody ever created.
    let accounts = vec![
        uninitialised(public_pda(&pid, &[marker_seed])),
        uninitialised(public_pda(&pid, &[f.multisig_id, forged_config])),
        f.proposal_account(true),
        signer([0xC1; 32]),
    ];
    let err = run(&elf, &pid, accounts, &ix)
        .expect_err("membership in an invented set must not be usable");
    assert_rejected(err, 5003, "anchor");
}

/// Can a proposal be executed while naming a *different* multisig,
/// one the attacker created with a lower threshold over the same member root?
///
/// `execute` takes `proposal_ref` as an argument and constrains the proposal
/// account to that PDA. It also takes `multisig_id`/`config_hash` and constrains
/// the multisig account to *those*. If nothing ties the two together, an
/// attacker can pair a real 5-of-N proposal with a 1-of-N multisig they created
/// themselves over the same member root, and execute it with one approval.
#[test]
fn executing_a_proposal_under_a_foreign_multisig_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid); // 3-of-5, proposal_ref bound to f.multisig_id

    // The attacker's own multisig: same member root, threshold lowered to 1.
    // Creating this is legitimate — anyone may create a multisig.
    let attacker_msig = [0xEE; 32];
    let attacker_threshold = 1u32;
    let attacker_config = compute_config_hash(&f.member_root, attacker_threshold, &no_tiers_hash());

    let ix = VerifierInstruction::Execute {
        multisig_id: attacker_msig,
        config_hash: attacker_config,
        member_root: f.member_root,
        threshold: attacker_threshold,
        // ...but the proposal is the real one, from the honest 3-of-5 multisig.
        proposal_ref: f.proposal_ref,
        approval_nullifiers: vec![f.nullifier(0)],
        execution_marker_seed: compute_execution_marker(&f.proposal_ref),
        tiers: encode_tier_table(&[]),
    };
    let accounts = vec![
        uninitialised(public_pda(
            &pid,
            &[compute_execution_marker(&f.proposal_ref)],
        )),
        // The attacker's multisig PDA, which they really did create.
        owned_by(pid, public_pda(&pid, &[attacker_msig, attacker_config])),
        f.proposal_account(true),
        uninitialised(public_pda(
            &pid,
            &[attacker_msig, attacker_config, literal_seed("treasury")],
        )),
        f.recipient_account(),
        signer([0xE1; 32]),
        f.marker_account(0, true),
    ];

    let err = run(&elf, &pid, accounts, &ix).expect_err(
        "a proposal must not be executable under a multisig it does not belong to: \
         that would let anyone lower the threshold of any proposal",
    );

    // Caught by SPEL's address validation, before the program body runs: the
    // proposal account is at [multisig_id_A, proposal_ref] and the arguments
    // name multisig B, so the derived address does not match the one presented.
    //
    // This is precisely what putting `multisig_id` in the proposal's seeds buys.
    // Before that change the same call SUCCEEDED — the probe that found it is
    // the reason this test exists.
    let msg = format!("{err:#}").to_lowercase();
    assert!(
        msg.contains("pdamismatch") && msg.contains("proposal"),
        "expected the proposal address to be rejected, got: {msg}"
    );
}

/// The approve-side half of the same attack: creating an approval marker for
/// someone else's proposal while naming a multisig you control.
///
/// Step 2 of a two-step attack: an outsider mints a valid-looking marker on a
/// proposal they are not a member of, which step 3 then counts. Binding the
/// proposal address to its multisig closes both halves at once.
#[test]
fn approving_a_proposal_under_a_foreign_multisig_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);

    // The attacker's own 1-of-1 multisig over a set containing only themselves.
    let attacker_msk = [0xF1; 32];
    let attacker_id = 7u128;
    let attacker_salt = [0xF2; 32];
    let aid = derive_account_id(&derive_npk(&attacker_msk), attacker_id);
    let (root, paths) = build_member_tree(&[compute_member_leaf(&aid, &attacker_salt)]);
    let attacker_msig = [0xF3; 32];
    let attacker_config = compute_config_hash(&root, 1, &no_tiers_hash());

    // ...used to approve the honest multisig's proposal.
    let nullifier = compute_approval_nullifier(&f.proposal_ref, &attacker_msk);
    let marker_seed = compute_approval_marker(&f.proposal_ref, &nullifier);
    let instruction = ApproveInstruction {
        witness: ApproveWitness {
            msk: attacker_msk,
            identifier: attacker_id,
            salt: attacker_salt,
            merkle_path: paths[0].1.clone(),
            leaf_index: paths[0].0,
        },
        statement: ApproveStatement {
            member_root: root,
            proposal_ref: f.proposal_ref,
            nullifier,
        },
    };
    let ix = VerifierInstruction::Approve {
        witness_words: risc0_zkvm::serde::to_vec(&instruction).expect("encode"),
        multisig_id: attacker_msig,
        config_hash: attacker_config,
        member_root: root,
        threshold: 1,
        proposal_ref: f.proposal_ref,
        nullifier,
        approval_marker_seed: marker_seed,
        tiers: encode_tier_table(&[]),
    };
    let accounts = vec![
        uninitialised(public_pda(&pid, &[marker_seed])),
        owned_by(pid, public_pda(&pid, &[attacker_msig, attacker_config])),
        f.proposal_account(true),
        signer([0xC9; 32]),
    ];

    let err = run(&elf, &pid, accounts, &ix)
        .expect_err("an outsider must not mint a marker on someone else's proposal");
    let msg = format!("{err:#}").to_lowercase();
    assert!(
        msg.contains("pdamismatch") && msg.contains("proposal"),
        "expected the proposal address to be rejected, got: {msg}"
    );
}

// ── Same multisig id, a second configuration ─────────────────────────────────
//
// Seeding the proposal PDA with `[multisig_id, proposal_ref]` closed the case
// where the attacker names a *different* multisig id. It does not close the case
// where they name the *same* one under a config of their own, because
// `config_hash` appears in neither `proposal_ref` nor the proposal's address.
//
// `create_multisig` places no constraint on `multisig_id` — by design, anyone
// may create a multisig — so an attacker can create a second config under a
// victim's id, and both PDAs still resolve.

/// The attacker's own one-member set, built exactly as `Fixture::new` builds
/// the honest one. Returns the root and the single member, reusing `TestMember`
/// rather than widening the tuple.
fn attacker_set() -> ([u8; 32], TestMember) {
    let msk = [0x99; 32];
    let identifier = 999u128;
    let salt = [0x77; 32];
    let aid = derive_account_id(&derive_npk(&msk), identifier);
    let leaf = compute_member_leaf(&aid, &salt);
    let (root, paths) = build_member_tree(&[leaf]);
    let (leaf_index, siblings) = paths[0].clone();
    (root, (msk, identifier, salt, leaf_index, siblings))
}

#[test]
fn approving_under_a_second_config_of_the_same_multisig_id_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid); // honest 3-of-5
    let (a_root, (a_msk, a_id, a_salt, a_leaf_index, a_siblings)) = attacker_set();

    let a_threshold = 1u32;
    let a_config = compute_config_hash(&a_root, a_threshold, &no_tiers_hash());
    // The victim's proposal, untouched.
    let nullifier = compute_approval_nullifier(&f.proposal_ref, &a_msk);
    let marker_seed = compute_approval_marker(&f.proposal_ref, &nullifier);

    let instruction = ApproveInstruction {
        witness: ApproveWitness {
            msk: a_msk,
            identifier: a_id,
            salt: a_salt,
            merkle_path: a_siblings,
            leaf_index: a_leaf_index,
        },
        statement: ApproveStatement {
            member_root: a_root,
            proposal_ref: f.proposal_ref,
            nullifier,
        },
    };
    let ix = VerifierInstruction::Approve {
        witness_words: risc0_zkvm::serde::to_vec(&instruction).expect("encode witness"),
        multisig_id: f.multisig_id, // the VICTIM's id
        config_hash: a_config,      // the ATTACKER's config
        member_root: a_root,
        threshold: a_threshold,
        proposal_ref: f.proposal_ref, // the VICTIM's proposal
        nullifier,
        approval_marker_seed: marker_seed,
        tiers: encode_tier_table(&[]),
    };
    let accounts = vec![
        uninitialised(public_pda(&pid, &[marker_seed])),
        // The attacker's multisig, which they really did create under the
        // victim's id: `create_multisig` allows it.
        owned_by(pid, public_pda(&pid, &[f.multisig_id, a_config])),
        f.proposal_account(true),
        signer([0xE2; 32]),
    ];

    let err = run(&elf, &pid, accounts, &ix).expect_err(
        "an outsider must not mint an approval marker on someone else's proposal \
         by naming a member set of their own under the same multisig id",
    );
    let msg = format!("{err:#}").to_lowercase();
    assert!(
        msg.contains("pdamismatch") || msg.contains("500"),
        "expected a rejection tying the proposal to its config, got: {msg}"
    );
}

#[test]
fn executing_under_a_second_config_of_the_same_multisig_id_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid); // honest 3-of-5
    let (a_root, (a_msk, ..)) = attacker_set();

    let a_threshold = 1u32;
    let a_config = compute_config_hash(&a_root, a_threshold, &no_tiers_hash());
    let nullifier = compute_approval_nullifier(&f.proposal_ref, &a_msk);
    let marker_seed = compute_approval_marker(&f.proposal_ref, &nullifier);

    let ix = VerifierInstruction::Execute {
        multisig_id: f.multisig_id, // the VICTIM's id
        config_hash: a_config,      // threshold 1, the attacker's
        member_root: a_root,
        threshold: a_threshold,
        proposal_ref: f.proposal_ref, // the VICTIM's 3-of-5 proposal
        approval_nullifiers: vec![nullifier],
        execution_marker_seed: compute_execution_marker(&f.proposal_ref),
        tiers: encode_tier_table(&[]),
    };
    let accounts = vec![
        uninitialised(public_pda(
            &pid,
            &[compute_execution_marker(&f.proposal_ref)],
        )),
        owned_by(pid, public_pda(&pid, &[f.multisig_id, a_config])),
        f.proposal_account(true),
        uninitialised(public_pda(
            &pid,
            &[f.multisig_id, a_config, literal_seed("treasury")],
        )),
        f.recipient_account(),
        signer([0xE3; 32]),
        owned_by(pid, public_pda(&pid, &[marker_seed])),
    ];

    let err = run(&elf, &pid, accounts, &ix).expect_err(
        "a 3-of-5 proposal must not execute on one outsider approval just because \
         the attacker created a 1-of-1 config under the same multisig id",
    );
    let msg = format!("{err:#}").to_lowercase();
    assert!(
        msg.contains("pdamismatch") || msg.contains("500"),
        "expected a rejection tying the proposal to its config, got: {msg}"
    );
}
