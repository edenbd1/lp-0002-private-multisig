//! The IDL and the error-code table are interfaces, so they are checked like
//! interfaces.
//!
//! `spel generate-idl` emits `errors: []` unconditionally, whatever the program
//! declares (`spel-framework-core/src/idl_gen.rs:228`). `scripts/idl-errors.py`
//! merges them back in from the guest's own `const E_*` block — but a generation
//! step that somebody forgets to run is exactly the kind of gap that survives a
//! review, so the committed artefact is checked here, by `cargo test`, which CI
//! runs whether or not anybody remembered.
//!
//! `docs/error-codes.md` is checked against the same source for the same reason:
//! it used to claim that every code was exercised, at a moment when three were
//! not.
//!
//! Run with: `cargo test -p multisig-verifier-tests --test idl_contract`

use std::collections::BTreeMap;

const VERIFIER_SRC: &str =
    "../../crates/multisig-verifier-spel/methods/guest/src/bin/multisig_verifier.rs";
const IDL: &str = "../../idl/multisig_verifier.idl.json";
const ERROR_DOC: &str = "../../docs/error-codes.md";

fn read(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Every `const E_NAME: u32 = 5NNN;` the guest declares, code to name.
fn declared_codes() -> BTreeMap<u32, String> {
    let src = read(VERIFIER_SRC);
    let mut out = BTreeMap::new();
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("const E_") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(": u32 = ") else {
            continue;
        };
        let code: u32 = value
            .trim_end_matches(';')
            .parse()
            .unwrap_or_else(|e| panic!("cannot parse the code in `{line}`: {e}"));
        let previous = out.insert(code, format!("E_{name}"));
        assert!(
            previous.is_none(),
            "code {code} is declared twice, as {previous:?} and E_{name}. \
             A code is a public interface; two meanings for one is a bug."
        );
    }
    assert!(
        !out.is_empty(),
        "found no error-code declarations in the verifier source"
    );
    out
}

#[test]
fn the_idl_carries_every_error_code_the_guest_declares() {
    let declared = declared_codes();
    let idl: serde_json::Value = serde_json::from_str(&read(IDL)).expect("the IDL must be JSON");

    let errors = idl
        .get("errors")
        .and_then(|e| e.as_array())
        .unwrap_or_else(|| {
            panic!(
                "the IDL carries no `errors` array. `spel generate-idl` never writes one; \
                 run scripts/idl-errors.py after regenerating it."
            )
        });

    let in_idl: BTreeMap<u32, String> = errors
        .iter()
        .map(|e| {
            let code = u32::try_from(e["code"].as_u64().expect("a code is a number"))
                .expect("an error code fits in u32");
            (
                code,
                e["name"].as_str().expect("a name is a string").to_string(),
            )
        })
        .collect();

    assert_eq!(
        in_idl, declared,
        "\nthe IDL's error codes and the guest's disagree.\n\
         Run: scripts/idl-errors.py\n"
    );

    for e in errors {
        let msg = e["msg"].as_str().unwrap_or("");
        assert!(
            !msg.is_empty(),
            "error {} has no message; a code without one tells an integrator nothing",
            e["code"]
        );
    }
}

#[test]
fn the_error_code_document_covers_every_code_the_guest_declares() {
    let declared = declared_codes();
    let doc = read(ERROR_DOC);
    let missing: Vec<u32> = declared
        .keys()
        .copied()
        .filter(|c| !doc.contains(&format!("`{c}`")))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/error-codes.md does not document {missing:?}. \
         An undocumented code is one an integrator meets for the first time in production."
    );
}

/// The account layouts are half the reason this IDL exists now: a client that
/// reads the chain needs to know what the bytes mean.
#[test]
fn the_idl_describes_every_record_the_program_writes() {
    let idl: serde_json::Value = serde_json::from_str(&read(IDL)).expect("the IDL must be JSON");
    let names: Vec<&str> = idl["accounts"]
        .as_array()
        .expect("the IDL must carry an `accounts` array")
        .iter()
        .map(|a| a["name"].as_str().expect("a name is a string"))
        .collect();
    for want in [
        "MultisigRecord",
        "TreasuryRecord",
        "ProposalRecord",
        "ApprovalMarkerRecord",
        "ExecutionMarkerRecord",
    ] {
        assert!(
            names.contains(&want),
            "the IDL does not describe {want}; it lists {names:?}"
        );
    }
}

/// The instruction set is an ABI: risc0 encodes the variant *index*, so
/// reordering these silently repoints every client's calls at a different
/// handler. Pinned in order, deliberately.
#[test]
fn the_instruction_order_is_the_abi_and_has_not_moved() {
    let idl: serde_json::Value = serde_json::from_str(&read(IDL)).expect("the IDL must be JSON");
    let names: Vec<&str> = idl["instructions"]
        .as_array()
        .expect("instructions")
        .iter()
        .map(|i| i["name"].as_str().expect("a name is a string"))
        .collect();
    assert_eq!(
        names,
        vec![
            "create_multisig",
            "fund_treasury",
            "create_proposal",
            "approve",
            "execute",
            // Appended, never inserted. The index is the wire ABI: putting a new
            // instruction anywhere but the end renumbers every one after it, and
            // this assertion is what caught exactly that during the change that
            // added it.
            "rotate_config",
        ],
        "the instruction order is the wire ABI; changing it changes every caller"
    );
}

/// `scripts/pda.py` derives the addresses every script and every reviewer uses.
/// If its `str:` padding disagreed with SPEL's `literal(...)`, the treasury
/// address in the docs would name an account nobody created — a false negative
/// on the one check that reads the chain.
#[test]
fn scripts_pda_py_derives_the_same_treasury_address_as_the_program() {
    let elf = multisig_verifier_tests::elf();
    let pid = multisig_verifier_tests::program_id(&elf);
    let f = multisig_verifier_tests::Fixture::new(&pid);

    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/pda.py");
    let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/programs/multisig_verifier.bin");
    let out = std::process::Command::new("python3")
        .arg(&script)
        .arg(&bin)
        .arg(hex::encode(f.multisig_id))
        .arg(hex::encode(f.config_hash))
        .arg("str:treasury")
        .output();
    let Ok(out) = out else {
        eprintln!("python3 unavailable; skipping the pda.py cross-check");
        return;
    };
    if !out.status.success() {
        // `pda.py` shells out to `spel` to read the ProgramId. Without it there
        // is nothing to compare, and reporting a pass would be worse than
        // reporting nothing.
        eprintln!(
            "pda.py could not run (spel on PATH?); skipping: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    let printed = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(
        printed,
        f.treasury_addr().to_string(),
        "scripts/pda.py and the program disagree about the treasury address"
    );
}

/// `scripts/decode-account.py` must decode the same records `docs/account-layout.md`
/// publishes.
///
/// That script is the one a reviewer actually runs: it reads an account off the
/// chain and prints it field by field, with no dependency on this repository's
/// Rust. `every_record_the_program_writes_decodes_at_the_published_offsets` ties
/// the *guest* to `multisig_core::state`, and nothing tied the standalone
/// decoder to either — so it could drift silently and the first person to notice
/// would be someone reading garbage off a live account.
///
/// Compared field for field rather than by total length, because two layouts can
/// sum to the same number while naming different things at every offset.
#[test]
fn the_standalone_decoder_reads_the_layout_the_document_publishes() {
    let script = read("../../scripts/decode-account.py");
    let doc = read("../../docs/account-layout.md");

    // (record kind in the script, heading in the document)
    for (kind, heading) in [
        ("multisig", "## `MultisigRecord`"),
        ("proposal", "## `ProposalRecord`"),
        ("treasury", "## `TreasuryRecord`"),
    ] {
        // The script's layout: ("name", width, renderer) tuples inside `"kind": [ … ]`.
        let start = script
            .find(&format!("\"{kind}\": ["))
            .unwrap_or_else(|| panic!("scripts/decode-account.py declares no {kind} layout"));
        let block = &script[start..start + script[start..].find("],").expect("closed layout")];
        let from_script: Vec<(String, usize)> = block
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                let inner = l.strip_prefix("(\"")?;
                let (name, rest) = inner.split_once("\", ")?;
                let width: usize = rest.split(',').next()?.trim().parse().ok()?;
                Some((name.to_string(), width))
            })
            .collect();

        // The document's table: `| offset | width | \`field\` | … |`.
        let dstart = doc
            .find(heading)
            .unwrap_or_else(|| panic!("docs/account-layout.md has no {heading} section"));
        let dblock = &doc[dstart..];
        let dblock = &dblock[..dblock[2..].find("\n## ").map_or(dblock.len(), |i| i + 2)];
        let from_doc: Vec<(String, usize)> = dblock
            .lines()
            .filter_map(|l| {
                let cells: Vec<&str> = l.split('|').map(str::trim).collect();
                if cells.len() < 5 {
                    return None;
                }
                let width: usize = cells[2].parse().ok()?;
                let name = cells[3].trim_matches('`');
                Some((name.to_string(), width))
            })
            .collect();

        assert!(
            !from_doc.is_empty() && !from_script.is_empty(),
            "{kind}: parsed {} field(s) from the document and {} from the script — \
             one of the two shapes this test keys on has changed, so it is asserting \
             nothing rather than passing",
            from_doc.len(),
            from_script.len()
        );
        assert_eq!(
            from_script, from_doc,
            "{kind}: scripts/decode-account.py and docs/account-layout.md disagree.\n\
             script: {from_script:?}\n\
             doc:    {from_doc:?}"
        );
    }
}
