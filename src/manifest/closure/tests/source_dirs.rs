//! The closure over source-directory dependencies: a chain longer than any
//! fixed pass budget still closes, and a dependency whose `project.json`
//! cannot be read stops the closure by naming it.

use super::*;

/// `root/lib0 .. lib{n-1}`, each declaring the next by source directory;
/// `root/app` declares `lib0` only.
fn chain(n: usize) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for i in 0..n {
        let next = if i + 1 < n {
            format!(
                "{{\"name\": \"lib{}\", \"version\": \"=0.1.0\", \"source\": \"file:../lib{}\"}}",
                i + 1,
                i + 1
            )
        } else {
            String::new()
        };
        write(
            &root.path().join(format!("lib{i}/project.json")),
            &manifest(&next),
        );
    }
    write(
        &root.path().join("app/project.json"),
        &manifest("{\"name\": \"lib0\", \"version\": \"=0.1.0\", \"source\": \"file:../lib0\"}"),
    );
    root
}

/// `reconcile` declares one dependency per pass, so a transitive chain of
/// N packages takes N passes. A chain past the old fixed 512-pass budget is a
/// legitimate project, and it closes: every link is declared, indirect, and
/// required by the link before it.
#[test]
fn a_dependency_chain_longer_than_512_links_closes() {
    let n = 520;
    let root = chain(n);
    let app = root.path().join("app");
    let contents = fs::read_to_string(app.join("project.json")).unwrap();
    let result = reconcile(&app, &contents).expect("a long chain still closes");
    assert_eq!(result.added.len(), n - 1);
    let manifest = parse_project_json(&result.contents, &app.join("project.json")).unwrap();
    let declared = declared_dependencies(&manifest);
    assert_eq!(declared.len(), n);
    let last = declared
        .iter()
        .find(|dependency| dependency.name == format!("lib{}", n - 1))
        .expect("the last link is declared");
    assert_eq!(last.direct, Some(false));
    assert_eq!(last.required_by, Some(vec![format!("lib{}", n - 2)]));
}

/// A source dependency whose `project.json` exists but is not text stops
/// the closure with an error naming the file, rather than being read as
/// needing nothing.
#[test]
fn a_source_dependency_whose_manifest_cannot_be_read_names_it() {
    let root = chain(2);
    let broken = root.path().join("lib0/project.json");
    fs::write(&broken, b"{\xff\xfe}").unwrap();
    let app = root.path().join("app");
    let contents = fs::read_to_string(app.join("project.json")).unwrap();
    let err = reconcile(&app, &contents).expect_err("an unreadable manifest is an error");
    assert!(err.contains("failed to read '"), "{err}");
    assert!(err.contains("lib0"), "{err}");
}
