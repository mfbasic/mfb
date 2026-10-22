use super::*;
use std::cmp::Ordering;

#[test]
fn version_comparison_orders_releases_and_prereleases() {
    assert_eq!(compare_versions("1.0.1", "1.0.0"), Ordering::Greater);
    assert_eq!(compare_versions("1.2.0", "1.10.0"), Ordering::Less);
    assert_eq!(compare_versions("2.0.0", "2.0.0"), Ordering::Equal);
    // A release outranks the same release with a pre-release suffix.
    assert_eq!(compare_versions("1.0.0", "1.0.0-rc1"), Ordering::Greater);
    assert_eq!(
        compare_versions("1.0.0-rc2", "1.0.0-rc1"),
        Ordering::Greater
    );
    // Missing trailing components are implicit zeroes.
    assert_eq!(compare_versions("1.2", "1.2.0"), Ordering::Equal);
}

#[test]
fn version_comparison_does_not_coerce_malformed_components_to_zero() {
    // `"2x"` used to parse as 0, ranking `2x.0.0` exactly like `0.0.0` — so
    // a malformed registry version could silently outrank `0.9.0`.
    assert_ne!(compare_versions("2x.0.0", "0.0.0"), Ordering::Equal);
    assert_eq!(compare_versions("2.0.0", "2x.0.0"), Ordering::Greater);
    assert_eq!(compare_versions("2x.0.0", "2.0.0"), Ordering::Less);
    // A component too large for u64 is malformed, not zero.
    assert_eq!(
        compare_versions("18446744073709551616.0.0", "1.0.0"),
        Ordering::Less
    );
    // Two malformed components order lexically, so the comparison is total.
    assert_eq!(compare_versions("2x.0.0", "2y.0.0"), Ordering::Less);
    assert_eq!(compare_versions("2x.0.0", "2x.0.0"), Ordering::Equal);
}

#[test]
fn superset_requires_every_symbol_hash() {
    let mut exports = BTreeMap::new();
    exports.insert("foo".to_string(), "aa".to_string());
    exports.insert("bar".to_string(), "bb".to_string());
    let mut needs_foo = BTreeMap::new();
    needs_foo.insert("foo".to_string(), "aa".to_string());
    assert!(is_superset(&exports, &needs_foo));
    // A missing symbol is not a superset.
    let mut needs_baz = BTreeMap::new();
    needs_baz.insert("baz".to_string(), "cc".to_string());
    assert!(!is_superset(&exports, &needs_baz));
    // A differing hash is not a superset.
    let mut needs_foo_v2 = BTreeMap::new();
    needs_foo_v2.insert("foo".to_string(), "ff".to_string());
    assert!(!is_superset(&exports, &needs_foo_v2));
}

fn index_version(version: &str, state: &str) -> mfb_repository::server::IndexVersion {
    mfb_repository::server::IndexVersion {
        version: version.to_string(),
        hash: format!("hash-{version}"),
        published_at: 0,
        state: state.to_string(),
        abi_index: serde_json::Value::Null,
        log_entry: None,
        publisher: None,
        epoch: 0,
    }
}

fn node_with(
    versions: Vec<mfb_repository::server::IndexVersion>,
    requirers: Vec<Requirer>,
) -> Node {
    let mut map = BTreeMap::new();
    for (index, requirer) in requirers.into_iter().enumerate() {
        map.insert(format!("req-{index}"), requirer);
    }
    Node {
        name: "shape".to_string(),
        ident: "ada#shape".to_string(),
        index: mfb_repository::server::IndexResponse {
            ident: "ada#shape".to_string(),
            owner: "ada".to_string(),
            ident_key: "ed25519:ik".to_string(),
            ident_fingerprint: "if".to_string(),
            name_binding_signature: String::new(),
            server_fingerprint: "sf".to_string(),
            versions,
            transfers: Vec::new(),
        },
        requested: "1.0.0".to_string(),
        requirers: map,
        selected: None,
        pinned_ident_key: String::new(),
    }
}

fn select_node_err(node: &Node) -> String {
    match select_node(node) {
        Ok(_) => panic!("expected select_node to fail"),
        Err(message) => message,
    }
}

fn requirer(who: &str, required: &[(&str, &str)], pin: Option<&str>) -> Requirer {
    Requirer {
        who: who.to_string(),
        required: required
            .iter()
            .map(|(name, hash)| (name.to_string(), hash.to_string()))
            .collect(),
        pin: pin.map(str::to_string),
    }
}

#[test]
fn read_manifest_reads_and_validates() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
    )
    .expect("manifest");
    let (manifest, _contents) = read_manifest(dir.path()).expect("read manifest");
    assert_eq!(
        manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .map(String::as_str),
        Some("app")
    );
    // A missing manifest is a read error.
    let empty = tempfile::tempdir().expect("temp dir");
    assert!(read_manifest(empty.path())
        .unwrap_err()
        .contains("failed to read"));
}

#[test]
fn install_without_a_lock_errors_before_network() {
    let dir = tempfile::tempdir().expect("temp dir");
    // plan-60-F §4.6: the fixture now DECLARES a dependency. That is what
    // this test was always about — "dependencies are declared but there is
    // no lock" is the error case — but it originally used an empty manifest,
    // which §4.6 reclassifies as a correctly configured project with nothing
    // to install. The protected behavior (early error, no /blob fetch) is
    // unchanged; the fixture now actually exercises it.
    std::fs::write(
        dir.path().join("project.json"),
        manifest_with_packages(
            "{\"name\":\"shape\",\"ident\":\"ada#shape\",\"version\":\"1.0.0\"}",
        ),
    )
    .expect("manifest");
    // No mfb.lock present -> early error, no /blob fetch.
    assert!(install(dir.path()).unwrap_err().contains("no mfb.lock"));
}

/// plan-60-F §4.6: a project that declares no registry dependencies has
/// nothing to install, which is success — not the "run `mfb pkg update`"
/// error, which told the user to run a command that would do nothing.
/// Reached after `mfb pkg remove` takes the last dependency, and in a fresh
/// `mfb init` project.
#[test]
fn install_without_declared_dependencies_is_a_no_op() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("project.json"), manifest_with_packages("")).expect("manifest");
    assert!(
        install(dir.path()).is_ok(),
        "a project with nothing declared has nothing to install"
    );
    // A local file:// package is not a registry dependency either, so it
    // must not resurrect the error.
    std::fs::write(
        dir.path().join("project.json"),
        manifest_with_packages("{\"name\":\"shape\",\"source\":\"file://shape.mfp\"}"),
    )
    .expect("manifest");
    assert!(install(dir.path()).is_ok());
}

#[test]
fn install_with_stale_lock_errors_before_network() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
    )
    .expect("manifest");
    // A lock whose projectHash does not match the current project.
    std::fs::write(
        dir.path().join("mfb.lock"),
        "{\"lockfileVersion\":1,\"projectHash\":\"stale\",\"repoFingerprint\":\"r\",\"checkpoint\":{\"size\":0,\"rootHash\":\"\"},\"packages\":[]}\n",
    )
    .expect("lock");

    // plan-60-D replaced the opaque "mfb.lock is stale" verdict with a field
    // diff. This fixture — no declared dependencies, no locked packages, but
    // a mismatched hash — is exactly §4.2's Unattributable case: the hash
    // differs and nothing the lock RECORDS does, because `pin` and `source`
    // are hashed but not stored in the lock.
    //
    // The test's original intent is unchanged and still asserted: install
    // refuses, and does so before any network call. Only the message moved,
    // deliberately, and the new one is more specific than the old.
    let err = install(dir.path()).unwrap_err();
    assert!(
        err.contains("not in a field the lock records"),
        "expected the unattributable-drift message, got: {err}"
    );
    assert!(err.contains("mfb pkg update"), "{err}");
}

/// plan-60-D §4.1: a moved pin is fatal, and — like the unattributable case
/// above — is decided before any network call. Complements that test by
/// covering a concrete drift class rather than the empty-diff fallback.
#[test]
fn install_with_a_moved_pin_errors_before_network() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        manifest_with_packages(
            "{\"name\":\"shape\",\"ident\":\"ada#shape\",\"version\":\"1.4.0\",\"pin\":true}",
        ),
    )
    .expect("manifest");
    std::fs::write(
        dir.path().join("mfb.lock"),
        "{\"lockfileVersion\":1,\"projectHash\":\"stale\",\"repoFingerprint\":\"r\",             \"checkpoint\":{\"size\":0,\"rootHash\":\"\"},\"packages\":[{\"name\":\"shape\",             \"ident\":\"ada#shape\",\"requested\":\"1.3.0\",\"selected\":\"1.3.0\",\"hash\":\"h\",             \"identKey\":\"\",\"identFingerprint\":\"\",\"state\":\"available\"}]}\n",
    )
    .expect("lock");

    let err = install(dir.path()).unwrap_err();
    assert!(err.contains("pinned to 1.4.0"), "{err}");
    assert!(err.contains("mfb.lock records 1.3.0"), "{err}");
}

/// plan-60-D §4.1: the same drift on a FLOATING dependency warns and
/// proceeds. It cannot reach `Ok` here — installing needs a registry — but
/// it must get PAST the drift gate, which the pin case does not. Asserted by
/// the error being about the key store rather than about drift.
#[test]
fn install_with_a_moved_floor_passes_the_drift_gate() {
    let _lock = super::super::tests::ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("home");
    let _mfb = super::super::tests::EnvVarGuard::set(
        "MFB_HOME",
        home.path().to_str().expect("utf-8 home"),
    );

    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        manifest_with_packages(
            "{\"name\":\"shape\",\"ident\":\"ada#shape\",\"version\":\"1.4.0\",\"pin\":false}",
        ),
    )
    .expect("manifest");
    std::fs::write(
        dir.path().join("mfb.lock"),
        "{\"lockfileVersion\":1,\"projectHash\":\"stale\",\"repoFingerprint\":\"r\",             \"checkpoint\":{\"size\":0,\"rootHash\":\"\"},\"packages\":[{\"name\":\"shape\",             \"ident\":\"ada#shape\",\"requested\":\"1.3.0\",\"selected\":\"1.3.2\",\"hash\":\"h\",             \"identKey\":\"\",\"identFingerprint\":\"\",\"state\":\"available\"}]}\n",
    )
    .expect("lock");

    let err = install(dir.path()).unwrap_err();
    assert!(
        !err.contains("floating") && !err.contains("not in a field"),
        "a moved ABI floor must NOT be refused as drift, got: {err}"
    );
}

#[test]
fn read_lock_absent_returns_none() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(read_lock(dir.path()).expect("read").is_none());
}

/// A manifest with the given `packages` array body.
fn manifest_with_packages(packages: &str) -> String {
    format!(
        "{{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\
             \"sources\":[{{\"root\":\"src\"}}],\"packages\":[{packages}]}}"
    )
}

/// plan-60-B §4.3: a manifest with no registry dependencies writes
/// `project.json` and **removes** `mfb.lock`. There is nothing left to lock,
/// and an absent lock is the same state a freshly-`mfb init`-ed project is
/// in, which `mfb pkg install` already reports correctly.
#[test]
fn apply_manifest_change_zero_dependencies_writes_manifest_and_drops_the_lock() {
    let dir = tempfile::tempdir().expect("temp dir");
    let project = dir.path().join("project.json");
    let lock = dir.path().join("mfb.lock");
    std::fs::write(&project, manifest_with_packages("")).expect("seed manifest");
    std::fs::write(&lock, "{\"lockfileVersion\":1}").expect("seed lock");

    let new_contents = manifest_with_packages("");
    apply_manifest_change(dir.path(), &new_contents).expect("zero-dependency path");

    assert_eq!(
        std::fs::read_to_string(&project).expect("read manifest"),
        new_contents
    );
    assert!(
        !lock.exists(),
        "a project with no registry deps keeps no lock"
    );
}

/// The same path must be fine when there is no lock to begin with — it is
/// the state `mfb init` leaves, and removing a file that is not there must
/// not be an error.
#[test]
fn apply_manifest_change_zero_dependencies_tolerates_a_missing_lock() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("project.json"), manifest_with_packages(""))
        .expect("seed manifest");

    apply_manifest_change(dir.path(), &manifest_with_packages("")).expect("no lock to remove");
    assert!(!dir.path().join("mfb.lock").exists());
}

/// Malformed proposed text must be rejected **before** anything is written,
/// so a caller that builds a bad manifest cannot corrupt the project.
#[test]
fn apply_manifest_change_rejects_bad_text_without_writing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let project = dir.path().join("project.json");
    let original = manifest_with_packages("");
    std::fs::write(&project, &original).expect("seed manifest");

    apply_manifest_change(dir.path(), "{not json").expect_err("must reject");
    assert_eq!(
        std::fs::read_to_string(&project).expect("read manifest"),
        original,
        "project.json must be byte-identical after a rejected change"
    );

    // `packages` present but not an array — caught by validate_packages_array.
    let bad = "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\
                   \"sources\":[{\"root\":\"src\"}],\"packages\":\"nope\"}";
    apply_manifest_change(dir.path(), bad).expect_err("must reject non-array packages");
    assert_eq!(
        std::fs::read_to_string(&project).expect("read manifest"),
        original
    );
}

/// plan-60-B §4.3 hinges on "zero registry dependencies" meaning exactly
/// what `resolve()` means by it. `resolve()` seeds from
/// `project_package_dependency` filtered by `ident.contains('#')` and errors
/// on an empty set; if `registry_dependency_count` drifted from that filter,
/// `apply_manifest_change` would either call `resolve()` on a set it is
/// about to reject, or skip locking dependencies that genuinely need it.
///
/// A local `file://`-added package has an `ident` equal to its name (no
/// `#`), so it must NOT count.
#[test]
fn registry_dependency_count_matches_the_resolver_seeding_filter() {
    let local = "{\"name\":\"shape\",\"source\":\"file://shape.mfp\"}";
    let registry = "{\"name\":\"shape\",\"ident\":\"ada#shape\",\"version\":\"1.0.0\"}";
    let unnamed = "{\"version\":\"1.0.0\"}";

    let count = |packages: &str| {
        let text = manifest_with_packages(packages);
        let manifest =
            parse_project_json(&text, std::path::Path::new("project.json")).expect("parse");
        registry_dependency_count(&manifest)
    };

    assert_eq!(count(""), 0, "no packages");
    assert_eq!(
        count(local),
        0,
        "a local file:// package is not a registry dep"
    );
    assert_eq!(count(unnamed), 0, "an unusable entry is not a registry dep");
    assert_eq!(count(registry), 1);
    assert_eq!(
        count(&format!("{local},{registry}")),
        1,
        "only the registry one"
    );

    // plan-60-C §5: the case that made this a data-loss bug. A package that
    // was published and THEN added by `file://` carries a registry-shaped
    // ident, because `add_package_from_file` copies the ident out of the
    // .mfp header rather than the URL. Keying on the ident alone admitted
    // it as a resolver node and `mfb pkg update` overwrote the user's local
    // file with a registry blob. `source` is what disambiguates.
    let published_then_file_added = "{\"name\":\"shape\",\"ident\":\"ada#shape\",\
             \"version\":\"1.0.0\",\"source\":\"file:///tmp/shape.mfp\"}";
    assert_eq!(
        count(published_then_file_added),
        0,
        "a file:// source is never a registry dep, whatever its ident says"
    );
}

#[test]
fn read_lock_rejects_non_json() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("mfb.lock"), "not json").expect("lock");
    assert!(read_lock(dir.path()).is_err());
}

#[test]
fn write_lock_writes_a_readable_lockfile() {
    let dir = tempfile::tempdir().expect("temp dir");
    let lock = Lock {
        project_hash: "p".to_string(),
        repo_fingerprint: "r".to_string(),
        checkpoint_size: 3,
        checkpoint_root: "root".to_string(),
        packages: Vec::new(),
    };
    write_lock(dir.path(), &lock).expect("write");
    assert!(dir.path().join("mfb.lock").is_file());
    // The written lock round-trips.
    let reread = read_lock(dir.path()).expect("read").expect("present");
    assert_eq!(reread.project_hash, "p");
    assert_eq!(reread.checkpoint_size, 3);
}

#[test]
fn select_node_picks_highest_superset_version() {
    let node = node_with(
        vec![
            index_version("1.0.0", "available"),
            index_version("2.0.0", "available"),
            index_version("1.5.0", "deprecated"),
        ],
        vec![requirer("project", &[], None)],
    );
    let selection = select_node(&node).expect("selection");
    // Highest floating-eligible version with no ABI needs is 2.0.0.
    assert_eq!(selection.version, "2.0.0");
}

#[test]
fn select_node_honors_a_pin() {
    let node = node_with(
        vec![
            index_version("1.0.0", "available"),
            index_version("2.0.0", "available"),
        ],
        vec![requirer("project", &[], Some("1.0.0"))],
    );
    let selection = select_node(&node).expect("selection");
    assert_eq!(selection.version, "1.0.0");
}

#[test]
fn select_node_rejects_conflicting_pins() {
    let node = node_with(
        vec![
            index_version("1.0.0", "available"),
            index_version("2.0.0", "available"),
        ],
        vec![
            requirer("a", &[], Some("1.0.0")),
            requirer("b", &[], Some("2.0.0")),
        ],
    );
    assert!(select_node_err(&node).contains("conflicting pins"));
}

#[test]
fn select_node_rejects_unpublished_pin() {
    let node = node_with(
        vec![index_version("1.0.0", "available")],
        vec![requirer("a", &[], Some("9.9.9"))],
    );
    assert!(select_node_err(&node).contains("is not published"));
}

#[test]
fn select_node_rejects_blocked_pin() {
    let node = node_with(
        vec![index_version("1.0.0", "blocked")],
        vec![requirer("a", &[], Some("1.0.0"))],
    );
    assert!(select_node_err(&node).contains("cannot be selected"));
}

#[test]
fn select_node_reports_diamond_conflict() {
    let node = node_with(
        vec![index_version("1.0.0", "available")],
        vec![
            requirer("a", &[("foo", "aa")], None),
            requirer("b", &[("foo", "bb")], None),
        ],
    );
    assert!(select_node_err(&node).contains("diamond conflict"));
}

#[test]
fn select_node_reports_no_satisfying_version() {
    // A required symbol no version provides -> no eligible candidate.
    let node = node_with(
        vec![index_version("1.0.0", "available")],
        vec![requirer("a", &[("missing", "zz")], None)],
    );
    assert!(select_node_err(&node).contains("no install-eligible version"));
}

/// plan-60-D §4.1, one case per row of the classification table.
///
/// The pair that matters most is `FloorMoved` vs `PinMoved`: identical
/// version drift, and `pin` alone decides whether install warns and
/// continues or refuses. Getting that backwards would either block every
/// floating project or silently install a pinned project's wrong version.
#[test]
fn classify_drift_covers_every_class() {
    let locked = |name: &str, ident: &str, requested: &str, selected: &str| LockedPackage {
        name: name.to_string(),
        ident: ident.to_string(),
        requested: requested.to_string(),
        selected: selected.to_string(),
        hash: "h".to_string(),
        ident_key: String::new(),
        ident_fingerprint: String::new(),
        state: "available".to_string(),
    };
    let manifest_of = |packages: &str| {
        let text = manifest_with_packages(packages);
        parse_project_json(&text, std::path::Path::new("project.json")).expect("parse")
    };
    let dep = |name: &str, ident: &str, version: &str, pin: bool| {
        format!(
            "{{\"name\":\"{name}\",\"ident\":\"{ident}\",\"version\":\"{version}\",\"pin\":{pin}}}"
        )
    };

    // Added: declared, not locked.
    assert_eq!(
        classify_drift(
            &manifest_of(&dep("shape", "ada#shape", "1.0.0", false)),
            &[]
        ),
        vec![("ada#shape".to_string(), DriftClass::Added)]
    );

    // Removed: locked, not declared.
    assert_eq!(
        classify_drift(
            &manifest_of(""),
            &[locked("shape", "ada#shape", "1.0.0", "1.0.0")]
        ),
        vec![("ada#shape".to_string(), DriftClass::Removed)]
    );

    // FloorMoved: pin: false, version differs. Warn-and-continue.
    assert_eq!(
        classify_drift(
            &manifest_of(&dep("shape", "ada#shape", "1.4.0", false)),
            &[locked("shape", "ada#shape", "1.3.0", "1.3.2")]
        ),
        vec![(
            "ada#shape".to_string(),
            DriftClass::FloorMoved {
                declared: "1.4.0".to_string(),
                locked: "1.3.0".to_string(),
                // The user needs the SELECTED version: it is what lands on disk.
                selected: "1.3.2".to_string(),
            }
        )]
    );

    // PinMoved: same drift, pin: true. Error.
    assert_eq!(
        classify_drift(
            &manifest_of(&dep("shape", "ada#shape", "1.4.0", true)),
            &[locked("shape", "ada#shape", "1.3.0", "1.3.0")]
        ),
        vec![(
            "ada#shape".to_string(),
            DriftClass::PinMoved {
                declared: "1.4.0".to_string(),
                locked: "1.3.0".to_string(),
            }
        )]
    );

    // Renamed: same ident, different name.
    assert_eq!(
        classify_drift(
            &manifest_of(&dep("newname", "ada#shape", "1.0.0", false)),
            &[locked("shape", "ada#shape", "1.0.0", "1.0.0")]
        ),
        vec![(
            "ada#shape".to_string(),
            DriftClass::Renamed {
                declared: "newname".to_string(),
                locked: "shape".to_string(),
            }
        )]
    );

    // No difference in any diffable field → empty. `install` reads this as
    // Unattributable (§4.2) because only it knows a hash mismatch occurred;
    // `pin`/`source` are hashed but not recorded in the lock.
    assert!(classify_drift(
        &manifest_of(&dep("shape", "ada#shape", "1.0.0", false)),
        &[locked("shape", "ada#shape", "1.0.0", "1.0.0")]
    )
    .is_empty());
}

/// §4.1: classification collects ALL differences before deciding, so a
/// project with several drifted dependencies reports every one rather than
/// stopping at the first.
#[test]
fn classify_drift_reports_every_drifted_dependency() {
    let locked = |name: &str, ident: &str, requested: &str| LockedPackage {
        name: name.to_string(),
        ident: ident.to_string(),
        requested: requested.to_string(),
        selected: requested.to_string(),
        hash: "h".to_string(),
        ident_key: String::new(),
        ident_fingerprint: String::new(),
        state: "available".to_string(),
    };
    let text = manifest_with_packages(
        "{\"name\":\"a\",\"ident\":\"o#a\",\"version\":\"2.0.0\",\"pin\":true},\
             {\"name\":\"b\",\"ident\":\"o#b\",\"version\":\"2.0.0\",\"pin\":false},\
             {\"name\":\"c\",\"ident\":\"o#c\",\"version\":\"1.0.0\",\"pin\":false}",
    );
    let manifest = parse_project_json(&text, std::path::Path::new("project.json")).expect("parse");
    let drift = classify_drift(
        &manifest,
        &[
            locked("a", "o#a", "1.0.0"),
            locked("b", "o#b", "1.0.0"),
            locked("gone", "o#gone", "1.0.0"),
        ],
    );
    // a: PinMoved, b: FloorMoved, c: Added, o#gone: Removed — all four.
    assert_eq!(drift.len(), 4, "{drift:?}");
    assert!(drift
        .iter()
        .any(|(i, c)| i == "o#a" && matches!(c, DriftClass::PinMoved { .. })));
    assert!(drift
        .iter()
        .any(|(i, c)| i == "o#b" && matches!(c, DriftClass::FloorMoved { .. })));
    assert!(drift
        .iter()
        .any(|(i, c)| i == "o#c" && *c == DriftClass::Added));
    assert!(drift
        .iter()
        .any(|(i, c)| i == "o#gone" && *c == DriftClass::Removed));
}

#[test]
fn print_lock_diff_covers_add_change_keep_and_remove() {
    let package = |name: &str, ident: &str, selected: &str, state: &str| LockedPackage {
        name: name.to_string(),
        ident: ident.to_string(),
        requested: "1.0.0".to_string(),
        selected: selected.to_string(),
        hash: "h".to_string(),
        ident_key: String::new(),
        ident_fingerprint: String::new(),
        state: state.to_string(),
    };
    let previous = Lock {
        project_hash: "p".to_string(),
        repo_fingerprint: "r".to_string(),
        checkpoint_size: 0,
        checkpoint_root: String::new(),
        packages: vec![
            package("kept", "a#kept", "1.0.0", "available"),
            package("bumped", "a#bumped", "1.0.0", "available"),
            package("gone", "a#gone", "1.0.0", "available"),
        ],
    };
    let next = Lock {
        project_hash: "p".to_string(),
        repo_fingerprint: "r".to_string(),
        checkpoint_size: 0,
        checkpoint_root: String::new(),
        packages: vec![
            package("kept", "a#kept", "1.0.0", "available"),
            package("bumped", "a#bumped", "2.0.0", "available"),
            package("added", "a#added", "1.0.0", "available"),
        ],
    };
    // Exercises +/~/keep/- lines; must not panic. Also covers the None-previous path.
    print_lock_diff(Some(&previous), &next);
    print_lock_diff(None, &next);
}

#[test]
fn lockfile_round_trips_byte_identically() {
    let lock = Lock {
        project_hash: "abc123".to_string(),
        repo_fingerprint: "def456".to_string(),
        checkpoint_size: 7,
        checkpoint_root: "0f0f".to_string(),
        packages: vec![
            LockedPackage {
                name: "beta".to_string(),
                ident: "alice#beta".to_string(),
                requested: "1.0.0".to_string(),
                selected: "1.0.1".to_string(),
                hash: "aa".to_string(),
                ident_key: "ed25519:xyz".to_string(),
                ident_fingerprint: "ff".to_string(),
                state: "available".to_string(),
            },
            LockedPackage {
                name: "alpha".to_string(),
                ident: "alice#alpha".to_string(),
                requested: "2.0.0".to_string(),
                selected: "2.0.0".to_string(),
                hash: "bb".to_string(),
                ident_key: "ed25519:uvw".to_string(),
                ident_fingerprint: "ee".to_string(),
                state: "deprecated".to_string(),
            },
        ],
    };
    let rendered = render_lock(&lock);
    // A rebuilt lock renders identically (deterministic resolution).
    let temp = std::env::temp_dir().join(format!("mfb-lock-test-{}", lock.project_hash));
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::write(temp.join("mfb.lock"), &rendered).unwrap();
    let reread = read_lock(&temp).unwrap().unwrap();
    assert_eq!(render_lock(&reread), rendered);
    assert_eq!(reread.packages.len(), 2);
    assert_eq!(reread.checkpoint_size, 7);
    std::fs::remove_dir_all(&temp).ok();
}

/// §4.1's outcome rule: every fatal class is reported, one line each, in
/// declaration-then-lock order.
#[test]
fn report_drift_names_every_added_removed_and_renamed_dependency() {
    let locked = |name: &str, ident: &str| LockedPackage {
        name: name.to_string(),
        ident: ident.to_string(),
        requested: "1.0.0".to_string(),
        selected: "1.0.0".to_string(),
        hash: "h".to_string(),
        ident_key: String::new(),
        ident_fingerprint: String::new(),
        state: "available".to_string(),
    };
    let text = manifest_with_packages(
        "{\"name\":\"new\",\"ident\":\"o#new\",\"version\":\"1.0.0\",\"pin\":false},\
         {\"name\":\"shape2\",\"ident\":\"o#shape\",\"version\":\"1.0.0\",\"pin\":false}",
    );
    let manifest = parse_project_json(&text, std::path::Path::new("project.json")).expect("parse");
    let lock = Lock {
        project_hash: "p".to_string(),
        repo_fingerprint: "r".to_string(),
        checkpoint_size: 0,
        checkpoint_root: String::new(),
        packages: vec![locked("shape", "o#shape"), locked("gone", "o#gone")],
    };
    assert_eq!(
        report_drift(&manifest, &lock),
        Err([
            "o#new is declared in project.json but absent from mfb.lock. Run `mfb pkg update`.",
            "o#shape is named shape2 in project.json but shape in mfb.lock. Run `mfb pkg update`.",
            "o#gone is recorded in mfb.lock but no longer declared in project.json. Run `mfb pkg \
             update`.",
        ]
        .join("\n"))
    );
}
