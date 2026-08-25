//! `multisig-sdk` — the reusable client library for LP-0002.
//!
//! This is the crate a Logos module depends on. It wraps [`multisig_core`]'s
//! primitives in the vocabulary of the lifecycle — create, propose, approve,
//! execute — and hands back the exact instruction arguments the on-chain
//! program expects. It makes no assumptions about transport: it never opens a
//! socket, never shells out to `spel`, and never touches the filesystem. Give it
//! data, it gives you arguments; you decide how they reach a sequencer.
//!
//! That split is deliberate. `msig` (the CLI) and the Basecamp app are both thin
//! shells over this crate, and a third integrator can be too.
//!
//! # Example
//!
//! ```
//! use multisig_sdk::{MemberSet, Multisig, Proposal};
//!
//! // The creator collects each member's account id and assigns a salt.
//! let members = MemberSet::from_entries(&[
//!     ([1u8; 32], [11u8; 32]),
//!     ([2u8; 32], [12u8; 32]),
//!     ([3u8; 32], [13u8; 32]),
//! ]);
//! let multisig = Multisig::new([0xaa; 32], members.root(), 2).unwrap();
//!
//! // A proposal binds a treasury payment to an id. The memo is the sentence
//! // members read before approving; it is bound by hash like everything else.
//! let alice = [0x5E; 32];
//! let proposal = Proposal::new(&multisig, [0x01; 32], alice, 250, b"pay the auditors");
//!
//! // Every approval on this proposal is scoped to the same reference.
//! assert_eq!(proposal.proposal_ref(), proposal.proposal_ref());
//! ```

#![forbid(unsafe_code)]

use multisig_core::{MerklePath, *};

/// The `literal("treasury")` PDA seed, as SPEL builds it: the ASCII bytes,
/// zero-padded to 32 (`spel_framework::pda::seed_from_str`). Spelled out here
/// rather than computed, so a client can see the exact bytes it will hash.
pub const TREASURY_SEED: [u8; 32] = [
    b't', b'r', b'e', b'a', b's', b'u', b'r', b'y', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
];

/// Errors the SDK returns before anything reaches a sequencer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdkError {
    /// A 0-of-N multisig would let anyone execute.
    ZeroThreshold,
    /// The threshold exceeds the member count, so it could never be met.
    ThresholdExceedsMembers { threshold: u32, members: usize },
    /// The member set is empty.
    EmptyMemberSet,
    /// The supplied secret does not correspond to the member entry given.
    NotAMember,
    /// Fewer approvals than the multisig's anchored threshold.
    ThresholdNotMet { have: usize, need: u32 },
    /// The same member's approval was supplied more than once.
    DuplicateApproval,
    /// A proposal that moves nothing would give the threshold nothing to gate.
    ZeroAmount,
    /// The tier table is one the program would refuse to anchor.
    ///
    /// The SDK applies `multisig_core::validate_tiers` — the same function the
    /// guest applies — rather than a copy of its rules, so the two cannot drift.
    BadTiers(TierError),
    /// A rotation to the configuration already in force changes nothing.
    NoopRotation,
    /// A proposal handed to the instruction for the other action shape.
    WrongActionKind,
}

impl core::fmt::Display for SdkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroThreshold => write!(f, "threshold must be at least 1"),
            Self::ThresholdExceedsMembers { threshold, members } => write!(
                f,
                "threshold {threshold} exceeds the {members} members: it could never be met"
            ),
            Self::EmptyMemberSet => write!(f, "a member set needs at least one member"),
            Self::NotAMember => write!(f, "that secret is not a member of this set"),
            Self::ThresholdNotMet { have, need } => {
                write!(f, "only {have} of {need} approvals gathered")
            }
            Self::DuplicateApproval => write!(f, "the same approval was supplied twice"),
            Self::ZeroAmount => write!(f, "a proposal must move a non-zero amount"),
            Self::BadTiers(e) => match e {
                TierError::CapsNotStrictlyIncreasing => {
                    write!(f, "tier caps must strictly increase")
                }
                TierError::ThresholdDecreases => write!(
                    f,
                    "a larger transfer must never need fewer approvals than a smaller one"
                ),
                TierError::ThresholdZero => write!(f, "a tier must require at least one approval"),
                TierError::ThresholdAboveDefault => write!(
                    f,
                    "a tier may only lower the bar, never raise it above the default threshold"
                ),
                TierError::TooManyTiers => write!(f, "at most {MAX_TIERS} tiers"),
                TierError::MalformedTable => write!(
                    f,
                    "an encoded tier table must be a whole number of {TIER_ENTRY_LEN}-byte entries"
                ),
            },
            Self::NoopRotation => write!(f, "that is the configuration already in force"),
            Self::WrongActionKind => write!(
                f,
                "a transfer is spent by execute and a rotation by rotate_config"
            ),
        }
    }
}

impl std::error::Error for SdkError {}

// ---------------------------------------------------------------------------
// Member set
// ---------------------------------------------------------------------------

/// A committed member set: the leaves, the root, and every member's Merkle path.
///
/// Built once by the multisig creator from `(account_id, salt)` pairs. In a real
/// deployment each member generates their own secret and hands the creator only
/// the derived `account_id` — the creator never needs anyone's `msk`.
#[derive(Debug, Clone)]
pub struct MemberSet {
    leaves: Vec<[u8; 32]>,
    root: [u8; 32],
    paths: Vec<MerklePath>,
    salts: Vec<[u8; 32]>,
}

impl MemberSet {
    /// Build a member set from `(account_id, salt)` pairs, in order.
    ///
    /// Order is part of the commitment: reordering the same members yields a
    /// different root, hence a different multisig.
    #[must_use]
    pub fn from_entries(entries: &[([u8; 32], [u8; 32])]) -> Self {
        assert!(
            !entries.is_empty(),
            "a member set needs at least one member"
        );
        let leaves: Vec<[u8; 32]> = entries
            .iter()
            .map(|(account_id, salt)| compute_member_leaf(account_id, salt))
            .collect();
        let (root, paths) = build_member_tree(&leaves);
        Self {
            leaves,
            root,
            paths,
            salts: entries.iter().map(|(_, s)| *s).collect(),
        }
    }

    /// The committed root.
    #[must_use]
    pub fn root(&self) -> [u8; 32] {
        self.root
    }

    /// Number of members.
    #[must_use]
    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    /// Whether the set is empty. Always false for a set built by this API.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    /// Member `index`'s `(leaf_index, siblings)` Merkle path.
    #[must_use]
    pub fn path(&self, index: usize) -> Option<&MerklePath> {
        self.paths.get(index)
    }

    /// Member `index`'s salt.
    #[must_use]
    pub fn salt(&self, index: usize) -> Option<[u8; 32]> {
        self.salts.get(index).copied()
    }

    /// Locate a member by the secret they hold, returning their index.
    ///
    /// This is how a member-side client finds its own row without holding
    /// anyone else's data beyond the public set.
    #[must_use]
    pub fn find_by_secret(&self, msk: &[u8; 32], identifier: u128) -> Option<usize> {
        let account_id = derive_account_id(&derive_npk(msk), identifier);
        (0..self.leaves.len())
            .find(|&i| compute_member_leaf(&account_id, &self.salts[i]) == self.leaves[i])
    }
}

// ---------------------------------------------------------------------------
// Multisig
// ---------------------------------------------------------------------------

/// A multisig instance: an id, a committed member root, and a threshold.
///
/// The `config_hash` this exposes is what anchors the pair on chain — the
/// multisig account's PDA address derives from `[id, config_hash]`, so neither
/// the member set nor the threshold can be swapped after creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Multisig {
    id: [u8; 32],
    member_root: [u8; 32],
    threshold: u32,
    tiers: Vec<TierPolicy>,
    config_hash: [u8; 32],
}

impl Multisig {
    /// Define a multisig. Rejects a zero threshold outright: the on-chain
    /// program would too, with `E_BAD_THRESHOLD` (5008), minutes later.
    pub fn new(id: [u8; 32], member_root: [u8; 32], threshold: u32) -> Result<Self, SdkError> {
        if threshold == 0 {
            return Err(SdkError::ZeroThreshold);
        }
        Ok(Self {
            id,
            member_root,
            threshold,
            tiers: Vec::new(),
            config_hash: compute_config_hash(&member_root, threshold, &no_tiers_hash()),
        })
    }

    /// Anchor spending tiers: below `max_amount`, `threshold` approvals suffice.
    ///
    /// Validated here with the guest's own `validate_tiers`, so a table this
    /// accepts is a table the chain accepts, and a table it refuses fails now
    /// rather than after a proof.
    ///
    /// # Errors
    /// [`SdkError::BadTiers`] for a table the program would refuse with
    /// `E_BAD_TIERS` (5023).
    pub fn with_tiers(mut self, tiers: &[TierPolicy]) -> Result<Self, SdkError> {
        validate_tiers(tiers, self.threshold).map_err(SdkError::BadTiers)?;
        self.tiers = tiers.to_vec();
        self.config_hash = compute_config_hash(
            &self.member_root,
            self.threshold,
            &compute_tiers_hash(&self.tiers),
        );
        Ok(self)
    }

    /// Define a multisig against a concrete member set, checking the threshold
    /// against the member count as well.
    pub fn for_members(
        id: [u8; 32],
        members: &MemberSet,
        threshold: u32,
    ) -> Result<Self, SdkError> {
        if members.is_empty() {
            return Err(SdkError::EmptyMemberSet);
        }
        if threshold as usize > members.len() {
            return Err(SdkError::ThresholdExceedsMembers {
                threshold,
                members: members.len(),
            });
        }
        Self::new(id, members.root(), threshold)
    }

    #[must_use]
    pub fn id(&self) -> [u8; 32] {
        self.id
    }
    #[must_use]
    pub fn member_root(&self) -> [u8; 32] {
        self.member_root
    }
    #[must_use]
    pub fn threshold(&self) -> u32 {
        self.threshold
    }
    #[must_use]
    pub fn tiers(&self) -> &[TierPolicy] {
        &self.tiers
    }

    /// How many approvals a transfer of `amount` actually needs here.
    ///
    /// The number a client should gather before calling `execute`, which is the
    /// default threshold unless a tier covers the amount.
    #[must_use]
    pub fn required_for(&self, amount: u128) -> u32 {
        required_threshold(amount, self.threshold, &self.tiers)
    }

    /// The commitment anchoring `(member_root, threshold, tiers)` in the PDA
    /// address.
    #[must_use]
    pub fn config_hash(&self) -> [u8; 32] {
        self.config_hash
    }

    /// The configuration this one would rotate into, and the arguments for the
    /// proposal that authorises it.
    ///
    /// A rotation does not mutate this multisig. It defines a second one, at its
    /// own address with its own treasury, and marks this one superseded — so the
    /// value returned here is a `Multisig` like any other, and everything that
    /// works on one works on it.
    ///
    /// # Errors
    /// [`SdkError::NoopRotation`] if the target is the configuration already in
    /// force, which the program refuses with `E_NOOP_ROTATION` (5026); plus
    /// whatever [`Multisig::new`] and [`Multisig::with_tiers`] would refuse.
    pub fn rotation(
        &self,
        new_member_root: [u8; 32],
        new_threshold: u32,
        new_tiers: &[TierPolicy],
    ) -> Result<Multisig, SdkError> {
        let next = Multisig::new(self.id, new_member_root, new_threshold)?.with_tiers(new_tiers)?;
        if next.config_hash == self.config_hash {
            return Err(SdkError::NoopRotation);
        }
        Ok(next)
    }

    /// The PDA seeds of this multisig's treasury, in order.
    ///
    /// The address itself needs the verifier's ProgramId, which this crate
    /// deliberately does not carry — a client derives it from the binary it
    /// intends to call, so the address it computes is the address of the program
    /// it is actually talking to. `scripts/pda.py` does the derivation.
    #[must_use]
    pub fn treasury_seeds(&self) -> [[u8; 32]; 3] {
        [self.id, self.config_hash, TREASURY_SEED]
    }

    /// Arguments for `fund_treasury`.
    ///
    /// # Errors
    /// Returns [`SdkError::ZeroAmount`] for a funding of nothing, which the
    /// on-chain program refuses with `E_BAD_AMOUNT` (5017).
    pub fn fund_args(&self, amount: u128) -> Result<FundTreasuryArgs, SdkError> {
        if amount == 0 {
            return Err(SdkError::ZeroAmount);
        }
        Ok(FundTreasuryArgs {
            multisig_id: self.id,
            config_hash: self.config_hash,
            amount,
        })
    }

    /// Arguments for the `create_multisig` instruction.
    #[must_use]
    pub fn create_args(&self) -> CreateMultisigArgs {
        CreateMultisigArgs {
            multisig_id: self.id,
            config_hash: self.config_hash,
            member_root: self.member_root,
            threshold: self.threshold,
            tiers: encode_tiers(&self.tiers),
        }
    }
}

/// The tier table as the wire carries it.
///
/// The typed [`TierPolicy`] is what a caller reasons about; these bytes are what
/// the instruction takes — and they are also the preimage `config_hash` commits
/// to, so an integrator cannot encode the table one way for the call and another
/// way for the commitment.
#[must_use]
pub fn encode_tiers(tiers: &[TierPolicy]) -> Vec<u8> {
    encode_tier_table(tiers)
}

// ---------------------------------------------------------------------------
// Proposal
// ---------------------------------------------------------------------------

/// A proposal: an action bound to an id under a specific multisig.
///
/// The `proposal_ref` is the single value every approval is scoped to. It
/// commits to the action, so approvals gathered for one action are worthless for
/// another published under the same id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    multisig_id: [u8; 32],
    config_hash: [u8; 32],
    proposal_id: [u8; 32],
    action_hash: [u8; 32],
    proposal_ref: [u8; 32],
    threshold: u32,
    /// The tier table the multisig anchors. Carried because the number of
    /// approvals this proposal needs depends on its amount, and a client that
    /// gathered the default threshold when a tier asked for two would do twice
    /// the proving work for nothing — while one that gathered two under a
    /// proposal no tier covers would be refused on chain.
    tiers: Vec<TierPolicy>,
    member_root: [u8; 32],
    recipient: [u8; 32],
    amount: u128,
    memo_hash: [u8; 32],
    /// Zero for a transfer; the configuration being installed for a rotation.
    rotate_to: [u8; 32],
}

impl Proposal {
    /// Bind a treasury payment to `proposal_id` under `multisig`.
    ///
    /// `memo` is the human sentence the members read — "pay the auditors" — and
    /// it is folded into the action hash by its own digest, so it is bound as
    /// tightly as the recipient and the amount without letting an
    /// arbitrary-length string into the fixed on-chain record.
    #[must_use]
    pub fn new(
        multisig: &Multisig,
        proposal_id: [u8; 32],
        recipient: [u8; 32],
        amount: u128,
        memo: &[u8],
    ) -> Self {
        let memo_hash = compute_memo_hash(memo);
        let action_hash =
            compute_transfer_action_hash(&multisig.id, &recipient, amount, &memo_hash);
        Self {
            multisig_id: multisig.id,
            config_hash: multisig.config_hash,
            proposal_id,
            action_hash,
            proposal_ref: compute_proposal_ref(
                &multisig.id,
                &multisig.config_hash,
                &proposal_id,
                &action_hash,
            ),
            threshold: multisig.threshold,
            tiers: multisig.tiers.clone(),
            member_root: multisig.member_root,
            recipient,
            amount,
            memo_hash,
            rotate_to: [0u8; 32],
        }
    }

    /// A proposal to rotate `multisig` into `next`.
    ///
    /// It names no recipient and moves nothing: the action it commits to is the
    /// configuration being installed, and `rotate_config` is the only
    /// instruction that can spend it.
    ///
    /// # Errors
    /// [`SdkError::NoopRotation`] if `next` is the configuration already in
    /// force. Rotating between different multisig ids is not a rotation at all,
    /// and is refused for the same reason.
    pub fn rotation(
        multisig: &Multisig,
        proposal_id: [u8; 32],
        next: &Multisig,
    ) -> Result<Self, SdkError> {
        if next.config_hash == multisig.config_hash || next.id != multisig.id {
            return Err(SdkError::NoopRotation);
        }
        let action_hash = compute_rotate_action_hash(&multisig.id, &next.config_hash);
        Ok(Self {
            multisig_id: multisig.id,
            config_hash: multisig.config_hash,
            proposal_id,
            action_hash,
            proposal_ref: compute_proposal_ref(
                &multisig.id,
                &multisig.config_hash,
                &proposal_id,
                &action_hash,
            ),
            threshold: multisig.threshold,
            tiers: multisig.tiers.clone(),
            member_root: multisig.member_root,
            recipient: [0u8; 32],
            amount: 0,
            memo_hash: [0u8; 32],
            rotate_to: next.config_hash,
        })
    }

    /// True if this proposal installs a configuration rather than paying.
    #[must_use]
    pub fn is_rotation(&self) -> bool {
        self.rotate_to != [0u8; 32]
    }

    /// How many approvals this proposal needs — the default threshold, unless a
    /// tier covers its amount. A rotation is never tiered.
    #[must_use]
    pub fn required(&self) -> u32 {
        if self.is_rotation() {
            self.threshold
        } else {
            required_threshold(self.amount, self.threshold, &self.tiers)
        }
    }

    /// Who this proposal pays.
    #[must_use]
    pub fn recipient(&self) -> [u8; 32] {
        self.recipient
    }
    /// How much it pays.
    #[must_use]
    pub fn amount(&self) -> u128 {
        self.amount
    }
    /// The commitment to its memo.
    #[must_use]
    pub fn memo_hash(&self) -> [u8; 32] {
        self.memo_hash
    }

    #[must_use]
    pub fn proposal_ref(&self) -> [u8; 32] {
        self.proposal_ref
    }
    #[must_use]
    pub fn action_hash(&self) -> [u8; 32] {
        self.action_hash
    }
    /// The PDA seed of the execution marker this proposal claims when executed.
    #[must_use]
    pub fn execution_marker_seed(&self) -> [u8; 32] {
        compute_execution_marker(&self.proposal_ref)
    }

    /// Arguments for the `create_proposal` instruction.
    ///
    /// # Errors
    /// Returns [`SdkError::ZeroAmount`] for a proposal that moves nothing; the
    /// on-chain program refuses it with `E_BAD_AMOUNT` (5017), minutes later.
    pub fn create_args(&self) -> Result<CreateProposalArgs, SdkError> {
        // A transfer of nothing gives the threshold nothing to gate. A rotation
        // moves nothing *by construction*, so the rule does not apply to it —
        // and the program agrees: it requires the amount to be zero there.
        if !self.is_rotation() && self.amount == 0 {
            return Err(SdkError::ZeroAmount);
        }
        Ok(CreateProposalArgs {
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            proposal_id: self.proposal_id,
            action_hash: self.action_hash,
            proposal_ref: self.proposal_ref,
            recipient: self.recipient,
            amount: self.amount,
            memo_hash: self.memo_hash,
            rotate_to: self.rotate_to,
        })
    }

    /// Build one member's approval: the witness, the statement, and the
    /// arguments the on-chain `approve` instruction takes.
    ///
    /// Verifies the witness against the statement locally first, so a bad
    /// witness fails here in microseconds rather than after minutes of proving.
    pub fn approval(
        &self,
        members: &MemberSet,
        index: usize,
        msk: [u8; 32],
        identifier: u128,
    ) -> Result<Approval, SdkError> {
        let (leaf_index, merkle_path) = members.path(index).ok_or(SdkError::NotAMember)?.clone();
        let salt = members.salt(index).ok_or(SdkError::NotAMember)?;

        let nullifier = compute_approval_nullifier(&self.proposal_ref, &msk);
        let witness = ApproveWitness {
            msk,
            identifier,
            salt,
            merkle_path,
            leaf_index,
        };
        let statement = ApproveStatement {
            member_root: self.member_root,
            proposal_ref: self.proposal_ref,
            nullifier,
        };

        approve(&witness, &statement).map_err(|_| SdkError::NotAMember)?;

        Ok(Approval {
            instruction: ApproveInstruction { witness, statement },
            nullifier,
            marker_seed: compute_approval_marker(&self.proposal_ref, &nullifier),
            args: ApproveArgs {
                multisig_id: self.multisig_id,
                config_hash: self.config_hash,
                member_root: self.member_root,
                threshold: self.threshold,
                tiers: encode_tiers(&self.tiers),
                proposal_ref: self.proposal_ref,
                nullifier,
                approval_marker_seed: compute_approval_marker(&self.proposal_ref, &nullifier),
            },
        })
    }

    /// Arguments for `execute`, given the approvals gathered so far.
    ///
    /// Rejects a short set or a duplicated approval before submission; the
    /// on-chain program would reject them too, with `E_THRESHOLD_NOT_MET`
    /// (5010) and `E_DUPLICATE_APPROVAL` (5011).
    pub fn execute_args(&self, approvals: &[Approval]) -> Result<ExecuteArgs, SdkError> {
        if self.is_rotation() {
            return Err(SdkError::WrongActionKind);
        }
        let need = self.required();
        if approvals.len() < need as usize {
            return Err(SdkError::ThresholdNotMet {
                have: approvals.len(),
                need,
            });
        }
        for i in 0..approvals.len() {
            for j in (i + 1)..approvals.len() {
                if approvals[i].nullifier == approvals[j].nullifier {
                    return Err(SdkError::DuplicateApproval);
                }
            }
        }
        // Present exactly the threshold. More would pass, but every extra marker
        // account costs compute for no additional guarantee.
        let chosen = &approvals[..need as usize];
        Ok(ExecuteArgs {
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            member_root: self.member_root,
            threshold: self.threshold,
            tiers: encode_tiers(&self.tiers),
            proposal_ref: self.proposal_ref,
            approval_nullifiers: chosen.iter().map(|a| a.nullifier).collect(),
            approval_marker_seeds: chosen.iter().map(|a| a.marker_seed).collect(),
            execution_marker_seed: self.execution_marker_seed(),
        })
    }

    /// Arguments for `rotate_config`.
    ///
    /// `next` must be the configuration this proposal committed to when it was
    /// raised — the proposal's `action_hash` is over that configuration, so a
    /// different one produces approvals that do not carry.
    ///
    /// The threshold required is the multisig's *default*, never a tier. Tiers
    /// price transfers; rewriting who may act is not a transfer, and letting a
    /// tier lower that bar would make the cheapest action available the one that
    /// hands the multisig to someone else.
    ///
    /// # Errors
    /// [`SdkError::WrongActionKind`] for a transfer proposal,
    /// [`SdkError::NoopRotation`] if `next` is not what this proposal names, and
    /// [`SdkError::ThresholdNotMet`] or [`SdkError::DuplicateApproval`] as for
    /// [`Proposal::execute_args`].
    pub fn rotate_args(
        &self,
        next: &Multisig,
        approvals: &[Approval],
    ) -> Result<RotateConfigArgs, SdkError> {
        if !self.is_rotation() {
            return Err(SdkError::WrongActionKind);
        }
        if next.config_hash != self.rotate_to {
            return Err(SdkError::NoopRotation);
        }
        if approvals.len() < self.threshold as usize {
            return Err(SdkError::ThresholdNotMet {
                have: approvals.len(),
                need: self.threshold,
            });
        }
        for i in 0..approvals.len() {
            for j in (i + 1)..approvals.len() {
                if approvals[i].nullifier == approvals[j].nullifier {
                    return Err(SdkError::DuplicateApproval);
                }
            }
        }
        let chosen = &approvals[..self.threshold as usize];
        Ok(RotateConfigArgs {
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            member_root: self.member_root,
            threshold: self.threshold,
            tiers: encode_tiers(&self.tiers),
            new_config_hash: next.config_hash,
            new_member_root: next.member_root,
            new_threshold: next.threshold,
            new_tiers: encode_tiers(&next.tiers),
            proposal_ref: self.proposal_ref,
            approval_nullifiers: chosen.iter().map(|a| a.nullifier).collect(),
            approval_marker_seeds: chosen.iter().map(|a| a.marker_seed).collect(),
            execution_marker_seed: self.execution_marker_seed(),
        })
    }
}

/// One member's approval, ready to submit.
#[derive(Debug, Clone)]
pub struct Approval {
    /// What the chained call to the membership program proves.
    pub instruction: ApproveInstruction,
    /// The secret-bound nullifier. Public once submitted; unlinkable to a member.
    pub nullifier: [u8; 32],
    /// The PDA seed this approval occupies.
    pub marker_seed: [u8; 32],
    /// Arguments for the on-chain `approve` instruction.
    pub args: ApproveArgs,
}

// ---------------------------------------------------------------------------
// Instruction arguments
// ---------------------------------------------------------------------------

/// Arguments for `create_multisig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateMultisigArgs {
    pub multisig_id: [u8; 32],
    pub config_hash: [u8; 32],
    pub member_root: [u8; 32],
    pub threshold: u32,
    /// The encoded tier table, empty for a multisig with no tiers. Present in
    /// every instruction that reads the configuration, because `config_hash`
    /// commits to these exact bytes and the program recomputes the commitment.
    pub tiers: Vec<u8>,
}

/// Arguments for `create_proposal`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProposalArgs {
    pub multisig_id: [u8; 32],
    pub config_hash: [u8; 32],
    pub proposal_id: [u8; 32],
    pub action_hash: [u8; 32],
    pub proposal_ref: [u8; 32],
    pub recipient: [u8; 32],
    pub amount: u128,
    pub memo_hash: [u8; 32],
    /// Zero for a transfer; the configuration installed for a rotation.
    pub rotate_to: [u8; 32],
}

/// Arguments for `fund_treasury`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FundTreasuryArgs {
    pub multisig_id: [u8; 32],
    pub config_hash: [u8; 32],
    pub amount: u128,
}

/// Arguments for `approve`, minus the witness words, which the caller encodes
/// from [`Approval::instruction`] with `risc0_zkvm::serde::to_vec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApproveArgs {
    pub multisig_id: [u8; 32],
    pub config_hash: [u8; 32],
    pub member_root: [u8; 32],
    pub threshold: u32,
    pub tiers: Vec<u8>,
    pub proposal_ref: [u8; 32],
    pub nullifier: [u8; 32],
    pub approval_marker_seed: [u8; 32],
}

/// Arguments for `execute`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteArgs {
    pub multisig_id: [u8; 32],
    pub config_hash: [u8; 32],
    pub member_root: [u8; 32],
    pub threshold: u32,
    pub tiers: Vec<u8>,
    pub proposal_ref: [u8; 32],
    pub approval_nullifiers: Vec<[u8; 32]>,
    /// The marker PDA seeds, in the same order as the nullifiers. The executor
    /// passes the corresponding accounts as the trailing `approvals` list.
    pub approval_marker_seeds: Vec<[u8; 32]>,
    pub execution_marker_seed: [u8; 32],
}

/// Arguments for `rotate_config`.
///
/// Both configurations appear, because the instruction reads one and writes the
/// other: `config_hash` is the address it is leaving, `new_config_hash` the
/// address it is claiming. Neither can be invented — each is recomputed by the
/// program from the fields beside it, and each is a PDA seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotateConfigArgs {
    pub multisig_id: [u8; 32],
    pub config_hash: [u8; 32],
    pub member_root: [u8; 32],
    pub threshold: u32,
    pub tiers: Vec<u8>,
    pub new_config_hash: [u8; 32],
    pub new_member_root: [u8; 32],
    pub new_threshold: u32,
    pub new_tiers: Vec<u8>,
    pub proposal_ref: [u8; 32],
    pub approval_nullifiers: Vec<[u8; 32]>,
    /// The marker PDA seeds, in the same order as the nullifiers.
    pub approval_marker_seeds: Vec<[u8; 32]>,
    pub execution_marker_seed: [u8; 32],
}
