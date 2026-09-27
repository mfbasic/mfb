//! Guard (bug-718): every `examples/<dir>/project.json` `name` field matches
//! what a reader would expect from its own path, so a directory can never
//! carry a misleading identity the way `examples/bugs/{tui,canvas}` did (a
//! mouse-driven "swat the bug" game, named and pathed as if it were a
//! `bugs/bug-NN-*.md`-style repro).
//!
//! The rule: for a single-project example (`examples/<name>/project.json`),
//! `name` must equal `<name>`. For a multi-project example
//! (`examples/<parent>/<sub>/project.json`), `name` must equal `<sub>`, OR be
//! listed in `EXCEPTIONS` below when the project intentionally differs (a
//! shared/primary sub-project named after the parent, or an existing name
//! that drops a character `project.json` names can't carry, e.g. a hyphen).
//!
//! Confirmed failing before bug-718 (`cargo test --test example_project_names`
//! against `examples/bugs/tui`/`examples/bugs/canvas`, both then named
//! `"bugs"`): both mismatched their own directory AND their parent directory
//! carried no game signal at all.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `(relative project.json path, expected name)` pairs that intentionally
/// differ from the bare directory-matches-name rule. Each entry needs a
/// reason a reader can check against the file itself.
const EXCEPTIONS: &[(&str, &str)] = &[
    // examples/browser is four sub-projects; `app` is the primary/entry one
    // and keeps the family's own name rather than the generic word "app".
    ("examples/browser/app/project.json", "browser"),
    // project.json names cannot carry a hyphen the directory does; "yamljson"
    // predates this guard and is an accepted, existing exception.
    ("examples/yaml-json/project.json", "yamljson"),
    // examples/swat is two editions (terminal, canvas); each is named after
    // the parent so neither artifact/binary is ambiguously just "tui" or
    // "canvas" (bug-718).
    ("examples/swat/tui/project.json", "swat-tui"),
    ("examples/swat/canvas/project.json", "swat-canvas"),
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `project.json` under `examples/`, at any depth, as a path relative to
/// the repo root (with `/` separators).
fn find_project_jsons(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir({}): {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            find_project_jsons(&path, out);
        } else if path.file_name().and_then(|n| n.to_str()) == Some("project.json") {
            out.push(path);
        }
    }
}

fn read_name(path: &Path) -> String {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let json: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    json.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("{}: no string \"name\" field", path.display()))
        .to_string()
}

#[test]
fn every_example_project_name_matches_its_path() {
    let root = repo_root();
    let examples_dir = root.join("examples");
    let mut project_jsons = Vec::new();
    find_project_jsons(&examples_dir, &mut project_jsons);
    assert!(
        !project_jsons.is_empty(),
        "found no examples/**/project.json at all"
    );

    let exceptions: BTreeMap<&str, &str> = EXCEPTIONS.iter().copied().collect();

    let mut failures = Vec::new();
    for path in &project_jsons {
        let rel = path
            .strip_prefix(&root)
            .expect("project.json under repo root")
            .to_str()
            .expect("utf-8 path")
            .replace('\\', "/");

        let name = read_name(path);

        if let Some(&expected) = exceptions.get(rel.as_str()) {
            if name != expected {
                failures.push(format!(
                    "{rel}: name is {name:?}, expected the documented exception {expected:?}"
                ));
            }
            continue;
        }

        // Non-exception case: name must equal the directory's own basename
        // (the immediate parent of project.json).
        let dir_name = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .expect("project.json has a parent directory name");

        if name != dir_name {
            failures.push(format!(
                "{rel}: name is {name:?}, expected {dir_name:?} to match its own \
                 directory (or add a documented exception to EXCEPTIONS in this test)"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "example project.json name mismatches:\n{}",
        failures.join("\n")
    );
}
