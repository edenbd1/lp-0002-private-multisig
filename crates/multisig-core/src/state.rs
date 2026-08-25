//! The five on-chain account layouts, and decoders for them.
//!
//! WHY THIS MODULE EXISTS SEPARATELY FROM THE PROGRAM
//!
//! The on-chain program writes these records with `borsh`. This module decodes
//! them **by offset**, with no borsh dependency at all, from the field table in
//! `docs/account-layout.md`. That is deliberate: a layout is only documented if
//! somebody who has only the document can read the bytes. If the program's
//! encoding ever drifts from the table, the tests that decode a real execution's
//! post-state through this module fail — so the document is checked by CI rather
//! than believed.
//!
//! Every field is little-endian and there is no padding anywhere. Each record
//! opens with a **format byte**, and decoding refuses any value it does not
//! know: a reader that guesses at an unfamiliar layout is worse than one that
//! stops.
//!
//! All lengths are exact. A record with trailing bytes is refused rather than
//! truncated, because trailing bytes mean the writer and the reader disagree
//! about the layout and the fields already parsed cannot be trusted either.

extern crate alloc;
use alloc::vec::Vec;

/// The layout version every record in this module carries at offset 0.
pub const STATE_FORMAT_V1: u8 = 1;

/// A proposal that has not been executed.
pub const STATUS_OPEN: u8 = 0;
/// A proposal whose threshold was met and whose action was carried out.
pub const STATUS_EXECUTED: u8 = 1;

/// Exact length of a serialised [`MultisigState`].
pub const MULTISIG_LEN: usize = 197;
/// Exact length of a serialised [`TreasuryState`].
pub const TREASURY_LEN: usize = 65;
/// Exact length of a serialised [`ProposalState`].
pub const PROPOSAL_LEN: usize = 242;
/// Exact length of a serialised [`ApprovalMarkerState`].
pub const APPROVAL_MARKER_LEN: usize = 65;
/// Length of a serialised [`ExecutionMarkerState`] before its nullifier list.
pub const EXECUTION_MARKER_HEADER_LEN: usize = 86;

/// Why a record could not be read.
///
/// Named cases rather than a bare `None`: "this account holds 0 bytes" and
/// "this account holds a layout I do not know" are different findings for
/// whoever is reading the chain, and conflating them is how an empty account
/// gets mistaken for a corrupt one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    /// Fewer bytes than the layout needs. `0` means the account was never
    /// written — an uninitialised PDA reads as empty, not as absent.
    TooShort { have: usize, need: usize },
    /// More bytes than the layout needs, so writer and reader disagree.
    TrailingBytes { have: usize, need: usize },
    /// The format byte names a layout this decoder does not implement.
    UnknownFormat(u8),
    /// A status byte outside the values this version defines.
    UnknownStatus(u8),
}

impl core::fmt::Display for StateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort { have, need } => {
                write!(f, "record is {have} bytes, layout needs {need}")
            }
            Self::TrailingBytes { have, need } => {
                write!(f, "record is {have} bytes, layout needs exactly {need}")
            }
            Self::UnknownFormat(v) => write!(f, "unknown record format {v}"),
            Self::UnknownStatus(v) => write!(f, "unknown status byte {v}"),
        }
    }
}

// ---------------------------------------------------------------------------
// A tiny cursor. Kept explicit so each `decode_*` reads like the offset table.
// ---------------------------------------------------------------------------

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], need: usize) -> Result<Self, StateError> {
        if bytes.len() < need {
            return Err(StateError::TooShort {
                have: bytes.len(),
                need,
            });
        }
        Ok(Self { bytes, at: 0 })
    }

    fn u8(&mut self) -> u8 {
        let v = self.bytes[self.at];
        self.at += 1;
        v
    }

    fn bytes32(&mut self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out.copy_from_slice(&self.bytes[self.at..self.at + 32]);
        self.at += 32;
        out
    }

    fn u32_le(&mut self) -> u32 {
        let mut b = [0u8; 4];
        b.copy_from_slice(&self.bytes[self.at..self.at + 4]);
        self.at += 4;
        u32::from_le_bytes(b)
    }

    fn u128_le(&mut self) -> u128 {
        let mut b = [0u8; 16];
        b.copy_from_slice(&self.bytes[self.at..self.at + 16]);
        self.at += 16;
        u128::from_le_bytes(b)
    }

    fn finish(self) -> Result<(), StateError> {
        if self.bytes.len() == self.at {
            Ok(())
        } else {
            Err(StateError::TrailingBytes {
                have: self.bytes.len(),
                need: self.at,
            })
        }
    }
}

fn expect_format(v: u8) -> Result<u8, StateError> {
    if v == STATE_FORMAT_V1 {
        Ok(v)
    } else {
        Err(StateError::UnknownFormat(v))
    }
}

fn expect_status(v: u8) -> Result<u8, StateError> {
    if v == STATUS_OPEN || v == STATUS_EXECUTED {
        Ok(v)
    } else {
        Err(StateError::UnknownStatus(v))
    }
}

// ---------------------------------------------------------------------------
// The records
// ---------------------------------------------------------------------------

/// The multisig account, at the PDA seeded by `[multisig_id, config_hash]`.
///
/// Its *address* already anchors the member root and the threshold — that is
/// what makes them unforgeable. What the data adds is readability: a third party
/// with the address alone can call `getAccount` and learn the configuration,
/// instead of having to know the root and threshold in advance in order to
/// confirm the address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultisigState {
    /// Layout version. Offset 0, 1 byte.
    pub format: u8,
    /// Offset 1, 32 bytes.
    pub multisig_id: [u8; 32],
    /// Offset 33, 32 bytes.
    pub member_root: [u8; 32],
    /// Offset 65, 4 bytes, little-endian.
    pub threshold: u32,
    /// Commitment to the spending-tier table, folded into `config_hash` and so
    /// anchored by this account's own address. Offset 69, 32 bytes.
    pub tiers_hash: [u8; 32],
    /// The configuration that replaced this one, or zero while it is in force.
    /// A rotation writes here rather than editing a member set: the new
    /// configuration is a different account at a different address.
    /// Offset 101, 32 bytes.
    pub superseded_by: [u8; 32],
    /// The treasury PDA this multisig pays from. Offset 133, 32 bytes.
    pub treasury: [u8; 32],
    /// The account that created it. Recorded, never trusted: creation is
    /// permissionless and the creator gets no privilege from being named here.
    /// Offset 165, 32 bytes.
    pub authority: [u8; 32],
}

/// Read a [`MultisigState`] from an account's `data`.
///
/// # Errors
/// Returns [`StateError`] if the bytes are not exactly one v1 multisig record.
pub fn decode_multisig(data: &[u8]) -> Result<MultisigState, StateError> {
    let mut r = Reader::new(data, MULTISIG_LEN)?;
    let out = MultisigState {
        format: expect_format(r.u8())?,
        multisig_id: r.bytes32(),
        member_root: r.bytes32(),
        threshold: r.u32_le(),
        tiers_hash: r.bytes32(),
        superseded_by: r.bytes32(),
        treasury: r.bytes32(),
        authority: r.bytes32(),
    };
    r.finish()?;
    Ok(out)
}

/// The treasury account, at the PDA seeded by
/// `[multisig_id, config_hash, "treasury"]`.
///
/// It exists to hold a balance the verifier program owns, which is the whole
/// reason a threshold can move value at all: LEZ refuses a post-state that
/// decreases a balance the executing program does not own, and permits any
/// program to increase any balance. The data here is identification only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreasuryState {
    /// Offset 0, 1 byte.
    pub format: u8,
    /// Offset 1, 32 bytes.
    pub multisig_id: [u8; 32],
    /// Offset 33, 32 bytes.
    pub config_hash: [u8; 32],
}

/// Read a [`TreasuryState`] from an account's `data`.
///
/// # Errors
/// Returns [`StateError`] if the bytes are not exactly one v1 treasury record.
pub fn decode_treasury(data: &[u8]) -> Result<TreasuryState, StateError> {
    let mut r = Reader::new(data, TREASURY_LEN)?;
    let out = TreasuryState {
        format: expect_format(r.u8())?,
        multisig_id: r.bytes32(),
        config_hash: r.bytes32(),
    };
    r.finish()?;
    Ok(out)
}

/// The proposal account, at the PDA seeded by
/// `[multisig_id, config_hash, proposal_ref]`.
///
/// This is the record that turns an approved proposal into a payable one. The
/// action fields are stored here and nowhere else, and `execute` re-derives
/// `action_hash` and `proposal_ref` from them before paying — so the bytes are
/// checked against the address that carries the approvals, on every execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProposalState {
    /// Offset 0, 1 byte.
    pub format: u8,
    /// Offset 1, 32 bytes.
    pub multisig_id: [u8; 32],
    /// Offset 33, 32 bytes.
    pub config_hash: [u8; 32],
    /// Offset 65, 32 bytes.
    pub proposal_id: [u8; 32],
    /// Offset 97, 32 bytes.
    pub action_hash: [u8; 32],
    /// Who the treasury pays. Offset 129, 32 bytes.
    pub recipient: [u8; 32],
    /// How much. Offset 161, 16 bytes, little-endian.
    pub amount: u128,
    /// Commitment to the human-readable memo. Offset 177, 32 bytes.
    pub memo_hash: [u8; 32],
    /// For a governance proposal, the configuration the rotation moves to; zero
    /// for a treasury transfer. Offset 209, 32 bytes.
    pub rotate_to: [u8; 32],
    /// [`STATUS_OPEN`] or [`STATUS_EXECUTED`]. Offset 209, 1 byte.
    pub status: u8,
}

/// Read a [`ProposalState`] from an account's `data`.
///
/// # Errors
/// Returns [`StateError`] if the bytes are not exactly one v1 proposal record.
pub fn decode_proposal(data: &[u8]) -> Result<ProposalState, StateError> {
    let mut r = Reader::new(data, PROPOSAL_LEN)?;
    let out = ProposalState {
        format: expect_format(r.u8())?,
        multisig_id: r.bytes32(),
        config_hash: r.bytes32(),
        proposal_id: r.bytes32(),
        action_hash: r.bytes32(),
        recipient: r.bytes32(),
        amount: r.u128_le(),
        memo_hash: r.bytes32(),
        rotate_to: r.bytes32(),
        status: expect_status(r.u8())?,
    };
    r.finish()?;
    Ok(out)
}

/// One approval marker, at the PDA seeded by
/// `SHA256(APPROVAL_MARKER_PREFIX ‖ proposal_ref ‖ nullifier)`.
///
/// Reading it tells you *which proposal* was approved and *which nullifier* was
/// spent. It tells you nothing about who: the nullifier is a function of a
/// member's secret, so an observer holding the entire candidate member set still
/// cannot attribute it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalMarkerState {
    /// Offset 0, 1 byte.
    pub format: u8,
    /// Offset 1, 32 bytes.
    pub proposal_ref: [u8; 32],
    /// Offset 33, 32 bytes.
    pub nullifier: [u8; 32],
}

/// Read an [`ApprovalMarkerState`] from an account's `data`.
///
/// # Errors
/// Returns [`StateError`] if the bytes are not exactly one v1 marker record.
pub fn decode_approval_marker(data: &[u8]) -> Result<ApprovalMarkerState, StateError> {
    let mut r = Reader::new(data, APPROVAL_MARKER_LEN)?;
    let out = ApprovalMarkerState {
        format: expect_format(r.u8())?,
        proposal_ref: r.bytes32(),
        nullifier: r.bytes32(),
    };
    r.finish()?;
    Ok(out)
}

/// The execution marker, at the PDA seeded by
/// `SHA256(EXECUTION_MARKER_PREFIX ‖ proposal_ref)`.
///
/// The one variable-length record: it carries the exact nullifiers the execution
/// consumed, in the order they were presented. That list is the audit trail —
/// `M` distinct secret-bound values, each of which had to resolve to a marker
/// the program itself owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionMarkerState {
    /// Offset 0, 1 byte.
    pub format: u8,
    /// Offset 1, 32 bytes.
    pub proposal_ref: [u8; 32],
    /// Offset 33, 32 bytes.
    pub recipient: [u8; 32],
    /// Offset 65, 16 bytes, little-endian.
    pub amount: u128,
    /// Always [`STATUS_EXECUTED`] — the record exists only after the payout.
    /// Offset 81, 1 byte.
    pub status: u8,
    /// Offset 82: a 4-byte little-endian count, then that many 32-byte
    /// nullifiers starting at offset 86.
    pub nullifiers: Vec<[u8; 32]>,
}

/// Read an [`ExecutionMarkerState`] from an account's `data`.
///
/// # Errors
/// Returns [`StateError`] if the bytes are not exactly one v1 execution record,
/// including the case where the declared nullifier count does not match the
/// bytes that follow it.
pub fn decode_execution_marker(data: &[u8]) -> Result<ExecutionMarkerState, StateError> {
    let mut r = Reader::new(data, EXECUTION_MARKER_HEADER_LEN)?;
    let format = expect_format(r.u8())?;
    let proposal_ref = r.bytes32();
    let recipient = r.bytes32();
    let amount = r.u128_le();
    let status = expect_status(r.u8())?;
    let count = r.u32_le() as usize;

    let need = EXECUTION_MARKER_HEADER_LEN
        .checked_add(count.saturating_mul(32))
        .ok_or(StateError::TooShort {
            have: data.len(),
            need: usize::MAX,
        })?;
    if data.len() < need {
        return Err(StateError::TooShort {
            have: data.len(),
            need,
        });
    }
    let mut nullifiers = Vec::with_capacity(count);
    for _ in 0..count {
        nullifiers.push(r.bytes32());
    }
    r.finish()?;
    Ok(ExecutionMarkerState {
        format,
        proposal_ref,
        recipient,
        amount,
        status,
        nullifiers,
    })
}

// ---------------------------------------------------------------------------
// Encoders
//
// The chain never runs these — the program writes with borsh. They exist so a
// client can build the exact bytes it expects to read back, which is what makes
// `a_written_record_reads_back_identically` a check on the layout rather than on
// itself, and what lets the tests construct realistic pre-states.
// ---------------------------------------------------------------------------

/// Serialise a [`MultisigState`] at the documented offsets.
#[must_use]
pub fn encode_multisig(s: &MultisigState) -> Vec<u8> {
    let mut out = Vec::with_capacity(MULTISIG_LEN);
    out.push(s.format);
    out.extend_from_slice(&s.multisig_id);
    out.extend_from_slice(&s.member_root);
    out.extend_from_slice(&s.threshold.to_le_bytes());
    out.extend_from_slice(&s.tiers_hash);
    out.extend_from_slice(&s.superseded_by);
    out.extend_from_slice(&s.treasury);
    out.extend_from_slice(&s.authority);
    out
}

/// Serialise a [`TreasuryState`] at the documented offsets.
#[must_use]
pub fn encode_treasury(s: &TreasuryState) -> Vec<u8> {
    let mut out = Vec::with_capacity(TREASURY_LEN);
    out.push(s.format);
    out.extend_from_slice(&s.multisig_id);
    out.extend_from_slice(&s.config_hash);
    out
}

/// Serialise a [`ProposalState`] at the documented offsets.
#[must_use]
pub fn encode_proposal(s: &ProposalState) -> Vec<u8> {
    let mut out = Vec::with_capacity(PROPOSAL_LEN);
    out.push(s.format);
    out.extend_from_slice(&s.multisig_id);
    out.extend_from_slice(&s.config_hash);
    out.extend_from_slice(&s.proposal_id);
    out.extend_from_slice(&s.action_hash);
    out.extend_from_slice(&s.recipient);
    out.extend_from_slice(&s.amount.to_le_bytes());
    out.extend_from_slice(&s.memo_hash);
    out.extend_from_slice(&s.rotate_to);
    out.push(s.status);
    out
}

/// Serialise an [`ApprovalMarkerState`] at the documented offsets.
#[must_use]
pub fn encode_approval_marker(s: &ApprovalMarkerState) -> Vec<u8> {
    let mut out = Vec::with_capacity(APPROVAL_MARKER_LEN);
    out.push(s.format);
    out.extend_from_slice(&s.proposal_ref);
    out.extend_from_slice(&s.nullifier);
    out
}

/// Serialise an [`ExecutionMarkerState`] at the documented offsets.
///
/// # Panics
/// Panics if the nullifier list is longer than `u32::MAX`, which no threshold is.
#[must_use]
pub fn encode_execution_marker(s: &ExecutionMarkerState) -> Vec<u8> {
    let count = u32::try_from(s.nullifiers.len()).expect("a threshold fits in u32");
    let mut out = Vec::with_capacity(EXECUTION_MARKER_HEADER_LEN + s.nullifiers.len() * 32);
    out.push(s.format);
    out.extend_from_slice(&s.proposal_ref);
    out.extend_from_slice(&s.recipient);
    out.extend_from_slice(&s.amount.to_le_bytes());
    out.push(s.status);
    out.extend_from_slice(&count.to_le_bytes());
    for n in &s.nullifiers {
        out.extend_from_slice(n);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The offset table in `docs/account-layout.md` is only useful if the
    /// lengths in it are the lengths a decoder needs.
    #[test]
    fn documented_lengths_are_the_lengths_the_decoders_require() {
        assert_eq!(MULTISIG_LEN, 1 + 32 + 32 + 4 + 32 + 32 + 32 + 32);
        assert_eq!(TREASURY_LEN, 1 + 32 + 32);
        assert_eq!(PROPOSAL_LEN, 1 + 32 + 32 + 32 + 32 + 32 + 16 + 32 + 32 + 1);
        assert_eq!(APPROVAL_MARKER_LEN, 1 + 32 + 32);
        assert_eq!(EXECUTION_MARKER_HEADER_LEN, 1 + 32 + 32 + 16 + 1 + 4);
    }

    #[test]
    fn an_empty_account_reads_as_too_short_not_as_corrupt() {
        assert_eq!(
            decode_multisig(&[]),
            Err(StateError::TooShort {
                have: 0,
                need: MULTISIG_LEN
            })
        );
    }

    #[test]
    fn an_unknown_format_byte_is_refused_rather_than_guessed_at() {
        let mut raw = [0u8; MULTISIG_LEN];
        raw[0] = 9;
        assert_eq!(decode_multisig(&raw), Err(StateError::UnknownFormat(9)));
    }

    #[test]
    fn trailing_bytes_are_refused() {
        let mut raw = [0u8; MULTISIG_LEN + 1];
        raw[0] = STATE_FORMAT_V1;
        assert_eq!(
            decode_multisig(&raw),
            Err(StateError::TrailingBytes {
                have: MULTISIG_LEN + 1,
                need: MULTISIG_LEN
            })
        );
    }

    /// A declared nullifier count larger than the bytes that follow must not be
    /// read past the end of the record.
    #[test]
    fn an_overlong_nullifier_count_is_refused() {
        let mut raw = [0u8; EXECUTION_MARKER_HEADER_LEN];
        raw[0] = STATE_FORMAT_V1;
        raw[81] = STATUS_EXECUTED;
        raw[82..86].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(
            decode_execution_marker(&raw),
            Err(StateError::TooShort {
                have: EXECUTION_MARKER_HEADER_LEN,
                need: EXECUTION_MARKER_HEADER_LEN + 7 * 32
            })
        );
    }

    #[test]
    fn a_status_byte_outside_the_two_defined_values_is_refused() {
        let mut raw = [0u8; PROPOSAL_LEN];
        raw[0] = STATE_FORMAT_V1;
        raw[241] = 4;
        assert_eq!(decode_proposal(&raw), Err(StateError::UnknownStatus(4)));
    }

    /// Decoding a record built field by field at the documented offsets must
    /// return those fields — which is the whole promise the table makes.
    #[test]
    fn a_record_laid_out_by_the_documented_offsets_round_trips() {
        let mut raw = [0u8; PROPOSAL_LEN];
        raw[0] = STATE_FORMAT_V1;
        raw[1..33].copy_from_slice(&[0xA1; 32]);
        raw[33..65].copy_from_slice(&[0xA2; 32]);
        raw[65..97].copy_from_slice(&[0xA3; 32]);
        raw[97..129].copy_from_slice(&[0xA4; 32]);
        raw[129..161].copy_from_slice(&[0xA5; 32]);
        raw[161..177].copy_from_slice(&250u128.to_le_bytes());
        raw[177..209].copy_from_slice(&[0xA6; 32]);
        raw[209..241].copy_from_slice(&[0xA7; 32]);
        raw[241] = STATUS_EXECUTED;

        let p = decode_proposal(&raw).expect("the documented layout must decode");
        assert_eq!(p.multisig_id, [0xA1; 32]);
        assert_eq!(p.config_hash, [0xA2; 32]);
        assert_eq!(p.proposal_id, [0xA3; 32]);
        assert_eq!(p.action_hash, [0xA4; 32]);
        assert_eq!(p.recipient, [0xA5; 32]);
        assert_eq!(p.amount, 250);
        assert_eq!(p.memo_hash, [0xA6; 32]);
        assert_eq!(p.rotate_to, [0xA7; 32]);
        assert_eq!(p.status, STATUS_EXECUTED);
    }

    /// Every record round-trips through the encoder and the decoder at the
    /// documented length. A layout change that updates one side and not the
    /// other lands here.
    #[test]
    fn every_record_round_trips_at_its_documented_length() {
        let m = MultisigState {
            format: STATE_FORMAT_V1,
            multisig_id: [1; 32],
            member_root: [2; 32],
            threshold: 3,
            treasury: [4; 32],
            authority: [5; 32],
            tiers_hash: [0u8; 32],
            superseded_by: [0u8; 32],
        };
        let raw = encode_multisig(&m);
        assert_eq!(raw.len(), MULTISIG_LEN);
        assert_eq!(decode_multisig(&raw), Ok(m));

        let t = TreasuryState {
            format: STATE_FORMAT_V1,
            multisig_id: [6; 32],
            config_hash: [7; 32],
        };
        let raw = encode_treasury(&t);
        assert_eq!(raw.len(), TREASURY_LEN);
        assert_eq!(decode_treasury(&raw), Ok(t));

        let p = ProposalState {
            format: STATE_FORMAT_V1,
            multisig_id: [8; 32],
            config_hash: [9; 32],
            proposal_id: [10; 32],
            action_hash: [11; 32],
            recipient: [12; 32],
            amount: u128::MAX,
            memo_hash: [13; 32],
            status: STATUS_OPEN,
            rotate_to: [0u8; 32],
        };
        let raw = encode_proposal(&p);
        assert_eq!(raw.len(), PROPOSAL_LEN);
        assert_eq!(decode_proposal(&raw), Ok(p));

        let a = ApprovalMarkerState {
            format: STATE_FORMAT_V1,
            proposal_ref: [14; 32],
            nullifier: [15; 32],
        };
        let raw = encode_approval_marker(&a);
        assert_eq!(raw.len(), APPROVAL_MARKER_LEN);
        assert_eq!(decode_approval_marker(&raw), Ok(a));

        let e = ExecutionMarkerState {
            format: STATE_FORMAT_V1,
            proposal_ref: [16; 32],
            recipient: [17; 32],
            amount: 250,
            status: STATUS_EXECUTED,
            nullifiers: alloc::vec![[18; 32], [19; 32], [20; 32]],
        };
        let raw = encode_execution_marker(&e);
        assert_eq!(raw.len(), EXECUTION_MARKER_HEADER_LEN + 3 * 32);
        assert_eq!(decode_execution_marker(&raw), Ok(e));
    }

    /// An execution marker with no nullifiers is still a well-formed record —
    /// the count is zero and nothing follows it. Worth pinning because an
    /// off-by-one in the header length would only show up here.
    #[test]
    fn an_empty_nullifier_list_encodes_to_the_header_alone() {
        let e = ExecutionMarkerState {
            format: STATE_FORMAT_V1,
            proposal_ref: [1; 32],
            recipient: [2; 32],
            amount: 1,
            status: STATUS_EXECUTED,
            nullifiers: alloc::vec![],
        };
        let raw = encode_execution_marker(&e);
        assert_eq!(raw.len(), EXECUTION_MARKER_HEADER_LEN);
        assert_eq!(decode_execution_marker(&raw), Ok(e));
    }
}
