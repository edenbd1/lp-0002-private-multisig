//! The verifier hard-codes `MEMBERSHIP_LEZ_PROGRAM_ID` so that a chained call
//! can only ever reach the audited membership binary. If that constant and the
//! committed binary drift apart, the verifier would compose a proof from some
//! other program — which is a security bug, not a build nuisance.
//!
//! This test computes the id from the committed binary and compares it against
//! the constant parsed out of the verifier's source. Both are files in this
//! repository, so the check needs no toolchain beyond cargo: no `spel`, no
//! Docker, no network. That is why it lives here rather than only in
//! `scripts/build-programs.sh`.
//!
//! Run with: `cargo test -p multisig-verifier-tests --test program_id_pin`

use lee_core::program::ProgramId;

const MEMBERSHIP_BIN: &str = "../../artifacts/programs/membership_lez.bin";
const VERIFIER_SRC: &str =
    "../../crates/multisig-verifier-spel/methods/guest/src/bin/multisig_verifier.rs";

fn repo_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// The eight `u32` words the verifier source pins.
fn pinned_id() -> ProgramId {
    let path = repo_path(VERIFIER_SRC);
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let start = src
        .find("pub const MEMBERSHIP_LEZ_PROGRAM_ID")
        .expect("the verifier must declare MEMBERSHIP_LEZ_PROGRAM_ID");
    let rest = &src[start..];
    let end = rest
        .find("];")
        .expect("the constant must be a closed array")
        + 2;
    let decl = &rest[..end];

    // Everything after the `=` is the array literal; the type annotation before
    // it contains no digits long enough to be confused for a word.
    let body = decl
        .split('=')
        .nth(1)
        .expect("the constant must be assigned");
    let words: Vec<u32> = body
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u32>().expect("word must fit u32"))
        .collect();

    assert_eq!(
        words.len(),
        8,
        "a ProgramId is 8 u32 words, found {} in:\n{decl}",
        words.len()
    );
    let mut id = ProgramId::default();
    id.copy_from_slice(&words);
    id
}

/// The id of the committed membership binary.
fn built_id() -> ProgramId {
    let path = repo_path(MEMBERSHIP_BIN);
    let elf = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}. Build it with:\n  ./scripts/build-programs.sh",
            path.display()
        )
    });
    risc0_binfmt::ProgramBinary::decode(&elf)
        .expect("the membership binary must decode")
        .compute_image_id()
        .expect("image id")
        .into()
}

#[test]
fn the_verifier_pins_the_committed_membership_binary() {
    let pinned = pinned_id();
    let built = built_id();
    assert_eq!(
        pinned, built,
        "\nMEMBERSHIP_LEZ_PROGRAM_ID drift.\n\
         \n  pinned in the verifier source:\n    {pinned:?}\
         \n  computed from artifacts/programs/membership_lez.bin:\n    {built:?}\
         \n\nThe verifier would chain to a binary other than the one committed.\n\
         Update the constant to the built value and rebuild the verifier with\n\
         ./scripts/build-programs.sh\n"
    );
}

/// A pin of all zeros would silently disable the guarantee, so reject it
/// explicitly rather than relying on the comparison above to catch it.
#[test]
fn the_pin_is_not_a_placeholder() {
    assert_ne!(
        pinned_id(),
        ProgramId::default(),
        "MEMBERSHIP_LEZ_PROGRAM_ID is still the all-zero placeholder"
    );
}

// ---------------------------------------------------------------------------
// The ImageIDs the documentation quotes
// ---------------------------------------------------------------------------

/// Every ImageID quoted in a document must be one this checkout builds.
///
/// `scripts/check-quoted-hashes.py` cannot do this: it compares a `SHA256` of a
/// file against a digest written beside its path, and an ImageID is a different
/// function of the same bytes — RISC0 derives it from the guest's memory image,
/// not from the file. So a document could name an ImageID no binary here
/// produces and every existing gate would stay green. Quoting an identifier the
/// branch does not build is a defect this repository has published before.
///
/// **Which 64-hex strings count.** Not all of them: a deploy transaction hash is
/// also 64 hex and is a different thing entirely, and `docs/DEPLOYMENT.md` prints
/// the same identity twice per row — once as an ImageID and once as a ProgramId,
/// which is the same words in a different byte order. Checking every hex string
/// would fail on values that are correct. So three shapes are recognised, and
/// only three:
///
///   - a line that names ImageID and carries one,
///   - the line *after* one that names ImageID, which is where a wrapped
///     sentence puts it,
///   - a table cell under a header column whose name contains ImageID.
///
/// The last is why this is column-aware rather than line-aware: the naive
/// version found one of the four places this repository quotes an ImageID, and
/// would have reported a pass it had not earned.
#[test]
fn every_image_id_the_docs_quote_is_one_this_checkout_builds() {
    use multisig_verifier_tests::{elf, program_id};

    // A `ProgramId` is `[u32; 8]`. The "ImageID (hex bytes)" form `spel` prints —
    // and the one the documents quote — is each word's little-endian bytes in
    // order, so that is what this rebuilds rather than formatting the words.
    let as_hex = |words: lee_core::program::ProgramId| -> String {
        hex::encode(words.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>())
    };
    let verifier = as_hex(program_id(&elf()));
    let membership_bytes =
        std::fs::read(repo_path(MEMBERSHIP_BIN)).expect("the committed membership binary");
    let membership = as_hex(program_id(&membership_bytes));

    // ImageIDs a document names *on purpose* while saying they are not current.
    // A repository that cannot mention a retired identity cannot explain its own
    // history, so these are allowed — with a reason, and with the same rule the
    // other checkers here use: an allowance that matches nothing is itself a
    // failure, because a stale exemption is how a real drift gets waved through.
    const RETIRED: &[(&str, &str)] = &[
        (
            "1346b65293ac9b11d4b1029a0d02559462238582124062925a3ad24298ff4e1e",
            "docs/DEPLOYMENT.md records the deployment the 2026-08-25 redeploy \
             replaced — the one whose config_hash anchored no tiers. Its accounts \
             are still on chain and still readable, and naming the ImageID is how \
             a reader following a stale link learns which of them they landed on",
        ),
        (
        "5bb4008273ddc31d1c2b5bad8835daaf4c567e029dbb059c20c7e83ba5966f82",
        "docs/DEPLOYMENT.md explains that the accounts of an earlier deployment \
         belong to this verifier, which the repository no longer contains — the \
         point of the passage is that the identity changed",
    )];

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut checked = 0usize;
    let mut wrong: Vec<String> = Vec::new();
    let mut retired_seen = vec![false; RETIRED.len()];

    let names_image_id = |l: &str| {
        let l = l.to_ascii_lowercase();
        l.contains("imageid") || l.contains("image id")
    };
    let cells = |l: &str| -> Vec<String> {
        l.trim().trim_matches('|').split('|').map(|c| c.trim().to_string()).collect()
    };

    for doc in ["README.md", "docs/DEPLOYMENT.md", "docs/cu-costs.md",
                "docs/security.md", "docs/account-layout.md", "docs/error-codes.md"] {
        let Ok(text) = std::fs::read_to_string(root.join(doc)) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        // Column indices of the table currently in scope, if its header names an
        // ImageID. Cleared by any line that is not a table row.
        let mut image_id_columns: Vec<usize> = Vec::new();

        for (n, line) in lines.iter().enumerate() {
            let is_row = line.trim_start().starts_with('|');
            if !is_row {
                image_id_columns.clear();
            } else if names_image_id(line) && image_id_columns.is_empty() {
                image_id_columns = cells(line)
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| names_image_id(c))
                    .map(|(i, _)| i)
                    .collect();
                continue;             // the header itself carries no value
            }

            let mut candidates: Vec<String> = Vec::new();
            if is_row && !image_id_columns.is_empty() {
                let row = cells(line);
                for &i in &image_id_columns {
                    if let Some(c) = row.get(i) {
                        candidates.push(c.clone());
                    }
                }
            } else if names_image_id(line)
                || (n > 0 && names_image_id(lines[n - 1]) && !lines[n - 1].trim_start().starts_with('|'))
            {
                candidates.push((*line).to_string());
            }

            for text in candidates {
                for token in text.split(|c: char| !c.is_ascii_hexdigit()) {
                    if token.len() != 64 {
                        continue;
                    }
                    checked += 1;
                    if let Some(i) = RETIRED.iter().position(|(h, _)| *h == token) {
                        retired_seen[i] = true;
                        continue;
                    }
                    if token != verifier && token != membership {
                        wrong.push(format!("{doc}:{}: {token}", n + 1));
                    }
                }
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "these documents quote an ImageID this checkout does not build.\n\
         verifier   {verifier}\n\
         membership {membership}\n\
         {}",
        wrong.join("\n")
    );
    for (i, (hash, why)) in RETIRED.iter().enumerate() {
        assert!(
            retired_seen[i],
            "the allowance for {hash} matched nothing. Either the passage it was \
             written for is gone — in which case delete the allowance rather than \
             leave it standing over nothing — or the shape this test recognises \
             has changed and the allowance is now hiding a real drift.\nreason              given: {why}"
        );
    }

    assert!(
        checked >= 2,
        "only {checked} ImageID(s) examined. This repository quotes several, so \
         either the documents stopped naming them — in which case delete this test \
         rather than leave it reporting a pass it did not earn — or one of the \
         three shapes it recognises has been reworded."
    );
}
