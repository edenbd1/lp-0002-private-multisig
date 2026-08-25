//! The SDK must derive byte-identical commitments to what the on-chain program
//! re-derives. If the two ever diverge, an integrator builds arguments the chain
//! rejects — and the failure surfaces minutes later as an opaque on-chain error
//! rather than here.
//!
//! The SDK needs a test of its own, so that nothing
//! stopped it drifting from `multisig-core`.

use multisig_core::*;
use multisig_sdk::*;

/// Every value the SDK exposes must equal the `multisig-core` derivation the
/// verifier program performs on chain.
#[test]
fn sdk_derivations_match_core_exactly() {
    let entries: Vec<([u8; 32], [u8; 32])> = (0..5u8)
        .map(|i| {
            let msk = [i + 1; 32];
            let ident = u128::from(i) + 1;
            let salt = [0x40 ^ i; 32];
            (derive_account_id(&derive_npk(&msk), ident), salt)
        })
        .collect();
    let members = MemberSet::from_entries(&entries);
    let ms = Multisig::for_members([0xA0; 32], &members, 3).unwrap();
    // core-level derivation
    let core_root = build_member_tree(
        &entries
            .iter()
            .map(|(a, s)| compute_member_leaf(a, s))
            .collect::<Vec<_>>(),
    )
    .0;
    assert_eq!(members.root(), core_root, "SDK root != core root");
    assert_eq!(
        ms.config_hash(),
        compute_config_hash(&core_root, 3, &no_tiers_hash()),
        "SDK config_hash != core"
    );
    let memo = b"transfer 250 to the grants treasury";
    let recipient = [0x5E; 32];
    let p = Proposal::new(&ms, [0x11; 32], recipient, 250, memo);
    let memo_hash = compute_memo_hash(memo);
    let ah = compute_transfer_action_hash(&[0xA0; 32], &recipient, 250, &memo_hash);
    assert_eq!(p.action_hash(), ah, "SDK action_hash != core");
    assert_eq!(p.memo_hash(), memo_hash, "SDK memo_hash != core");
    assert_eq!(p.recipient(), recipient);
    assert_eq!(p.amount(), 250);
    // The canonical action bytes are what the digest above commits to, and the
    // on-chain program rebuilds them from the record it stores.
    assert_eq!(
        ah,
        compute_action_hash(&[0xA0; 32], &encode_action(&recipient, 250, &memo_hash)),
        "the transfer helper must equal the long form"
    );
    assert_eq!(
        p.proposal_ref(),
        compute_proposal_ref(&[0xA0; 32], &ms.config_hash(), &[0x11; 32], &ah),
        "SDK proposal_ref != core"
    );
    assert_eq!(
        p.execution_marker_seed(),
        compute_execution_marker(&p.proposal_ref()),
        "SDK exec seed != core"
    );
    // approval derivation
    let a = p
        .approval(&members, 2, [3u8; 32], 3u128)
        .expect("member 2 approves");
    let n = compute_approval_nullifier(&p.proposal_ref(), &[3u8; 32]);
    assert_eq!(a.nullifier, n, "SDK nullifier != core");
    assert_eq!(
        a.marker_seed,
        compute_approval_marker(&p.proposal_ref(), &n),
        "SDK marker != core"
    );
    // find_by_secret must locate the right member
    assert_eq!(
        members.find_by_secret(&[3u8; 32], 3u128),
        Some(2),
        "find_by_secret wrong"
    );
    // threshold guards
    assert!(
        Multisig::new([0; 32], [0; 32], 0).is_err(),
        "zero threshold must be refused"
    );
    assert!(
        Multisig::for_members([0; 32], &members, 9).is_err(),
        "threshold > N must be refused"
    );
    assert!(
        p.execute_args(std::slice::from_ref(&a)).is_err(),
        "short threshold must be refused"
    );
    assert!(
        p.execute_args(&[a.clone(), a.clone(), a.clone()]).is_err(),
        "duplicates must be refused"
    );
    // A payment of nothing is refused before it can be published, with the same
    // verdict the chain would give minutes later.
    let empty = Proposal::new(&ms, [0x12; 32], recipient, 0, memo);
    assert_eq!(
        empty.create_args().unwrap_err(),
        SdkError::ZeroAmount,
        "a zero-amount proposal must be refused"
    );
    assert_eq!(ms.fund_args(0).unwrap_err(), SdkError::ZeroAmount);
    // The treasury seeds are the multisig's own two, then the literal.
    assert_eq!(
        ms.treasury_seeds(),
        [[0xA0; 32], ms.config_hash(), TREASURY_SEED]
    );
    assert_eq!(&TREASURY_SEED[..8], b"treasury");
    assert_eq!(&TREASURY_SEED[8..], &[0u8; 24]);
}

/// The tier and rotation surface must derive exactly what the chain re-derives.
///
/// The SDK is the only path an integrator has that is not the CLI, and every
/// value it computes is one the on-chain program recomputes and compares. A
/// divergence here does not surface as a wrong answer: it surfaces minutes later
/// as `E_CONFIG_MISMATCH` on a transaction that already cost a proof.
#[test]
fn tier_and_rotation_derivations_match_core_exactly() {
    let entries: Vec<([u8; 32], [u8; 32])> = (0..5u8)
        .map(|i| {
            let msk = [i + 1; 32];
            let ident = u128::from(i) + 1;
            let salt = [0x40 ^ i; 32];
            (derive_account_id(&derive_npk(&msk), ident), salt)
        })
        .collect();
    let members = MemberSet::from_entries(&entries);

    let table = [
        TierPolicy { max_amount: 300, threshold: 2 },
        TierPolicy { max_amount: 10_000, threshold: 3 },
    ];
    let ms = Multisig::for_members([0xA0; 32], &members, 3)
        .unwrap()
        .with_tiers(&table)
        .expect("a monotone table that never exceeds the default");

    // The commitment, and the bytes under it.
    assert_eq!(
        ms.config_hash(),
        compute_config_hash(&members.root(), 3, &compute_tiers_hash(&table)),
        "SDK config_hash does not fold the tiers the way core does"
    );
    assert_ne!(
        ms.config_hash(),
        compute_config_hash(&members.root(), 3, &no_tiers_hash()),
        "if tiers did not move config_hash they would not be anchored at all"
    );
    assert_eq!(
        encode_tiers(&table),
        encode_tier_table(&table),
        "the SDK must put the same bytes on the wire that core hashes"
    );
    assert_eq!(
        decode_tier_table(&encode_tiers(&table)).unwrap().len(),
        2,
        "and those bytes must decode back"
    );

    // What a client should gather, which is the number the chain will require.
    assert_eq!(ms.required_for(1), 2);
    assert_eq!(ms.required_for(300), 2, "the cap is inclusive");
    assert_eq!(ms.required_for(301), 3);
    assert_eq!(ms.required_for(u128::MAX), 3, "past every tier, the default");

    // A table the chain would refuse is refused here, before any proving.
    assert!(
        Multisig::for_members([0xA0; 32], &members, 3)
            .unwrap()
            .with_tiers(&[TierPolicy { max_amount: 300, threshold: 4 }])
            .is_err(),
        "a tier above the default threshold must not be constructible"
    );
    assert!(
        Multisig::for_members([0xA0; 32], &members, 3)
            .unwrap()
            .with_tiers(&[
                TierPolicy { max_amount: 300, threshold: 2 },
                TierPolicy { max_amount: 200, threshold: 3 },
            ])
            .is_err(),
        "caps must strictly increase"
    );

    // A transfer under a tier presents the tier's count, not the default.
    let p = Proposal::new(&ms, [0x11; 32], [0x5E; 32], 250, b"small");
    assert_eq!(p.required(), 2, "a 250 transfer is covered by the 300 tier");
    let approvals: Vec<Approval> = (0..2)
        .map(|i| p.approval(&members, i, [(i as u8) + 1; 32], i as u128 + 1).unwrap())
        .collect();
    let args = p.execute_args(&approvals).expect("two approvals satisfy the tier");
    assert_eq!(args.approval_nullifiers.len(), 2);
    assert_eq!(args.tiers, encode_tier_table(&table));

    // Rotation: a second configuration, at its own address.
    let next = ms
        .rotation(members.root(), 4, &[])
        .expect("a different threshold is a different configuration");
    assert_eq!(
        next.config_hash(),
        compute_config_hash(&members.root(), 4, &no_tiers_hash()),
        "SDK rotation target != core"
    );
    assert_ne!(next.config_hash(), ms.config_hash());
    assert!(
        ms.rotation(members.root(), 3, &table).is_err(),
        "rotating to the configuration already in force must be refused"
    );

    let r = Proposal::rotation(&ms, [0x22; 32], &next).expect("a real rotation");
    assert!(r.is_rotation());
    assert_eq!(
        r.action_hash(),
        compute_rotate_action_hash(&[0xA0; 32], &next.config_hash()),
        "SDK rotate action_hash != core"
    );
    assert_eq!(
        r.proposal_ref(),
        compute_proposal_ref(&[0xA0; 32], &ms.config_hash(), &[0x22; 32], &r.action_hash()),
        "a rotation proposal is scoped to the configuration that raised it"
    );
    assert_eq!(
        r.required(),
        3,
        "a rotation costs the default threshold, never the tier that would price a transfer"
    );

    // The two shapes cannot be spent by each other's instruction.
    let rot_approvals: Vec<Approval> = (0..3)
        .map(|i| r.approval(&members, i, [(i as u8) + 1; 32], i as u128 + 1).unwrap())
        .collect();
    assert!(
        r.execute_args(&rot_approvals).is_err(),
        "a rotation must not be spendable by execute"
    );
    assert!(
        p.rotate_args(&next, &approvals).is_err(),
        "a transfer must not be spendable by rotate_config"
    );
    let ra = r
        .rotate_args(&next, &rot_approvals)
        .expect("the honest rotation");
    assert_eq!(ra.new_config_hash, next.config_hash());
    assert_eq!(ra.new_threshold, 4);
    assert_eq!(ra.tiers, encode_tier_table(&table));
    assert_eq!(ra.new_tiers, encode_tier_table(&[]));
    assert_eq!(ra.approval_nullifiers.len(), 3);
}
