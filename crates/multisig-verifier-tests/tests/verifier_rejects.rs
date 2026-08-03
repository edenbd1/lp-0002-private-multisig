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

/// Require the call to be refused, by the program's own code or by SPEL's
/// address validation before the body runs.
///
/// A `PdaMismatch` on `proposal` counts, and it counts as *better* than the
/// code. Once `config_hash` is a seed of the proposal account, naming a config
/// the proposal does not belong to resolves to an address nobody ever created —
/// so the forgery stops being something the program checks and rejects, and
/// becomes something that cannot be expressed. Several of these tests used to
/// see `5002`/`5003`; they now see the address check, one layer earlier, and
/// that is the fix working rather than the test weakening.
fn assert_rejected(err: anyhow::Error, code: u32, keyword: &str) {
    let msg = format!("{err:#}").to_lowercase();
    let by_code = msg.contains(&code.to_string()) || msg.contains(keyword);
    let by_address = msg.contains("pdamismatch");
    assert!(
        by_code || by_address,
        "expected rejection {code} ({keyword}) or an address mismatch, got: {msg}"
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
        let proposal_ref =
            compute_proposal_ref(&multisig_id, &config_hash, &proposal_id, &action_hash);

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
        // Seeded by [multisig_id, proposal_ref]: the multisig_id in the address
        // is what stops a proposal being paired with a foreign multisig.
        let id = public_pda(
            &self.verifier,
            &[self.multisig_id, self.config_hash, self.proposal_ref],
        );
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
    let other_ref = compute_proposal_ref(
        &f.multisig_id,
        &f.config_hash,
        &f.proposal_id,
        &other_action,
    );
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

// ---------------------------------------------------------------------------
// create_multisig and create_proposal
//
// These two instructions had no coverage against the binary until an audit
// pass noticed that three documented error codes — 5001, 5007, 5008 — were
// never exercised, while docs/error-codes.md claimed every code was. The
// claim is only worth making if the tests exist, so here they are.
// ---------------------------------------------------------------------------

/// Accounts for `create_multisig`, in declaration order.
fn create_multisig_accounts(f: &Fixture, config_hash: [u8; 32]) -> Vec<AccountWithMetadata> {
    vec![
        uninitialised(public_pda(&f.verifier, &[f.multisig_id, config_hash])),
        signer([0xA1; 32]),
    ]
}

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
    };
    run(&elf, &pid, create_multisig_accounts(&f, f.config_hash), &ix)
        .expect("a well-formed multisig must be creatable");
}

/// A 0-of-N multisig would let anyone execute, so creation must refuse it
/// outright rather than leave such an instance reachable on chain.
#[test]
fn creating_a_zero_threshold_multisig_is_rejected() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let config = compute_config_hash(&f.member_root, 0);
    let ix = VerifierInstruction::CreateMultisig {
        multisig_id: f.multisig_id,
        config_hash: config,
        member_root: f.member_root,
        threshold: 0,
    };
    let err = run(&elf, &pid, create_multisig_accounts(&f, config), &ix)
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
    };
    let err = run(&elf, &pid, create_multisig_accounts(&f, forged), &ix)
        .expect_err("an inconsistent config hash must be rejected");
    assert_rejected(err, 5002, "config");
}

/// Accounts for `create_proposal`, in declaration order.
fn create_proposal_accounts(
    f: &Fixture,
    proposal_ref: [u8; 32],
    multisig_anchored: bool,
) -> Vec<AccountWithMetadata> {
    vec![
        uninitialised(public_pda(
            &f.verifier,
            &[f.multisig_id, f.config_hash, proposal_ref],
        )),
        f.multisig_account(multisig_anchored),
        signer([0xA2; 32]),
    ]
}

/// The control: an honest proposal against an anchored multisig is accepted.
#[test]
fn an_honest_create_proposal_is_accepted() {
    let elf = elf();
    let pid = program_id(&elf);
    let f = Fixture::new(&pid);
    let action_hash = compute_action_hash(&f.multisig_id, b"transfer 100 to the treasury");
    let ix = VerifierInstruction::CreateProposal {
        multisig_id: f.multisig_id,
        config_hash: f.config_hash,
        proposal_id: f.proposal_id,
        action_hash,
        proposal_ref: f.proposal_ref,
    };
    run(
        &elf,
        &pid,
        create_proposal_accounts(&f, f.proposal_ref, true),
        &ix,
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
    let action_hash = compute_action_hash(&f.multisig_id, b"transfer 100 to the treasury");
    let forged_ref = [0xEF; 32];
    let ix = VerifierInstruction::CreateProposal {
        multisig_id: f.multisig_id,
        config_hash: f.config_hash,
        proposal_id: f.proposal_id,
        action_hash,
        proposal_ref: forged_ref,
    };
    let err = run(
        &elf,
        &pid,
        create_proposal_accounts(&f, forged_ref, true),
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
    let action_hash = compute_action_hash(&f.multisig_id, b"transfer 100 to the treasury");
    let ix = VerifierInstruction::CreateProposal {
        multisig_id: f.multisig_id,
        config_hash: f.config_hash,
        proposal_id: f.proposal_id,
        action_hash,
        proposal_ref: f.proposal_ref,
    };
    let err = run(
        &elf,
        &pid,
        create_proposal_accounts(&f, f.proposal_ref, false),
        &ix,
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
// Attacks an audit pass asked for that the original suite did not cover
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
    accounts[4] = owned_by(pid, public_pda(&pid, &[other_marker]));
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
    let forged_config = compute_config_hash(&fake_root, f.threshold);
    let ix = VerifierInstruction::Approve {
        witness_words: risc0_zkvm::serde::to_vec(&instruction).expect("encode"),
        multisig_id: f.multisig_id,
        config_hash: forged_config,
        member_root: fake_root,
        threshold: f.threshold,
        proposal_ref: f.proposal_ref,
        nullifier,
        approval_marker_seed: marker_seed,
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

/// AUDIT PROBE: can a proposal be executed while naming a *different* multisig,
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
    let attacker_config = compute_config_hash(&f.member_root, attacker_threshold);

    let ix = VerifierInstruction::Execute {
        multisig_id: attacker_msig,
        config_hash: attacker_config,
        member_root: f.member_root,
        threshold: attacker_threshold,
        // ...but the proposal is the real one, from the honest 3-of-5 multisig.
        proposal_ref: f.proposal_ref,
        approval_nullifiers: vec![f.nullifier(0)],
        execution_marker_seed: compute_execution_marker(&f.proposal_ref),
    };
    let accounts = vec![
        uninitialised(public_pda(
            &pid,
            &[compute_execution_marker(&f.proposal_ref)],
        )),
        // The attacker's multisig PDA, which they really did create.
        owned_by(pid, public_pda(&pid, &[attacker_msig, attacker_config])),
        f.proposal_account(true),
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
/// This was step 2 of the chain the audit probe uncovered — an outsider mints a
/// valid-looking marker on a proposal they are not a member of, which step 3
/// then counts. Binding the proposal address to its multisig closes both halves
/// at once.
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
    let attacker_config = compute_config_hash(&root, 1);

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

// ── The variant the first fix did not close ──────────────────────────────────
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
    let a_config = compute_config_hash(&a_root, a_threshold);
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
    let a_config = compute_config_hash(&a_root, a_threshold);
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
    };
    let accounts = vec![
        uninitialised(public_pda(
            &pid,
            &[compute_execution_marker(&f.proposal_ref)],
        )),
        owned_by(pid, public_pda(&pid, &[f.multisig_id, a_config])),
        f.proposal_account(true),
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
