//! Two reliability criteria, exercised the way the brief phrases them.
//!
//! *"A partial set of approvals (fewer than M) is preserved and resumable
//! across client restarts"* and *"the system handles proof generation failures
//! gracefully and surfaces a clear error to the member."*
//!
//! Both were true and neither was tested. They are asserted here through the
//! **built binary**, one process per step, because "across client restarts" is
//! a claim about processes: an in-memory round-trip would pass while a broken
//! `proposals/<id>.json` writer shipped.
//!
//! No proving happens here. `approve-args` builds and locally re-verifies a
//! witness, which is the point of the second test: a member who cannot satisfy
//! the statement finds out in milliseconds rather than after two and a half
//! minutes of proving.

use std::path::Path;
use std::process::{Command, Output};

const MSIG: &str = env!("CARGO_BIN_EXE_msig");
const PROPOSAL: &str = "00000000000000000000000000000000000000000000000000000000000000ab";
const ACTION: &str = "transfer 100 LEZ to the grants treasury";

/// One CLI invocation — a fresh process, which is what "restart" means.
fn run(args: &[&str]) -> Output {
    Command::new(MSIG)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("could not run {MSIG}: {e}"))
}

fn ok(args: &[&str]) -> String {
    let out = run(args);
    assert!(
        out.status.success(),
        "msig {args:?} failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn gathered(dir: &Path) -> String {
    let out = ok(&[
        "status",
        "--dir",
        dir.to_str().unwrap(),
        "--proposal-id",
        PROPOSAL,
    ]);
    out.lines()
        .find(|l| l.trim_start().starts_with("gathered"))
        .unwrap_or_else(|| panic!("no `gathered` line in status output:\n{out}"))
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned()
}

#[test]
fn a_partial_set_of_approvals_survives_client_restarts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    let d = dir.to_str().unwrap();

    // Each of these is a separate process. Nothing is carried in memory
    // between them; every step reads the state the previous one wrote.
    ok(&[
        "new-multisig",
        "--members",
        "5",
        "--threshold",
        "3",
        "--id",
        "00000000000000000000000000000000000000000000000000000000000000cd",
        "--out",
        d,
    ]);
    ok(&[
        "propose",
        "--dir",
        d,
        "--proposal-id",
        PROPOSAL,
        "--action",
        ACTION,
    ]);

    assert_eq!(gathered(dir), "0/3", "a fresh proposal has no approvals");

    // Member 0 approves, then the "client" exits.
    ok(&[
        "approve-args",
        "--dir",
        d,
        "--proposal-id",
        PROPOSAL,
        "--member",
        "0",
        "--out",
        &format!("{d}/a0.args"),
    ]);
    assert_eq!(gathered(dir), "1/3", "one approval survived the restart");

    // A different member, on what is for this purpose a different day.
    ok(&[
        "approve-args",
        "--dir",
        d,
        "--proposal-id",
        PROPOSAL,
        "--member",
        "3",
        "--out",
        &format!("{d}/a3.args"),
    ]);
    let two = gathered(dir);
    assert_eq!(two, "2/3", "the partial set accumulated across processes");

    // Still short of the threshold, so it must not claim otherwise.
    let status = ok(&["status", "--dir", d, "--proposal-id", PROPOSAL]);
    assert!(
        !status.contains("READY TO EXECUTE"),
        "2 of 3 is not ready to execute:\n{status}"
    );

    // The state lives in a file, not in a process.
    let record = dir.join("proposals").join(format!("{PROPOSAL}.json"));
    assert!(record.is_file(), "no resumable record at {record:?}");

    // Reaching the threshold flips it, and only then.
    ok(&[
        "approve-args",
        "--dir",
        d,
        "--proposal-id",
        PROPOSAL,
        "--member",
        "4",
        "--out",
        &format!("{d}/a4.args"),
    ]);
    assert_eq!(gathered(dir), "3/3");
    let status = ok(&["status", "--dir", d, "--proposal-id", PROPOSAL]);
    assert!(
        status.contains("READY TO EXECUTE"),
        "3 of 3 should be ready:\n{status}"
    );
}

#[test]
fn a_witness_that_cannot_satisfy_the_statement_fails_before_proving() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    let d = dir.to_str().unwrap();

    ok(&[
        "new-multisig",
        "--members",
        "5",
        "--threshold",
        "3",
        "--id",
        "00000000000000000000000000000000000000000000000000000000000000ef",
        "--out",
        d,
    ]);
    ok(&[
        "propose",
        "--dir",
        d,
        "--proposal-id",
        PROPOSAL,
        "--action",
        ACTION,
    ]);

    // A secret that belongs to nobody in this member set.
    let out = run(&[
        "approve-args",
        "--dir",
        d,
        "--proposal-id",
        PROPOSAL,
        "--msk",
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        "--out",
        &format!("{d}/nope.args"),
    ]);

    assert!(
        !out.status.success(),
        "a non-member must not get approval arguments"
    );
    let msg = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Clear to a member, not a stack trace. The CLI catches this before it even
    // builds a witness and says so in plain words — "that secret does not
    // correspond to any member of this set" — rather than surfacing the
    // circuit's `NotAMember`. Either is acceptable; a panic or a bare exit code
    // is not.
    let clear = msg.contains("does not correspond to any member")
        || msg.contains("witness does not satisfy the statement")
        || msg.contains("NotAMember");
    assert!(clear, "the refusal should say what failed, got:\n{msg}");
    assert!(
        !msg.contains("panicked"),
        "a non-member is an expected outcome, not a panic:\n{msg}"
    );

    // And it must not have left a half-written artefact behind for `spel` to
    // pick up and spend two and a half minutes proving.
    assert!(
        !dir.join("nope.args").exists(),
        "a refused approval must not write arguments"
    );
    assert_eq!(gathered(dir), "0/3", "a refused approval changes no state");
}
