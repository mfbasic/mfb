use super::*;

const UNSIGNED_FIXTURE: &str =
    "tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp";

fn s(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn usage(result: Result<(), PkgCommandError>) -> String {
    match result {
        Err(PkgCommandError::Usage(message)) => message,
        Err(PkgCommandError::Failed(message)) => {
            panic!("expected usage error, got failure: {message}")
        }
        Ok(()) => panic!("expected usage error, got Ok"),
    }
}

fn index_version(
    version: &str,
    state: &str,
    published_at: i64,
) -> mfb_repository::server::IndexVersion {
    mfb_repository::server::IndexVersion {
        version: version.to_string(),
        hash: format!("hash-{version}"),
        published_at,
        state: state.to_string(),
        abi_index: serde_json::Value::Null,
        log_entry: None,
        publisher: None,
        epoch: 0,
    }
}

fn index(
    versions: Vec<mfb_repository::server::IndexVersion>,
) -> mfb_repository::server::IndexResponse {
    mfb_repository::server::IndexResponse {
        ident: "ada#shape".to_string(),
        owner: "ada".to_string(),
        ident_key: "ed25519:ik".to_string(),
        ident_fingerprint: "if".to_string(),
        name_binding_signature: String::new(),
        server_fingerprint: "sf".to_string(),
        versions,
        transfers: Vec::new(),
    }
}

#[test]
fn signature_type_name_maps_known_and_unknown() {
    assert_eq!(signature_type_name(0), "unsigned");
    assert_eq!(signature_type_name(1), "Ed25519");
    assert_eq!(signature_type_name(9), "unknown (9)");
}

#[test]
fn hex_bytes_formats_lowercase_two_digit() {
    assert_eq!(hex_bytes(&[0x00, 0x0f, 0xab, 0xff]), "000fabff");
    assert_eq!(hex_bytes(&[]), "");
}

#[test]
fn empty_marker_marks_empty_strings() {
    assert_eq!(empty_marker(""), "<empty>");
    assert_eq!(empty_marker("value"), "value");
}

#[test]
fn package_export_kind_names() {
    use binary_repr::BinaryReprExportKind::*;
    assert_eq!(package_export_kind_name(Func), "FUNC");
    assert_eq!(package_export_kind_name(Type), "TYPE");
    assert_eq!(package_export_kind_name(Union), "UNION");
    assert_eq!(package_export_kind_name(Enum), "ENUM");
}

#[test]
fn package_verify_status_messages() {
    assert_eq!(PackageVerifyStatus::Ok.message(), "OK");
    assert_eq!(PackageVerifyStatus::NeedsUpdate.message(), "Needs Update");
    assert_eq!(
        PackageVerifyStatus::InvalidPackage.message(),
        "Invalid Package"
    );
}

#[test]
fn state_is_floating_eligible_classifies_states() {
    assert!(state_is_floating_eligible("available"));
    assert!(state_is_floating_eligible("deprecated"));
    assert!(!state_is_floating_eligible("yanked"));
    assert!(!state_is_floating_eligible("blocked"));
    assert!(!state_is_floating_eligible("legal-tombstoned"));
}

#[test]
fn select_index_version_empty_index_errors() {
    let index = index(Vec::new());
    assert!(select_index_version(&index, None)
        .unwrap_err()
        .contains("no published versions"));
}

#[test]
fn select_index_version_exact_request() {
    let index = index(vec![
        index_version("1.0.0", "available", 1),
        index_version("2.0.0", "available", 2),
    ]);
    let chosen = select_index_version(&index, Some("1.0.0")).expect("chosen");
    assert_eq!(chosen.version, "1.0.0");
    // A missing exact version errors.
    assert!(select_index_version(&index, Some("9.9.9"))
        .unwrap_err()
        .contains("no version `9.9.9`"));
}

#[test]
fn select_index_version_exact_request_rejects_blocked() {
    let index = index(vec![index_version("1.0.0", "blocked", 1)]);
    assert!(select_index_version(&index, Some("1.0.0"))
        .unwrap_err()
        .contains("cannot be installed"));
}

#[test]
fn select_index_version_floating_picks_newest_eligible() {
    let index = index(vec![
        index_version("1.0.0", "available", 10),
        index_version("2.0.0", "yanked", 20),
        index_version("1.5.0", "deprecated", 15),
    ]);
    // Newest floating-eligible is 1.5.0 (2.0.0 is yanked, pin-only).
    let chosen = select_index_version(&index, None).expect("chosen");
    assert_eq!(chosen.version, "1.5.0");
}

#[test]
fn select_index_version_floating_none_eligible_errors() {
    let index = index(vec![index_version("1.0.0", "yanked", 1)]);
    assert!(select_index_version(&index, None)
        .unwrap_err()
        .contains("no install-eligible version"));
}

#[test]
fn a_pinned_dependency_demands_an_exact_version() {
    let dependency = |version: &str, pin: bool| ProjectPackageDependency {
        name: "shape".to_string(),
        ident: "ada#shape".to_string(),
        version: version.to_string(),
        pin,
        source: "ada#shape".to_string(),
        ident_key: String::new(),
        direct: None,
        required_by: None,
    };
    let status = |version: &str, pin: bool, actual: &str| {
        package_dependency_status(&dependency(version, pin), "shape", "ada#shape", actual)
    };

    assert_eq!(status("1.0.0", true, "1.0.0"), PackageVerifyStatus::Ok);
    assert_eq!(
        status("1.0.0", true, "1.2.0"),
        PackageVerifyStatus::NeedsUpdate
    );
    // An empty pinned version can never match: builds reject it too
    // (`installed_package_files`), so `pkg verify` must not report `Ok`.
    assert_eq!(status("", true, "1.0.0"), PackageVerifyStatus::NeedsUpdate);
    // Unpinned: an empty version means "any version".
    assert_eq!(status("", false, "1.0.0"), PackageVerifyStatus::Ok);
    assert_eq!(status("1.0.0", false, "1.0.0"), PackageVerifyStatus::Ok);
    assert_eq!(
        status("1.0.0", false, "1.2.0"),
        PackageVerifyStatus::NeedsUpdate
    );
    // A name or ident disagreement outranks the version check either way.
    assert_eq!(
        package_dependency_status(&dependency("1.0.0", true), "other", "ada#shape", "1.0.0"),
        PackageVerifyStatus::InvalidPackage
    );
}

#[test]
fn package_verify_line_formats_with_and_without_version() {
    let dependency = ProjectPackageDependency {
        name: "shape".to_string(),
        ident: "ada#shape".to_string(),
        version: "1.0.0".to_string(),
        pin: false,
        source: "ada#shape".to_string(),
        ident_key: String::new(),
        direct: None,
        required_by: None,
    };
    assert_eq!(
        package_verify_line(
            &dependency,
            &PackageVerifyResult {
                version: String::new(),
                status: PackageVerifyStatus::InvalidPackage,
            }
        ),
        "shape @ 1.0.0 : Invalid Package"
    );
    assert_eq!(
        package_verify_line(
            &dependency,
            &PackageVerifyResult {
                version: "1.2.0".to_string(),
                status: PackageVerifyStatus::Ok,
            }
        ),
        "shape @ 1.0.0 : OK (1.2.0)"
    );
}

#[test]
fn run_pkg_requires_a_subcommand() {
    assert!(usage(run_pkg_command(&s(&[]))).contains("mfb pkg requires a subcommand"));
}

#[test]
fn run_pkg_rejects_unknown_command() {
    assert!(usage(run_pkg_command(&s(&["frobnicate"]))).contains("unknown pkg command"));
}

/// plan-42 §4.5: the top-level screen advertises only a few pkg commands, so
/// pkg's own usage errors must interpolate `PKG_HELP` — the complete list —
/// not the trimmed top-level `USAGE` they used to show.
///
/// plan-60-A: the witness commands changed. This test used to probe for
/// `check-abi`/`release-state`/`transfer-accept`, chosen because they were
/// in `PKG_HELP` but not in `USAGE`. All three moved to `mfb repo`, so they
/// are no longer valid witnesses *for pkg* — the assertion's intent
/// (pkg errors show the full pkg set) is unchanged, only the commands that
/// can witness it. `info` and `validate` are the surviving pkg-only
/// commands absent from the top-level screen.
#[test]
fn run_pkg_usage_errors_show_the_full_pkg_command_set() {
    let message = usage(run_pkg_command(&s(&["frobnicate"])));
    // Present in PKG_HELP, absent from the trimmed top-level screen.
    for command in ["info", "validate"] {
        assert!(
            message.contains(command),
            "pkg error must list `{command}`: {message}"
        );
    }
    assert!(message.contains("Usage: mfb pkg <command>"));
    // ...and must not be the top-level screen.
    assert!(!message.contains("Project Setup:"));
}

/// plan-60-A §1: every command is listed under exactly one parent. `PKG_HELP`
/// must not advertise a command that `mfb pkg` no longer dispatches, and
/// `REPO_HELP` must advertise every command it now owns. Without this, the
/// help text can drift back into naming a command that hard-errors.
#[test]
fn help_lists_each_moved_command_under_repo_only() {
    for command in [
        "publish",
        "check-abi",
        "release-state",
        "transfer",
        "transfer-accept",
    ] {
        assert!(
            crate::cli::help::REPO_HELP.contains(&format!("repo {command}")),
            "REPO_HELP must document `repo {command}`"
        );
        assert!(
            !crate::cli::help::PKG_HELP.contains(&format!("  {command} ")),
            "PKG_HELP must not still list `{command}` as a pkg command"
        );
    }
    // The surviving consumer-side commands stay exactly where they were.
    for command in [
        "add", "info", "doc", "verify", "validate", "install", "update",
    ] {
        assert!(
            crate::cli::help::PKG_HELP.contains(command),
            "PKG_HELP must still document `{command}`"
        );
    }
}

#[test]
fn run_pkg_usage_errors_for_wrong_arity() {
    assert!(usage(run_pkg_command(&s(&["add"]))).contains("mfb pkg add requires"));
    assert!(usage(run_pkg_command(&s(&["info"]))).contains("mfb pkg info requires"));
    assert!(usage(run_pkg_command(&s(&["verify", "extra", "junk"])))
        .contains("mfb pkg verify accepts only"));
    assert!(usage(run_pkg_command(&s(&["validate"]))).contains("mfb pkg validate requires"));
}

/// plan-60-A: the five publisher-side commands moved to `mfb repo`. They are
/// a hard error under `pkg` — no alias, no deprecation shim — but the error
/// names the new location rather than falling through to the generic
/// "unknown pkg command" arm.
///
/// This replaces the five arity assertions that used to live in
/// `run_pkg_usage_errors_for_wrong_arity`: arity is now `repo`'s
/// responsibility, and is asserted in `super::repo`'s tests. What must be
/// pinned *here* is that the old spelling no longer reaches an
/// implementation, whatever its arity.
#[test]
fn run_pkg_rejects_the_moved_publisher_commands() {
    for command in [
        "publish",
        "check-abi",
        "release-state",
        "transfer",
        "transfer-accept",
    ] {
        // Every arity must be rejected identically — a bare command, a
        // plausible one, and an over-long one. If any of these reached a
        // surviving `pkg` arm the move would be incomplete.
        for args in [
            vec![command],
            vec![command, "a"],
            vec![command, "a", "b"],
            vec![command, "a", "b", "c"],
        ] {
            let message = usage(run_pkg_command(&s(&args)));
            assert!(
                message.contains(&format!(
                    "mfb pkg {command} has moved to mfb repo {command}"
                )),
                "`pkg {args:?}` must name the new location: {message}"
            );
            // Not the generic fallback — that would leave the user grepping.
            assert!(!message.contains("unknown pkg command"), "{message}");
        }
    }
}

/// Parse `add` arguments, panicking with the usage text on failure.
/// `PkgCommandError` has no `Debug`, so this unwraps explicitly rather than
/// deriving `Debug` on a production type for test ergonomics.
fn parsed(args: &[&str]) -> AddOptions {
    match parse_add_options(&s(args)) {
        Ok(options) => options,
        Err(PkgCommandError::Usage(message) | PkgCommandError::Failed(message)) => {
            panic!("expected `{args:?}` to parse, got: {message}")
        }
    }
}

/// A bare `add` invocation with no flags — the common shape in these tests.
fn add_opts(target: &str) -> AddOptions {
    AddOptions {
        target: target.to_string(),
        pin: None,
    }
}

fn index_version_with_abi(
    version: &str,
    state: &str,
    abi: serde_json::Value,
) -> mfb_repository::server::IndexVersion {
    mfb_repository::server::IndexVersion {
        version: version.to_string(),
        hash: format!("hash-{version}"),
        published_at: 0,
        state: state.to_string(),
        abi_index: abi,
        log_entry: None,
        publisher: None,
        epoch: 0,
    }
}

fn imports_map(edges: &[(&str, &[&str])]) -> std::collections::BTreeMap<String, Vec<String>> {
    edges
        .iter()
        .map(|(importer, imported)| {
            (
                (*importer).to_string(),
                imported.iter().map(|i| (*i).to_string()).collect(),
            )
        })
        .collect()
}

#[test]
fn parse_remove_options_reads_the_target_and_yes() {
    let (target, yes) = parse_remove_options(&s(&["ada#shape"])).unwrap_or_else(|_| panic!());
    assert_eq!(target, "ada#shape");
    assert!(!yes);
    let (_, yes) = parse_remove_options(&s(&["ada#shape", "--yes"])).unwrap_or_else(|_| panic!());
    assert!(yes);

    let err = |args: &[&str]| usage(parse_remove_options(&s(args)).map(|_| ()));
    assert!(err(&[]).contains("mfb pkg remove requires"));
    assert!(err(&["a", "b"]).contains("exactly one"));
    assert!(err(&["--bogus", "a"]).contains("unknown flag"));
}

#[test]
fn remove_rejects_a_target_that_is_not_declared() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
    )
    .expect("manifest");
    let err = run_remove(dir.path(), "ada#shape", false).expect_err("must refuse");
    assert!(err.contains("is not declared in project.json"), "{err}");
}

/// plan-60-F §4.4: a declared-but-not-installed dependency makes the closure
/// untrustworthy — it could omit a package that imports the target — so the
/// command refuses.
///
/// **`--yes` must NOT bypass this.** It is a correctness gate, not a
/// confirmation, and conflating the two is the mistake a careless
/// implementation makes: proceeding would print a confident list, remove too
/// little, and leave exactly the dangling-import state the cascade exists to
/// prevent.
#[test]
fn remove_refuses_when_a_declared_package_is_not_installed() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\
             \"sources\":[{\"root\":\"src\"}],\"packages\":[\
             {\"name\":\"shape\",\"ident\":\"ada#shape\",\"version\":\"1.0.0\"},\
             {\"name\":\"widget\",\"ident\":\"ada#widget\",\"version\":\"1.0.0\"}]}",
    )
    .expect("manifest");
    // Neither .mfp exists, so the import graph cannot be read.
    for assume_yes in [false, true] {
        let err = run_remove(dir.path(), "ada#shape", assume_yes)
            .expect_err("must refuse whatever --yes says");
        assert!(
            err.contains("not installed") && err.contains("mfb pkg install"),
            "--yes={assume_yes} must not bypass the correctness gate: {err}"
        );
    }
}

/// plan-60-F §4.2: the cascade. Asserts the returned PAIRS, not just the
/// ident set, so the "imports X" attribution each line depends on is
/// covered.
#[test]
fn removal_closure_walks_reverse_dependencies() {
    // No importers: the closure is just the named target.
    assert_eq!(
        removal_closure("a#target", &imports_map(&[])),
        vec![("a#target".to_string(), None)]
    );

    // One level: widget imports target.
    assert_eq!(
        removal_closure("a#target", &imports_map(&[("a#widget", &["a#target"])])),
        vec![
            ("a#target".to_string(), None),
            ("a#widget".to_string(), Some("a#target".to_string())),
        ]
    );

    // Three levels: each line attributes its DIRECT importer, so a deep
    // cascade reads as a chain rather than a flat list.
    assert_eq!(
        removal_closure(
            "a#target",
            &imports_map(&[("a#widget", &["a#target"]), ("a#dashboard", &["a#widget"]),])
        ),
        vec![
            ("a#target".to_string(), None),
            ("a#widget".to_string(), Some("a#target".to_string())),
            ("a#dashboard".to_string(), Some("a#widget".to_string())),
        ]
    );

    // An unrelated package is untouched.
    let closure = removal_closure(
        "a#target",
        &imports_map(&[("a#widget", &["a#target"]), ("a#other", &["a#unrelated"])]),
    );
    assert!(
        !closure.iter().any(|(ident, _)| ident == "a#other"),
        "{closure:?}"
    );
}

/// A cycle must TERMINATE. Recursion without a visited set would not, and an
/// import cycle between two packages is legal.
#[test]
fn removal_closure_terminates_on_a_cycle() {
    let closure = removal_closure(
        "a#target",
        &imports_map(&[("a#one", &["a#target", "a#two"]), ("a#two", &["a#one"])]),
    );
    let idents: Vec<&str> = closure.iter().map(|(i, _)| i.as_str()).collect();
    assert_eq!(idents, vec!["a#target", "a#one", "a#two"]);
}

/// A diamond: two packages import the target and a third imports both. Every
/// ident must appear exactly once, or the cascade would double-count and the
/// printed total would be wrong.
#[test]
fn removal_closure_yields_each_ident_once_in_a_diamond() {
    let closure = removal_closure(
        "a#target",
        &imports_map(&[
            ("a#left", &["a#target"]),
            ("a#right", &["a#target"]),
            ("a#top", &["a#left", "a#right"]),
        ]),
    );
    let idents: Vec<&str> = closure.iter().map(|(i, _)| i.as_str()).collect();
    assert_eq!(idents.len(), 4, "{closure:?}");
    let unique: std::collections::BTreeSet<&&str> = idents.iter().collect();
    assert_eq!(unique.len(), 4, "each ident exactly once: {closure:?}");
    assert!(idents.contains(&"a#top"));
}

/// plan-60-E §4.2/§4.3: the selection matrix for a bare targeted update.
#[test]
fn select_update_version_covers_the_selection_matrix() {
    let abi = |symbols: &[&str]| {
        serde_json::Value::Object(
            symbols
                .iter()
                .map(|s| ((*s).to_string(), serde_json::json!("hash")))
                .collect(),
        )
    };

    // Nothing newer -> already latest, no change at all.
    assert_eq!(
        select_update_version(
            "1.0.0",
            &[index_version_with_abi("1.0.0", "available", abi(&["foo"]))]
        ),
        UpdateChoice::AlreadyLatest("1.0.0".to_string())
    );

    // A newer, eligible, ABI-compatible version -> take it, nothing skipped.
    assert_eq!(
        select_update_version(
            "1.0.0",
            &[
                index_version_with_abi("1.0.0", "available", abi(&["foo"])),
                index_version_with_abi("1.1.0", "available", abi(&["foo", "bar"])),
            ]
        ),
        UpdateChoice::Select {
            version: "1.1.0".to_string(),
            skipped: None
        }
    );

    // A newer version that is YANKED must be skipped entirely: the spec makes
    // yanked reachable only by exact pin, so a bare update must never move a
    // dependency onto one.
    assert_eq!(
        select_update_version(
            "1.0.0",
            &[
                index_version_with_abi("1.0.0", "available", abi(&["foo"])),
                index_version_with_abi("2.0.0", "yanked", abi(&["foo"])),
            ]
        ),
        UpdateChoice::AlreadyLatest("1.0.0".to_string()),
        "a yanked release is not an eligible target"
    );

    // The NEWEST drops a symbol, an older compatible one sits behind it:
    // select the older AND report the skip, naming the dropped symbol.
    let choice = select_update_version(
        "1.0.0",
        &[
            index_version_with_abi("1.0.0", "available", abi(&["foo", "bar"])),
            index_version_with_abi("1.6.0", "available", abi(&["foo", "bar", "baz"])),
            index_version_with_abi("2.0.0", "available", abi(&["foo"])),
        ],
    );
    assert_eq!(
        choice,
        UpdateChoice::Select {
            version: "1.6.0".to_string(),
            skipped: Some(("2.0.0".to_string(), vec!["bar".to_string()])),
        },
        "must select the compatible 1.6.0 and name 2.0.0's dropped `bar`"
    );

    // Nothing newer is compatible at all.
    assert_eq!(
        select_update_version(
            "1.0.0",
            &[
                index_version_with_abi("1.0.0", "available", abi(&["foo", "bar"])),
                index_version_with_abi("2.0.0", "available", abi(&["foo"])),
            ]
        ),
        UpdateChoice::NoCompatibleCandidate
    );
}

#[test]
fn parse_update_options_reads_the_target_and_flags() {
    let parse = |args: &[&str]| match parse_update_options(&s(args)) {
        Ok(options) => options,
        Err(PkgCommandError::Usage(m) | PkgCommandError::Failed(m)) => {
            panic!("expected `{args:?}` to parse, got: {m}")
        }
    };

    // Bare form: no target, no pin opinion.
    let bare = parse(&[]);
    assert_eq!(bare.target, None);
    assert_eq!(bare.pin, None);
    assert!(!bare.assume_yes);

    let targeted = parse(&["ada#shape@1.4.0", "--no-pin", "--yes"]);
    assert_eq!(targeted.target.as_deref(), Some("ada#shape@1.4.0"));
    assert_eq!(targeted.pin, Some(false));
    assert!(targeted.assume_yes);
}

#[test]
fn parse_update_options_rejects_bad_argument_shapes() {
    let err = |args: &[&str]| usage(parse_update_options(&s(args)).map(|_| ()));
    assert!(err(&["ada#shape", "--pin", "--no-pin"]).contains("cannot take both"));
    assert!(err(&["ada#shape", "--no-pin", "--pin"]).contains("cannot take both"));
    assert!(err(&["--bogus"]).contains("unknown flag"));
    assert!(err(&["a", "b"]).contains("at most one"));
}

/// plan-60-E §4.4: an undeclared target errors with an actionable hint,
/// **before** any network access.
#[test]
fn update_rejects_a_target_that_is_not_declared() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
    )
    .expect("manifest");

    let options = UpdateOptions {
        target: Some("ada#shape".to_string()),
        pin: None,
        assume_yes: false,
    };
    let err = run_update(dir.path(), &options).expect_err("must refuse");
    assert!(err.contains("is not declared in project.json"), "{err}");
    assert!(err.contains("mfb pkg add ada#shape"), "{err}");
}

/// §4.4 drops the `[location]` form, so a path-looking positional is an
/// ident that is not declared — not a directory to update in. Without this
/// the change would silently reinterpret an old invocation.
#[test]
fn update_treats_a_path_positional_as_an_undeclared_ident() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
    )
    .expect("manifest");

    let options = UpdateOptions {
        target: Some("./foo".to_string()),
        pin: None,
        assume_yes: false,
    };
    let err = run_update(dir.path(), &options).expect_err("must refuse");
    assert!(err.contains("is not declared in project.json"), "{err}");
}

/// plan-60-C §4.1, one case per row of the inference matrix.
///
/// | Invocation | `pin` |
/// |---|---|
/// | `add alice#shape` | `false` |
/// | `add alice#shape@1.4.0` | `true` |
/// | `add alice#shape --pin` | `true` |
/// | `add alice#shape@1.4.0 --no-pin` | `false` |
#[test]
fn infer_pin_follows_the_matrix() {
    // No flag: the presence of an explicit @version decides.
    assert!(!infer_pin(false, None), "bare add floats");
    assert!(infer_pin(true, None), "an explicit @version implies a pin");
    // An explicit flag always wins, in both directions.
    assert!(infer_pin(false, Some(true)), "--pin on a bare add");
    assert!(
        !infer_pin(true, Some(false)),
        "--no-pin overrides the @version implication; the version becomes the ABI floor"
    );
    // ...and is honoured even when it merely restates the default.
    assert!(infer_pin(true, Some(true)));
    assert!(!infer_pin(false, Some(false)));
}

#[test]
fn parse_add_options_reads_the_target_and_flags() {
    let parse = parsed;

    let bare = parse(&["ada#shape"]);
    assert_eq!(bare.target, "ada#shape");
    assert_eq!(bare.pin, None, "no flag means infer, not a default");

    assert_eq!(parse(&["ada#shape", "--pin"]).pin, Some(true));
    assert_eq!(parse(&["ada#shape", "--no-pin"]).pin, Some(false));
    // Flags may precede the positional.
    let leading = parse(&["--no-pin", "ada#shape"]);
    assert_eq!(leading.target, "ada#shape");
    assert_eq!(leading.pin, Some(false));
}

/// `--pin --no-pin` is a usage error rather than last-flag-wins: the two
/// orderings would otherwise mean different things with no way to tell from
/// the command line which was intended (§4.1).
#[test]
fn parse_add_options_rejects_bad_argument_shapes() {
    let err = |args: &[&str]| usage(parse_add_options(&s(args)).map(|_| ()));

    assert!(err(&["ada#shape", "--pin", "--no-pin"]).contains("cannot take both"));
    // ...in either order — neither wins.
    assert!(err(&["ada#shape", "--no-pin", "--pin"]).contains("cannot take both"));
    assert!(err(&["--bogus", "ada#shape"]).contains("unknown flag"));
    assert!(err(&["a", "b"]).contains("accepts exactly one"));
    assert!(err(&[]).contains("mfb pkg add requires"));
    assert!(err(&["--pin"]).contains("mfb pkg add requires"));
}

/// §4.2: `--no-pin` on a `file://` target is a usage error naming the
/// reason. `--pin` is accepted as a redundant statement of the truth.
#[test]
fn add_rejects_no_pin_on_a_file_url() {
    let options = parsed(&["file:///tmp/x.mfp", "--no-pin"]);
    let err = add_package(Path::new("."), &options).expect_err("must refuse");
    assert!(err.contains("no registry"), "{err}");
    assert!(err.contains("--no-pin"), "{err}");

    // `--pin` on a file:// target is a no-op, not an error: it gets past the
    // §4.2 guard and fails later, on the missing file.
    let pinned = parsed(&["file:///tmp/does-not-exist.mfp", "--pin"]);
    let err = add_package(Path::new("."), &pinned).expect_err("missing file");
    assert!(
        !err.contains("no registry"),
        "--pin must not be refused: {err}"
    );
}

#[test]
fn add_package_rejects_bad_target() {
    // Neither a file:// URL nor an <owner>#<package> ident.
    let err = add_package(Path::new("."), &add_opts("just-a-name")).unwrap_err();
    assert!(err.contains("expects a file:// URL or an <owner>#<package>"));
}

#[test]
fn add_package_from_registry_rejects_malformed_ident() {
    assert!(add_package_from_registry(Path::new("."), "no-hash", None)
        .unwrap_err()
        .contains("must use <owner>#<package>"));
    assert!(add_package_from_registry(Path::new("."), "#pkg", None)
        .unwrap_err()
        .contains("must use <owner>#<package>"));
    assert!(add_package_from_registry(Path::new("."), "owner#", None)
        .unwrap_err()
        .contains("must use <owner>#<package>"));
    // Empty version after `@`.
    assert!(
        add_package_from_registry(Path::new("."), "ada#shape@", None)
            .unwrap_err()
            .contains("must not be empty")
    );
}

#[test]
fn transfer_offer_rejects_bad_ident() {
    assert!(transfer_offer("no-hash", "bob")
        .unwrap_err()
        .contains("<owner>#<package>"));
}

#[test]
fn transfer_accept_requires_at_sign() {
    assert!(transfer_accept("ada#shape")
        .unwrap_err()
        .contains("<owner>#<package>@<to-owner>"));
}

#[test]
fn set_release_state_rejects_bad_state() {
    assert!(set_release_state(Path::new("."), "bogus", None)
        .unwrap_err()
        .contains("state must be one of"));
}

#[test]
fn set_release_state_requires_package_project() {
    // A directory without a project.json fails manifest validation.
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(set_release_state(dir.path(), "available", None)
        .unwrap_err()
        .contains("validation failed"));
}

#[test]
fn run_pkg_doc_rejects_bad_arguments() {
    assert!(usage(run_pkg_doc(&s(&["--out"]))).contains("--out requires a file"));
    assert!(usage(run_pkg_doc(&s(&["--bogus"]))).contains("unknown flag"));
    assert!(usage(run_pkg_doc(&s(&["a", "b"]))).contains("accepts exactly one"));
    assert!(usage(run_pkg_doc(&s(&[]))).contains("mfb pkg doc requires"));
}

#[test]
fn write_package_doc_reports_missing_package() {
    let err = write_package_doc("no-such-package", Path::new("/tmp/out.html")).unwrap_err();
    assert!(err.contains("no package named"));
}

#[test]
fn write_package_doc_renders_from_a_real_package() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = dir.path().join("doc.html");
    write_package_doc(UNSIGNED_FIXTURE, &out).expect("doc render");
    assert!(out.is_file());
}

#[test]
fn print_package_info_reads_a_real_package() {
    // Exercises the whole info printer over an unsigned fixture. It attempts
    // a best-effort registry lookup for the release state; the ident has no
    // `#`, so that lookup is skipped and no network call is made.
    assert!(print_package_info(Path::new(UNSIGNED_FIXTURE)).is_ok());
}

#[test]
fn print_package_info_reports_missing_file() {
    assert!(print_package_info(Path::new("/no/such/pkg.mfp"))
        .unwrap_err()
        .contains("failed to read"));
}

#[test]
fn validate_package_file_reports_missing_package() {
    let dir = tempfile::tempdir().expect("temp dir");
    let err = validate_package_file(dir.path(), "no-such-package").unwrap_err();
    assert!(err.contains("no package named"));
}

#[test]
fn validate_package_file_validates_an_unsigned_fixture() {
    let dir = tempfile::tempdir().expect("temp dir");
    // Unsigned package: container + payload hash checks pass, no trust chain.
    assert!(validate_package_file(dir.path(), UNSIGNED_FIXTURE).is_ok());
}

#[test]
fn validate_package_file_rejects_garbage_container() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garbage.mfp");
    std::fs::write(&path, b"not a container").expect("write");
    assert!(validate_package_file(dir.path(), path.to_str().unwrap()).is_err());
}

#[test]
fn project_pinned_ident_key_finds_declared_pin() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",",
            "\"sources\":[{\"root\":\"src\"}],",
            "\"packages\":[{\"name\":\"shape\",\"ident\":\"ada#shape\",\"version\":\"1.0.0\",\"pin\":true,\"source\":\"ada#shape\",\"identKey\":\"ed25519:pinned\"}]}"
        ),
    )
    .expect("manifest");
    assert_eq!(
        project_pinned_ident_key(dir.path(), "shape"),
        Some("ed25519:pinned".to_string())
    );
    // An undeclared package has no pin.
    assert_eq!(project_pinned_ident_key(dir.path(), "other"), None);
    // A missing manifest yields None (best-effort).
    let empty = tempfile::tempdir().expect("temp dir");
    assert_eq!(project_pinned_ident_key(empty.path(), "shape"), None);
}

#[test]
fn verify_packages_reports_no_manifest() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(verify_packages(dir.path(), false)
        .unwrap_err()
        .contains("failed to read"));
}

#[test]
fn verify_packages_ok_with_no_dependencies() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
    )
    .expect("manifest");
    assert!(verify_packages(dir.path(), false).is_ok());
}

#[test]
fn check_abi_requires_package_project() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(check_abi(dir.path())
        .unwrap_err()
        .contains("validation failed"));
}

/// plan-60-A: `publish` dispatches from `mfb repo` now, so this drives the
/// implementation directly rather than through `run_pkg_command` (which
/// correctly rejects the old spelling — see
/// `run_pkg_rejects_the_moved_publisher_commands`). The assertion is
/// unchanged: an empty directory is not a package project, and `publish`
/// must refuse it before doing anything else. The `repo` dispatch that now
/// reaches this code is covered by
/// `super::repo::tests::repo_publisher_commands_pin_their_arity`.
#[test]
fn publish_requires_package_project() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(publish_package_project("ada", dir.path())
        .unwrap_err()
        .contains("validation failed"));
}

#[test]
fn print_publish_verify_report_handles_both_diagnostic_states() {
    // With diagnostics.
    print_publish_verify_report(&mfb_repository::server::ValidatePackageResponse {
        valid: false,
        content_hash: "abc".to_string(),
        abi_index: serde_json::Value::Null,
        diagnostics: vec!["one".to_string(), "two".to_string()],
    });
    // Without diagnostics (the <none> branch) and an empty content hash.
    print_publish_verify_report(&mfb_repository::server::ValidatePackageResponse {
        valid: true,
        content_hash: String::new(),
        abi_index: serde_json::Value::Null,
        diagnostics: Vec::new(),
    });
}

#[test]
fn verify_package_dependency_reports_missing_package_as_invalid() {
    let dir = tempfile::tempdir().expect("temp dir");
    let dependency = ProjectPackageDependency {
        name: "absent".to_string(),
        ident: "ada#absent".to_string(),
        version: "1.0.0".to_string(),
        pin: false,
        source: "ada#absent".to_string(),
        ident_key: String::new(),
        direct: None,
        required_by: None,
    };
    // Neither a .mfp nor a source manifest exists -> InvalidPackage.
    assert_eq!(
        verify_package_dependency(dir.path(), &dependency).status,
        PackageVerifyStatus::InvalidPackage
    );
}

#[test]
fn add_package_from_file_reports_missing_source() {
    // A file:// URL to a non-existent .mfp surfaces a read error before any
    // manifest work.
    let err = add_package(Path::new("."), &add_opts("file:///no/such/pkg.mfp")).unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn package_verify_status_checks_name_and_version() {
    let dependency = ProjectPackageDependency {
        name: "shape".to_string(),
        ident: "ada#shape".to_string(),
        version: "1.2.3".to_string(),
        pin: true,
        source: "registry:mfb".to_string(),
        ident_key: String::new(),
        direct: None,
        required_by: None,
    };
    assert_eq!(
        package_dependency_status(&dependency, "shape", "ada#shape", "1.2.3"),
        PackageVerifyStatus::Ok
    );
    assert_eq!(
        package_dependency_status(&dependency, "shape", "ada#shape", "1.2.4"),
        PackageVerifyStatus::NeedsUpdate
    );
    assert_eq!(
        package_dependency_status(&dependency, "color", "ada#shape", "1.2.3"),
        PackageVerifyStatus::InvalidPackage
    );
    assert_eq!(
        package_dependency_status(&dependency, "shape", "other#shape", "1.2.3"),
        PackageVerifyStatus::InvalidPackage
    );
}

#[test]
fn package_verify_rejects_range_syntax_as_literal_version() {
    assert!(!package_version_matches("^1.2.3", "1.9.0"));
    assert!(!package_version_matches("~1.2.3", "1.2.9"));
    assert!(package_version_matches("1.2.3", "1.2.3"));
}

#[test]
fn package_verify_line_shows_project_and_package_versions() {
    let dependency = ProjectPackageDependency {
        name: "shape".to_string(),
        ident: "ada#shape".to_string(),
        version: "1.2.0".to_string(),
        pin: false,
        source: "registry:mfb".to_string(),
        ident_key: String::new(),
        direct: None,
        required_by: None,
    };

    assert_eq!(
        package_verify_line(
            &dependency,
            &PackageVerifyResult {
                version: "1.2.3".to_string(),
                status: PackageVerifyStatus::Ok,
            }
        ),
        "shape @ 1.2.0 : OK (1.2.3)"
    );
    assert_eq!(
        package_verify_line(
            &dependency,
            &PackageVerifyResult {
                version: String::new(),
                status: PackageVerifyStatus::InvalidPackage,
            }
        ),
        "shape @ 1.2.0 : Invalid Package"
    );
}

#[test]
fn package_verify_reads_source_package_manifest() {
    let root = test_temp_dir("package_verify_reads_source_package_manifest");
    let package_dir = root.join("packages").join("shape");
    fs::create_dir_all(&package_dir).expect("package dir");
    fs::write(
        package_dir.join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"shape\",\n",
            "  \"ident\": \"ada#shape\",\n",
            "  \"version\": \"1.2.3\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"package\",\n",
            "  \"sources\": [{ \"root\": \"src\" }]\n",
            "}\n"
        ),
    )
    .expect("package manifest");

    let dependency = ProjectPackageDependency {
        name: "shape".to_string(),
        ident: "ada#shape".to_string(),
        version: "1.2.3".to_string(),
        pin: false,
        source: "registry:mfb".to_string(),
        ident_key: String::new(),
        direct: None,
        required_by: None,
    };

    assert_eq!(
        verify_package_dependency(&root, &dependency),
        PackageVerifyResult {
            version: "1.2.3".to_string(),
            status: PackageVerifyStatus::Ok,
        }
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

fn test_temp_dir(name: &str) -> PathBuf {
    let current_thread = std::thread::current();
    let thread = current_thread.name().unwrap_or("test");
    let thread: String = thread
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{}_{}", std::process::id(), thread));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temp dir");
    root
}
