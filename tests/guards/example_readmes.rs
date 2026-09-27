//! Guard (bug-720): every buildable `examples/**/project.json` (`"kind":
//! "executable"`) ships a `README.md` a reader can find without already
//! knowing the project — either right beside its own `project.json`, or at
//! the documented shared parent for a multi-project example directory (the
//! `examples/browser` case: `dom`/`fetch`/`display` are `"kind": "package"`
//! and excluded by the executable filter, but `app/` is the one executable
//! and its README lives one level up, at `examples/browser/README.md`,
//! covering the whole family).
//!
//! Before bug-720 only 4 of 20 executable example projects had a README at
//! all (`brogue`, `browser` via its shared parent, `network-server`,
//! `yaml-json`); this guard is what keeps that gap from reopening as new
//! examples are added — `scripts/build-examples.sh` had (and has) no such
//! check (bug-720's Root Cause).

use std::path::{Path, PathBuf};

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

fn is_executable(path: &Path) -> bool {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let json: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    json.get("kind").and_then(|v| v.as_str()) == Some("executable")
}

#[test]
fn every_executable_example_ships_a_readme() {
    let root = repo_root();
    let examples_dir = root.join("examples");
    let mut project_jsons = Vec::new();
    find_project_jsons(&examples_dir, &mut project_jsons);
    assert!(
        !project_jsons.is_empty(),
        "found no examples/**/project.json at all"
    );

    let mut failures = Vec::new();
    for path in &project_jsons {
        if !is_executable(path) {
            continue;
        }

        let proj_dir = path.parent().expect("project.json has a parent dir");
        let own_readme = proj_dir.join("README.md");
        if own_readme.is_file() {
            continue;
        }

        // Multi-project example directory: a README one level up, beside the
        // family's shared top-level directory, also satisfies the guard (the
        // `examples/browser` case: `examples/browser/README.md` covers
        // `examples/browser/app`, since `dom`/`fetch`/`display` are
        // `"kind": "package"` and never reach this loop).
        let shared_parent_readme = proj_dir
            .parent()
            .filter(|p| *p != examples_dir)
            .map(|p| p.join("README.md"));
        if let Some(shared) = &shared_parent_readme {
            if shared.is_file() {
                continue;
            }
        }

        let rel = path
            .strip_prefix(&root)
            .expect("project.json under repo root")
            .to_str()
            .expect("utf-8 path")
            .replace('\\', "/");
        failures.push(format!(
            "{rel}: no README.md in {} (and no shared-parent README.md either)",
            proj_dir.display()
        ));
    }

    assert!(
        failures.is_empty(),
        "{} executable example project(s) have no README.md:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
