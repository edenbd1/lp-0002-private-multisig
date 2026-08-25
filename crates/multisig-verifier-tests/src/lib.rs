//! The harness the LP-0002 verifier tests share.
//!
//! Everything here runs the **committed** `multisig_verifier.bin` through the
//! *sequencer's own* execution path — same input order, same 32M session limit,
//! same executor (`lee/state_machine/src/program/mod.rs:55-110`) — so a
//! rejection here is the same rejection the chain performs, and an accepted
//! post-state is the one the chain would write.
//!
//! It lives in the crate's library rather than in one of the test files because
//! two test binaries need it, and a harness copied into both is a harness that
//! drifts: the day the account order changes, one copy gets updated.

use lee_core::account::{Account, AccountId, AccountWithMetadata, Nonce};
use lee_core::program::{ProgramId, ProgramOutput};
use multisig_core::state::*;
use multisig_core::*;
use risc0_zkvm::{default_executor, ExecutorEnv};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The session limit the sequencer applies to a public execution.
pub const MAX_NUM_CYCLES_PUBLIC_EXECUTION: u64 = 1024 * 1024 * 32;

const ELF_PATH: &str = "../../artifacts/programs/multisig_verifier.bin";

/// ProgramId of the LEZ-native `authenticated_transfer` program at tag v0.2.4.
///
/// The verifier pins this constant too, and for the same reason: it is the only
/// program whose ownership of an account means that account's balance can
/// actually be moved. Reproduce with
/// `spel program-id _external/lez/artifacts/lez/programs/authenticated_transfer.bin`.
pub const AUTH_TRANSFER_PROGRAM_ID: ProgramId = [
    583309054, 2344528779, 3806558405, 2890696795, 2257354672, 3978764116, 2273929063, 1518858078,
];

/// Index of the first approval marker in `execute`'s account list.
///
/// Named rather than written as `6` at each call site: the fixed accounts have
/// changed once already, and every test that pokes at a marker by index was
/// silently pointing at the wrong account until it was.
pub const EXEC_FIRST_APPROVAL: usize = 6;

/// The instruction enum `#[lez_program]` generates, in declaration order.
/// risc0's serde encodes the variant index as a leading u32, then the fields in
/// order. `ProgramContext` is injected by the dispatcher and is deliberately
/// absent here — it is not part of the ABI.
#[derive(Serialize)]
pub enum VerifierInstruction {
    CreateMultisig {
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        member_root: [u8; 32],
        threshold: u32,
        tiers: Vec<u8>,
    },
    FundTreasury {
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        amount: u128,
    },
    CreateProposal {
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        proposal_id: [u8; 32],
        action_hash: [u8; 32],
        proposal_ref: [u8; 32],
        recipient: [u8; 32],
        amount: u128,
        memo_hash: [u8; 32],
        rotate_to: [u8; 32],
    },
    Approve {
        witness_words: Vec<u32>,
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        member_root: [u8; 32],
        threshold: u32,
        tiers: Vec<u8>,
        proposal_ref: [u8; 32],
        nullifier: [u8; 32],
        approval_marker_seed: [u8; 32],
    },
    Execute {
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        member_root: [u8; 32],
        threshold: u32,
        tiers: Vec<u8>,
        proposal_ref: [u8; 32],
        approval_nullifiers: Vec<[u8; 32]>,
        execution_marker_seed: [u8; 32],
    },
    /// Appended last, and it must stay last. risc0's serde puts the variant
    /// index on the wire, so moving this renumbers every instruction after it
    /// and silently changes the meaning of every already-signed payload.
    RotateConfig {
        multisig_id: [u8; 32],
        config_hash: [u8; 32],
        member_root: [u8; 32],
        threshold: u32,
        tiers: Vec<u8>,
        new_config_hash: [u8; 32],
        new_member_root: [u8; 32],
        new_threshold: u32,
        new_tiers: Vec<u8>,
        proposal_ref: [u8; 32],
        approval_nullifiers: Vec<[u8; 32]>,
        execution_marker_seed: [u8; 32],
    },
}

/// The committed verifier binary.
///
/// # Panics
/// Panics with build instructions if the artefact is missing.
#[must_use]
pub fn elf() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(ELF_PATH);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}. Build it with:\n  ./scripts/build-programs.sh",
            path.display()
        )
    })
}

/// The ProgramId of a binary, which on LEZ is its ImageID.
///
/// # Panics
/// Panics if the binary does not decode as a risc0 program.
#[must_use]
pub fn program_id(elf: &[u8]) -> ProgramId {
    risc0_binfmt::ProgramBinary::decode(elf)
        .expect("verifier binary must decode")
        .compute_image_id()
        .expect("image id")
        .into()
}

/// The public PDA derivation SPEL uses: multi-seed combines with SHA256, then
/// `AccountId::for_public_pda`. Byte-identical to `lee_core`'s `for_public_pda`.
///
/// # Panics
/// Panics if `seeds` is empty.
#[must_use]
pub fn public_pda(program: &ProgramId, seeds: &[[u8; 32]]) -> AccountId {
    assert!(!seeds.is_empty(), "a PDA needs at least one seed");
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

/// A `literal("...")` PDA seed, as SPEL builds it: the ASCII bytes, zero-padded
/// to 32 (`spel_framework::pda::seed_from_str`).
///
/// # Panics
/// Panics if the literal exceeds 32 bytes, which SPEL would too.
#[must_use]
pub fn literal_seed(s: &str) -> [u8; 32] {
    let src = s.as_bytes();
    assert!(src.len() <= 32, "a literal seed is at most 32 bytes");
    let mut out = [0u8; 32];
    out[..src.len()].copy_from_slice(src);
    out
}

/// An account with an owner, a balance and a data record.
#[must_use]
pub fn owned_with(
    owner: ProgramId,
    id: AccountId,
    balance: u128,
    data: Vec<u8>,
) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account {
            program_owner: owner,
            balance,
            data: data.try_into().expect("record fits an account"),
            nonce: Nonce(0),
        },
        is_authorized: false,
        account_id: id,
    }
}

/// An empty account owned by `owner`.
#[must_use]
pub fn owned_by(owner: ProgramId, id: AccountId) -> AccountWithMetadata {
    owned_with(owner, id, 0, Vec::new())
}

/// An account nobody has claimed: the state every PDA starts in.
#[must_use]
pub fn uninitialised(id: AccountId) -> AccountWithMetadata {
    owned_by(ProgramId::default(), id)
}

/// A funded public account, held by the native transfer program — the only
/// shape of account that can spend what it is paid.
#[must_use]
pub fn held_by_transfer(id: [u8; 32], balance: u128) -> AccountWithMetadata {
    owned_with(
        AUTH_TRANSFER_PROGRAM_ID,
        AccountId::new(id),
        balance,
        Vec::new(),
    )
}

/// An authorised (signing) account.
#[must_use]
pub fn signer(id: [u8; 32]) -> AccountWithMetadata {
    AccountWithMetadata {
        account: Account::default(),
        is_authorized: true,
        account_id: AccountId::new(id),
    }
}

/// A signing account that also holds a balance the transfer program owns, which
/// is what a real funder is.
#[must_use]
pub fn funding_signer(id: [u8; 32], balance: u128) -> AccountWithMetadata {
    let mut a = held_by_transfer(id, balance);
    a.is_authorized = true;
    a
}

fn env_for(
    pid: &ProgramId,
    pre: &[AccountWithMetadata],
    ix: &VerifierInstruction,
) -> anyhow::Result<ExecutorEnv<'static>> {
    let caller: Option<ProgramId> = None;
    let data = risc0_zkvm::serde::to_vec(ix)?;
    let mut b = ExecutorEnv::builder();
    b.session_limit(Some(MAX_NUM_CYCLES_PUBLIC_EXECUTION));
    b.write(pid)?;
    b.write(&caller)?;
    b.write(&pre.to_vec())?;
    b.write(&data)?;
    b.build()
}

/// Run one instruction through the executor, discarding the journal.
///
/// # Errors
/// Returns the executor's error, which carries the guest's panic message and
/// therefore the program's own error code.
pub fn run(
    elf: &[u8],
    pid: &ProgramId,
    pre: Vec<AccountWithMetadata>,
    ix: &VerifierInstruction,
) -> anyhow::Result<()> {
    default_executor().execute(env_for(pid, &pre, ix)?, elf)?;
    Ok(())
}

/// Run one instruction and keep the session, for cycle counts.
///
/// # Errors
/// Returns the executor's error.
pub fn session(
    elf: &[u8],
    pid: &ProgramId,
    pre: Vec<AccountWithMetadata>,
    ix: &VerifierInstruction,
) -> anyhow::Result<risc0_zkvm::SessionInfo> {
    default_executor().execute(env_for(pid, &pre, ix)?, elf)
}

/// Run one instruction and decode the `ProgramOutput` it committed.
///
/// This is what turns "the call was accepted" into "and here is the state it
/// wrote": the journal carries the post-states the sequencer would apply, so a
/// test can assert on balances and account data rather than only on the absence
/// of an error.
///
/// # Errors
/// Returns the executor's error, or a decode error if the journal is not a
/// `ProgramOutput`.
pub fn output(
    elf: &[u8],
    pid: &ProgramId,
    pre: Vec<AccountWithMetadata>,
    ix: &VerifierInstruction,
) -> anyhow::Result<ProgramOutput> {
    let info = session(elf, pid, pre, ix)?;
    Ok(info.journal.decode::<ProgramOutput>()?)
}

/// Find a post-state by account id, and return `(pre, post_account)`.
///
/// # Panics
/// Panics if the id is absent, naming the ids that were present — a post-state
/// the dispatcher filtered out is a real possibility and a confusing one.
#[must_use]
pub fn state_of<'a>(out: &'a ProgramOutput, id: &AccountId) -> (&'a Account, &'a Account) {
    let idx = out
        .pre_states
        .iter()
        .position(|p| &p.account_id == id)
        .unwrap_or_else(|| {
            let present: Vec<String> = out
                .pre_states
                .iter()
                .map(|p| p.account_id.to_string())
                .collect();
            panic!("no post-state for {id}; the output carries {present:?}")
        });
    (&out.pre_states[idx].account, out.post_states[idx].account())
}

/// Require the call to be refused, by the program's own code or by SPEL's
/// address validation before the body runs.
///
/// A `PdaMismatch` counts, and it counts as *better* than the code. Once
/// `config_hash` is a seed of the proposal account, naming a config the proposal
/// does not belong to resolves to an address nobody ever created — so the
/// forgery stops being something the program checks and rejects, and becomes
/// something that cannot be expressed.
///
/// # Panics
/// Panics if the error names neither the code, the keyword, nor an address
/// mismatch.
pub fn assert_rejected(err: anyhow::Error, code: u32, keyword: &str) {
    let msg = format!("{err:#}").to_lowercase();
    let by_code = msg.contains(&code.to_string()) || msg.contains(keyword);
    let by_address = msg.contains("pdamismatch");
    assert!(
        by_code || by_address,
        "expected rejection {code} ({keyword}) or an address mismatch, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Fixture: a complete, consistent multisig with one funded treasury and one
// proposal, from which each test breaks exactly one thing.
// ---------------------------------------------------------------------------

/// One member's private data plus their Merkle authentication path:
/// `(msk, identifier, salt, leaf_index, siblings)`.
pub type TestMember = ([u8; 32], u128, [u8; 32], u64, Vec<[u8; 32]>);

/// The account id the fixture's proposals pay. Held by the native transfer
/// program, because the verifier refuses to pay anything that is not.
pub const FIXTURE_RECIPIENT: [u8; 32] = [0x5E; 32];
/// The fixture's creator, recorded in the multisig record.
pub const FIXTURE_AUTHORITY: [u8; 32] = [0xA1; 32];
/// What the fixture's treasury holds before the proposal is executed.
pub const FIXTURE_TREASURY_BALANCE: u128 = 500;
/// What the fixture's proposal pays out. Half the treasury, so a successful
/// execution is visible as a change in both directions.
pub const FIXTURE_AMOUNT: u128 = 250;
/// The human-readable memo the fixture's members are approving.
pub const FIXTURE_MEMO: &[u8] = b"transfer 250 to the grants treasury";

/// A complete multisig, its treasury, and one proposal against it.
pub struct Fixture {
    pub verifier: ProgramId,
    pub multisig_id: [u8; 32],
    pub member_root: [u8; 32],
    pub threshold: u32,
    /// The spending tiers this fixture anchors. Empty by default: a tier table
    /// only ever lowers the bar for small transfers, so "no tiers" is the plain
    /// case every existing test means, and the tiered cases say so explicitly.
    pub tiers: Vec<(u128, u32)>,
    pub config_hash: [u8; 32],
    pub proposal_id: [u8; 32],
    pub action_hash: [u8; 32],
    pub proposal_ref: [u8; 32],
    pub members: Vec<TestMember>,
    pub recipient: [u8; 32],
    pub amount: u128,
    pub memo_hash: [u8; 32],
    /// What `treasury_account(true)` reports as its balance. A test that wants
    /// an underfunded treasury lowers this.
    pub treasury_balance: u128,
}

impl Fixture {
    /// A 3-of-5 multisig, which is what almost every test wants.
    #[must_use]
    pub fn new(verifier: &ProgramId) -> Self {
        Self::with_members(verifier, 5, 3)
    }

    /// An M-of-N multisig with deterministic members, so the tests are
    /// reproducible.
    ///
    /// # Panics
    /// Panics if the threshold exceeds the member count, which no honest
    /// fixture would ask for.
    #[must_use]
    pub fn with_members(verifier: &ProgramId, n: usize, threshold: u32) -> Self {
        assert!(
            threshold as usize <= n && n > 0,
            "a {threshold}-of-{n} multisig could never execute"
        );
        let mut leaves = Vec::new();
        let mut raw = Vec::new();
        for i in 0..n {
            let byte = u8::try_from(i).expect("fixtures stay well under 256 members");
            let msk = [byte + 1; 32];
            let identifier = i as u128 + 1;
            let salt = [0x40 ^ byte; 32];
            let aid = derive_account_id(&derive_npk(&msk), identifier);
            leaves.push(compute_member_leaf(&aid, &salt));
            raw.push((msk, identifier, salt));
        }
        let (member_root, paths) = build_member_tree(&leaves);
        let multisig_id = [0xA0; 32];
        let config_hash = compute_config_hash(&member_root, threshold, &no_tiers_hash());
        let proposal_id = [0x11; 32];
        let memo_hash = compute_memo_hash(FIXTURE_MEMO);
        let action_hash = compute_transfer_action_hash(
            &multisig_id,
            &FIXTURE_RECIPIENT,
            FIXTURE_AMOUNT,
            &memo_hash,
        );
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
            tiers: Vec::new(),
            config_hash,
            proposal_id,
            action_hash,
            proposal_ref,
            members,
            recipient: FIXTURE_RECIPIENT,
            amount: FIXTURE_AMOUNT,
            memo_hash,
            treasury_balance: FIXTURE_TREASURY_BALANCE,
        }
    }

    // ── addresses ────────────────────────────────────────────────────────

    #[must_use]
    pub fn multisig_id_addr(&self) -> AccountId {
        public_pda(&self.verifier, &[self.multisig_id, self.config_hash])
    }

    #[must_use]
    pub fn treasury_addr(&self) -> AccountId {
        public_pda(
            &self.verifier,
            &[self.multisig_id, self.config_hash, literal_seed("treasury")],
        )
    }

    #[must_use]
    pub fn proposal_addr(&self) -> AccountId {
        public_pda(
            &self.verifier,
            &[self.multisig_id, self.config_hash, self.proposal_ref],
        )
    }

    #[must_use]
    pub fn execution_addr(&self) -> AccountId {
        public_pda(
            &self.verifier,
            &[compute_execution_marker(&self.proposal_ref)],
        )
    }

    #[must_use]
    pub fn recipient_addr(&self) -> AccountId {
        AccountId::new(self.recipient)
    }

    // ── records ──────────────────────────────────────────────────────────

    #[must_use]
    pub fn multisig_record(&self) -> MultisigState {
        MultisigState {
            format: STATE_FORMAT_V1,
            multisig_id: self.multisig_id,
            member_root: self.member_root,
            threshold: self.threshold,
            treasury: *self.treasury_addr().value(),
            authority: FIXTURE_AUTHORITY,
            // Derived from `self.tiers`, never hardcoded: a record whose
            // `tiers_hash` disagreed with the `config_hash` in its own address
            // is a state the program rejects (E_TIERS_MISMATCH), so a fixture
            // that built one would make every tiered test fail for a reason
            // that has nothing to do with what it is testing.
            tiers_hash: compute_tiers_hash(&self.tier_table()),
            superseded_by: [0u8; 32],
        }
    }

    /// `self.tiers` as the typed table the core crate works in.
    #[must_use]
    pub fn tier_table(&self) -> Vec<TierPolicy> {
        self.tiers
            .iter()
            .map(|&(max_amount, threshold)| TierPolicy {
                max_amount,
                threshold,
            })
            .collect()
    }

    /// The multisig record as it stands *after* a rotation has left it: still
    /// anchored, still decodable, and pointing at its successor.
    #[must_use]
    pub fn multisig_account_superseded_by(&self, r: &Rotation) -> AccountWithMetadata {
        let mut record = self.multisig_record();
        record.superseded_by = r.new_config_hash;
        owned_with(
            self.verifier,
            self.multisig_id_addr(),
            0,
            encode_multisig(&record),
        )
    }

    #[must_use]
    pub fn proposal_record(&self) -> ProposalState {
        ProposalState {
            format: STATE_FORMAT_V1,
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            proposal_id: self.proposal_id,
            action_hash: self.action_hash,
            recipient: self.recipient,
            amount: self.amount,
            memo_hash: self.memo_hash,
            status: STATUS_OPEN,
            rotate_to: [0u8; 32],
        }
    }

    #[must_use]
    pub fn treasury_record(&self) -> TreasuryState {
        TreasuryState {
            format: STATE_FORMAT_V1,
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
        }
    }

    // ── accounts ─────────────────────────────────────────────────────────

    /// The multisig account as `create_multisig` leaves it, or uninitialised.
    #[must_use]
    pub fn multisig_account(&self, anchored: bool) -> AccountWithMetadata {
        let id = self.multisig_id_addr();
        if anchored {
            owned_with(
                self.verifier,
                id,
                0,
                encode_multisig(&self.multisig_record()),
            )
        } else {
            uninitialised(id)
        }
    }

    /// The treasury as `create_multisig` plus `fund_treasury` leave it.
    #[must_use]
    pub fn treasury_account(&self, anchored: bool) -> AccountWithMetadata {
        let id = self.treasury_addr();
        if anchored {
            owned_with(
                self.verifier,
                id,
                self.treasury_balance,
                encode_treasury(&self.treasury_record()),
            )
        } else {
            uninitialised(id)
        }
    }

    /// The proposal account as `create_proposal` leaves it.
    ///
    /// Seeded by `[multisig_id, config_hash, proposal_ref]`: the multisig id and
    /// the config in the address are what stop a proposal being paired with a
    /// foreign multisig.
    #[must_use]
    pub fn proposal_account(&self, anchored: bool) -> AccountWithMetadata {
        self.proposal_account_holding(anchored, &self.proposal_record())
    }

    /// The proposal account carrying a record a test has altered.
    #[must_use]
    pub fn proposal_account_holding(
        &self,
        anchored: bool,
        record: &ProposalState,
    ) -> AccountWithMetadata {
        let id = self.proposal_addr();
        if anchored {
            owned_with(self.verifier, id, 0, encode_proposal(record))
        } else {
            uninitialised(id)
        }
    }

    /// The account the proposal pays, as a funded public account.
    #[must_use]
    pub fn recipient_account(&self) -> AccountWithMetadata {
        held_by_transfer(self.recipient, 0)
    }

    #[must_use]
    pub fn nullifier(&self, member: usize) -> [u8; 32] {
        compute_approval_nullifier(&self.proposal_ref, &self.members[member].0)
    }

    #[must_use]
    pub fn marker_seed(&self, member: usize) -> [u8; 32] {
        compute_approval_marker(&self.proposal_ref, &self.nullifier(member))
    }

    /// An anchored approval marker for `member`, as `approve` would leave it.
    #[must_use]
    pub fn marker_account(&self, member: usize, anchored: bool) -> AccountWithMetadata {
        let id = public_pda(&self.verifier, &[self.marker_seed(member)]);
        if anchored {
            owned_with(
                self.verifier,
                id,
                0,
                encode_approval_marker(&ApprovalMarkerState {
                    format: STATE_FORMAT_V1,
                    proposal_ref: self.proposal_ref,
                    nullifier: self.nullifier(member),
                }),
            )
        } else {
            uninitialised(id)
        }
    }

    // ── instructions ─────────────────────────────────────────────────────

    /// Accounts for `create_multisig`, in declaration order.
    #[must_use]
    pub fn create_multisig_accounts(&self, config_hash: [u8; 32]) -> Vec<AccountWithMetadata> {
        vec![
            uninitialised(public_pda(&self.verifier, &[self.multisig_id, config_hash])),
            uninitialised(public_pda(
                &self.verifier,
                &[self.multisig_id, config_hash, literal_seed("treasury")],
            )),
            signer(FIXTURE_AUTHORITY),
        ]
    }

    /// An honest `create_multisig`.
    #[must_use]
    pub fn create_multisig_ix(&self) -> VerifierInstruction {
        VerifierInstruction::CreateMultisig {
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            member_root: self.member_root,
            threshold: self.threshold,
            tiers: encode_tier_table(&self.tier_table()),
        }
    }

    /// Accounts for `create_proposal`, in declaration order.
    #[must_use]
    pub fn create_proposal_accounts(
        &self,
        proposal_ref: [u8; 32],
        multisig_anchored: bool,
    ) -> Vec<AccountWithMetadata> {
        vec![
            uninitialised(public_pda(
                &self.verifier,
                &[self.multisig_id, self.config_hash, proposal_ref],
            )),
            self.multisig_account(multisig_anchored),
            signer([0xA2; 32]),
        ]
    }

    /// An honest `create_proposal`.
    #[must_use]
    pub fn create_proposal_ix(&self) -> VerifierInstruction {
        VerifierInstruction::CreateProposal {
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            proposal_id: self.proposal_id,
            action_hash: self.action_hash,
            proposal_ref: self.proposal_ref,
            recipient: self.recipient,
            amount: self.amount,
            memo_hash: self.memo_hash,
            rotate_to: [0u8; 32],
        }
    }

    /// Accounts for `fund_treasury`, in declaration order.
    #[must_use]
    pub fn fund_accounts(&self, funder: AccountWithMetadata) -> Vec<AccountWithMetadata> {
        vec![
            self.multisig_account(true),
            self.treasury_account(true),
            funder,
        ]
    }

    /// An honest `fund_treasury`.
    #[must_use]
    pub fn fund_ix(&self, amount: u128) -> VerifierInstruction {
        VerifierInstruction::FundTreasury {
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            amount,
        }
    }

    /// An honest `approve` call by `member`.
    ///
    /// # Panics
    /// Panics if the witness cannot be encoded, which would be a bug in the
    /// fixture rather than in the program.
    #[must_use]
    pub fn approve_ix(&self, member: usize) -> VerifierInstruction {
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
            tiers: encode_tier_table(&self.tier_table()),
        }
    }

    /// Accounts for `approve`, in declaration order.
    #[must_use]
    pub fn approve_accounts(
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
    #[must_use]
    pub fn execute_ix(&self, members: &[usize]) -> VerifierInstruction {
        VerifierInstruction::Execute {
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            member_root: self.member_root,
            threshold: self.threshold,
            proposal_ref: self.proposal_ref,
            approval_nullifiers: members.iter().map(|&m| self.nullifier(m)).collect(),
            execution_marker_seed: compute_execution_marker(&self.proposal_ref),
            tiers: encode_tier_table(&self.tier_table()),
        }
    }

    /// `execute`'s six fixed accounts, in declaration order.
    #[must_use]
    pub fn execute_fixed(&self) -> Vec<AccountWithMetadata> {
        vec![
            uninitialised(self.execution_addr()),
            self.multisig_account(true),
            self.proposal_account(true),
            self.treasury_account(true),
            self.recipient_account(),
            signer([0xE1; 32]),
        ]
    }

    /// Accounts for `execute`: the six fixed ones, then the approval markers.
    #[must_use]
    pub fn execute_accounts(&self, members: &[usize]) -> Vec<AccountWithMetadata> {
        let mut v = self.execute_fixed();
        v.extend(members.iter().map(|&m| self.marker_account(m, true)));
        v
    }

    // ── tiers ────────────────────────────────────────────────────────────

    /// Anchor a tier table, which moves `config_hash` and therefore every
    /// address and every hash derived from it.
    ///
    /// The recomputation is the point. A test that set `self.tiers` and left
    /// `config_hash` alone would be asking the program to accept a table the
    /// address does not commit to — which is a *different* test, and one that
    /// belongs in `verifier_rejects.rs`.
    #[must_use]
    pub fn with_tiers(mut self, tiers: &[(u128, u32)]) -> Self {
        self.tiers = tiers.to_vec();
        let table: Vec<TierPolicy> = tiers
            .iter()
            .map(|&(max_amount, threshold)| TierPolicy {
                max_amount,
                threshold,
            })
            .collect();
        self.config_hash = compute_config_hash(
            &self.member_root,
            self.threshold,
            &compute_tiers_hash(&table),
        );
        self.rederive();
        self
    }

    /// Propose a different amount. `amount` is inside `action_hash`, so this
    /// moves the proposal address too.
    #[must_use]
    pub fn with_amount(mut self, amount: u128) -> Self {
        self.amount = amount;
        self.rederive();
        self
    }

    /// Recompute everything downstream of `config_hash` and `amount`.
    fn rederive(&mut self) {
        self.action_hash = compute_transfer_action_hash(
            &self.multisig_id,
            &self.recipient,
            self.amount,
            &self.memo_hash,
        );
        self.proposal_ref = compute_proposal_ref(
            &self.multisig_id,
            &self.config_hash,
            &self.proposal_id,
            &self.action_hash,
        );
    }

    // ── rotation ─────────────────────────────────────────────────────────

    /// The configuration this fixture would rotate *into*: same members, a
    /// different threshold, and whatever tiers are asked for.
    #[must_use]
    pub fn rotation(&self, new_threshold: u32, new_tiers: &[(u128, u32)]) -> Rotation {
        let table: Vec<TierPolicy> = new_tiers
            .iter()
            .map(|&(max_amount, threshold)| TierPolicy {
                max_amount,
                threshold,
            })
            .collect();
        let new_config_hash = compute_config_hash(
            &self.member_root,
            new_threshold,
            &compute_tiers_hash(&table),
        );
        // A rotation proposal commits to the configuration it installs, and to
        // nothing else — no recipient, no amount. The action shape is what tells
        // `execute` and `rotate_config` apart on the same proposal record.
        let action_hash = compute_rotate_action_hash(&self.multisig_id, &new_config_hash);
        let proposal_ref = compute_proposal_ref(
            &self.multisig_id,
            &self.config_hash,
            &self.proposal_id,
            &action_hash,
        );
        Rotation {
            new_member_root: self.member_root,
            new_threshold,
            new_tiers: new_tiers.to_vec(),
            new_config_hash,
            action_hash,
            proposal_ref,
        }
    }

    /// The address the rotated configuration will live at.
    #[must_use]
    pub fn rotated_multisig_addr(&self, r: &Rotation) -> AccountId {
        public_pda(&self.verifier, &[self.multisig_id, r.new_config_hash])
    }

    /// The rotated configuration's treasury — a *second* treasury, empty on
    /// creation. Nothing about a rotation moves value.
    #[must_use]
    pub fn rotated_treasury_addr(&self, r: &Rotation) -> AccountId {
        public_pda(
            &self.verifier,
            &[
                self.multisig_id,
                r.new_config_hash,
                literal_seed("treasury"),
            ],
        )
    }

    /// `create_proposal` for a rotation rather than a transfer.
    #[must_use]
    pub fn rotate_proposal_ix(&self, r: &Rotation) -> VerifierInstruction {
        VerifierInstruction::CreateProposal {
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            proposal_id: self.proposal_id,
            action_hash: r.action_hash,
            proposal_ref: r.proposal_ref,
            // A rotation carries no transfer. These are the zero values the
            // program requires when `rotate_to` is set, and it checks them.
            recipient: [0u8; 32],
            amount: 0,
            memo_hash: [0u8; 32],
            rotate_to: r.new_config_hash,
        }
    }

    #[must_use]
    pub fn rotate_ix(&self, r: &Rotation, members: &[usize]) -> VerifierInstruction {
        VerifierInstruction::RotateConfig {
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            member_root: self.member_root,
            threshold: self.threshold,
            tiers: encode_tier_table(&self.tier_table()),
            new_config_hash: r.new_config_hash,
            new_member_root: r.new_member_root,
            new_threshold: r.new_threshold,
            new_tiers: encode_tier_table(&r.tier_table()),
            proposal_ref: r.proposal_ref,
            approval_nullifiers: members.iter().map(|&m| self.nullifier(m)).collect(),
            execution_marker_seed: compute_execution_marker(&r.proposal_ref),
        }
    }

    /// `rotate_config`'s six fixed accounts, in declaration order.
    #[must_use]
    pub fn rotate_fixed(&self, r: &Rotation) -> Vec<AccountWithMetadata> {
        vec![
            uninitialised(public_pda(
                &self.verifier,
                &[compute_execution_marker(&r.proposal_ref)],
            )),
            self.multisig_account(true),
            uninitialised(self.rotated_multisig_addr(r)),
            uninitialised(self.rotated_treasury_addr(r)),
            self.rotate_proposal_account(r, true),
            signer([0xE1; 32]),
        ]
    }

    #[must_use]
    pub fn rotate_accounts(&self, r: &Rotation, members: &[usize]) -> Vec<AccountWithMetadata> {
        let mut v = self.rotate_fixed(r);
        v.extend(
            members
                .iter()
                .map(|&m| self.rotate_marker_account(r, m, true)),
        );
        v
    }

    /// The proposal record a rotation votes on: `rotate_to` set, transfer
    /// fields zero.
    #[must_use]
    pub fn rotate_proposal_account(&self, r: &Rotation, anchored: bool) -> AccountWithMetadata {
        let id = public_pda(
            &self.verifier,
            &[self.multisig_id, self.config_hash, r.proposal_ref],
        );
        if !anchored {
            return uninitialised(id);
        }
        let record = ProposalState {
            format: STATE_FORMAT_V1,
            multisig_id: self.multisig_id,
            config_hash: self.config_hash,
            proposal_id: self.proposal_id,
            action_hash: r.action_hash,
            recipient: [0u8; 32],
            amount: 0,
            memo_hash: [0u8; 32],
            status: STATUS_OPEN,
            rotate_to: r.new_config_hash,
        };
        owned_with(self.verifier, id, 0, encode_proposal(&record))
    }

    /// An approval marker under a rotation's `proposal_ref`.
    #[must_use]
    pub fn rotate_marker_account(
        &self,
        r: &Rotation,
        member: usize,
        anchored: bool,
    ) -> AccountWithMetadata {
        let seed = compute_approval_marker(&r.proposal_ref, &self.nullifier(member));
        let id = public_pda(&self.verifier, &[seed]);
        if anchored {
            owned_by(self.verifier, id)
        } else {
            uninitialised(id)
        }
    }

    /// `approve` against a rotation proposal rather than a transfer one.
    #[must_use]
    pub fn rotate_approve_ix(&self, r: &Rotation, member: usize) -> VerifierInstruction {
        match self.approve_ix(member) {
            VerifierInstruction::Approve {
                witness_words,
                multisig_id,
                config_hash,
                member_root,
                threshold,
                tiers,
                nullifier,
                ..
            } => VerifierInstruction::Approve {
                witness_words,
                multisig_id,
                config_hash,
                member_root,
                threshold,
                tiers,
                proposal_ref: r.proposal_ref,
                nullifier,
                approval_marker_seed: compute_approval_marker(&r.proposal_ref, &nullifier),
            },
            _ => unreachable!("approve_ix builds an Approve"),
        }
    }
}

/// A configuration a fixture can rotate into.
///
/// Held separately from `Fixture` because a rotation is not a mutation: both
/// configurations exist at once, at their own addresses, and a test usually
/// needs to reach for either one.
impl Rotation {
    /// `new_tiers` as the typed table.
    #[must_use]
    pub fn tier_table(&self) -> Vec<TierPolicy> {
        self.new_tiers
            .iter()
            .map(|&(max_amount, threshold)| TierPolicy {
                max_amount,
                threshold,
            })
            .collect()
    }
}

pub struct Rotation {
    pub new_member_root: [u8; 32],
    pub new_threshold: u32,
    pub new_tiers: Vec<(u128, u32)>,
    pub new_config_hash: [u8; 32],
    pub action_hash: [u8; 32],
    pub proposal_ref: [u8; 32],
}
