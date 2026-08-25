//! `msig` — the LP-0002 client.
//!
//! Every command is offline. Nothing here talks to a sequencer: the CLI builds
//! member sets, derives commitments, and emits the exact argument files `spel`
//! consumes. `scripts/demo.sh` and `scripts/deploy-and-run.sh` wire those into
//! transactions.
//!
//! Typical flow:
//!
//!   msig new-multisig --members 5 --threshold 3 --out ms/
//!   msig create-multisig-args --dir ms/ --out ms/create.args
//!   msig fund-treasury-args --dir ms/ --amount 500 --out ms/fund.args
//!   msig propose --dir ms/ --proposal-id 01..01 \
//!        --recipient <account-id> --amount 250 --memo "pay the auditors"
//!   msig create-proposal-args --dir ms/ --proposal-id 01..01 --out ms/prop.args
//!   msig approve-args --dir ms/ --proposal-id 01..01 --member 0 --out ms/a0.args
//!   msig approve-args --dir ms/ --proposal-id 01..01 --member 3 --out ms/a3.args
//!   msig approve-args --dir ms/ --proposal-id 01..01 --member 4 --out ms/a4.args
//!   msig status  --dir ms/ --proposal-id 01..01
//!   msig execute-args --dir ms/ --proposal-id 01..01 --out ms/exec.args
//!
//! RESUMABILITY
//!
//! `approve-args` records each approval it generates in
//! `<dir>/proposals/<proposal_id>.json` before returning. That file is the
//! partial-approval state the LP-0002 reliability criterion asks to survive a
//! client restart: `status` and `execute-args` read it back, so a threshold can
//! be gathered across days, machine reboots, and separate member sessions. It
//! holds no secrets — only nullifiers and marker seeds, which are already public
//! once submitted.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use multisig_core::*;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "msig", about = "LP-0002 private M-of-N multisig client")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate a fresh member set and commit to (member_root, threshold).
    NewMultisig {
        #[arg(long)]
        members: usize,
        #[arg(long)]
        threshold: u32,
        /// A spending tier, `MAX_AMOUNT:THRESHOLD`, repeatable. Transfers at or
        /// below `MAX_AMOUNT` need `THRESHOLD` approvals instead of the default.
        /// A tier may only lower the bar: caps must strictly increase,
        /// thresholds must not fall, and none may be zero or above the default.
        #[arg(long = "tier", value_name = "MAX:THRESHOLD")]
        tiers: Vec<String>,
        /// 32-byte hex multisig id. Random if omitted.
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Emit `spel` args for `create_multisig`.
    CreateMultisigArgs {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Emit `spel` args for `fund_treasury`.
    FundTreasuryArgs {
        #[arg(long)]
        dir: PathBuf,
        /// How much to move into the treasury.
        #[arg(long)]
        amount: u128,
        #[arg(long)]
        out: PathBuf,
    },
    /// Print this multisig's treasury PDA seeds, for `scripts/pda.py`.
    TreasurySeeds {
        #[arg(long)]
        dir: PathBuf,
    },
    /// Register a proposal locally: bind a treasury payment to a proposal id.
    Propose {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        proposal_id: String,
        /// The account the treasury pays. 32 bytes, hex.
        #[arg(long)]
        recipient: String,
        /// How much it pays. Must be non-zero: a proposal that moves nothing
        /// gives the threshold nothing to gate.
        #[arg(long)]
        amount: u128,
        /// The sentence members read before approving. Bound by hash, like
        /// everything else, so approvals cannot be carried to a different one.
        #[arg(long)]
        memo: String,
    },
    /// Emit `spel` args for `create_proposal`.
    CreateProposalArgs {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        proposal_id: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Emit `spel` args for one member's `approve`, and record it locally.
    ApproveArgs {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        proposal_id: String,
        /// Member index in the generated set. Mutually exclusive with --msk.
        #[arg(long)]
        member: Option<usize>,
        /// A member's own secret, for a real member who holds only their key.
        #[arg(long)]
        msk: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Show how many approvals have been gathered against the threshold.
    Status {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        proposal_id: String,
    },
    /// Register a proposal to rotate into a new member set or threshold.
    ///
    /// A rotation does not mutate this multisig. It defines a second one, at its
    /// own address with its own treasury, and marks this one superseded — so the
    /// directory given to `--to` describes the configuration being moved *to*.
    ProposeRotation {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        proposal_id: String,
        /// Directory holding the configuration to move to, as written by
        /// `new-multisig`. Its multisig id must match this one's.
        #[arg(long)]
        to: PathBuf,
    },
    /// Emit `spel` args for `rotate_config`.
    RotateArgs {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        proposal_id: String,
        #[arg(long)]
        to: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Emit `spel` args for `execute`, once the threshold is reached.
    ExecuteArgs {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        proposal_id: String,
        #[arg(long)]
        out: PathBuf,
    },
}

// ---------------------------------------------------------------------------
// On-disk state
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct MultisigFile {
    id_hex: String,
    threshold: u32,
    member_count: usize,
    member_root_hex: String,
    config_hash_hex: String,
    /// Spending tiers, as `(max_amount, threshold)` pairs. Defaulted so a
    /// directory written before tiers existed still loads: absent means none,
    /// which is what those multisigs anchored.
    #[serde(default)]
    tiers: Vec<(u128, u32)>,
}

#[derive(Serialize, Deserialize, Clone)]
struct MemberFile {
    index: usize,
    msk_hex: String,
    identifier: u128,
    salt_hex: String,
    account_id_hex: String,
    leaf_hex: String,
    leaf_index: u64,
    merkle_path_hex: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct ProposalFile {
    proposal_id_hex: String,
    /// The human-readable sentence. Bound on chain by `memo_hash`, not stored
    /// there: a fixed-width record has no room for prose.
    memo: String,
    memo_hash_hex: String,
    recipient_hex: String,
    amount: u128,
    action_hash_hex: String,
    proposal_ref_hex: String,
    /// Approvals generated so far. This is the resumable partial state.
    approvals: Vec<ApprovalRecord>,
    /// Set when this proposal installs a configuration rather than paying: the
    /// `config_hash` it moves to. Defaulted, so proposals written before
    /// rotations existed still load as the transfers they are.
    #[serde(default)]
    rotate_to_hex: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ApprovalRecord {
    nullifier_hex: String,
    marker_seed_hex: String,
    /// Which member generated it, when known locally. Never leaves this file —
    /// it is not part of any argument, transaction, or on-chain record.
    member_index: Option<usize>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn hex32(s: &str) -> Result<[u8; 32]> {
    let raw = hex::decode(s.trim().trim_start_matches("0x"))
        .with_context(|| format!("`{s}` is not valid hex"))?;
    if raw.len() != 32 {
        bail!("expected 32 bytes, got {}", raw.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    Ok(out)
}

fn read_json<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("cannot parse {}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn proposal_path(dir: &Path, proposal_id: &str) -> PathBuf {
    dir.join("proposals").join(format!("{proposal_id}.json"))
}

fn quoted(b: &[u8; 32]) -> String {
    format!("'{}'", hex::encode(b))
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Parse `MAX:THRESHOLD` pairs and require the table the chain would require.
///
/// Validated with `multisig_core::validate_tiers` — the guest's own function,
/// not a copy of its rules — so a table this accepts is one the chain accepts,
/// and a table it refuses fails here rather than after a proof.
fn parse_tiers(specs: &[String]) -> Result<Vec<TierPolicy>> {
    let mut out = Vec::with_capacity(specs.len());
    for spec in specs {
        let (cap, threshold) = spec
            .split_once(':')
            .with_context(|| format!("tier '{spec}' is not MAX_AMOUNT:THRESHOLD"))?;
        out.push(TierPolicy {
            max_amount: cap
                .trim()
                .parse()
                .with_context(|| format!("tier '{spec}': '{cap}' is not an amount"))?,
            threshold: threshold
                .trim()
                .parse()
                .with_context(|| format!("tier '{spec}': '{threshold}' is not a threshold"))?,
        });
    }
    Ok(out)
}

/// The tier table as `spel` takes it on the command line.
///
/// SPEL reads a `Vec<u8>` as comma-separated decimals, so that is what this
/// emits — and the leading count byte is why "no tiers" is `0` here rather than
/// an empty string, which SPEL's serialiser rejects outright.
fn tiers_flag(tiers: &[TierPolicy]) -> String {
    let bytes = encode_tier_table(tiers);
    let list: Vec<String> = bytes.iter().map(u8::to_string).collect();
    format!("--tiers {}", list.join(","))
}

/// `MultisigFile::tiers` as the typed table.
fn tier_table(f: &MultisigFile) -> Vec<TierPolicy> {
    f.tiers
        .iter()
        .map(|&(max_amount, threshold)| TierPolicy {
            max_amount,
            threshold,
        })
        .collect()
}

fn new_multisig(
    members: usize,
    threshold: u32,
    tier_specs: &[String],
    id: Option<String>,
    out: &Path,
) -> Result<()> {
    if members == 0 {
        bail!("a multisig needs at least one member");
    }
    if threshold == 0 {
        bail!("threshold must be at least 1: a 0-of-N multisig would let anyone execute");
    }
    if threshold as usize > members {
        bail!("threshold {threshold} exceeds the {members} members: it could never be met");
    }

    let mut rng = rand::thread_rng();
    let multisig_id = match id {
        Some(s) => hex32(&s)?,
        None => {
            let mut b = [0u8; 32];
            rng.fill_bytes(&mut b);
            b
        }
    };

    // Generate each member's secret and per-entry salt. In a real deployment the
    // members generate their own `msk` and hand the creator only the derived
    // account id; the creator never needs the secret. This demo generates both
    // so the flow is runnable end to end by one person.
    let mut secrets = Vec::with_capacity(members);
    for i in 0..members {
        let mut msk = [0u8; 32];
        let mut salt = [0u8; 32];
        rng.fill_bytes(&mut msk);
        rng.fill_bytes(&mut salt);
        let identifier = (i as u128) + 1;
        let account_id = derive_account_id(&derive_npk(&msk), identifier);
        let leaf = compute_member_leaf(&account_id, &salt);
        secrets.push((msk, salt, identifier, account_id, leaf));
    }

    let tiers = parse_tiers(tier_specs)?;
    validate_tiers(&tiers, threshold).map_err(|e| {
        anyhow::anyhow!(
            "{e:?}: a tier may only lower the bar for small amounts — caps must \
             strictly increase, thresholds must not fall, and none may be zero or \
             above the default threshold of {threshold}. The verifier would refuse \
             this with E_BAD_TIERS (5023)."
        )
    })?;

    let leaves: Vec<[u8; 32]> = secrets.iter().map(|s| s.4).collect();
    let (member_root, paths) = build_member_tree(&leaves);
    let config_hash = compute_config_hash(&member_root, threshold, &compute_tiers_hash(&tiers));

    let member_files: Vec<MemberFile> = secrets
        .iter()
        .enumerate()
        .map(|(i, (msk, salt, identifier, account_id, leaf))| {
            let (leaf_index, siblings) = paths[i].clone();
            MemberFile {
                index: i,
                msk_hex: hex::encode(msk),
                identifier: *identifier,
                salt_hex: hex::encode(salt),
                account_id_hex: hex::encode(account_id),
                leaf_hex: hex::encode(leaf),
                leaf_index,
                merkle_path_hex: siblings.iter().map(hex::encode).collect(),
            }
        })
        .collect();

    write_json(
        &out.join("multisig.json"),
        &MultisigFile {
            id_hex: hex::encode(multisig_id),
            threshold,
            member_count: members,
            member_root_hex: hex::encode(member_root),
            config_hash_hex: hex::encode(config_hash),
            tiers: tiers.iter().map(|t| (t.max_amount, t.threshold)).collect(),
        },
    )?;
    write_json(&out.join("members.json"), &member_files)?;

    println!("multisig      {}-of-{}", threshold, members);
    println!("id            {}", hex::encode(multisig_id));
    println!("member root   {}", hex::encode(member_root));
    println!(
        "config hash   {}   (anchors root, threshold AND tiers)",
        hex::encode(config_hash)
    );
    for t in &tiers {
        println!(
            "tier          <= {} needs {} approvals",
            t.max_amount, t.threshold
        );
    }
    println!(
        "wrote         {}/multisig.json, {}/members.json",
        out.display(),
        out.display()
    );
    Ok(())
}

fn create_multisig_args(dir: &Path, out: &Path) -> Result<()> {
    let ms: MultisigFile = read_json(&dir.join("multisig.json"))?;
    let lines = [
        format!("--multisig-id {}", quoted(&hex32(&ms.id_hex)?)),
        format!("--config-hash {}", quoted(&hex32(&ms.config_hash_hex)?)),
        format!("--member-root {}", quoted(&hex32(&ms.member_root_hex)?)),
        format!("--threshold {}", ms.threshold),
        tiers_flag(&tier_table(&ms)),
    ];
    std::fs::write(out, lines.join("\n") + "\n")?;
    println!("wrote {}", out.display());
    Ok(())
}

/// Read the configuration a rotation moves to, and check it is one.
///
/// Both halves are refused here rather than on chain: a different multisig id is
/// not a rotation at all, and rotating to the configuration already in force
/// spends a threshold of approvals to change nothing (`E_NOOP_ROTATION`, 5026).
fn read_rotation_target(from: &MultisigFile, to: &Path) -> Result<MultisigFile> {
    let next: MultisigFile = read_json(&to.join("multisig.json"))?;
    if next.id_hex != from.id_hex {
        bail!(
            "that directory holds a different multisig ({} rather than {}). A rotation \
             moves one multisig between configurations; it cannot move it to another \
             multisig.",
            next.id_hex,
            from.id_hex
        );
    }
    if next.config_hash_hex == from.config_hash_hex {
        bail!(
            "that is the configuration already in force. The verifier refuses it with \
             E_NOOP_ROTATION (5026)."
        );
    }
    Ok(next)
}

fn propose_rotation(dir: &Path, proposal_id: &str, to: &Path) -> Result<()> {
    let ms: MultisigFile = read_json(&dir.join("multisig.json"))?;
    let next = read_rotation_target(&ms, to)?;
    let multisig_id = hex32(&ms.id_hex)?;
    let config_hash = hex32(&ms.config_hash_hex)?;
    let new_config_hash = hex32(&next.config_hash_hex)?;
    let pid = hex32(proposal_id)?;

    // A rotation names no recipient and moves nothing: the action it commits to
    // is the configuration being installed, and nothing else.
    let action_hash = compute_rotate_action_hash(&multisig_id, &new_config_hash);
    let proposal_ref = compute_proposal_ref(&multisig_id, &config_hash, &pid, &action_hash);

    let path = proposal_path(dir, proposal_id);
    if path.exists() {
        let existing: ProposalFile = read_json(&path)?;
        if existing.action_hash_hex != hex::encode(action_hash) {
            bail!(
                "proposal {proposal_id} is already registered with a different action.\n\
                 Changing what it does changes proposal_ref, so the {} approval(s)\n\
                 already gathered do not carry over. Use a fresh proposal id.",
                existing.approvals.len()
            );
        }
        println!("rotation proposal already registered, unchanged");
    } else {
        write_json(
            &path,
            &ProposalFile {
                proposal_id_hex: proposal_id.to_string(),
                memo: format!("rotate to {}-of-{}", next.threshold, next.member_count),
                memo_hash_hex: hex::encode([0u8; 32]),
                recipient_hex: hex::encode([0u8; 32]),
                amount: 0,
                action_hash_hex: hex::encode(action_hash),
                proposal_ref_hex: hex::encode(proposal_ref),
                approvals: Vec::new(),
                rotate_to_hex: Some(next.config_hash_hex.clone()),
            },
        )?;
    }

    println!("proposal id   {proposal_id}");
    println!(
        "rotates to    {}-of-{}, config {}",
        next.threshold, next.member_count, next.config_hash_hex
    );
    println!("action hash   {}", hex::encode(action_hash));
    println!(
        "proposal ref  {}   (scoped to the configuration that raised it)",
        hex::encode(proposal_ref)
    );
    println!(
        "costs         {} approvals — the default threshold, never a tier",
        ms.threshold
    );
    println!("wrote         {}", path.display());
    Ok(())
}

fn rotate_args(dir: &Path, proposal_id: &str, to: &Path, out: &Path) -> Result<()> {
    let ms: MultisigFile = read_json(&dir.join("multisig.json"))?;
    let next = read_rotation_target(&ms, to)?;
    let p: ProposalFile = read_json(&proposal_path(dir, proposal_id))?;

    let Some(rotate_to) = p.rotate_to_hex.as_deref() else {
        bail!(
            "proposal {proposal_id} is a transfer: spend it with `execute-args`. The \
             verifier refuses the other way round with E_WRONG_ACTION_KIND (5027)."
        );
    };
    if rotate_to != next.config_hash_hex {
        bail!(
            "this proposal approves a rotation to {rotate_to}, not to {}. Approvals are \
             bound to the action, so they do not carry to a different one.",
            next.config_hash_hex
        );
    }

    // Governance is priced at the default threshold, never at a tier: letting a
    // tier lower the bar for changing the member set would make the cheapest
    // action available the one that rewrites who may act.
    if p.approvals.len() < ms.threshold as usize {
        bail!(
            "only {} of {} approvals gathered: a rotation costs the default threshold \
             whatever the tiers say, and the verifier would reject this with \
             E_THRESHOLD_NOT_MET (5010)",
            p.approvals.len(),
            ms.threshold
        );
    }
    let chosen = &p.approvals[..ms.threshold as usize];
    let proposal_ref = hex32(&p.proposal_ref_hex)?;
    let exec_seed = compute_execution_marker(&proposal_ref);
    let nullifiers: Vec<String> = chosen.iter().map(|a| a.nullifier_hex.clone()).collect();
    let new_tiers = tiers_flag(&tier_table(&next));

    let lines = [
        format!("--multisig-id {}", quoted(&hex32(&ms.id_hex)?)),
        format!("--config-hash {}", quoted(&hex32(&ms.config_hash_hex)?)),
        format!("--member-root {}", quoted(&hex32(&ms.member_root_hex)?)),
        format!("--threshold {}", ms.threshold),
        tiers_flag(&tier_table(&ms)),
        format!(
            "--new-config-hash {}",
            quoted(&hex32(&next.config_hash_hex)?)
        ),
        format!(
            "--new-member-root {}",
            quoted(&hex32(&next.member_root_hex)?)
        ),
        format!("--new-threshold {}", next.threshold),
        format!("--new-tiers {}", &new_tiers["--tiers ".len()..]),
        format!("--proposal-ref {}", quoted(&proposal_ref)),
        format!("--approval-nullifiers '{}'", nullifiers.join(",")),
        format!("--execution-marker-seed {}", quoted(&exec_seed)),
    ];
    std::fs::write(out, lines.join("\n") + "\n")?;

    let markers: Vec<String> = chosen.iter().map(|a| a.marker_seed_hex.clone()).collect();
    std::fs::write(out.with_extension("markers"), markers.join("\n") + "\n")?;

    println!("rotates to    {}", next.config_hash_hex);
    println!(
        "approvals     {} (default threshold {})",
        chosen.len(),
        ms.threshold
    );
    println!("exec marker   {}", hex::encode(exec_seed));
    println!("wrote         {}", out.display());
    println!(
        "wrote         {}   (marker seeds, in nullifier order)",
        out.with_extension("markers").display()
    );
    Ok(())
}

fn propose(dir: &Path, proposal_id: &str, recipient: &str, amount: u128, memo: &str) -> Result<()> {
    let ms: MultisigFile = read_json(&dir.join("multisig.json"))?;
    let multisig_id = hex32(&ms.id_hex)?;
    let config_hash = hex32(&ms.config_hash_hex)?;
    let pid = hex32(proposal_id)?;
    let recipient_bytes = hex32(recipient)?;

    // Refused here rather than after minutes of proving: the on-chain program
    // rejects it with E_BAD_AMOUNT (5017), and so does the SDK.
    if amount == 0 {
        bail!("a proposal must move a non-zero amount: the threshold would gate nothing");
    }

    let memo_hash = compute_memo_hash(memo.as_bytes());
    let action_hash =
        compute_transfer_action_hash(&multisig_id, &recipient_bytes, amount, &memo_hash);
    let proposal_ref = compute_proposal_ref(&multisig_id, &config_hash, &pid, &action_hash);

    let path = proposal_path(dir, proposal_id);
    if path.exists() {
        let existing: ProposalFile = read_json(&path)?;
        if existing.action_hash_hex != hex::encode(action_hash) {
            bail!(
                "proposal {proposal_id} is already registered with a different action.\n\
                 That is not an error you can force past: changing the recipient, the\n\
                 amount or the memo changes proposal_ref, so the {} approval(s) already\n\
                 gathered do not carry over. Use a fresh proposal id.",
                existing.approvals.len()
            );
        }
        println!("proposal already registered, unchanged");
    } else {
        write_json(
            &path,
            &ProposalFile {
                proposal_id_hex: proposal_id.to_string(),
                memo: memo.to_string(),
                memo_hash_hex: hex::encode(memo_hash),
                recipient_hex: hex::encode(recipient_bytes),
                amount,
                action_hash_hex: hex::encode(action_hash),
                proposal_ref_hex: hex::encode(proposal_ref),
                approvals: Vec::new(),
                rotate_to_hex: None,
            },
        )?;
    }

    println!("proposal id   {proposal_id}");
    println!("memo          {memo}");
    println!("pays          {amount} to {}", hex::encode(recipient_bytes));
    println!("action hash   {}", hex::encode(action_hash));
    println!(
        "proposal ref  {}   (binds multisig + id + recipient + amount + memo)",
        hex::encode(proposal_ref)
    );
    println!("wrote         {}", path.display());
    Ok(())
}

fn fund_treasury_args(dir: &Path, amount: u128, out: &Path) -> Result<()> {
    let ms: MultisigFile = read_json(&dir.join("multisig.json"))?;
    if amount == 0 {
        bail!("funding nothing: the program rejects this with E_BAD_AMOUNT (5017)");
    }
    let lines = [
        format!("--multisig-id {}", quoted(&hex32(&ms.id_hex)?)),
        format!("--config-hash {}", quoted(&hex32(&ms.config_hash_hex)?)),
        format!("--amount {amount}"),
    ];
    std::fs::write(out, lines.join("\n") + "\n")?;
    println!("wrote {}", out.display());
    Ok(())
}

/// Print the treasury PDA's three seeds, ready for `scripts/pda.py`.
///
/// The address needs the verifier's ProgramId, which this client deliberately
/// does not hold: a caller derives it from the binary they intend to call, so
/// the address they compute belongs to the program they are actually talking to.
fn treasury_seeds(dir: &Path) -> Result<()> {
    let ms: MultisigFile = read_json(&dir.join("multisig.json"))?;
    let built = multisig_sdk::Multisig::new(
        hex32(&ms.id_hex)?,
        hex32(&ms.member_root_hex)?,
        ms.threshold,
    )?
    .with_tiers(&tier_table(&ms))?;

    // The seeds are an address, and an address computed from the wrong
    // configuration is not a near miss — it is a different account. This
    // rebuilt the commitment from the file's members and threshold and forgot
    // its tiers, so a tiered multisig reported the treasury of the untiered one:
    // a real address, owned by nobody, that reads as an empty account. Nothing
    // failed; the balances were simply read from somewhere else. So the rebuilt
    // commitment is now checked against the one the file records, and a
    // disagreement stops here rather than becoming a number in a document.
    if hex::encode(built.config_hash()) != ms.config_hash_hex {
        bail!(
            "the configuration in {} does not hash to the config_hash it records.\n\
             recorded:   {}\n\
             recomputed: {}\n\
             Every address this multisig uses derives from that value, so nothing \
             downstream would be pointing at the right account.",
            dir.join("multisig.json").display(),
            ms.config_hash_hex,
            hex::encode(built.config_hash())
        );
    }
    let seeds = built.treasury_seeds();
    println!(
        "{} {} {}",
        hex::encode(seeds[0]),
        hex::encode(seeds[1]),
        hex::encode(seeds[2])
    );
    Ok(())
}

fn create_proposal_args(dir: &Path, proposal_id: &str, out: &Path) -> Result<()> {
    let ms: MultisigFile = read_json(&dir.join("multisig.json"))?;
    let p: ProposalFile = read_json(&proposal_path(dir, proposal_id))?;
    let lines = [
        format!("--multisig-id {}", quoted(&hex32(&ms.id_hex)?)),
        format!("--config-hash {}", quoted(&hex32(&ms.config_hash_hex)?)),
        format!("--proposal-id {}", quoted(&hex32(&p.proposal_id_hex)?)),
        format!("--action-hash {}", quoted(&hex32(&p.action_hash_hex)?)),
        format!("--proposal-ref {}", quoted(&hex32(&p.proposal_ref_hex)?)),
        format!("--recipient {}", quoted(&hex32(&p.recipient_hex)?)),
        format!("--amount {}", p.amount),
        format!("--memo-hash {}", quoted(&hex32(&p.memo_hash_hex)?)),
        format!(
            "--rotate-to {}",
            quoted(&match &p.rotate_to_hex {
                Some(h) => hex32(h)?,
                None => [0u8; 32],
            })
        ),
    ];
    std::fs::write(out, lines.join("\n") + "\n")?;
    println!("wrote {}", out.display());
    Ok(())
}

fn approve_args(
    dir: &Path,
    proposal_id: &str,
    member: Option<usize>,
    msk_hex: Option<String>,
    out: &Path,
) -> Result<()> {
    let ms: MultisigFile = read_json(&dir.join("multisig.json"))?;
    let members: Vec<MemberFile> = read_json(&dir.join("members.json"))?;
    let ppath = proposal_path(dir, proposal_id);
    let mut p: ProposalFile = read_json(&ppath)?;

    let member_root = hex32(&ms.member_root_hex)?;
    let proposal_ref = hex32(&p.proposal_ref_hex)?;

    // Resolve the approving member, either by index (demo) or by their own
    // secret (a real member, who has no reason to hold anyone else's data).
    let m: MemberFile = match (member, msk_hex.as_deref()) {
        (Some(i), None) => members
            .get(i)
            .cloned()
            .with_context(|| format!("no member at index {i}"))?,
        (None, Some(h)) => {
            let msk = hex32(h)?;
            let npk = derive_npk(&msk);
            members
                .iter()
                .find(|m| {
                    hex32(&m.account_id_hex)
                        .map(|id| id == derive_account_id(&npk, m.identifier))
                        .unwrap_or(false)
                })
                .cloned()
                .context("that secret does not correspond to any member of this set")?
        }
        _ => bail!("pass exactly one of --member or --msk"),
    };

    let msk = hex32(&m.msk_hex)?;
    let salt = hex32(&m.salt_hex)?;
    let merkle_path: Vec<[u8; 32]> = m
        .merkle_path_hex
        .iter()
        .map(|s| hex32(s))
        .collect::<Result<_>>()?;

    let nullifier = compute_approval_nullifier(&proposal_ref, &msk);
    let marker_seed = compute_approval_marker(&proposal_ref, &nullifier);

    // Refuse a second approval by the same member rather than let the chain
    // reject it after minutes of proving. Same outcome, minutes earlier.
    if p.approvals
        .iter()
        .any(|a| a.nullifier_hex == hex::encode(nullifier))
    {
        bail!(
            "this member has already approved this proposal.\n\
             The on-chain marker PDA is already occupied, so a second approval\n\
             would be rejected by the verifier."
        );
    }

    let witness = ApproveWitness {
        msk,
        identifier: m.identifier,
        salt,
        merkle_path,
        leaf_index: m.leaf_index,
    };
    let statement = ApproveStatement {
        member_root,
        proposal_ref,
        nullifier,
    };

    // Sanity: prove locally that the witness satisfies the statement before
    // asking the chain to. Fails loudly here rather than after minutes of
    // proving, which is the Reliability criterion on graceful proof failure.
    let leaf = approve(&witness, &statement)
        .map_err(|e| anyhow::anyhow!("witness does not satisfy the statement: {e:?}"))?;

    // Encode the witness exactly as the guest reads it.
    let instruction = ApproveInstruction { witness, statement };
    let words: Vec<u32> = risc0_zkvm::serde::to_vec(&instruction)?;

    let lines = [
        format!(
            "--witness-words '{}'",
            words
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        format!("--multisig-id {}", quoted(&hex32(&ms.id_hex)?)),
        format!("--config-hash {}", quoted(&hex32(&ms.config_hash_hex)?)),
        format!("--member-root {}", quoted(&member_root)),
        format!("--threshold {}", ms.threshold),
        tiers_flag(&tier_table(&ms)),
        format!("--proposal-ref {}", quoted(&proposal_ref)),
        format!("--nullifier {}", quoted(&nullifier)),
        format!("--approval-marker-seed {}", quoted(&marker_seed)),
    ];
    std::fs::write(out, lines.join("\n") + "\n")?;

    // Record before returning, so an interrupted session loses nothing.
    p.approvals.push(ApprovalRecord {
        nullifier_hex: hex::encode(nullifier),
        marker_seed_hex: hex::encode(marker_seed),
        member_index: Some(m.index),
    });
    let gathered = p.approvals.len();
    write_json(&ppath, &p)?;

    println!("member leaf   {}", hex::encode(leaf));
    println!("nullifier     {}", hex::encode(nullifier));
    println!(
        "marker seed   {}   (the approval marker PDA seed)",
        hex::encode(marker_seed)
    );
    println!("witness       {} u32 words", words.len());
    println!("gathered      {}/{}", gathered, ms.threshold);
    println!("wrote         {}", out.display());
    Ok(())
}

fn status(dir: &Path, proposal_id: &str) -> Result<()> {
    let ms: MultisigFile = read_json(&dir.join("multisig.json"))?;
    let p: ProposalFile = read_json(&proposal_path(dir, proposal_id))?;
    let n = p.approvals.len();
    let t = ms.threshold as usize;

    println!("proposal      {}", p.proposal_id_hex);
    println!("memo          {}", p.memo);
    println!("pays          {} to {}", p.amount, p.recipient_hex);
    println!("proposal ref  {}", p.proposal_ref_hex);
    println!("threshold     {}-of-{}", ms.threshold, ms.member_count);
    println!(
        "gathered      {n}/{t}{}",
        if n >= t { "  READY TO EXECUTE" } else { "" }
    );
    for (i, a) in p.approvals.iter().enumerate() {
        println!("  approval {i}   marker {}", a.marker_seed_hex);
    }
    if n < t {
        println!("\nneed {} more approval(s)", t - n);
    }
    Ok(())
}

fn execute_args(dir: &Path, proposal_id: &str, out: &Path) -> Result<()> {
    let ms: MultisigFile = read_json(&dir.join("multisig.json"))?;
    let p: ProposalFile = read_json(&proposal_path(dir, proposal_id))?;
    let proposal_ref = hex32(&p.proposal_ref_hex)?;

    if p.rotate_to_hex.is_some() {
        bail!(
            "this proposal is a rotation: spend it with `rotate-args`, not \
             `execute-args`. The verifier refuses the other way round with \
             E_WRONG_ACTION_KIND (5027)."
        );
    }
    // A tier can lower what this transfer costs. Gathering the default when a
    // tier asks for two is minutes of proving spent for nothing; presenting two
    // when no tier covers the amount is a rejection on chain.
    let tiers = tier_table(&ms);
    let need = required_threshold(p.amount, ms.threshold, &tiers);
    if p.approvals.len() < need as usize {
        bail!(
            "only {} of {} approvals gathered for an amount of {}: the verifier \
             would reject this with E_THRESHOLD_NOT_MET (5010)",
            p.approvals.len(),
            need,
            p.amount
        );
    }

    // Present exactly `threshold` approvals. Presenting more would also pass,
    // but every extra marker account costs compute for no additional guarantee.
    let chosen = &p.approvals[..need as usize];
    let nullifiers: Vec<String> = chosen
        .iter()
        .map(|a| a.nullifier_hex.clone())
        .collect::<Vec<_>>();
    let exec_seed = compute_execution_marker(&proposal_ref);

    let lines = [
        format!("--multisig-id {}", quoted(&hex32(&ms.id_hex)?)),
        format!("--config-hash {}", quoted(&hex32(&ms.config_hash_hex)?)),
        format!("--member-root {}", quoted(&hex32(&ms.member_root_hex)?)),
        format!("--threshold {}", ms.threshold),
        tiers_flag(&tiers),
        format!("--proposal-ref {}", quoted(&proposal_ref)),
        format!("--approval-nullifiers '{}'", nullifiers.join(",")),
        format!("--execution-marker-seed {}", quoted(&exec_seed)),
    ];
    std::fs::write(out, lines.join("\n") + "\n")?;

    // The marker PDAs the executor must pass as the trailing `approvals`
    // accounts, in the same order as the nullifiers above.
    let markers: Vec<String> = chosen.iter().map(|a| a.marker_seed_hex.clone()).collect();
    std::fs::write(out.with_extension("markers"), markers.join("\n") + "\n")?;

    println!("proposal ref   {}", hex::encode(proposal_ref));
    println!(
        "approvals      {} (needs {}, default {})",
        chosen.len(),
        need,
        ms.threshold
    );
    println!("exec marker    {}", hex::encode(exec_seed));
    println!("wrote          {}", out.display());
    println!(
        "wrote          {}   (marker seeds, in nullifier order)",
        out.with_extension("markers").display()
    );
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::NewMultisig {
            members,
            threshold,
            tiers,
            id,
            out,
        } => new_multisig(members, threshold, &tiers, id, &out),
        Cmd::CreateMultisigArgs { dir, out } => create_multisig_args(&dir, &out),
        Cmd::FundTreasuryArgs { dir, amount, out } => fund_treasury_args(&dir, amount, &out),
        Cmd::TreasurySeeds { dir } => treasury_seeds(&dir),
        Cmd::Propose {
            dir,
            proposal_id,
            recipient,
            amount,
            memo,
        } => propose(&dir, &proposal_id, &recipient, amount, &memo),
        Cmd::CreateProposalArgs {
            dir,
            proposal_id,
            out,
        } => create_proposal_args(&dir, &proposal_id, &out),
        Cmd::ApproveArgs {
            dir,
            proposal_id,
            member,
            msk,
            out,
        } => approve_args(&dir, &proposal_id, member, msk, &out),
        Cmd::Status { dir, proposal_id } => status(&dir, &proposal_id),
        Cmd::ProposeRotation {
            dir,
            proposal_id,
            to,
        } => propose_rotation(&dir, &proposal_id, &to),
        Cmd::RotateArgs {
            dir,
            proposal_id,
            to,
            out,
        } => rotate_args(&dir, &proposal_id, &to, &out),
        Cmd::ExecuteArgs {
            dir,
            proposal_id,
            out,
        } => execute_args(&dir, &proposal_id, &out),
    }
}
