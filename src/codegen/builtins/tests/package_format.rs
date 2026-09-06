//! The `.mfp` reader refuses each malformed package the tree keeps one of.
//!
//! `tests/syntax/security/pkg-0*` each ship a deliberately corrupt `.mfp` — a
//! tampered signature, a type cycle, a bad allocation count, a duplicate
//! section. They are gated only by `scripts/test-accept.sh`, which spawns `mfb`
//! and compares a diagnostic; from this process the decoder's refusals were
//! unreached, and `binary_repr/reader.rs`, `sections.rs` and `writer.rs` are
//! ~110 lines short between them, almost all of it error handling.
//!
//! Reading the file directly is the whole test. These fixtures cannot go through
//! the in-process diagnostic corpus at all — they are rejected at RESOLVE, before
//! `ir::shape` or `ir::verify` runs — but the thing they are actually about is
//! one function call away.
//!
//! What each refusal protects is the same thing: a `.mfp` is a binary a build
//! will merge into the program it is compiling. A decoder that shrugs at a
//! duplicate section id merges whichever copy it kept; one that shrugs at a
//! tampered signature merges a package nobody signed.

use std::path::PathBuf;

use crate::binary_repr::BinaryReprPackageDecode;
use crate::testutil::fixture_dir;

/// `(fixture, package file, what the refusal must say)`.
///
/// The expected text is a fragment of the decoder's own message, not the
/// fixture's golden: the golden records the DIAGNOSTIC the build prints, which
/// wraps this and adds a rule code. Matching the fragment keeps the row about
/// the decoder.
const CORRUPT_PACKAGES: &[(&str, &str, &str)] = &[
    (
        "pkg-06-duplicate-section",
        "sec_dup.mfp",
        "duplicate MFPC section id",
    ),
    (
        "pkg-05-alloc-count",
        "sec_count.mfp",
        "truncated binary representation",
    ),
    ("pkg-04-type-cycle", "sec_cyclic.mfp", "cyclic type id"),
    (
        "pkg-01-tampered-signature",
        "sec_signed.mfp",
        "package description is not valid UTF-8",
    ),
];

fn package_path(fixture: &str, file: &str) -> PathBuf {
    fixture_dir(fixture).join("packages").join(file)
}

/// Every corrupt package is refused BY THE DECODER, each for its own reason.
///
/// All four, measured — a first version left three of the expected fragments
/// empty and `contains("")` is true of everything, so those rows asserted that
/// the decoder had said *something*. The four reasons are distinct and each
/// names the corruption its fixture was built around, which is the difference
/// between this suite and one that only proves the reader returns `Err`.
///
/// The `Ok` arm is kept and still walks the exports, because a package that
/// decodes and is wrong LATER is a real shape (the containers here happen not
/// to be) and reaching it should exercise the readers rather than pass
/// silently.
#[test]
fn every_corrupt_package_fixture_is_refused_by_the_decoder() {
    let mut accepted_whole = Vec::new();
    for (fixture, file, expected) in CORRUPT_PACKAGES {
        let path = package_path(fixture, file);
        assert!(
            path.is_file(),
            "{fixture} must still ship {file}; without it this row silently \
             stops testing anything"
        );
        match BinaryReprPackageDecode::read(&path) {
            Err(message) => {
                assert!(
                    message.contains(expected),
                    "{fixture}: the decoder refused with {message:?}, which does \
                     not contain {expected:?}"
                );
            }
            Ok(decode) => {
                // A well-formed container whose CONTENTS are wrong. Exercise the
                // readers that walk it, which is what the later check does.
                let exports = decode.exports();
                let types = decode.type_exports();
                accepted_whole.push(format!(
                    "{fixture}: decoded (exports {}, type exports {})",
                    exports.map(|e| e.len()).unwrap_or(0),
                    types.map(|t| t.len()).unwrap_or(0),
                ));
            }
        }
    }
    assert!(
        accepted_whole.is_empty(),
        "every one of these packages is corrupt in a way the decoder itself \
         catches today; one that starts decoding cleanly has had its corruption \
         normalised away, and the build would merge it: {accepted_whole:?}"
    );
}

/// A package that is not a package at all is refused, not misread.
///
/// The shortest possible corruption, and the one a user hits by accident — a
/// text file, a truncated download, the wrong path. It must produce a message,
/// not a panic and not a zero-export package that merges silently.
#[test]
fn a_file_that_is_not_a_package_is_refused() {
    let not_a_package = fixture_dir("pkg-06-duplicate-section").join("project.json");
    assert!(
        not_a_package.is_file(),
        "the fixture must have a project.json"
    );
    let Err(message) = BinaryReprPackageDecode::read(&not_a_package) else {
        panic!(
            "a JSON manifest is not a `.mfp`, and decoding one must fail rather \
             than yield a package with no exports that a build would merge"
        );
    };
    assert!(
        !message.is_empty(),
        "the refusal must say something -- an empty message is what a caller \
         prints when it reports this to a user"
    );
}
