//! Guard: every test file in a `tests/` SUBDIRECTORY is actually registered as
//! a Cargo test target.
//!
//! Cargo auto-discovers `tests/*.rs` and `tests/<dir>/main.rs`, and nothing
//! else. A `.rs` file sitting in `tests/cli/` or `tests/interop/` is invisible
//! to it unless a `[[test]]` stanza in `Cargo.toml` names the path — and the
//! failure mode is the worst kind there is: the file compiles nowhere, runs
//! never, and the suite stays green. Nobody notices a test that was never run.
//!
//! So the registration is checked here rather than remembered. Add a file to
//! `tests/cli/` without its stanza and THIS test fails, with the exact TOML to
//! paste. That is the whole point: the invariant is enforced by the suite, not
//! by a comment asking the next person to be careful.
//!
//! Why stanzas at all, rather than a `tests/<dir>/main.rs` that pulls each file
//! in as a module: `main.rs` IS auto-discovered, but it would merge every file
//! in the directory into one binary under one target name, and forgetting a
//! `mod` line there is exactly as silent as forgetting a stanza. The stanzas
//! keep one binary per test and keep `cargo test --test cli_repo_publish`
//! working, which ~40 plan and bug documents quote.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Directories under `tests/` whose `.rs` files are deliberately NOT test
/// targets. `common` is the shared helper module every integration test pulls in
/// with `#[path = "../common/mod.rs"] mod common;`.
const NOT_TEST_TARGETS: &[&str] = &["common"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `tests/<dir>/*.rs` that ought to be a registered target, as paths
/// relative to the manifest directory and written with `/` so they compare
/// directly against what `Cargo.toml` spells.
fn test_files_in_subdirectories() -> BTreeSet<String> {
    let tests = repo_root().join("tests");
    let mut found = BTreeSet::new();
    let entries = std::fs::read_dir(&tests).expect("read tests/");
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let dir_name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .expect("test subdirectory name")
            .to_string();
        if NOT_TEST_TARGETS.contains(&dir_name.as_str()) {
            continue;
        }
        for file in std::fs::read_dir(&dir)
            .expect("read a tests/ subdirectory")
            .flatten()
        {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("test file stem");
            // `main.rs` in a subdirectory IS auto-discovered by Cargo, so it
            // needs no stanza and must not be reported as missing one.
            if stem == "main" {
                continue;
            }
            found.insert(format!("tests/{dir_name}/{stem}.rs"));
        }
    }
    found
}

/// The `path = "..."` of every `[[test]]` stanza in `Cargo.toml`, paired with
/// its `name`. Deliberately a small hand-rolled scan rather than a TOML
/// dependency: the file is machine-written in a fixed shape, and a guard that
/// needed a new crate to run would not be worth its cost.
fn registered_targets() -> Vec<(String, String)> {
    let manifest =
        std::fs::read_to_string(repo_root().join("Cargo.toml")).expect("read Cargo.toml");
    let mut out = Vec::new();
    let mut lines = manifest.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() != "[[test]]" {
            continue;
        }
        let (mut name, mut path) = (None, None);
        // Read to the next table header or blank-line-terminated stanza end.
        for body in lines.by_ref() {
            let t = body.trim();
            if t.starts_with('[') {
                break;
            }
            if let Some(v) = t.strip_prefix("name = ") {
                name = Some(v.trim_matches('"').to_string());
            } else if let Some(v) = t.strip_prefix("path = ") {
                path = Some(v.trim_matches('"').to_string());
            }
            if t.is_empty() && (name.is_some() || path.is_some()) {
                break;
            }
        }
        if let (Some(n), Some(p)) = (name, path) {
            out.push((n, p));
        }
    }
    out
}

#[test]
fn every_test_file_in_a_subdirectory_is_a_registered_target() {
    let on_disk = test_files_in_subdirectories();
    let registered: BTreeSet<String> = registered_targets().into_iter().map(|(_, p)| p).collect();

    let missing: Vec<&String> = on_disk.difference(&registered).collect();
    assert!(
        missing.is_empty(),
        "{} test file(s) in a tests/ subdirectory are not registered as Cargo \
         test targets, so they COMPILE NOWHERE AND NEVER RUN.\n\
         Cargo only auto-discovers `tests/*.rs` and `tests/<dir>/main.rs`.\n\
         Add to Cargo.toml:\n\n{}",
        missing.len(),
        missing
            .iter()
            .map(|p| {
                let stem = Path::new(p)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<name>");
                format!("[[test]]\nname = \"{stem}\"\npath = \"{p}\"\n")
            })
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn every_registered_target_points_at_a_file_that_exists() {
    let root = repo_root();
    let dangling: Vec<String> = registered_targets()
        .into_iter()
        .filter(|(_, path)| !root.join(path).is_file())
        .map(|(name, path)| format!("  {name} -> {path}"))
        .collect();
    assert!(
        dangling.is_empty(),
        "{} [[test]] stanza(s) name a path that does not exist. Cargo fails the \
         whole build on these, so this is a fast local signal rather than a \
         confusing one from CI:\n{}",
        dangling.len(),
        dangling.join("\n")
    );
}

#[test]
fn a_registered_target_is_named_after_its_file() {
    // The two can legally differ, but every stanza here follows the same rule as
    // an auto-discovered `tests/*.rs`: the target name IS the file stem. Keeping
    // that true is what lets `cargo test --test <stem>` work uniformly whether a
    // test sits in `tests/` or in a subdirectory of it.
    let mismatched: Vec<String> = registered_targets()
        .into_iter()
        .filter(|(name, path)| {
            Path::new(path).file_stem().and_then(|s| s.to_str()) != Some(name.as_str())
        })
        .map(|(name, path)| format!("  name = {name:?} but path = {path:?}"))
        .collect();
    assert!(
        mismatched.is_empty(),
        "{} [[test]] stanza(s) are not named after their file, so \
         `cargo test --test <file stem>` would not find them:\n{}",
        mismatched.len(),
        mismatched.join("\n")
    );
}
