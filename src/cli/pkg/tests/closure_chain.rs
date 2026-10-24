//! `mfb pkg add file://…` over a long chain of compiled local packages: the
//! closure learns one link per pass (a link's import table is readable only
//! once it is copied into `packages/`), so the pass count is the chain's
//! length, and no fixed pass budget may refuse a legitimate chain.

use super::removal::{package, project};
use super::*;

/// `root/libs/lib0.mfp .. lib{n-1}.mfp`, each importing the next; returns
/// `lib0`'s path.
fn chain(root: &Path, n: usize) -> PathBuf {
    let libs = root.join("libs");
    let mut next: Option<PathBuf> = None;
    for i in (0..n).rev() {
        let ident = format!("ada#lib{i}");
        let built = match &next {
            Some(file) => {
                let name = format!("lib{}", i + 1);
                let import_ident = format!("ada#{name}");
                package(&libs, &ident, &[(&name, &import_ident, file)])
            }
            None => package(&libs, &ident, &[]),
        };
        next = Some(built);
    }
    next.unwrap()
}

/// A 40-link chain is deeper than the old 32-pass budget of
/// `close_dependency_set`; adding its head declares and installs every link.
#[test]
fn adding_the_head_of_a_40_link_compiled_chain_closes_it() {
    let n = 40;
    let root = tempfile::tempdir().unwrap();
    let head = chain(root.path(), n);
    let app = root.path().join("app");
    std::fs::create_dir_all(&app).unwrap();
    project(&app, &[]);

    let url = format!("file://{}", head.to_string_lossy().replace('\\', "/"));
    add_package(&app, &add_opts(&url)).expect("a long compiled chain closes");

    let path = app.join("project.json");
    let manifest = parse_project_json(&std::fs::read_to_string(&path).unwrap(), &path).unwrap();
    let declared = crate::manifest::closure::declared_dependencies(&manifest);
    assert_eq!(declared.len(), n);
    for i in 0..n {
        let name = format!("lib{i}");
        assert!(
            app.join("packages").join(format!("{name}.mfp")).is_file(),
            "{name} is installed"
        );
        let entry = declared
            .iter()
            .find(|dependency| dependency.name == name)
            .unwrap_or_else(|| panic!("{name} is declared"));
        if i == 0 {
            assert_eq!(entry.direct, Some(true));
            assert_eq!(entry.required_by, Some(vec![]));
        } else {
            assert_eq!(entry.direct, Some(false));
            assert_eq!(entry.required_by, Some(vec![format!("ada#lib{}", i - 1)]));
        }
    }
    assert!(crate::manifest::closure::check(&app, &manifest)
        .unwrap()
        .is_empty());
}
