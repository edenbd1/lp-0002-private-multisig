//! The SDK must derive byte-identical commitments to what the on-chain program
//! re-derives. If the two ever diverge, an integrator builds arguments the chain
//! rejects — and the failure surfaces minutes later as an opaque on-chain error
//! rather than here.
//!
//! Added by a third audit pass: the SDK had no test of its own, so nothing
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
        compute_config_hash(&core_root, 3),
        "SDK config_hash != core"
    );
    let action = b"transfer 100 to the treasury";
    let p = Proposal::new(&ms, [0x11; 32], action);
    let ah = compute_action_hash(&[0xA0; 32], action);
    assert_eq!(p.action_hash(), ah, "SDK action_hash != core");
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
}
