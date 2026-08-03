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
//!   msig propose --dir ms/ --proposal-id 01..01 --action "transfer 100 to alice"
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
    /// Register a proposal locally: bind an action to a proposal id.
    Propose {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        proposal_id: String,
        /// The action payload. Opaque to the protocol; bound by hash.
        #[arg(long)]
        action: String,
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
    action: String,
    action_hash_hex: String,
    proposal_ref_hex: String,
    /// Approvals generated so far. This is the resumable partial state.
    approvals: Vec<ApprovalRecord>,
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

fn new_multisig(members: usize, threshold: u32, id: Option<String>, out: &Path) -> Result<()> {
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

    let leaves: Vec<[u8; 32]> = secrets.iter().map(|s| s.4).collect();
    let (member_root, paths) = build_member_tree(&leaves);
    let config_hash = compute_config_hash(&member_root, threshold);

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
        },
    )?;
    write_json(&out.join("members.json"), &member_files)?;

    println!("multisig      {}-of-{}", threshold, members);
    println!("id            {}", hex::encode(multisig_id));
    println!("member root   {}", hex::encode(member_root));
    println!(
        "config hash   {}   (anchors root AND threshold)",
        hex::encode(config_hash)
    );
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
    ];
    std::fs::write(out, lines.join("\n") + "\n")?;
    println!("wrote {}", out.display());
    Ok(())
}

fn propose(dir: &Path, proposal_id: &str, action: &str) -> Result<()> {
    let ms: MultisigFile = read_json(&dir.join("multisig.json"))?;
    let multisig_id = hex32(&ms.id_hex)?;
    let config_hash = hex32(&ms.config_hash_hex)?;
    let pid = hex32(proposal_id)?;

    let action_hash = compute_action_hash(&multisig_id, action.as_bytes());
    let proposal_ref = compute_proposal_ref(&multisig_id, &config_hash, &pid, &action_hash);

    let path = proposal_path(dir, proposal_id);
    if path.exists() {
        let existing: ProposalFile = read_json(&path)?;
        if existing.action != action {
            bail!(
                "proposal {proposal_id} is already registered with a different action.\n\
                 That is not an error you can force past: changing the action changes\n\
                 proposal_ref, so the {} approval(s) already gathered do not carry over.\n\
                 Use a fresh proposal id.",
                existing.approvals.len()
            );
        }
        println!("proposal already registered, unchanged");
    } else {
        write_json(
            &path,
            &ProposalFile {
                proposal_id_hex: proposal_id.to_string(),
                action: action.to_string(),
                action_hash_hex: hex::encode(action_hash),
                proposal_ref_hex: hex::encode(proposal_ref),
                approvals: Vec::new(),
            },
        )?;
    }

    println!("proposal id   {proposal_id}");
    println!("action        {action}");
    println!("action hash   {}", hex::encode(action_hash));
    println!(
        "proposal ref  {}   (binds multisig + id + action)",
        hex::encode(proposal_ref)
    );
    println!("wrote         {}", path.display());
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
    println!("action        {}", p.action);
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

    if p.approvals.len() < ms.threshold as usize {
        bail!(
            "only {} of {} approvals gathered: the verifier would reject this with \
             E_THRESHOLD_NOT_MET (5010)",
            p.approvals.len(),
            ms.threshold
        );
    }

    // Present exactly `threshold` approvals. Presenting more would also pass,
    // but every extra marker account costs compute for no additional guarantee.
    let chosen = &p.approvals[..ms.threshold as usize];
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
        "approvals      {} (threshold {})",
        chosen.len(),
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
            id,
            out,
        } => new_multisig(members, threshold, id, &out),
        Cmd::CreateMultisigArgs { dir, out } => create_multisig_args(&dir, &out),
        Cmd::Propose {
            dir,
            proposal_id,
            action,
        } => propose(&dir, &proposal_id, &action),
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
        Cmd::ExecuteArgs {
            dir,
            proposal_id,
            out,
        } => execute_args(&dir, &proposal_id, &out),
    }
}
