//! The closure read from compiled packages: an installed `.mfp`'s import
//! table, the sibling it was added beside, the build cache, and the ABI and
//! pin conflicts between two installed packages. Every `.mfp` here is written
//! by the real package writer (`target::package_mfp::write_package`).

use super::*;
use crate::binary_repr::{BinaryReprDependency, BinaryReprMetadata};
use crate::ir::{IrFunction, IrOp, IrProject, IrSourceLoc, IrValue};
use crate::types::ParameterType;
use std::fs;

fn function(name: &str, returns: &str, calls: Option<&str>) -> IrFunction {
    let body = match calls {
        Some(target) => vec![IrOp::Return {
            value: Some(IrValue::Call {
                target: target.to_string(),
                args: vec![],
                loc: IrSourceLoc::default(),
                type_: ParameterType::parse(returns),
            }),
            loc: IrSourceLoc::default(),
        }],
        None => vec![],
    };
    IrFunction {
        name: name.to_string(),
        visibility: "export".to_string(),
        kind: "function".to_string(),
        isolated: false,
        params: vec![],
        returns: ParameterType::parse(returns),
        body,
        file: "src/main.mfb".to_string(),
        resource_owners: std::collections::HashMap::new(),
        loc: IrSourceLoc::default(),
    }
}

/// One dependency a package is compiled against: name, ident, version, pin,
/// and the compiled dependency itself.
struct Dep<'a>(&'a str, &'a str, &'a str, bool, &'a Path);

/// Compile package `name` into `dir/<name>.mfp`.
fn package(
    dir: &Path,
    name: &str,
    version: &str,
    functions: Vec<IrFunction>,
    deps: &[Dep],
) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let ir = IrProject {
        name: name.to_string(),
        entry: None,
        bindings: vec![],
        types: vec![],
        functions,
        native_resources: vec![],
        link_functions: vec![],
        link_cstructs: Vec::new(),
        link_aliases: vec![],
        docs: crate::ir::ProjectDocs::default(),
        native_libraries: Default::default(),
        max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
        canvas_auto_paint_ms: crate::manifest::DEFAULT_CANVAS_AUTO_PAINT_MS,
        selected_config: Default::default(),
    };
    let mut metadata = BinaryReprMetadata::new(name.to_string(), version.to_string());
    metadata.dependencies = deps
        .iter()
        .map(|Dep(name, ident, version, pin, _)| BinaryReprDependency {
            name: name.to_string(),
            ident: ident.to_string(),
            version: version.to_string(),
            pin: *pin,
            flags: 0,
        })
        .collect();
    let paths: Vec<PathBuf> = deps.iter().map(|dep| dep.4.to_path_buf()).collect();
    crate::target::package_mfp::write_package(dir, &ir, &metadata, &paths, None).unwrap()
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to.parent().unwrap()).unwrap();
    fs::copy(from, to).unwrap();
}

fn manifest(packages: &str) -> String {
    format!(
        "{{\n  \"name\": \"app\",\n  \"version\": \"0.1.0\",\n  \"mfb\": \"1.0\",\n  \
         \"packages\": [{packages}]\n}}\n"
    )
}

fn entry(name: &str, source: &str) -> String {
    format!(
        "{{\"name\": \"{name}\", \"version\": \"1.0.0\", \"source\": \"{source}\", \
         \"direct\": true, \"requiredBy\": []}}"
    )
}

fn parsed(app: &Path, contents: &str) -> std::collections::HashMap<String, JsonValue> {
    parse_project_json(contents, &app.join("project.json")).unwrap()
}

/// `root/libs/base.mfp` (exports `helper`) and `root/libs/user.mfp`, which
/// calls `base.helper`; `user` is installed into `root/app/packages/`.
struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new(base_ident: &str) -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let libs = root.path().join("libs");
        let base = package(
            &libs,
            "base",
            "1.0.0",
            vec![function("helper", "Integer", None)],
            &[],
        );
        let user = package(
            &root.path().join("build-user"),
            "user",
            "1.0.0",
            vec![function("run", "Integer", Some("base.helper"))],
            &[Dep("base", base_ident, "1.0.0", true, &base)],
        );
        copy(&user, &libs.join("user.mfp"));
        copy(&user, &root.path().join("app/packages/user.mfp"));
        Fixture { root }
    }

    fn app(&self) -> PathBuf {
        self.root.path().join("app")
    }

    fn libs(&self) -> PathBuf {
        self.root.path().join("libs")
    }
}

fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn an_installed_package_declares_the_sibling_it_was_added_beside() {
    let fixture = Fixture::new("");
    let app = fixture.app();
    let contents = manifest(&entry("user", "file:../libs/user.mfp"));
    assert_eq!(
        check(&app, &parsed(&app, &contents)).unwrap(),
        vec![Issue::Undeclared {
            requirer: "user".to_string(),
            name: "base".to_string(),
        }]
    );
    let result = reconcile(&app, &contents).unwrap();
    assert_eq!(result.added, vec![("base".to_string(), "user".to_string())]);
    assert_eq!(
        result.installs,
        vec![("base".to_string(), app.join("../libs/base.mfp"))]
    );
    assert!(result.unresolved.is_empty());
    let declared = declared_dependencies(&parsed(&app, &result.contents));
    let base = declared.iter().find(|d| d.name == "base").unwrap();
    assert_eq!(base.source, "file:packages/base.mfp");
    assert_eq!(base.version, "1.0.0");
    assert!(base.pin);
    assert_eq!(base.direct, Some(false));
    assert_eq!(base.required_by, Some(vec!["user".to_string()]));
}

#[test]
fn a_package_added_by_file_url_declares_its_sibling_by_url() {
    let fixture = Fixture::new("");
    let app = fixture.app();
    let url = format!("file://{}", fixture.libs().join("user.mfp").display());
    let contents = manifest(&entry("user", &url));
    let result = reconcile(&app, &contents).unwrap();
    let declared = declared_dependencies(&parsed(&app, &result.contents));
    let base = declared.iter().find(|d| d.name == "base").unwrap();
    assert_eq!(
        base.source,
        format!("file://{}", slash(&fixture.libs().join("base.mfp")))
    );
    assert_eq!(
        result.installs,
        vec![("base".to_string(), fixture.libs().join("base.mfp"))]
    );
}

#[test]
fn a_dependency_with_no_sibling_to_install_is_left_unresolved() {
    let fixture = Fixture::new("");
    let app = fixture.app();
    fs::remove_file(fixture.libs().join("base.mfp")).unwrap();
    let contents = manifest(&entry("user", "file:../libs/user.mfp"));
    let result = reconcile(&app, &contents).unwrap();
    assert!(result.added.is_empty() && result.installs.is_empty());
    assert_eq!(result.contents, contents);
    let [(requirer, need)] = result.unresolved.as_slice() else {
        panic!("one unresolved requirement: {:?}", result.unresolved);
    };
    assert_eq!(requirer, "user");
    assert_eq!(
        (
            need.name.as_str(),
            need.ident.as_str(),
            need.source.as_ref()
        ),
        ("base", "base", None)
    );

    // An installed package whose source is a directory, not a `.mfp`, has no
    // sibling to look beside either.
    let contents = manifest(&entry("user", "file:../libs/user-src"));
    let result = reconcile(&app, &contents).unwrap();
    assert_eq!(result.unresolved.len(), 1);
}

#[test]
fn a_declared_package_of_the_same_name_but_another_ident_is_a_clash() {
    let fixture = Fixture::new("");
    let app = fixture.app();
    let contents = manifest(&format!(
        "{}, {{\"name\": \"base\", \"ident\": \"ada#base\", \"version\": \"1.0.0\", \
         \"source\": \"ada#base\", \"direct\": true, \"requiredBy\": []}}",
        entry("user", "file:../libs/user.mfp")
    ));
    let clash = Issue::NameClash {
        requirer: "user".to_string(),
        name: "base".to_string(),
        ident: "base".to_string(),
        declared_ident: "ada#base".to_string(),
    };
    assert_eq!(
        check(&app, &parsed(&app, &contents)).unwrap(),
        vec![clash.clone()]
    );
    assert_eq!(reconcile(&app, &contents).unwrap_err(), clash.message());
    assert_eq!(
        clash.message(),
        "package `user` requires `base` as ident `base`, but project.json declares \
         `base` as ident `ada#base`"
    );
}

#[test]
fn an_import_recorded_by_ident_is_required_by_that_ident() {
    let fixture = Fixture::new("ada#base");
    let app = fixture.app();
    let contents = manifest(&entry("user", "file:../libs/user.mfp"));
    let result = reconcile(&app, &contents).unwrap();
    let declared = declared_dependencies(&parsed(&app, &result.contents));
    let base = declared.iter().find(|d| d.name == "base").unwrap();
    assert_eq!(base.ident, "ada#base");
    assert_eq!(base.required_by, Some(vec!["user".to_string()]));
}

#[test]
fn a_registry_package_is_read_from_the_build_cache() {
    let fixture = Fixture::new("");
    let app = fixture.app();
    let cached = crate::manifest::package::source_package_cache_dir(&app).join("user.mfp");
    copy(&app.join("packages/user.mfp"), &cached);
    fs::remove_file(app.join("packages/user.mfp")).unwrap();
    let contents = manifest(&entry("user", "ada#user"));
    assert_eq!(
        check(&app, &parsed(&app, &contents)).unwrap(),
        vec![Issue::Undeclared {
            requirer: "user".to_string(),
            name: "base".to_string(),
        }]
    );
    // A source directory with no `project.json` to read falls back to the
    // cache too.
    let contents = manifest(&entry("user", "file:../user-src"));
    assert_eq!(check(&app, &parsed(&app, &contents)).unwrap().len(), 1);
    // Not installed and not cached: nothing is known about what it needs.
    fs::remove_file(&cached).unwrap();
    assert_eq!(
        check(&app, &parsed(&app, &contents)).unwrap(),
        Vec::<Issue>::new()
    );
}

#[test]
fn an_undecodable_installed_package_needs_nothing_the_closure_can_name() {
    let fixture = Fixture::new("");
    let app = fixture.app();
    fs::write(app.join("packages/user.mfp"), b"not a package").unwrap();
    let contents = manifest(&entry("user", "file:../libs/user.mfp"));
    assert_eq!(
        check(&app, &parsed(&app, &contents)).unwrap(),
        Vec::<Issue>::new()
    );
}

/// `app/packages/` holds `user` (compiled against `base` 1.0.0, pinned when
/// `pin`) and `base` as given; returns the app directory and `user`'s import
/// of `helper`'s hash.
fn installed_pair(root: &Path, base: PathBuf, pin: bool) -> (PathBuf, String) {
    let base1 = package(
        &root.join("base1"),
        "base",
        "1.0.0",
        vec![function("helper", "Integer", None)],
        &[],
    );
    let user = package(
        &root.join("user"),
        "user",
        "1.0.0",
        vec![function("run", "Integer", Some("base.helper"))],
        &[Dep("base", "", "1.0.0", pin, &base1)],
    );
    let app = root.join("app");
    copy(&user, &app.join("packages/user.mfp"));
    copy(&base, &app.join("packages/base.mfp"));
    let info = binary_repr::read_package_info(&user).unwrap();
    let wanted = info.imports[0].used_symbols[0].sig_hash.clone();
    (app, wanted)
}

fn both_declared() -> String {
    manifest(&format!(
        "{}, {}",
        entry("user", "file:packages/user.mfp"),
        entry("base", "file:packages/base.mfp")
    ))
}

#[test]
fn an_installed_package_that_breaks_a_pin_or_an_abi_is_a_conflict() {
    let root = tempfile::tempdir().unwrap();
    // base 2.0.0: `helper` now returns a String.
    let base2 = package(
        &root.path().join("base2"),
        "base",
        "2.0.0",
        vec![function("helper", "String", None)],
        &[],
    );
    let installed_hash = binary_repr::read_package_info(&base2).unwrap().exports[0]
        .sig_hash
        .clone();
    let (app, wanted) = installed_pair(root.path(), base2, true);
    assert_ne!(wanted, installed_hash);
    let found = conflicts(&app, &parsed(&app, &both_declared()));
    let pin = Conflict {
        requirer: "user".to_string(),
        name: "base".to_string(),
        installed_version: "2.0.0".to_string(),
        kind: ConflictKind::Pin {
            pinned: "1.0.0".to_string(),
        },
    };
    let abi = Conflict {
        kind: ConflictKind::Symbols(vec![(
            "helper".to_string(),
            wanted.clone(),
            Some(installed_hash.clone()),
        )]),
        ..pin.clone()
    };
    assert_eq!(found, vec![pin.clone(), abi.clone()]);
    assert_eq!(
        pin.message(),
        "package `user` pins `base` 1.0.0, but the installed `base` is 2.0.0"
    );
    assert_eq!(
        pin.details(),
        vec!["`user` requires exactly 1.0.0; installed: 2.0.0".to_string()]
    );
    assert_eq!(
        abi.message(),
        "package `user` was built against a `base` incompatible with the installed one: \
         its `helper` differs from `base` 2.0.0"
    );
    assert_eq!(
        abi.details(),
        vec![format!(
            "`helper`: `user` needs ABI {wanted}; `base` 2.0.0 exports {installed_hash}"
        )]
    );

    // The base it was built against conflicts with nothing.
    let base1 = root.path().join("base1/base.mfp");
    copy(&base1, &app.join("packages/base.mfp"));
    assert!(conflicts(&app, &parsed(&app, &both_declared())).is_empty());

    // A package that is not declared is not checked against.
    let user_only = manifest(&entry("user", "file:packages/user.mfp"));
    assert!(conflicts(&app, &parsed(&app, &user_only)).is_empty());
}

#[test]
fn an_unpinned_importer_accepts_any_version_with_the_same_abi() {
    let root = tempfile::tempdir().unwrap();
    let base2 = package(
        &root.path().join("base2"),
        "base",
        "2.0.0",
        vec![function("helper", "Integer", None)],
        &[],
    );
    let (app, _) = installed_pair(root.path(), base2, false);
    assert!(conflicts(&app, &parsed(&app, &both_declared())).is_empty());
}

#[test]
fn a_symbol_the_installed_package_no_longer_exports_is_named() {
    let root = tempfile::tempdir().unwrap();
    // base 1.0.0 again (the pin holds), but without `helper`.
    let renamed = package(
        &root.path().join("renamed"),
        "base",
        "1.0.0",
        vec![function("assist", "Integer", None)],
        &[],
    );
    let (app, wanted) = installed_pair(root.path(), renamed, true);
    let found = conflicts(&app, &parsed(&app, &both_declared()));
    assert_eq!(
        found,
        vec![Conflict {
            requirer: "user".to_string(),
            name: "base".to_string(),
            installed_version: "1.0.0".to_string(),
            kind: ConflictKind::Symbols(vec![("helper".to_string(), wanted.clone(), None)]),
        }]
    );
    assert_eq!(
        found[0].details(),
        vec![format!(
            "`helper`: `user` needs ABI {wanted}; `base` 1.0.0 does not export it"
        )]
    );
}

#[test]
fn each_manifest_issue_explains_itself() {
    let names = |idents: &[&str]| idents.iter().map(|i| i.to_string()).collect::<Vec<_>>();
    let cases = [
        (
            Issue::Undeclared {
                requirer: "user".to_string(),
                name: "base".to_string(),
            },
            "package `user` requires `base`, which project.json does not declare",
        ),
        (
            Issue::MissingDirect {
                name: "user".to_string(),
            },
            "package `user` has no boolean `direct` field",
        ),
        (
            Issue::MissingRequiredBy {
                name: "base".to_string(),
                expected: names(&["user", "web"]),
            },
            "package `base` has no `requiredBy` array of idents; it is required by \
             [`user`, `web`]",
        ),
        (
            Issue::MissingRequiredBy {
                name: "base".to_string(),
                expected: vec![],
            },
            "package `base` has no `requiredBy` array of idents; it is required by no \
             declared package",
        ),
        (
            Issue::RequiredBy {
                name: "base".to_string(),
                declared: names(&["nobody", "user"]),
                expected: names(&["user"]),
            },
            "package `base` lists requiredBy [`nobody`, `user`], but it is required by \
             [`user`]",
        ),
        (
            Issue::Orphan {
                name: "extra".to_string(),
            },
            "package `extra` is not `direct` and no declared package requires it",
        ),
    ];
    for (issue, message) in cases {
        assert_eq!(issue.message(), message);
    }
}

#[test]
fn a_requirers_compiled_sources_are_rebased_onto_the_importer() {
    let root = tempfile::tempdir().unwrap();
    let app = root.path().join("app");
    let user = root.path().join("libs/user");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(user.join("packages")).unwrap();
    let entry = |source: &str| ProjectPackageDependency {
        name: "base".to_string(),
        ident: "base".to_string(),
        version: "1.0.0".to_string(),
        pin: false,
        source: source.to_string(),
        ident_key: String::new(),
        direct: Some(true),
        required_by: Some(Vec::new()),
    };
    // A `file:<rel>.mfp` the requirer has not built yet: declared where the
    // importer will install it, with nothing to copy.
    assert_eq!(
        rebase_source(&app, &user, &entry("file:dist/base.mfp")),
        ("file:packages/base.mfp".to_string(), None)
    );
    let compiled = user.join("dist/base.mfp");
    fs::create_dir_all(compiled.parent().unwrap()).unwrap();
    fs::write(&compiled, b"mfp").unwrap();
    assert_eq!(
        rebase_source(&app, &user, &entry("file:dist/base.mfp")),
        ("file:packages/base.mfp".to_string(), Some(compiled.clone()))
    );
    // A `file://` URL is the same everywhere; the file it names is copied...
    let url = format!("file://{}", compiled.display());
    assert_eq!(
        rebase_source(&app, &user, &entry(&url)),
        (url.clone(), Some(compiled.clone()))
    );
    // ...unless the requirer installed its own copy, which is what it built
    // against.
    let installed = user.join("packages/base.mfp");
    fs::write(&installed, b"mfp").unwrap();
    assert_eq!(
        rebase_source(&app, &user, &entry(&url)),
        (url, Some(installed))
    );
}

#[test]
fn relative_paths_resolve_a_relative_directory_against_the_working_directory() {
    assert_eq!(
        relative_path(Path::new("a/b"), Path::new("a/c/d.mfp")).as_deref(),
        Some("../c/d.mfp")
    );
}
