//! Shared primitives for LP-0002, a private M-of-N multisig on the Logos
//! Execution Zone.
//!
//! WHAT THIS PROVES, AND WHY EACH BINDING IS HERE
//!
//! A multisig owner commits to a member set on chain as a single Merkle root,
//! together with a threshold M. A member later approves a proposal by proving,
//! in zero knowledge, that they hold an entry in that set — without revealing
//! which one, to on-chain observers *or to the other members*. When M distinct
//! approvals exist, anyone can execute the proposal, and the execution is
//! unlinkable to any individual member's shielded account.
//!
//! The design is built so that none of the seven ways this could be cheated is
//! left open. Each is enforced in-circuit by [`approve`], or on chain by the
//! verifier program's PDA-anchoring checks.
//!
//!   1. **Membership is against an anchored root.** The multisig account is a
//!      PDA whose address derives from `[multisig_id, config_hash]`, and
//!      `config_hash` commits to `(member_root, threshold)`. Only
//!      `create_multisig` initialises that address. A prover who invents a
//!      one-leaf tree containing themselves names a different `member_root`,
//!      hence a different `config_hash`, hence a PDA that was never initialised
//!      and whose owner is the default — so the approval is rejected on chain.
//!
//!   2. **The threshold cannot be lowered.** `threshold` is inside
//!      `config_hash`, so it is anchored by the same address that anchors the
//!      member set. An executor who supplies `threshold = 1` for a 3-of-5
//!      multisig computes a different `config_hash`, lands on an uninitialised
//!      PDA, and is rejected. There is no code path that reads a threshold from
//!      caller-supplied data.
//!
//!   3. **Approvals are bound to the exact action.** A proposal is identified by
//!      `proposal_ref = H(multisig_id || proposal_id || action_hash)`, and the
//!      nullifier and approval-marker seeds are both derived from it. Approving
//!      proposal `P` carrying action `A` produces markers that are worthless for
//!      the same `P` carrying action `B`. You cannot gather signatures for a
//!      harmless action and execute a different one.
//!
//!   4. **Double-approval is caught by a secret-bound nullifier.** The nullifier
//!      is `H(prefix || proposal_ref || msk)`, a function of the member's secret
//!      key. It is deterministic per `(proposal, member)`, so a second approval
//!      reuses the marker PDA and fails; and it is unlinkable to any member
//!      because an observer who knows the entire candidate member set still
//!      cannot compute it without the secret.
//!
//!   5. **Approvals do not cross multisigs or proposals.** `proposal_ref`
//!      carries `multisig_id`, so a marker earned in one multisig or on one
//!      proposal is not valid anywhere else.
//!
//!   6. **The public key ties the secret to the committed leaf.** The circuit
//!      proves `npk = H(prefix || msk)` and
//!      `account_id = derive_account_id(npk, identifier)`, so the nullifier's
//!      secret is the same secret that owns the committed member entry. Without
//!      this, a prover could pair someone else's leaf with their own nullifier.
//!
//!   7. **Distinct approvals are genuinely distinct members.** Each approval
//!      occupies a PDA seeded by its nullifier, and the nullifier is a function
//!      of the member's secret. Two markers at different addresses therefore
//!      come from two different secrets. `execute` requires M *pairwise
//!      distinct* marker addresses, which is exactly M distinct members.
//!
//! WHAT IS DELIBERATELY NOT HIDDEN
//!
//! `N` (the member-set size) and `M` (the threshold) are public: both are
//! committed on chain at creation. The *number* of approvals gathered so far is
//! public, because each is a marker PDA. The proposed action is public. What is
//! private is **which** members approved, and whether any two approvals came
//! from the same person. See `docs/security.md` for the full threat model.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
use alloc::vec::Vec;
use sha2::{Digest, Sha256};

/// The on-chain account layouts and their decoders. See `docs/account-layout.md`.
#[cfg(feature = "records")]
pub mod state;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Domain separators
//
// Every separator is exactly 32 bytes: an ASCII tag zero-padded to the right.
// Fixed width means the hash inputs below are unambiguous without an explicit
// length prefix, and the distinct tags mean no digest computed for one purpose
// can ever be reinterpreted as another.
// ---------------------------------------------------------------------------

/// 32-byte domain separator for LEZ private account ids.
/// Byte-identical to the LEZ derivation (`nssa/core/src/nullifier.rs`), reused so
/// a member's multisig account is the same account the rest of LEZ sees.
/// ASCII `"/LEE/v0.3/AccountId/Private/"` (28 bytes) + 4 zero bytes.
pub const PRIVATE_ACCOUNT_ID_PREFIX: [u8; 32] = [
    b'/', b'L', b'E', b'E', b'/', b'v', b'0', b'.', b'3', b'/', b'A', b'c', b'c', b'o', b'u', b'n',
    b't', b'I', b'd', b'/', b'P', b'r', b'i', b'v', b'a', b't', b'e', b'/', 0, 0, 0, 0,
];

/// Derives the public nullifier key from the secret one.
/// `npk = SHA256(NPK_DERIVE_PREFIX || msk)`.
/// ASCII `"/lp-0002/v0.1/npk-from-msk/"` (27 bytes) + 5 zero bytes.
pub const NPK_DERIVE_PREFIX: [u8; 32] = [
    b'/', b'l', b'p', b'-', b'0', b'0', b'0', b'2', b'/', b'v', b'0', b'.', b'1', b'/', b'n', b'p',
    b'k', b'-', b'f', b'r', b'o', b'm', b'-', b'm', b's', b'k', b'/', 0, 0, 0, 0, 0,
];

/// A member-set leaf.
/// ASCII `"/lp-0002/v0.1/MemberLeaf/"` (25 bytes) + 7 zero bytes.
pub const MEMBER_LEAF_PREFIX: [u8; 32] = [
    b'/', b'l', b'p', b'-', b'0', b'0', b'0', b'2', b'/', b'v', b'0', b'.', b'1', b'/', b'M', b'e',
    b'm', b'b', b'e', b'r', b'L', b'e', b'a', b'f', b'/', 0, 0, 0, 0, 0, 0, 0,
];

/// The multisig configuration commitment, binding the member set to its
/// threshold so neither can be swapped without changing the on-chain address.
/// ASCII `"/lp-0002/v0.1/MsigConfig/"` (25 bytes) + 7 zero bytes.
pub const MULTISIG_CONFIG_PREFIX: [u8; 32] = [
    b'/', b'l', b'p', b'-', b'0', b'0', b'0', b'2', b'/', b'v', b'0', b'.', b'1', b'/', b'M', b's',
    b'i', b'g', b'C', b'o', b'n', b'f', b'i', b'g', b'/', 0, 0, 0, 0, 0, 0, 0,
];

/// The action commitment for a proposal.
/// ASCII `"/lp-0002/v0.1/Action/"` (21 bytes) + 11 zero bytes.
pub const ACTION_PREFIX: [u8; 32] = [
    b'/', b'l', b'p', b'-', b'0', b'0', b'0', b'2', b'/', b'v', b'0', b'.', b'1', b'/', b'A', b'c',
    b't', b'i', b'o', b'n', b'/', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// The proposal reference: the single value every approval is scoped to.
/// ASCII `"/lp-0002/v0.1/ProposalRef/"` (26 bytes) + 6 zero bytes.
pub const PROPOSAL_REF_PREFIX: [u8; 32] = [
    b'/', b'l', b'p', b'-', b'0', b'0', b'0', b'2', b'/', b'v', b'0', b'.', b'1', b'/', b'P', b'r',
    b'o', b'p', b'o', b's', b'a', b'l', b'R', b'e', b'f', b'/', 0, 0, 0, 0, 0, 0,
];

/// An approval nullifier, bound to the member's secret and the proposal.
/// ASCII `"/lp-0002/v0.1/ApprovalNul/"` (26 bytes) + 6 zero bytes.
pub const APPROVAL_NULLIFIER_PREFIX: [u8; 32] = [
    b'/', b'l', b'p', b'-', b'0', b'0', b'0', b'2', b'/', b'v', b'0', b'.', b'1', b'/', b'A', b'p',
    b'p', b'r', b'o', b'v', b'a', b'l', b'N', b'u', b'l', b'/', 0, 0, 0, 0, 0, 0,
];

/// The on-chain approval-marker PDA seed.
/// ASCII `"/lp-0002/v0.1/ApprovalMark/"` (27 bytes) + 5 zero bytes.
pub const APPROVAL_MARKER_PREFIX: [u8; 32] = [
    b'/', b'l', b'p', b'-', b'0', b'0', b'0', b'2', b'/', b'v', b'0', b'.', b'1', b'/', b'A', b'p',
    b'p', b'r', b'o', b'v', b'a', b'l', b'M', b'a', b'r', b'k', b'/', 0, 0, 0, 0, 0,
];

/// The on-chain execution-marker PDA seed, claimed once per executed proposal.
/// ASCII `"/lp-0002/v0.1/ExecMark/"` (23 bytes) + 9 zero bytes.
pub const EXECUTION_MARKER_PREFIX: [u8; 32] = [
    b'/', b'l', b'p', b'-', b'0', b'0', b'0', b'2', b'/', b'v', b'0', b'.', b'1', b'/', b'E', b'x',
    b'e', b'c', b'M', b'a', b'r', b'k', b'/', 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// The human-readable memo folded into an action.
/// ASCII `"/lp-0002/v0.1/ActionMemo/"` (25 bytes) + 7 zero bytes.
#[cfg(feature = "records")]
pub const ACTION_MEMO_PREFIX: [u8; 32] = [
    b'/', b'l', b'p', b'-', b'0', b'0', b'0', b'2', b'/', b'v', b'0', b'.', b'1', b'/', b'A', b'c',
    b't', b'i', b'o', b'n', b'M', b'e', b'm', b'o', b'/', 0, 0, 0, 0, 0, 0, 0,
];

/// Sentinel used to pad a member set up to a power of two. Domain-separated so
/// it cannot collide with any real leaf: it commits to no account.
#[must_use]
pub fn padding_leaf() -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(MEMBER_LEAF_PREFIX);
    h.update(b"PAD");
    h.finalize().into()
}

// ---------------------------------------------------------------------------
// Reused LEZ primitives (byte-identical to upstream LEZ)
// ---------------------------------------------------------------------------

/// Re-derive a LEZ regular private-account id from `(npk, identifier)`.
#[must_use]
pub fn derive_account_id(npk: &[u8; 32], identifier: u128) -> [u8; 32] {
    let mut bytes = [0u8; 80];
    bytes[0..32].copy_from_slice(&PRIVATE_ACCOUNT_ID_PREFIX);
    bytes[32..64].copy_from_slice(npk);
    bytes[64..80].copy_from_slice(&identifier.to_le_bytes());
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().into()
}

/// Derive the public nullifier key from the secret one. The member keeps `msk`
/// private; the member-set leaf commits to the account derived from it.
#[must_use]
pub fn derive_npk(msk: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(NPK_DERIVE_PREFIX);
    h.update(msk);
    h.finalize().into()
}

/// Fold a Merkle path from a leaf to its root.
/// `hash_two(L, R) = SHA256(L || R)`, LEZ ordering by the index bit.
#[must_use]
pub fn fold_merkle_path(leaf: &[u8; 32], leaf_index: u64, siblings: &[[u8; 32]]) -> [u8; 32] {
    let mut node = *leaf;
    let mut idx = leaf_index;
    for sib in siblings {
        let mut h = Sha256::new();
        if idx & 1 == 0 {
            h.update(node);
            h.update(sib);
        } else {
            h.update(sib);
            h.update(node);
        }
        node = h.finalize().into();
        idx >>= 1;
    }
    node
}

// ---------------------------------------------------------------------------
// LP-0002 primitives
// ---------------------------------------------------------------------------

/// One member's committed entry in the member set.
///
/// `leaf = SHA256(MEMBER_LEAF_PREFIX || account_id || salt)`
///
/// `salt` is chosen per entry by the multisig creator. It matters for a reason
/// specific to small sets: without it, an observer who guesses a candidate
/// account can recompute that account's leaf and test it against the published
/// tree, learning membership. With a secret per-entry salt the leaf is
/// unguessable, so the member set itself stays hidden behind the root.
#[must_use]
pub fn compute_member_leaf(account_id: &[u8; 32], salt: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(MEMBER_LEAF_PREFIX);
    h.update(account_id);
    h.update(salt);
    h.finalize().into()
}

/// The multisig configuration commitment.
///
/// `config_hash = SHA256(MULTISIG_CONFIG_PREFIX || member_root || threshold_le)`
///
/// This is what makes binding 2 work. The on-chain multisig PDA is seeded by
/// `[multisig_id, config_hash]`, so the member set *and* the threshold are both
/// fixed by the account's address. Neither can be changed without landing on a
/// different, uninitialised address.
#[must_use]
pub fn compute_config_hash(member_root: &[u8; 32], threshold: u32) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(MULTISIG_CONFIG_PREFIX);
    h.update(member_root);
    h.update(threshold.to_le_bytes());
    h.finalize().into()
}

/// The action commitment for a proposal.
///
/// `action_hash = SHA256(ACTION_PREFIX || multisig_id || action)`
///
/// `action` is the opaque, caller-defined payload describing what executing the
/// proposal should do (a transfer, a parameter change, a program call). This
/// crate does not interpret it; it only guarantees that approvals are bound to
/// exactly these bytes.
#[must_use]
pub fn compute_action_hash(multisig_id: &[u8; 32], action: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(ACTION_PREFIX);
    h.update(multisig_id);
    h.update(action);
    h.finalize().into()
}

// ---------------------------------------------------------------------------
// The action, given a shape
//
// `action` used to be opaque bytes: the protocol bound them by hash and never
// looked inside. That is enough to make approvals unforgeable, and not enough to
// make an execution *do* anything — the chain had no way to know what the
// approved action was, so the threshold gated a marker and nothing else.
//
// A v1 action is therefore a fixed 81-byte record. It is behind the `records`
// feature, off for the membership guest, which has no business knowing what an
// action is — see the note in Cargo.toml.
//
// A v1 action is a fixed 81-byte record. `create_proposal` checks that
// the record it is handed hashes to the `action_hash` the proposal's address
// commits to, and stores it; `execute` reads it back, re-derives the hash, and
// pays it out. The memo keeps the human sentence bound without putting arbitrary
// length inside the record.
// ---------------------------------------------------------------------------

/// The only action format this program understands. A leading version byte
/// rather than a type tag: there is one shape, and a future second shape must
/// announce itself rather than be inferred from a length.
#[cfg(feature = "records")]
pub const ACTION_FORMAT_V1: u8 = 1;

/// Length of a v1 encoded action: `format(1) ‖ recipient(32) ‖ amount_le(16) ‖ memo_hash(32)`.
#[cfg(feature = "records")]
pub const ACTION_ENCODED_LEN: usize = 81;

/// The canonical bytes of a v1 action.
///
/// `format(1) ‖ recipient(32) ‖ amount_le(16) ‖ memo_hash(32)`
///
/// Fixed width and little-endian throughout, so two clients that agree on the
/// fields cannot disagree on the digest.
#[cfg(feature = "records")]
#[must_use]
pub fn encode_action(
    recipient: &[u8; 32],
    amount: u128,
    memo_hash: &[u8; 32],
) -> [u8; ACTION_ENCODED_LEN] {
    let mut out = [0u8; ACTION_ENCODED_LEN];
    out[0] = ACTION_FORMAT_V1;
    out[1..33].copy_from_slice(recipient);
    out[33..49].copy_from_slice(&amount.to_le_bytes());
    out[49..81].copy_from_slice(memo_hash);
    out
}

/// The commitment to an action's human-readable memo.
///
/// `memo_hash = SHA256(ACTION_MEMO_PREFIX || memo)`
///
/// The memo is what a member reads before approving — "pay the auditors" — and
/// it is bound as tightly as the recipient and the amount are, without letting
/// an arbitrary-length string into the fixed record.
#[cfg(feature = "records")]
#[must_use]
pub fn compute_memo_hash(memo: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(ACTION_MEMO_PREFIX);
    h.update(memo);
    h.finalize().into()
}

/// The action hash of a v1 treasury transfer, from its fields.
///
/// Equivalent to `compute_action_hash(multisig_id, &encode_action(..))`, spelled
/// out because every caller — the CLI, the SDK, the tests and the on-chain
/// program — must agree on it exactly.
#[cfg(feature = "records")]
#[must_use]
pub fn compute_transfer_action_hash(
    multisig_id: &[u8; 32],
    recipient: &[u8; 32],
    amount: u128,
    memo_hash: &[u8; 32],
) -> [u8; 32] {
    compute_action_hash(multisig_id, &encode_action(recipient, amount, memo_hash))
}

/// The proposal reference every approval is scoped to.
///
/// `proposal_ref = SHA256(PROPOSAL_REF_PREFIX || multisig_id || proposal_id
///                        || action_hash)`
///
/// Folding `action_hash` in here is binding 3, and it closes a real attack. If
/// approvals were scoped to `proposal_id` alone, a proposer could publish a
/// harmless action, collect M approvals, then publish a second proposal under
/// the same id with a malicious action — and the approvals already gathered
/// would count for it. Because `proposal_ref` changes with the action, the two
/// are different proposals with disjoint approval markers.
///
/// It also removes the mirror-image griefing vector: re-using an id with a
/// junk action cannot burn the markers of the real proposal, because they live
/// at different addresses.
#[must_use]
pub fn compute_proposal_ref(
    multisig_id: &[u8; 32],
    config_hash: &[u8; 32],
    proposal_id: &[u8; 32],
    action_hash: &[u8; 32],
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(PROPOSAL_REF_PREFIX);
    h.update(multisig_id);
    h.update(config_hash);
    h.update(proposal_id);
    h.update(action_hash);
    h.finalize().into()
}

/// The approval nullifier, bound to the member's secret and the proposal.
///
/// `nullifier = SHA256(APPROVAL_NULLIFIER_PREFIX || proposal_ref || msk)`
///
/// Deterministic per `(proposal, member)` so a second approval collides on the
/// same marker PDA and fails, and unlinkable because it depends on `msk`, which
/// an observer who knows the entire candidate member set still does not have.
#[must_use]
pub fn compute_approval_nullifier(proposal_ref: &[u8; 32], msk: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(APPROVAL_NULLIFIER_PREFIX);
    h.update(proposal_ref);
    h.update(msk);
    h.finalize().into()
}

/// The on-chain approval-marker PDA seed.
///
/// `seed = SHA256(APPROVAL_MARKER_PREFIX || proposal_ref || nullifier)`
#[must_use]
pub fn compute_approval_marker(proposal_ref: &[u8; 32], nullifier: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(APPROVAL_MARKER_PREFIX);
    h.update(proposal_ref);
    h.update(nullifier);
    h.finalize().into()
}

/// The on-chain execution-marker PDA seed, claimed once per executed proposal.
///
/// `seed = SHA256(EXECUTION_MARKER_PREFIX || proposal_ref)`
///
/// Its existence, owned by the verifier program, is the proof-of-execution that
/// a downstream integration consumes. Because `init` refuses to overwrite, it is
/// also the replay guard: a proposal executes at most once.
#[must_use]
pub fn compute_execution_marker(proposal_ref: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(EXECUTION_MARKER_PREFIX);
    h.update(proposal_ref);
    h.finalize().into()
}

/// Private inputs to an approval proof. None of these reach the journal, and on
/// the privacy-preserving transaction path none of them reach the chain either.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApproveWitness {
    /// The member's secret nullifier key.
    pub msk: [u8; 32],
    /// LEZ per-account identifier.
    pub identifier: u128,
    /// Per-entry salt the multisig creator used when building this leaf.
    pub salt: [u8; 32],
    /// Merkle siblings from the leaf up to (but excluding) the root.
    pub merkle_path: Vec<[u8; 32]>,
    /// 0-indexed leaf position in the member set.
    pub leaf_index: u64,
}

/// The public statement an approval proof establishes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApproveStatement {
    /// The committed member-set root. Anchored on chain through `config_hash`.
    pub member_root: [u8; 32],
    /// The proposal this approval is scoped to, itself binding the multisig,
    /// the proposal id, and the exact action.
    pub proposal_ref: [u8; 32],
    /// The nullifier that prevents a second approval by the same member.
    pub nullifier: [u8; 32],
}

/// The instruction an approval proof runs over: the private witness plus the
/// public statement it must satisfy. The LEZ-native guest reads this, calls
/// [`approve`], and emits a `ProgramOutput` the privacy circuit composes with
/// `env::verify`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApproveInstruction {
    pub witness: ApproveWitness,
    pub statement: ApproveStatement,
}

/// Errors an approval proof can fail with, mapped to on-chain codes by the
/// verifier program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApproveError {
    /// The member's account is not in the committed member set.
    NotAMember,
    /// The supplied nullifier is not the one this witness yields.
    NullifierMismatch,
}

/// The in-circuit approval logic. Returns the member leaf on success, or an
/// [`ApproveError`] naming exactly which binding failed. The guest calls this
/// and emits its `ProgramOutput` only if it returns `Ok`.
///
/// The shape is deliberate: a witness plus a public statement it must satisfy,
/// which is the vocabulary an auditor already reads elsewhere in this codebase.
pub fn approve(
    witness: &ApproveWitness,
    statement: &ApproveStatement,
) -> Result<[u8; 32], ApproveError> {
    // Tie the secret to the committed public account (binding 6).
    let npk = derive_npk(&witness.msk);
    let account_id = derive_account_id(&npk, witness.identifier);

    // Membership against the claimed root (binding 1). The verifier program
    // then checks this root is the one anchored in the multisig PDA address.
    let leaf = compute_member_leaf(&account_id, &witness.salt);
    let recovered = fold_merkle_path(&leaf, witness.leaf_index, &witness.merkle_path);
    if recovered != statement.member_root {
        return Err(ApproveError::NotAMember);
    }

    // Double-approval nullifier, bound to the secret and the proposal
    // (bindings 4 and 5; `proposal_ref` carries the multisig and the action).
    let nullifier = compute_approval_nullifier(&statement.proposal_ref, &witness.msk);
    if nullifier != statement.nullifier {
        return Err(ApproveError::NullifierMismatch);
    }

    Ok(leaf)
}

/// A member's Merkle authentication path: their 0-indexed leaf position and the
/// sibling hashes from the leaf up to (but excluding) the root.
pub type MerklePath = (u64, Vec<[u8; 32]>);

/// Creator-side helper: build a Merkle tree over the member leaves and return
/// the root plus, for each leaf, its `(leaf_index, siblings)` path. Pads to a
/// power of two with [`padding_leaf`] so paths are well-formed. Host only,
/// because a creator builds the set off chain and publishes just the root.
#[cfg(feature = "std")]
#[must_use]
pub fn build_member_tree(leaves: &[[u8; 32]]) -> ([u8; 32], Vec<MerklePath>) {
    assert!(!leaves.is_empty(), "a member set needs at least one member");

    let sentinel = padding_leaf();
    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    while level.len() < 2 || (level.len() & (level.len() - 1)) != 0 {
        level.push(sentinel);
    }
    let width = level.len();

    let mut paths: Vec<MerklePath> = (0..leaves.len()).map(|i| (i as u64, Vec::new())).collect();

    let mut nodes = level;
    let mut idx_width = width;
    while idx_width > 1 {
        let mut next = Vec::with_capacity(idx_width / 2);
        for pair in 0..idx_width / 2 {
            let l = nodes[2 * pair];
            let r = nodes[2 * pair + 1];
            for (leaf_i, path) in paths.iter_mut() {
                let pos_at_level = (*leaf_i as usize) >> path.len();
                if pos_at_level == 2 * pair {
                    path.push(r);
                } else if pos_at_level == 2 * pair + 1 {
                    path.push(l);
                }
            }
            let mut h = Sha256::new();
            h.update(l);
            h.update(r);
            next.push(h.finalize().into());
        }
        nodes = next;
        idx_width /= 2;
    }

    (nodes[0], paths)
}
