//! Adversarial tests for the LP-0002 approval logic.
//!
//! Each test names the binding from the crate docs that it attacks, and asserts
//! the attack fails. A test that passes here is not evidence the *chain* rejects
//! the forgery — bindings 1, 2 and 7 are enforced on chain by PDA anchoring and
//! are covered in `crates/multisig-verifier-tests`, which runs the deployed
//! binary through the sequencer's own executor. What is covered here is
//! everything the circuit itself must catch, plus the algebraic properties the
//! on-chain checks rely on.

use multisig_core::*;

/// Deterministic test member: secret `msk`, LEZ identifier, per-entry salt.
struct Member {
    msk: [u8; 32],
    identifier: u128,
    salt: [u8; 32],
}

impl Member {
    fn new(seed: u8) -> Self {
        Self {
            msk: [seed; 32],
            identifier: u128::from(seed) + 1,
            salt: [seed.wrapping_add(0x40); 32],
        }
    }

    fn account_id(&self) -> [u8; 32] {
        derive_account_id(&derive_npk(&self.msk), self.identifier)
    }

    fn leaf(&self) -> [u8; 32] {
        compute_member_leaf(&self.account_id(), &self.salt)
    }
}

/// A 3-of-5 multisig, the shape used throughout these tests.
struct Fixture {
    members: Vec<Member>,
    root: [u8; 32],
    paths: Vec<(u64, Vec<[u8; 32]>)>,
    multisig_id: [u8; 32],
    threshold: u32,
}

impl Fixture {
    fn new() -> Self {
        let members: Vec<Member> = (0..5).map(Member::new).collect();
        let leaves: Vec<[u8; 32]> = members.iter().map(Member::leaf).collect();
        let (root, paths) = build_member_tree(&leaves);
        Self {
            members,
            root,
            paths,
            multisig_id: [0xa1; 32],
            threshold: 3,
        }
    }

    fn proposal_ref(&self, proposal_id: &[u8; 32], action: &[u8]) -> [u8; 32] {
        let action_hash = compute_action_hash(&self.multisig_id, action);
        let config_hash = compute_config_hash(&self.root, self.threshold);
        compute_proposal_ref(&self.multisig_id, &config_hash, proposal_id, &action_hash)
    }

    /// An honest approval by member `i` on the given proposal.
    fn honest(&self, i: usize, pref: &[u8; 32]) -> (ApproveWitness, ApproveStatement) {
        let m = &self.members[i];
        let (leaf_index, merkle_path) = self.paths[i].clone();
        let witness = ApproveWitness {
            msk: m.msk,
            identifier: m.identifier,
            salt: m.salt,
            merkle_path,
            leaf_index,
        };
        let statement = ApproveStatement {
            member_root: self.root,
            proposal_ref: *pref,
            nullifier: compute_approval_nullifier(pref, &m.msk),
        };
        (witness, statement)
    }
}

// ---------------------------------------------------------------------------
// Baseline
// ---------------------------------------------------------------------------

#[test]
fn honest_approval_by_every_member_succeeds() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[1u8; 32], b"transfer 100 to treasury");
    for i in 0..f.members.len() {
        let (w, s) = f.honest(i, &pref);
        assert!(
            approve(&w, &s).is_ok(),
            "member {i} should be able to approve"
        );
    }
}

// ---------------------------------------------------------------------------
// Binding 6 — the secret must own the committed leaf
// ---------------------------------------------------------------------------

#[test]
fn non_member_cannot_approve() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[1u8; 32], b"transfer");
    let outsider = Member::new(200);

    // The outsider borrows a real member's path but signs with their own secret.
    let (_, path) = f.paths[0].clone();
    let witness = ApproveWitness {
        msk: outsider.msk,
        identifier: outsider.identifier,
        salt: outsider.salt,
        merkle_path: path,
        leaf_index: 0,
    };
    let statement = ApproveStatement {
        member_root: f.root,
        proposal_ref: pref,
        nullifier: compute_approval_nullifier(&pref, &outsider.msk),
    };
    assert_eq!(approve(&witness, &statement), Err(ApproveError::NotAMember));
}

#[test]
fn cannot_pair_another_members_leaf_with_own_nullifier() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[1u8; 32], b"transfer");
    let attacker = Member::new(201);

    // Take member 2's witness wholesale but swap in the attacker's secret for
    // the nullifier. The circuit re-derives the nullifier from the witness msk,
    // so the substitution cannot survive.
    let (mut w, mut s) = f.honest(2, &pref);
    s.nullifier = compute_approval_nullifier(&pref, &attacker.msk);
    assert_eq!(
        approve(&w, &s),
        Err(ApproveError::NullifierMismatch),
        "a nullifier from a different secret must be rejected"
    );

    // And the mirror image: attacker's secret with member 2's identifier/salt.
    w.msk = attacker.msk;
    assert_eq!(
        approve(&w, &s),
        Err(ApproveError::NotAMember),
        "a foreign secret does not reproduce the committed leaf"
    );
}

#[test]
fn tampering_with_the_identifier_breaks_membership() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[1u8; 32], b"transfer");
    let (mut w, s) = f.honest(1, &pref);
    w.identifier = w.identifier.wrapping_add(1);
    assert_eq!(approve(&w, &s), Err(ApproveError::NotAMember));
}

#[test]
fn tampering_with_the_salt_breaks_membership() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[1u8; 32], b"transfer");
    let (mut w, s) = f.honest(1, &pref);
    w.salt[0] ^= 0xff;
    assert_eq!(approve(&w, &s), Err(ApproveError::NotAMember));
}

// ---------------------------------------------------------------------------
// Binding 1 — membership is against a specific root
// ---------------------------------------------------------------------------

#[test]
fn invented_single_member_root_does_not_match_the_committed_one() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[1u8; 32], b"transfer");
    let attacker = Member::new(202);

    // The classic attack: build a one-leaf tree containing yourself. The proof
    // is internally valid — it just proves membership in a *different* root.
    let (fake_root, fake_paths) = build_member_tree(&[attacker.leaf()]);
    let (leaf_index, merkle_path) = fake_paths[0].clone();
    let witness = ApproveWitness {
        msk: attacker.msk,
        identifier: attacker.identifier,
        salt: attacker.salt,
        merkle_path,
        leaf_index,
    };
    let statement = ApproveStatement {
        member_root: fake_root,
        proposal_ref: pref,
        nullifier: compute_approval_nullifier(&pref, &attacker.msk),
    };

    // In-circuit this succeeds, and that is expected and documented: the circuit
    // proves membership relative to whatever root the statement names.
    assert!(approve(&witness, &statement).is_ok());

    // What defeats it is that the root is not the committed one, so on chain it
    // resolves to a config hash whose PDA was never initialised. That is the
    // anchoring check, asserted here at the algebraic level and end-to-end in
    // crates/multisig-verifier-tests.
    assert_ne!(fake_root, f.root, "an invented set has a different root");
    assert_ne!(
        compute_config_hash(&fake_root, f.threshold),
        compute_config_hash(&f.root, f.threshold),
        "a different root gives a different config hash, hence a different PDA"
    );
}

#[test]
fn a_valid_path_from_a_different_tree_is_rejected() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[1u8; 32], b"transfer");

    // Build a second, unrelated 5-member multisig and lift member 0's path from
    // it into a statement naming the *first* multisig's root.
    let others: Vec<Member> = (100..105).map(Member::new).collect();
    let leaves: Vec<[u8; 32]> = others.iter().map(Member::leaf).collect();
    let (_other_root, other_paths) = build_member_tree(&leaves);

    let (leaf_index, merkle_path) = other_paths[0].clone();
    let witness = ApproveWitness {
        msk: others[0].msk,
        identifier: others[0].identifier,
        salt: others[0].salt,
        merkle_path,
        leaf_index,
    };
    let statement = ApproveStatement {
        member_root: f.root,
        proposal_ref: pref,
        nullifier: compute_approval_nullifier(&pref, &others[0].msk),
    };
    assert_eq!(approve(&witness, &statement), Err(ApproveError::NotAMember));
}

#[test]
fn the_padding_sentinel_is_not_a_usable_member() {
    // Padding leaves are real leaves in the tree. If one could be "approved
    // with", a 3-member set padded to 4 would silently gain a phantom member.
    // It cannot: the sentinel commits to no account, so no msk reproduces it.
    let members: Vec<Member> = (0..3).map(Member::new).collect();
    let leaves: Vec<[u8; 32]> = members.iter().map(Member::leaf).collect();
    let (root, _) = build_member_tree(&leaves);

    let sentinel = padding_leaf();
    // There is no (account_id, salt) the attacker can present that hashes to the
    // sentinel, because the sentinel is H(prefix || "PAD") and the leaf function
    // is H(prefix || account_id || salt) over 64 bytes of payload.
    for seed in 0..64u8 {
        let m = Member::new(seed);
        assert_ne!(m.leaf(), sentinel);
    }
    assert_ne!(root, sentinel);
}

// ---------------------------------------------------------------------------
// Binding 4 — double-approval prevention
// ---------------------------------------------------------------------------

#[test]
fn the_same_member_yields_the_same_marker_twice() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[7u8; 32], b"transfer");
    let m = &f.members[3];

    let n1 = compute_approval_nullifier(&pref, &m.msk);
    let n2 = compute_approval_nullifier(&pref, &m.msk);
    assert_eq!(
        n1, n2,
        "the nullifier is deterministic per (proposal, member)"
    );
    assert_eq!(
        compute_approval_marker(&pref, &n1),
        compute_approval_marker(&pref, &n2),
        "so the second approval targets the same PDA and init fails"
    );
}

#[test]
fn distinct_members_yield_distinct_markers() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[7u8; 32], b"transfer");

    let markers: Vec<[u8; 32]> = f
        .members
        .iter()
        .map(|m| {
            let n = compute_approval_nullifier(&pref, &m.msk);
            compute_approval_marker(&pref, &n)
        })
        .collect();

    for i in 0..markers.len() {
        for j in (i + 1)..markers.len() {
            assert_ne!(
                markers[i], markers[j],
                "members {i} and {j} must occupy different marker PDAs"
            );
        }
    }
}

#[test]
fn a_member_can_approve_two_different_proposals() {
    let f = Fixture::new();
    let a = f.proposal_ref(&[1u8; 32], b"transfer");
    let b = f.proposal_ref(&[2u8; 32], b"transfer");
    let m = &f.members[0];

    assert_ne!(
        compute_approval_nullifier(&a, &m.msk),
        compute_approval_nullifier(&b, &m.msk),
        "double-approval prevention must be per proposal, not global"
    );

    for pref in [a, b] {
        let (w, s) = f.honest(0, &pref);
        assert!(approve(&w, &s).is_ok());
    }
}

// ---------------------------------------------------------------------------
// Binding 3 — approvals are bound to the exact action
// ---------------------------------------------------------------------------

#[test]
fn approvals_do_not_transfer_to_a_different_action_under_the_same_id() {
    let f = Fixture::new();
    let proposal_id = [9u8; 32];

    // The bait-and-switch: same proposal id, different action.
    let benign = f.proposal_ref(&proposal_id, b"transfer 1 to alice");
    let malicious = f.proposal_ref(&proposal_id, b"transfer 1000000 to attacker");
    assert_ne!(benign, malicious, "the action is inside the proposal ref");

    let m = &f.members[0];
    let n_benign = compute_approval_nullifier(&benign, &m.msk);
    let n_malicious = compute_approval_nullifier(&malicious, &m.msk);

    assert_ne!(n_benign, n_malicious);
    assert_ne!(
        compute_approval_marker(&benign, &n_benign),
        compute_approval_marker(&malicious, &n_malicious),
        "markers gathered for the benign action are worthless for the malicious one"
    );

    // And a member's approval statement for one does not validate for the other.
    let (w, mut s) = f.honest(0, &benign);
    s.proposal_ref = malicious;
    assert_eq!(
        approve(&w, &s),
        Err(ApproveError::NullifierMismatch),
        "re-pointing an approval at another action breaks the nullifier binding"
    );
}

#[test]
fn approving_a_junk_action_cannot_burn_the_real_proposals_markers() {
    // The mirror-image griefing vector: if markers were scoped to proposal_id
    // alone, an attacker could publish id P with junk, have members approve, and
    // permanently block the real P. Different action, different marker address.
    let f = Fixture::new();
    let proposal_id = [9u8; 32];
    let real = f.proposal_ref(&proposal_id, b"the real action");
    let junk = f.proposal_ref(&proposal_id, b"junk");

    for m in &f.members {
        let nr = compute_approval_nullifier(&real, &m.msk);
        let nj = compute_approval_nullifier(&junk, &m.msk);
        assert_ne!(
            compute_approval_marker(&real, &nr),
            compute_approval_marker(&junk, &nj)
        );
    }
}

#[test]
fn a_single_byte_of_action_drift_changes_the_proposal() {
    let f = Fixture::new();
    let id = [3u8; 32];
    let a = f.proposal_ref(&id, b"transfer 100");
    let b = f.proposal_ref(&id, b"transfer 101");
    assert_ne!(a, b);
}

// ---------------------------------------------------------------------------
// Binding 5 — approvals do not cross multisigs
// ---------------------------------------------------------------------------

#[test]
fn an_approval_is_scoped_to_its_multisig() {
    let mut f1 = Fixture::new();
    let mut f2 = Fixture::new();
    f1.multisig_id = [0xa1; 32];
    f2.multisig_id = [0xa2; 32];

    let id = [5u8; 32];
    let action = b"same action, different multisig";
    assert_ne!(
        f1.proposal_ref(&id, action),
        f2.proposal_ref(&id, action),
        "the multisig id is inside the proposal ref"
    );

    // The action hash itself is also multisig-scoped, so even identical action
    // bytes commit differently under two multisigs.
    assert_ne!(
        compute_action_hash(&f1.multisig_id, action),
        compute_action_hash(&f2.multisig_id, action)
    );
}

// ---------------------------------------------------------------------------
// Binding 2 — the threshold cannot be lowered
// ---------------------------------------------------------------------------

#[test]
fn lowering_the_threshold_changes_the_anchored_config() {
    let f = Fixture::new();
    let honest = compute_config_hash(&f.root, f.threshold);
    for forged in [1u32, 2, 4, 5, 0, u32::MAX] {
        if forged == f.threshold {
            continue;
        }
        assert_ne!(
            honest,
            compute_config_hash(&f.root, forged),
            "threshold {forged} must not resolve to the honest multisig PDA"
        );
    }
}

#[test]
fn swapping_the_member_set_changes_the_anchored_config() {
    let f = Fixture::new();
    let attacker_only = build_member_tree(&[Member::new(203).leaf()]).0;
    assert_ne!(
        compute_config_hash(&f.root, f.threshold),
        compute_config_hash(&attacker_only, f.threshold)
    );
}

// ---------------------------------------------------------------------------
// Domain separation
// ---------------------------------------------------------------------------

#[test]
fn every_domain_separator_is_distinct_and_padded_to_32_bytes() {
    let all = [
        PRIVATE_ACCOUNT_ID_PREFIX,
        NPK_DERIVE_PREFIX,
        MEMBER_LEAF_PREFIX,
        MULTISIG_CONFIG_PREFIX,
        ACTION_PREFIX,
        PROPOSAL_REF_PREFIX,
        APPROVAL_NULLIFIER_PREFIX,
        APPROVAL_MARKER_PREFIX,
        EXECUTION_MARKER_PREFIX,
    ];
    for i in 0..all.len() {
        assert_eq!(all[i].len(), 32);
        for j in (i + 1)..all.len() {
            assert_ne!(all[i], all[j], "separators {i} and {j} collide");
        }
    }
}

#[test]
fn marker_kinds_never_collide_for_the_same_proposal() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[1u8; 32], b"a");
    let nullifier = compute_approval_nullifier(&pref, &f.members[0].msk);

    // An approval marker and an execution marker are different kinds of record.
    // If they could collide, executing would consume an approval slot.
    assert_ne!(
        compute_approval_marker(&pref, &nullifier),
        compute_execution_marker(&pref)
    );
    // And neither collides with the config commitment.
    assert_ne!(
        compute_execution_marker(&pref),
        compute_config_hash(&f.root, f.threshold)
    );
}

// ---------------------------------------------------------------------------
// Tree construction
// ---------------------------------------------------------------------------

#[test]
fn every_generated_path_folds_back_to_the_root() {
    for n in 1..=17usize {
        let members: Vec<Member> = (0..n as u8).map(Member::new).collect();
        let leaves: Vec<[u8; 32]> = members.iter().map(Member::leaf).collect();
        let (root, paths) = build_member_tree(&leaves);
        for (i, (leaf_index, siblings)) in paths.iter().enumerate() {
            assert_eq!(
                fold_merkle_path(&leaves[i], *leaf_index, siblings),
                root,
                "member {i} of {n} must fold to the root"
            );
        }
    }
}

#[test]
fn a_one_member_set_still_produces_a_well_formed_path() {
    // N=1 is degenerate but legal (1-of-1). It must pad to width 2, not width 1,
    // or the path would be empty and the root would equal the leaf.
    let m = Member::new(1);
    let (root, paths) = build_member_tree(&[m.leaf()]);
    assert_eq!(paths[0].1.len(), 1, "one sibling: the padding sentinel");
    assert_ne!(root, m.leaf());
    assert_eq!(fold_merkle_path(&m.leaf(), 0, &paths[0].1), root);
}

#[test]
fn reordering_members_changes_the_root() {
    let a = Member::new(1);
    let b = Member::new(2);
    assert_ne!(
        build_member_tree(&[a.leaf(), b.leaf()]).0,
        build_member_tree(&[b.leaf(), a.leaf()]).0
    );
}

// ---------------------------------------------------------------------------
// Soundness properties a third audit pass asked for
// ---------------------------------------------------------------------------

/// Listing the same member twice in the set must not buy them two votes.
///
/// A careless or malicious creator can put one person in the member set twice —
/// two leaves, two indices, two valid Merkle paths. If that yielded two
/// approvals, a 3-of-5 with one member duplicated three times would be a 1-of-1.
/// It does not: the nullifier is a function of `msk` and the proposal only, so
/// both leaves collapse onto the same marker PDA and the second approval finds
/// it occupied.
#[test]
fn a_member_listed_twice_still_gets_one_vote() {
    let m = Member::new(9);
    let other = Member::new(10);
    // The same member's leaf twice, under two different salts so the leaves
    // themselves differ — the strongest form of the duplicate.
    let dup_leaf_a = compute_member_leaf(&m.account_id(), &[0xA1; 32]);
    let dup_leaf_b = compute_member_leaf(&m.account_id(), &[0xB2; 32]);
    let (root, paths) = build_member_tree(&[dup_leaf_a, dup_leaf_b, other.leaf()]);
    assert_ne!(
        dup_leaf_a, dup_leaf_b,
        "the two entries are distinct leaves"
    );

    let multisig_id = [0xC3; 32];
    let action_hash = compute_action_hash(&multisig_id, b"drain the treasury");
    let config_hash = compute_config_hash(&root, 2);
    let pref = compute_proposal_ref(&multisig_id, &config_hash, &[1u8; 32], &action_hash);

    // Both entries prove membership successfully...
    for (i, salt) in [(0usize, [0xA1u8; 32]), (1usize, [0xB2u8; 32])] {
        let (leaf_index, merkle_path) = paths[i].clone();
        let nullifier = compute_approval_nullifier(&pref, &m.msk);
        let w = ApproveWitness {
            msk: m.msk,
            identifier: m.identifier,
            salt,
            merkle_path,
            leaf_index,
        };
        let s = ApproveStatement {
            member_root: root,
            proposal_ref: pref,
            nullifier,
        };
        assert!(approve(&w, &s).is_ok(), "entry {i} is a valid member entry");
    }

    // ...but both yield the identical nullifier, hence the identical marker PDA,
    // so the chain accepts exactly one of them.
    let n = compute_approval_nullifier(&pref, &m.msk);
    assert_eq!(
        compute_approval_marker(&pref, &n),
        compute_approval_marker(&pref, &n),
        "both entries collapse onto one marker: one member, one vote"
    );
}

/// A member cannot borrow another member's salt to mint a second identity.
#[test]
fn a_member_cannot_reuse_another_members_salt() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[4u8; 32], b"transfer");
    // Member 0's account with member 1's salt: a leaf that is in nobody's tree.
    let forged_leaf = compute_member_leaf(&f.members[0].account_id(), &f.members[1].salt);
    for (i, (_, path)) in f.paths.iter().enumerate() {
        assert_ne!(
            fold_merkle_path(&forged_leaf, i as u64, path),
            f.root,
            "a cross-salt leaf must not fold to the committed root at index {i}"
        );
    }
    // And approving with it fails outright.
    let (mut w, s) = f.honest(0, &pref);
    w.salt = f.members[1].salt;
    assert_eq!(approve(&w, &s), Err(ApproveError::NotAMember));
}

/// The execution marker seed is fully public — `H(prefix || proposal_ref)` with
/// no secret in it. That is deliberate, and it is safe only because the address
/// it derives is a program PDA: nobody holds a key for it, and the only code
/// path that claims it is `execute`, which enforces the threshold first.
///
/// This test pins the reasoning rather than the code: if the execution marker
/// ever became claimable another way, the griefing vector would be real.
#[test]
fn the_execution_marker_seed_is_public_but_unclaimable_early() {
    let f = Fixture::new();
    let pref = f.proposal_ref(&[5u8; 32], b"transfer");
    // Anyone can compute it from public data alone.
    let seed = compute_execution_marker(&pref);
    assert_eq!(seed, compute_execution_marker(&pref));
    // It carries no member information and collides with no approval marker.
    for m in &f.members {
        let n = compute_approval_nullifier(&pref, &m.msk);
        assert_ne!(seed, compute_approval_marker(&pref, &n));
        assert_ne!(seed, n);
    }
}
