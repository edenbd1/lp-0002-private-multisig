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
