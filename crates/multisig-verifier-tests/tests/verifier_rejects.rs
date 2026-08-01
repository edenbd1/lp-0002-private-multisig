//! Adversarial audit of the deployed LP-0002 multisig verifier.
//!
//! These tests run the committed `multisig_verifier.bin` through the
//! *sequencer's own* execution path — same input order, same 32M session limit,
//! same executor (`lee/state_machine/src/program.rs:55-110`) — so a rejection
//! here is the same rejection the chain performs. They deliberately do not
//! prove: proving costs minutes and would establish nothing extra about which
//! inputs are accepted.
//!
//! Each test constructs a specific way to steal an execution, or to approve
//! without being a member, and requires the deployed binary to reject it with
//! the matching error code. Honest calls are the controls.
//!
//! Run with: `cargo test -p multisig-verifier-tests --test verifier_rejects`

use lee_core::account::{Account, AccountId, AccountWithMetadata, Nonce};
use lee_core::program::ProgramId;
use multisig_core::*;
use risc0_zkvm::{default_executor, ExecutorEnv};
use serde::Serialize;
use sha2::{Digest, Sha256};

const MAX_NUM_CYCLES_PUBLIC_EXECUTION: u64 = 1024 * 1024 * 32;
const ELF_PATH: &str = "../../artifacts/programs/multisig_verifier.bin";

/// The instruction enum `#[lez_program]` generates, in declaration order.
/// risc0's serde encodes the variant index as a leading u32, then the fields in
/// order. `ProgramContext` is injected by the dispatcher and is deliberately
/// absent here — it is not part of the ABI.
#[derive(Serialize)]
enum VerifierInstruction {
    #[allow(dead_code)]
    CreateMultisig {
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        member_root: [u8; 32],
        threshold: u32,
    },
    #[allow(dead_code)]
    CreateProposal {
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        proposal_id: [u8; 32],
        action_hash: [u8; 32],
        proposal_ref: [u8; 32],
    },
    Approve {
        witness_words: Vec<u32>,
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        member_root: [u8; 32],
        threshold: u32,
        proposal_ref: [u8; 32],
        nullifier: [u8; 32],
        approval_marker_seed: [u8; 32],
    },
    Execute {
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        member_root: [u8; 32],
        threshold: u32,
        proposal_ref: [u8; 32],
        approval_nullifiers: Vec<[u8; 32]>,
        execution_marker_seed: [u8; 32],
    },
}

fn elf() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(ELF_PATH);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}. Build it with:\n  cargo risczero build --manifest-path \
             crates/multisig-verifier-spel/methods/guest/Cargo.toml",
            path.display()
        )
    })
}

fn program_id(elf: &[u8]) -> ProgramId {
    risc0_binfmt::ProgramBinary::decode(elf)
        .expect("verifier binary must decode")
        .compute_image_id()
        .expect("image id")
        .into()
}

/// The public PDA derivation SPEL uses: multi-seed combines with SHA256, then
/// `AccountId::for_public_pda`. Byte-identical to `lee_core`'s `for_public_pda`.
fn public_pda(program: &ProgramId, seeds: &[[u8; 32]]) -> AccountId {
    let combined: [u8; 32] = if seeds.len() == 1 {
        seeds[0]
    } else {
        let mut h = Sha256::new();
        for s in seeds {
            h.update(s);
        }
        h.finalize().into()
    };
    let mut bytes = [0u8; 96];
    bytes[0..32].copy_from_slice(b"/LEE/v0.2/AccountId/PDA/\x00\x00\x00\x00\x00\x00\x00\x00");
    let pid: &[u8] = bytemuck::cast_slice(program);
    bytes[32..64].copy_from_slice(pid);
    bytes[64..96].copy_from_slice(&combined);
    AccountId::new(Sha256::digest(bytes).into())
}

fn owned_by(owner: ProgramId, id: AccountId) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: owner,
            balance: 0,
            data: Default::default(),
            nonce: Nonce(0),
        },
        is_authorized: false,
        account_id: id,
    }
}

fn uninitialised(id: AccountId) -> AccountWithMetadata {
    owned_by(ProgramId::default(), id)
}

fn signer(id: [u8; 32]) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: true,
        account_id: AccountId::new(id),
    }
}

fn run(
    elf: &[u8],
    pid: &ProgramId,
    pre: Vec<AccountWithMetadata>,
    ix: &VerifierInstruction,
) -> anyhow::Result<()> {
    let caller: Option<ProgramId> = None;
    let data = risc0_zkvm::serde::to_vec(ix)?;
    let mut b = ExecutorEnv::builder();
    b.session_limit(Some(MAX_NUM_CYCLES_PUBLIC_EXECUTION));
    b.write(pid)?;
    b.write(&caller)?;
    b.write(&pre)?;
    b.write(&data)?;
    default_executor().execute(b.build()?, elf)?;
    Ok(())
}

fn session(
    elf: &[u8],
    pid: &ProgramId,
    pre: Vec<AccountWithMetadata>,
    ix: &VerifierInstruction,
) -> anyhow::Result<risc0_zkvm::SessionInfo> {
    let caller: Option<ProgramId> = None;
    let data = risc0_zkvm::serde::to_vec(ix)?;
    let mut b = ExecutorEnv::builder();
    b.session_limit(Some(MAX_NUM_CYCLES_PUBLIC_EXECUTION));
    b.write(pid)?;
    b.write(&caller)?;
    b.write(&pre)?;
    b.write(&data)?;
    default_executor().execute(b.build()?, elf)
}

fn assert_rejected(err: anyhow::Error, code: u32, keyword: &str) {
    let msg = format!("{err:#}").to_lowercase();
    assert!(
        msg.contains(&code.to_string()) || msg.contains(keyword),
        "expected rejection {code} ({keyword}), got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Fixture: a 3-of-5 multisig with one proposal
// ---------------------------------------------------------------------------

/// A complete, consistent 3-of-5 multisig, from which each test breaks one
/// thing. Members are deterministic so the tests are reproducible.
struct Fixture {
    verifier: ProgramId,
    multisig_id: [u8; 32],
    member_root: [u8; 32],
    threshold: u32,
    config_hash: [u8; 32],
    proposal_id: [u8; 32],
    proposal_ref: [u8; 32],
    members: Vec<TestMember>,
}

/// One member's private data plus their Merkle authentication path:
/// `(msk, identifier, salt, leaf_index, siblings)`.
type TestMember = ([u8; 32], u128, [u8; 32], u64, Vec<[u8; 32]>);

impl Fixture {
    fn new(verifier: &ProgramId) -> Self {
        let mut leaves = Vec::new();
        let mut raw = Vec::new();
        for i in 0..5u8 {
            let msk = [i + 1; 32];
            let identifier = u128::from(i) + 1;
            let salt = [0x40 ^ i; 32];
            let aid = derive_account_id(&derive_npk(&msk), identifier);
            leaves.push(compute_member_leaf(&aid, &salt));
            raw.push((msk, identifier, salt));
        }
        let (member_root, paths) = build_member_tree(&leaves);
        let threshold = 3u32;
        let multisig_id = [0xA0; 32];
        let config_hash = compute_config_hash(&member_root, threshold);
        let proposal_id = [0x11; 32];
        let action_hash = compute_action_hash(&multisig_id, b"transfer 100 to the treasury");
        let proposal_ref = compute_proposal_ref(&multisig_id, &proposal_id, &action_hash);

        let members = raw
            .into_iter()
            .enumerate()
            .map(|(i, (msk, identifier, salt))| {
                let (leaf_index, siblings) = paths[i].clone();
                (msk, identifier, salt, leaf_index, siblings)
            })
            .collect();

        Self {
            verifier: *verifier,
            multisig_id,
            member_root,
            threshold,
            config_hash,
            proposal_id,
            proposal_ref,
            members,
        }
    }

    fn multisig_account(&self, anchored: bool) -> AccountWithMetadata {
        let id = public_pda(&self.verifier, &[self.multisig_id, self.config_hash]);
        if anchored {
            owned_by(self.verifier, id)
        } else {
            uninitialised(id)
        }
    }

    fn proposal_account(&self, anchored: bool) -> AccountWithMetadata {
        let id = public_pda(&self.verifier, &[self.proposal_ref]);
        if anchored {
            owned_by(self.verifier, id)
        } else {
            uninitialised(id)
        }
    }

    fn nullifier(&self, member: usize) -> [u8; 32] {
        compute_approval_nullifier(&self.proposal_ref, &self.members[member].0)
    }

    fn marker_seed(&self, member: usize) -> [u8; 32] {
        compute_approval_marker(&self.proposal_ref, &self.nullifier(member))
    }

    /// An anchored approval marker for `member`, as `approve` would leave it.
    fn marker_account(&self, member: usize, anchored: bool) -> AccountWithMetadata {
        let id = public_pda(&self.verifier, &[self.marker_seed(member)]);
        if anchored {
            owned_by(self.verifier, id)
        } else {
            uninitialised(id)
        }
    }

    /// An honest `approve` call by `member`.
    fn approve_ix(&self, member: usize) -> VerifierInstruction {
        let (msk, identifier, salt, leaf_index, siblings) = self.members[member].clone();
        let nullifier = self.nullifier(member);
        let instruction = ApproveInstruction {
            witness: ApproveWitness {
                msk,
                identifier,
                salt,
                merkle_path: siblings,
                leaf_index,
            },
            statement: ApproveStatement {
                member_root: self.member_root,
                proposal_ref: self.proposal_ref,
                nullifier,
            },
        };
        VerifierInstruction::Approve {
            witness_words: risc0_zkvm::serde::to_vec(&instruction).expect("encode witness"),
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            member_root: self.member_root,
            threshold: self.threshold,
            proposal_ref: self.proposal_ref,
            nullifier,
            approval_marker_seed: compute_approval_marker(&self.proposal_ref, &nullifier),
        }
    }

    /// Accounts for `approve`, in declaration order.
    fn approve_accounts(
        &self,
        member: usize,
        ms_anchored: bool,
        prop_anchored: bool,
    ) -> Vec<AccountWithMetadata> {
        vec![
            uninitialised(public_pda(&self.verifier, &[self.marker_seed(member)])),
            self.multisig_account(ms_anchored),
            self.proposal_account(prop_anchored),
            signer([0xC1; 32]),
        ]
    }

    /// An honest `execute` presenting `members`' approvals.
    fn execute_ix(&self, members: &[usize]) -> VerifierInstruction {
        VerifierInstruction::Execute {
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            member_root: self.member_root,
            threshold: self.threshold,
            proposal_ref: self.proposal_ref,
            approval_nullifiers: members.iter().map(|&m| self.nullifier(m)).collect(),
            execution_marker_seed: compute_execution_marker(&self.proposal_ref),
        }
    }

    /// Accounts for `execute`: the four fixed ones, then the approval markers.
    fn execute_accounts(&self, members: &[usize]) -> Vec<AccountWithMetadata> {
        let mut v = vec![
            uninitialised(public_pda(
                &self.verifier,
                &[compute_execution_marker(&self.proposal_ref)],
            )),
            self.multisig_account(true),
            self.proposal_account(true),
            signer([0xE1; 32]),
        ];
        v.extend(members.iter().map(|&m| self.marker_account(m, true)));
        v
    }
}

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
    accounts[4] = f.marker_account(0, false); // right address, default owner
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
    let other_ref = compute_proposal_ref(&f.multisig_id, &f.proposal_id, &other_action);
    let other_nullifier = compute_approval_nullifier(&other_ref, &f.members[0].0);
    let other_marker = compute_approval_marker(&other_ref, &other_nullifier);

    let mut accounts = f.execute_accounts(&[0, 2, 4]);
    accounts[4] = owned_by(pid, public_pda(&pid, &[other_marker]));
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
    let forged_config = compute_config_hash(&f.member_root, forged_threshold);
    let ix = VerifierInstruction::Execute {
        multisig_id: f.multisig_id,
        config_hash: forged_config,
        member_root: f.member_root,
        threshold: forged_threshold,
        proposal_ref: f.proposal_ref,
        approval_nullifiers: vec![f.nullifier(0)],
        execution_marker_seed: compute_execution_marker(&f.proposal_ref),
    };
    // The forged config resolves to a PDA nobody ever initialised.
    let accounts = vec![
        uninitialised(public_pda(
            &pid,
            &[compute_execution_marker(&f.proposal_ref)],
        )),
        uninitialised(public_pda(&pid, &[f.multisig_id, forged_config])),
        f.proposal_account(true),
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
        let members: Vec<usize> = (0..m.min(5)).collect();
        if members.len() < m {
            continue;
        }
        // Re-derive a fixture whose threshold matches the count under test, so
        // each measurement is a real accepted execution rather than a rejection.
        let mut fx = Fixture::new(&pid);
        fx.threshold = members.len() as u32;
        fx.config_hash = compute_config_hash(&fx.member_root, fx.threshold);
        report(
            &format!("execute (M={})", members.len()),
            session(
                &elf,
                &pid,
                fx.execute_accounts(&members),
                &fx.execute_ix(&members),
            )
            .expect("execute executes"),
        );
    }
}
