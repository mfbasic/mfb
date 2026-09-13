//! plan-131-E: `scripts/` cannot turn back into a dumping ground silently.
//!
//! Every regular file directly under `scripts/` must have exactly one index entry in
//! `scripts/README.md`, and every entry must name a file that exists. Every
//! `tools/<name>/` directory must carry a `README.md`. The rule these guard (what
//! belongs in `scripts/` versus `tools/`) is written in AGENTS.md and at the top of
//! `scripts/README.md`.
//!
//! Index entries are the README's `- **<name>**` bullets. Dot-files and directories
//! under `scripts/` are not counted: a gitignored `__pycache__` or a selftest's
//! temporary `.selftest-*` copy is local state, and a census that counts local state
//! passes or fails depending on whose machine runs it.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The index-entry marker. Spelled with `concat!` so this test's own failure text
/// cannot be mistaken for an entry by anything that greps the source.
const ENTRY_OPEN: &str = concat!("- *", "*");
const ENTRY_CLOSE: &str = concat!("*", "*");

fn indexed_names(readme: &str) -> Vec<String> {
    readme
        .lines()
        .filter_map(|line| line.strip_prefix(ENTRY_OPEN))
        .filter_map(|rest| {
            rest.split_once(ENTRY_CLOSE)
                .map(|(name, _)| name.to_string())
        })
        .collect()
}

#[test]
fn every_script_is_indexed_and_every_index_entry_exists() {
    let root = repo_root();
    let mut on_disk = BTreeSet::new();
    for entry in std::fs::read_dir(root.join("scripts")).expect("read scripts/") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "README.md" {
            continue;
        }
        if entry.file_type().expect("file type").is_file() {
            on_disk.insert(name);
        }
    }

    let readme =
        std::fs::read_to_string(root.join("scripts/README.md")).expect("read scripts/README.md");
    let entries = indexed_names(&readme);
    let mut indexed = BTreeSet::new();
    let mut duplicated = Vec::new();
    for name in &entries {
        if !indexed.insert(name.clone()) {
            duplicated.push(name.clone());
        }
    }

    let unindexed: Vec<_> = on_disk.difference(&indexed).cloned().collect();
    let dangling: Vec<_> = indexed.difference(&on_disk).cloned().collect();
    assert!(
        unindexed.is_empty() && dangling.is_empty() && duplicated.is_empty(),
        "scripts/README.md is out of step with scripts/.\n\
         files with no index entry (add a `{ENTRY_OPEN}name{ENTRY_CLOSE} — purpose. Usage: … Run by: …` bullet): {unindexed:?}\n\
         index entries naming no file (remove them): {dangling:?}\n\
         names indexed more than once: {duplicated:?}\n\
         A generator, probe, oracle or benchmark belongs under tools/<name>/ instead (AGENTS.md)."
    );
}

#[test]
fn every_tools_directory_has_a_readme() {
    let root = repo_root();
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(root.join("tools")).expect("read tools/") {
        let entry = entry.expect("dir entry");
        if !entry.file_type().expect("file type").is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if !entry.path().join("README.md").is_file() {
            missing.push(format!("tools/{name}"));
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "these tools/ directories have no README.md saying what they are and who uses them: {missing:?}"
    );
}
