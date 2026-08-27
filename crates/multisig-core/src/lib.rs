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
//!      `config_hash` commits to `(member_root, threshold, tiers_hash)`. Only
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

#[cfg(feature = "records")]
/// The spending-tier commitment, folded into `config_hash` so a tier table is
/// anchored by the multisig's address exactly as the member set and the default
/// threshold are. A caller who invents a tier that needs one approval for a
/// large transfer computes a different tiers hash, hence a different config
/// hash, hence a PDA nobody created.
/// ASCII `"/lp-0002/v0.1/TierPolicy/"` (25 bytes) + 7 zero bytes.
pub const TIER_POLICY_PREFIX: [u8; 32] = [
    b'/', b'l', b'p', b'-', b'0', b'0', b'0', b'2', b'/', b'v', b'0', b'.', b'1', b'/', b'T', b'i',
    b'e', b'r', b'P', b'o', b'l', b'i', b'c', b'y', b'/', 0, 0, 0, 0, 0, 0, 0,
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
/// `config_hash = SHA256(MULTISIG_CONFIG_PREFIX || member_root || threshold_le
///                        || tiers_hash)`
///
/// This is what makes binding 2 work. The on-chain multisig PDA is seeded by
/// `[multisig_id, config_hash]`, so the member set *and* the threshold are both
/// fixed by the account's address. Neither can be changed without landing on a
/// different, uninitialised address.
#[must_use]
pub fn compute_config_hash(
    member_root: &[u8; 32],
    threshold: u32,
    tiers_hash: &[u8; 32],
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(MULTISIG_CONFIG_PREFIX);
    h.update(member_root);
    h.update(threshold.to_le_bytes());
    h.update(tiers_hash);
    h.finalize().into()
}

#[cfg(feature = "records")]
/// One spending tier: transfers of at most `max_amount` need `threshold`
/// approvals instead of the default.
///
/// Tiers exist to make small payments cheap to authorise without weakening
/// large ones, so the table is constrained rather than free-form — see
/// [`validate_tiers`]. A table that does not satisfy those rules has no
/// canonical hash and therefore no anchored configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TierPolicy {
    /// Inclusive upper bound on the transfer amount this tier covers.
    pub max_amount: u128,
    /// Approvals required for amounts at or below `max_amount`.
    pub threshold: u32,
}

#[cfg(feature = "records")]
/// Why a tier table is not a legal configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TierError {
    /// Caps must strictly increase, so exactly one tier covers each amount.
    CapsNotStrictlyIncreasing,
    /// A larger amount must never need fewer approvals than a smaller one.
    ThresholdDecreases,
    /// A tier of zero would let one caller move money alone.
    ThresholdZero,
    /// A tier may lower the requirement for small amounts. It may not raise it
    /// above the default, because the default is what the address anchors and
    /// what governance runs at.
    ThresholdAboveDefault,
    /// More tiers than the record can hold.
    TooManyTiers,
    /// The encoded table is not a whole number of fixed-width entries.
    MalformedTable,
}

#[cfg(feature = "records")]
/// The largest tier table a configuration may carry.
pub const MAX_TIERS: usize = 8;

#[cfg(feature = "records")]
/// Checks the rules that make a tier table safe to anchor.
///
/// The rules are monotonicity rules, and together they say one thing: tiers may
/// only ever *relax* the requirement, and only for amounts below a cap. There is
/// no table that makes a large transfer easier than the default, which is the
/// attack a free-form table would invite.
///
/// # Errors
/// See [`TierError`].
pub fn validate_tiers(tiers: &[TierPolicy], default_threshold: u32) -> Result<(), TierError> {
    if tiers.len() > MAX_TIERS {
        return Err(TierError::TooManyTiers);
    }
    let mut previous_cap: Option<u128> = None;
    let mut previous_threshold = 0u32;
    for tier in tiers {
        if tier.threshold == 0 {
            return Err(TierError::ThresholdZero);
        }
        if tier.threshold > default_threshold {
            return Err(TierError::ThresholdAboveDefault);
        }
        if tier.threshold < previous_threshold {
            return Err(TierError::ThresholdDecreases);
        }
        if let Some(cap) = previous_cap {
            if tier.max_amount <= cap {
                return Err(TierError::CapsNotStrictlyIncreasing);
            }
        }
        previous_cap = Some(tier.max_amount);
        previous_threshold = tier.threshold;
    }
    Ok(())
}

// These eight lines hold a line count, and that is worth stating plainly rather
// than leaving as a puzzle. RISC0 derives a guest's ImageID from its memory
// image, and this crate is linked into the verifier guest, so *where* code sits
// in this file changes the program's on-chain identity. The deployed verifier
// was built with an orphaned doc block here — eight lines that had drifted onto
// the wrong item, carrying a formula the encoding had outgrown. Deleting the
// text was right; deleting the lines would have stopped a clean rebuild from
// reproducing the deployed ImageID. Remove these at the next redeploy, not now.
/// One encoded tier: `max_amount` little-endian, then `threshold`.
#[cfg(feature = "records")]
pub const TIER_ENTRY_LEN: usize = 16 + 4;

/// The tier table as bytes — the wire form, and the preimage of its hash.
///
/// There is deliberately one encoding rather than two. The instruction carries
/// these exact bytes and [`compute_tiers_hash`] hashes these exact bytes, so a
/// table cannot be serialised one way for the chain and another way for the
/// commitment that anchors it. It is also the only form the SPEL CLI can carry:
/// a `Vec<(u128, u32)>` has no representation in the IDL — it types as
/// `{"vec": "unknown"}` — and the CLI's serialiser has no case for it, so a
/// tuple-shaped tier table would compile, pass every test that speaks Rust, and
/// be unreachable from the command line that drives the deployment.
///
/// Layout: `count(1) ‖ (max_amount_le(16) ‖ threshold_le(4)) * count`. The two
/// must agree, and a buffer that disagrees is refused rather than truncated.
#[cfg(feature = "records")]
#[must_use]
pub fn encode_tier_table(tiers: &[TierPolicy]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + tiers.len() * TIER_ENTRY_LEN);
    // The leading count is not redundant with the length, and it is not there
    // for parsing. It is there so the encoding of *no tiers* is one byte rather
    // than zero: the SPEL CLI carries a `Vec<u8>` as comma-separated decimals
    // and has no case for an empty one — `--tiers ""` parses as `Raw("")` and
    // fails to serialise, while omitting the flag is a missing-argument error.
    // A multisig with no tiers is the ordinary case, so an encoding it cannot
    // express is an encoding that does not work. Measured against the vendored
    // `spel --dry-run`, not assumed.
    out.push(tiers.len() as u8);
    for tier in tiers {
        out.extend_from_slice(&tier.max_amount.to_le_bytes());
        out.extend_from_slice(&tier.threshold.to_le_bytes());
    }
    out
}

/// Read a tier table back.
///
/// # Errors
/// [`TierError::MalformedTable`] if the buffer is empty or its length disagrees
/// with the count it declares, [`TierError::TooManyTiers`] beyond [`MAX_TIERS`].
/// This does *not*
/// check monotonicity — call [`validate_tiers`] for that, which needs the
/// default threshold this table sits under.
#[cfg(feature = "records")]
pub fn decode_tier_table(bytes: &[u8]) -> Result<Vec<TierPolicy>, TierError> {
    let Some((&count_byte, entries)) = bytes.split_first() else {
        return Err(TierError::MalformedTable);
    };
    let count = count_byte as usize;
    if count > MAX_TIERS {
        return Err(TierError::TooManyTiers);
    }
    // The declared count and the actual length must agree. A table that says
    // three and carries two is refused rather than truncated to what arrived.
    if entries.len() != count * TIER_ENTRY_LEN {
        return Err(TierError::MalformedTable);
    }
    let mut out = Vec::with_capacity(count);
    for chunk in entries.chunks_exact(TIER_ENTRY_LEN) {
        let mut amount = [0u8; 16];
        amount.copy_from_slice(&chunk[..16]);
        let mut threshold = [0u8; 4];
        threshold.copy_from_slice(&chunk[16..]);
        out.push(TierPolicy {
            max_amount: u128::from_le_bytes(amount),
            threshold: u32::from_le_bytes(threshold),
        });
    }
    Ok(out)
}

/// `SHA256(TIER_POLICY_PREFIX ‖ encode_tier_table(tiers))`.
#[cfg(feature = "records")]
#[must_use]
pub fn compute_tiers_hash(tiers: &[TierPolicy]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(TIER_POLICY_PREFIX);
    h.update(encode_tier_table(tiers));
    h.finalize().into()
}

#[cfg(feature = "records")]
/// The tiers hash of a multisig that has none. Spelled out so "no tiers" is a
/// value every party computes the same way rather than a zero somebody invents.
#[must_use]
pub fn no_tiers_hash() -> [u8; 32] {
    compute_tiers_hash(&[])
}

#[cfg(feature = "records")]
/// How many approvals a transfer of `amount` needs.
///
/// The first tier whose cap covers the amount decides; above every cap the
/// default applies. Callers must have validated the table with
/// [`validate_tiers`] — an unvalidated table cannot reach here through an
/// anchored configuration, because its hash would not match one.
#[must_use]
pub fn required_threshold(amount: u128, default_threshold: u32, tiers: &[TierPolicy]) -> u32 {
    for tier in tiers {
        if amount <= tier.max_amount {
            return tier.threshold;
        }
    }
    default_threshold
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

/// A governance action: replace this multisig's configuration with another.
///
/// `format(1) ‖ new_config_hash(32)` — 33 bytes.
///
/// **Why a rotation is not a mutation.** The configuration lives in the
/// multisig's *address*: the PDA is seeded by `[multisig_id, config_hash]`. A
/// rotation therefore does not edit anything — it anchors a second
/// configuration at its own address and records, in the first, that it has been
/// superseded. Every property the address gives the old configuration, the new
/// one has by the same construction: a member set or threshold nobody approved
/// still lands on a PDA nobody created.
///
/// It also means a proposal cannot outlive the configuration it was made under.
/// `proposal_ref` already carries `config_hash`, so proposals of the old
/// configuration live at addresses the new one never reads. There is no stale
/// proposal to detect, because there is no shared mutable state to go stale.
#[cfg(feature = "records")]
pub const ACTION_FORMAT_V2_ROTATE: u8 = 2;

/// Length of a v2 rotate action: `format(1) ‖ new_config_hash(32)`.
#[cfg(feature = "records")]
pub const ROTATE_ACTION_ENCODED_LEN: usize = 33;

/// The canonical bytes of a rotation action.
#[cfg(feature = "records")]
#[must_use]
pub fn encode_rotate_action(new_config_hash: &[u8; 32]) -> [u8; ROTATE_ACTION_ENCODED_LEN] {
    let mut out = [0u8; ROTATE_ACTION_ENCODED_LEN];
    out[0] = ACTION_FORMAT_V2_ROTATE;
    out[1..33].copy_from_slice(new_config_hash);
    out
}

/// The action hash of a rotation, from the configuration it moves to.
#[cfg(feature = "records")]
#[must_use]
pub fn compute_rotate_action_hash(multisig_id: &[u8; 32], new_config_hash: &[u8; 32]) -> [u8; 32] {
    // Exactly three lines, and they are load-bearing. rustfmt folds the
    // signature above onto one line — three shorter than when the deployed
    // verifier was built. See the note above `TIER_ENTRY_LEN` for why that matters.
    compute_action_hash(multisig_id, &encode_rotate_action(new_config_hash))
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
/// `proposal_ref = SHA256(PROPOSAL_REF_PREFIX || multisig_id || config_hash
///                        || proposal_id || action_hash)`
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

#[cfg(all(test, feature = "records"))]
mod tier_wire_tests {
    use super::*;

    fn tiers(pairs: &[(u128, u32)]) -> Vec<TierPolicy> {
        pairs
            .iter()
            .map(|&(max_amount, threshold)| TierPolicy {
                max_amount,
                threshold,
            })
            .collect()
    }

    /// The encoding is the wire format *and* the preimage of `tiers_hash`, so a
    /// round trip that loses anything would let the table a program applies
    /// differ from the table its address commits to.
    #[test]
    fn a_tier_table_survives_a_round_trip() {
        for table in [
            vec![],
            vec![(300u128, 2u32)],
            vec![(100, 1), (500, 2), (u128::MAX, 3)],
        ] {
            let t = tiers(&table);
            let encoded = encode_tier_table(&t);
            assert_eq!(encoded.len(), 1 + table.len() * TIER_ENTRY_LEN);
            let decoded = decode_tier_table(&encoded).expect("our own encoding decodes");
            assert_eq!(decoded.len(), t.len());
            for (a, b) in decoded.iter().zip(t.iter()) {
                assert_eq!((a.max_amount, a.threshold), (b.max_amount, b.threshold));
            }
        }
    }

    /// The empty table must still encode to something, because the tool that
    /// carries it cannot express an empty byte vector — `--tiers ""` reaches
    /// SPEL's serialiser as `Raw("")` and fails, and omitting the flag is a
    /// missing-argument error. One byte is what makes "no tiers" sendable.
    #[test]
    fn no_tiers_encodes_to_one_byte_rather_than_none() {
        assert_eq!(encode_tier_table(&[]), vec![0u8]);
    }

    #[test]
    fn an_empty_buffer_is_not_a_tier_table() {
        assert_eq!(decode_tier_table(&[]), Err(TierError::MalformedTable));
    }

    /// A count that disagrees with the bytes present is refused, not truncated
    /// to whatever arrived.
    #[test]
    fn a_declared_count_must_match_the_bytes_present() {
        let mut short = encode_tier_table(&tiers(&[(300, 2)]));
        short[0] = 2;
        assert_eq!(decode_tier_table(&short), Err(TierError::MalformedTable));

        let mut long = encode_tier_table(&tiers(&[(300, 2), (600, 3)]));
        long[0] = 1;
        assert_eq!(decode_tier_table(&long), Err(TierError::MalformedTable));

        let truncated = &encode_tier_table(&tiers(&[(300, 2)]))[..10];
        assert_eq!(decode_tier_table(truncated), Err(TierError::MalformedTable));
    }

    #[test]
    fn more_tiers_than_the_maximum_are_refused() {
        let many: Vec<(u128, u32)> = (1..=(MAX_TIERS as u128 + 1))
            .map(|i| (i * 100, 1u32))
            .collect();
        let encoded = encode_tier_table(&tiers(&many));
        assert_eq!(decode_tier_table(&encoded), Err(TierError::TooManyTiers));
    }

    /// Distinct tables must not collide, and the same table must always give the
    /// same hash — this is the value folded into `config_hash`, so a collision
    /// would be two configurations sharing one address.
    #[test]
    fn the_tiers_hash_separates_tables_that_differ() {
        let a = compute_tiers_hash(&tiers(&[(300, 2)]));
        let b = compute_tiers_hash(&tiers(&[(300, 3)]));
        let c = compute_tiers_hash(&tiers(&[(301, 2)]));
        let empty = no_tiers_hash();
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, empty);
        assert_eq!(a, compute_tiers_hash(&tiers(&[(300, 2)])));
    }

    /// `required_threshold` is what prices a transfer, and the tier table may
    /// only ever lower the bar for amounts at or below a cap.
    #[test]
    fn a_tier_applies_at_its_cap_and_not_past_it() {
        let table = tiers(&[(300, 2), (1000, 3)]);
        assert_eq!(required_threshold(1, 4, &table), 2);
        assert_eq!(
            required_threshold(300, 4, &table),
            2,
            "the cap is inclusive"
        );
        assert_eq!(required_threshold(301, 4, &table), 3);
        assert_eq!(required_threshold(1000, 4, &table), 3);
        assert_eq!(required_threshold(1001, 4, &table), 4, "past every tier");
        assert_eq!(
            required_threshold(u128::MAX, 4, &[]),
            4,
            "no tiers, no change"
        );
    }
}
