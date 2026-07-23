use std::process::Command;

mod common;
use common::*;

#[test]
fn repo_end_to_end_install_verifies_signed_package() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "alice"])
        .status
        .success());

    // Build a signed package.
    let package_dir = work.path().join("verified_pkg");
    let package_dir_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir_arg])
        .status
        .success());
    assert!(run_mfb(
        &repo,
        home.path(),
        &["build", "--sign", "alice", package_dir_arg],
    )
    .status
    .success());
    let mfp_path = package_dir.join("verified_pkg.mfp");
    assert!(mfp_path.is_file());

    // Consumer project: `pkg add` pins the identKey on first use (TOFU).
    let app_dir = work.path().join("consumer_app");
    let app_dir_arg = app_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_dir_arg]).status.success());
    let add = Command::new(mfb_exe())
        .args(["pkg", "add", &format!("file://{}", mfp_path.display())])
        .current_dir(&app_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("pkg add");
    assert!(
        add.status.success(),
        "pkg add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let manifest = std::fs::read_to_string(app_dir.join("project.json")).unwrap();
    assert!(
        manifest.contains("\"identKey\": \"ed25519:"),
        "pkg add must pin the identKey: {manifest}"
    );

    // The consumer build walks the full §3.5 chain and reports Verified.
    let build = Command::new(mfb_exe())
        .args(["build"])
        .current_dir(&app_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("consumer build");
    let stdout = String::from_utf8_lossy(&build.stdout);
    assert!(
        build.status.success(),
        "consumer build failed: stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(
        stdout.contains("uses verified_pkg - [Verified]"),
        "{stdout}"
    );

    // `pkg verify` reports the same trust state per dependency.
    let verify = Command::new(mfb_exe())
        .args(["pkg", "verify"])
        .current_dir(&app_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("pkg verify");
    let verify_stdout = String::from_utf8_lossy(&verify.stdout);
    assert!(verify.status.success(), "{verify_stdout}");
    assert!(verify_stdout.contains("[Verified]"), "{verify_stdout}");

    // `pkg validate <pkg>` checks the existing package end-to-end.
    let validate = Command::new(mfb_exe())
        .args(["pkg", "validate", "verified_pkg"])
        .current_dir(&app_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("pkg validate");
    let validate_stdout = String::from_utf8_lossy(&validate.stdout);
    assert!(
        validate.status.success(),
        "pkg validate failed: {validate_stdout}\n{}",
        String::from_utf8_lossy(&validate.stderr)
    );
    assert!(
        validate_stdout.contains("result: valid"),
        "{validate_stdout}"
    );
    assert!(
        validate_stdout.contains("attestation: OK"),
        "{validate_stdout}"
    );
    assert!(validate_stdout.contains("proof: OK"), "{validate_stdout}");
    assert!(
        validate_stdout.contains("ident pin: OK"),
        "{validate_stdout}"
    );

    // Tamper with the installed package: the consumer build must refuse.
    let installed = app_dir.join("packages/verified_pkg.mfp");
    let mut bytes = std::fs::read(&installed).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(&installed, &bytes).unwrap();
    let build = Command::new(mfb_exe())
        .args(["build"])
        .current_dir(&app_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("tampered consumer build");
    assert!(!build.status.success());
    let stdout = String::from_utf8_lossy(&build.stdout);
    let stderr = String::from_utf8_lossy(&build.stderr);
    assert!(
        stdout.contains("uses verified_pkg - [Tampered]"),
        "{stdout}"
    );
    assert!(
        stderr.contains("6-605-0006") || stderr.contains("PACKAGE_PAYLOAD_HASH_MISMATCH"),
        "{stderr}"
    );
}

/// plan-60-F: `mfb pkg remove` cascades to reverse dependencies.
///
/// Publishes `alice#dep` and `alice#user` (which imports `dep`), adds both to a
/// consumer, then removes `dep`. **`user` must go too** — otherwise its import
/// edge would name an undeclared ident, which `resolve()` silently drops,
/// leaving a project that resolves clean and fails at build time.
///
/// Asserting only that the command succeeded would pass even if the cascade
/// removed nothing but the named target, so this asserts `alice#user` is gone
/// from `project.json` and that both `.mfp` files are deleted.
#[test]
fn remove_cascades_to_packages_that_import_the_target() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "alice"])
        .status
        .success());

    // alice#dep — a leaf package.
    let dep_dir = work.path().join("dep");
    let dep_arg = dep_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", dep_arg]).status.success());
    let dep_manifest = dep_dir.join("project.json");
    std::fs::write(
        &dep_manifest,
        std::fs::read_to_string(&dep_manifest).unwrap().replace(
            "  \"version\": \"0.1.0\",\n",
            "  \"version\": \"1.0.0\",\n  \"ident\": \"alice#dep\",\n",
        ),
    )
    .unwrap();
    std::fs::write(
        dep_dir.join("src/lib.mfb"),
        "EXPORT FUNC shared() AS Integer\n  RETURN 1\nEND FUNC\n",
    )
    .unwrap();
    assert!(
        run_mfb(&repo, home.path(), &["repo", "publish", "alice", dep_arg])
            .status
            .success()
    );
    let dep_mfp = dep_dir.join("dep.mfp");

    // alice#user — imports dep.
    let user_dir = work.path().join("user");
    let user_arg = user_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", user_arg]).status.success());
    let user_manifest = user_dir.join("project.json");
    std::fs::write(
        &user_manifest,
        std::fs::read_to_string(&user_manifest).unwrap().replace(
            "  \"version\": \"0.1.0\",\n",
            "  \"version\": \"1.0.0\",\n  \"ident\": \"alice#user\",\n",
        ),
    )
    .unwrap();
    assert!(run_mfb_in(
        &repo,
        home.path(),
        &user_dir,
        &["pkg", "add", &format!("file://{}", dep_mfp.display())]
    )
    .status
    .success());
    std::fs::write(
        user_dir.join("src/lib.mfb"),
        "IMPORT dep\nEXPORT FUNC callShared() AS Integer\n  RETURN dep::shared()\nEND FUNC\n",
    )
    .unwrap();
    let publish_user = run_mfb(&repo, home.path(), &["repo", "publish", "alice", user_arg]);
    assert!(
        publish_user.status.success(),
        "publish user failed: {}",
        String::from_utf8_lossy(&publish_user.stderr)
    );

    // Consumer declares both.
    let app = work.path().join("cascade_consumer");
    let app_arg = app.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_arg]).status.success());
    let run_in = |args: &[&str]| run_mfb_in(&repo, home.path(), &app, args);
    assert!(run_in(&["pkg", "add", "alice#dep"]).status.success());
    assert!(run_in(&["pkg", "add", "alice#user"]).status.success());
    assert!(app.join("packages/dep.mfp").is_file());
    assert!(app.join("packages/user.mfp").is_file());

    // Remove the leaf: `user` must cascade.
    let removed = run_in(&["pkg", "remove", "alice#dep", "--yes"]);
    assert!(
        removed.status.success(),
        "remove failed: {}",
        String::from_utf8_lossy(&removed.stderr)
    );
    let stdout = String::from_utf8_lossy(&removed.stdout);
    assert!(
        stdout.contains("imports alice#dep"),
        "the cascade must explain WHY user is going: {stdout}"
    );

    let manifest = std::fs::read_to_string(app.join("project.json")).unwrap();
    assert!(!manifest.contains("alice#dep"), "{manifest}");
    assert!(
        !manifest.contains("alice#user"),
        "the cascade must remove the importer too, not just the named target: {manifest}"
    );
    assert!(
        !app.join("packages/dep.mfp").exists(),
        "dep.mfp must be deleted"
    );
    assert!(
        !app.join("packages/user.mfp").exists(),
        "user.mfp must be deleted"
    );

    // That was the last dependency: mfb.lock goes, and install is a clean no-op.
    assert!(
        !app.join("mfb.lock").exists(),
        "removing the last dependency must delete mfb.lock"
    );
    let install = run_in(&["pkg", "install"]);
    assert!(
        install.status.success(),
        "install must be a no-op with nothing declared: {}",
        String::from_utf8_lossy(&install.stderr)
    );
    assert!(
        String::from_utf8_lossy(&install.stdout).contains("nothing to install"),
        "{}",
        String::from_utf8_lossy(&install.stdout)
    );
}

/// plan-60-E §4.2/§4.3/§4.1: the targeted `mfb pkg update` form.
///
/// Publishes 1.0.0 (exports `answer` + `extra`), 1.1.0 (a compatible superset)
/// and 2.0.0 (**drops `extra`**). A bare targeted update must select 1.1.0, not
/// 2.0.0, and must say why — the ABI filter exists because `select_node`'s pin
/// branch takes an exact version with no ABI check at all.
///
/// Also asserts the two properties that separate this form from `add`: an
/// explicit `@version` is honored regardless of the filter, and **pin state is
/// preserved** unless a flag says otherwise.
#[test]
fn update_targeted_applies_the_abi_advisory_and_preserves_pin() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "alice"])
        .status
        .success());

    let pkg_dir = work.path().join("adv_pkg");
    let pkg_arg = pkg_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", pkg_arg]).status.success());
    let manifest_path = pkg_dir.join("project.json");
    let base = std::fs::read_to_string(&manifest_path).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"1.0.0\",\n  \"ident\": \"alice#adv_pkg\",\n",
    );
    let publish_version = |version: &str, source: &str| {
        std::fs::write(
            &manifest_path,
            base.replace(
                "\"version\": \"1.0.0\"",
                &format!("\"version\": \"{version}\""),
            ),
        )
        .unwrap();
        std::fs::write(pkg_dir.join("src/lib.mfb"), source).unwrap();
        let out = run_mfb(&repo, home.path(), &["repo", "publish", "alice", pkg_arg]);
        assert!(
            out.status.success(),
            "publish {version} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    let both = "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n\
                EXPORT FUNC extra() AS Integer\n  RETURN 1\nEND FUNC\n";
    publish_version("1.0.0", both);
    publish_version("1.1.0", both);
    // 2.0.0 drops `extra` — a real breaking change, not a synthetic ABI map.
    publish_version(
        "2.0.0",
        "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n",
    );

    // Consumer takes 1.0.0, floating.
    let app = work.path().join("adv_consumer");
    let app_arg = app.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_arg]).status.success());
    let run_in = |args: &[&str]| run_mfb_in(&repo, home.path(), &app, args);
    assert!(run_in(&["pkg", "add", "alice#adv_pkg@1.0.0", "--no-pin"])
        .status
        .success());
    let app_manifest = app.join("project.json");
    assert!(std::fs::read_to_string(&app_manifest)
        .unwrap()
        .contains("\"pin\": false"));

    // --- Bare targeted update: must take 1.1.0 and report skipping 2.0.0.
    let update = run_in(&["pkg", "update", "alice#adv_pkg"]);
    assert!(
        update.status.success(),
        "targeted update failed: {}",
        String::from_utf8_lossy(&update.stderr)
    );
    let stderr = String::from_utf8_lossy(&update.stderr);
    assert!(
        stderr.contains("2.0.0 is available but drops symbols"),
        "the advisory must name the skipped version: {stderr}"
    );
    assert!(
        stderr.contains("extra"),
        "must name the dropped symbol: {stderr}"
    );
    assert!(stderr.contains("selecting 1.1.0"), "{stderr}");

    let after = std::fs::read_to_string(&app_manifest).unwrap();
    assert!(after.contains("\"version\": \"1.1.0\""), "{after}");
    // §4.1: pin state survives a version bump.
    assert!(
        after.contains("\"pin\": false"),
        "a targeted update must NOT change pin state: {after}"
    );

    // --- An explicit @version is the escape hatch: 2.0.0 is taken anyway.
    let forced = run_in(&["pkg", "update", "alice#adv_pkg@2.0.0"]);
    assert!(
        forced.status.success(),
        "an explicit @version must be honored: {}",
        String::from_utf8_lossy(&forced.stderr)
    );
    let forced_manifest = std::fs::read_to_string(&app_manifest).unwrap();
    assert!(
        forced_manifest.contains("\"version\": \"2.0.0\""),
        "{forced_manifest}"
    );
    assert!(
        forced_manifest.contains("\"pin\": false"),
        "still floating: {forced_manifest}"
    );

    // --- An unpublished version leaves everything byte-identical.
    let lock_before = std::fs::read_to_string(app.join("mfb.lock")).unwrap();
    let missing = run_in(&["pkg", "update", "alice#adv_pkg@9.9.9"]);
    assert!(!missing.status.success());
    assert_eq!(
        std::fs::read_to_string(&app_manifest).unwrap(),
        forced_manifest,
        "a failed targeted update must leave project.json byte-identical"
    );
    assert_eq!(
        std::fs::read_to_string(app.join("mfb.lock")).unwrap(),
        lock_before,
        "a failed targeted update must leave mfb.lock byte-identical"
    );

    // --- An undeclared target errors locally, with the `add` hint.
    let undeclared = run_in(&["pkg", "update", "alice#nope"]);
    assert!(!undeclared.status.success());
    assert!(
        String::from_utf8_lossy(&undeclared.stderr).contains("mfb pkg add alice#nope"),
        "{}",
        String::from_utf8_lossy(&undeclared.stderr)
    );
}

/// plan-60-E §4.5: updating a PINNED dependency changes a deliberate choice, so
/// it is confirmed. Non-interactive without `--yes` must error rather than hang
/// or guess (plan-60-B §4.1); with `--yes` it proceeds and keeps the pin.
#[test]
fn update_of_a_pinned_dependency_requires_confirmation() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "alice"])
        .status
        .success());

    let pkg_dir = work.path().join("pin_pkg");
    let pkg_arg = pkg_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", pkg_arg]).status.success());
    let manifest_path = pkg_dir.join("project.json");
    let base = std::fs::read_to_string(&manifest_path).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"1.0.0\",\n  \"ident\": \"alice#pin_pkg\",\n",
    );
    for version in ["1.0.0", "1.1.0"] {
        std::fs::write(
            &manifest_path,
            base.replace(
                "\"version\": \"1.0.0\"",
                &format!("\"version\": \"{version}\""),
            ),
        )
        .unwrap();
        assert!(
            run_mfb(&repo, home.path(), &["repo", "publish", "alice", pkg_arg])
                .status
                .success()
        );
    }

    let app = work.path().join("pin_consumer");
    let app_arg = app.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_arg]).status.success());
    let run_in = |args: &[&str]| run_mfb_in(&repo, home.path(), &app, args);
    // An @version add pins (plan-60-C §4.1).
    assert!(run_in(&["pkg", "add", "alice#pin_pkg@1.0.0"])
        .status
        .success());
    let app_manifest = app.join("project.json");
    assert!(std::fs::read_to_string(&app_manifest)
        .unwrap()
        .contains("\"pin\": true"));

    // Without --yes on a non-TTY: refuse, and leave the manifest alone.
    let before = std::fs::read_to_string(&app_manifest).unwrap();
    let unconfirmed = run_in(&["pkg", "update", "alice#pin_pkg@1.1.0"]);
    assert!(
        !unconfirmed.status.success(),
        "a pinned update must not proceed unconfirmed"
    );
    assert!(
        String::from_utf8_lossy(&unconfirmed.stderr).contains("non-interactive"),
        "{}",
        String::from_utf8_lossy(&unconfirmed.stderr)
    );
    assert_eq!(std::fs::read_to_string(&app_manifest).unwrap(), before);

    // With --yes: proceeds, and the pin SURVIVES.
    let confirmed = run_in(&["pkg", "update", "alice#pin_pkg@1.1.0", "--yes"]);
    assert!(
        confirmed.status.success(),
        "--yes must bypass the prompt: {}",
        String::from_utf8_lossy(&confirmed.stderr)
    );
    let after = std::fs::read_to_string(&app_manifest).unwrap();
    assert!(after.contains("\"version\": \"1.1.0\""), "{after}");
    assert!(
        after.contains("\"pin\": true"),
        "the pin must survive the update: {after}"
    );
}

/// plan-60-D §4.1: `install` diffs the manifest against the lock instead of
/// refusing on the opaque `projectHash` alone.
///
/// A moved ABI **floor** on a `pin: false` dependency warns and installs the
/// LOCKED selection; the same drift on a `pin: true` dependency is fatal.
///
/// Asserting exit codes alone would not distinguish "warned and installed the
/// locked version" from "warned and installed the manifest's version", so this
/// compares the installed bytes against each published version.
#[test]
fn install_warns_on_floor_drift_and_errors_on_pin_drift() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "alice"])
        .status
        .success());

    // Publish 0.1.0 and 0.2.0 of the same package.
    let package_dir = work.path().join("drift_pkg");
    let package_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_arg]).status.success());
    let manifest_path = package_dir.join("project.json");
    let seed = std::fs::read_to_string(&manifest_path).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#drift_pkg\",\n",
    );
    std::fs::write(&manifest_path, &seed).unwrap();
    assert!(run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", package_arg]
    )
    .status
    .success());
    let v1_bytes = std::fs::read(package_dir.join("drift_pkg.mfp")).unwrap();

    std::fs::write(
        &manifest_path,
        seed.replace("\"version\": \"0.1.0\"", "\"version\": \"0.2.0\""),
    )
    .unwrap();
    assert!(run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", package_arg]
    )
    .status
    .success());
    let v2_bytes = std::fs::read(package_dir.join("drift_pkg.mfp")).unwrap();
    assert_ne!(v1_bytes, v2_bytes, "the two versions must differ on disk");

    // Consumer pins 0.1.0, so the lock records exactly that.
    let app = work.path().join("drift_consumer");
    let app_arg = app.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_arg]).status.success());
    let run_in = |args: &[&str]| run_mfb_in(&repo, home.path(), &app, args);
    assert!(run_in(&["pkg", "add", "alice#drift_pkg@0.1.0"])
        .status
        .success());
    let installed = app.join("packages/drift_pkg.mfp");
    assert_eq!(std::fs::read(&installed).unwrap(), v1_bytes);

    let app_manifest = app.join("project.json");
    let pinned_text = std::fs::read_to_string(&app_manifest).unwrap();
    assert!(pinned_text.contains("\"pin\": true"), "{pinned_text}");

    // --- pin: true, version bumped past the lock -> ERROR, packages/ untouched.
    std::fs::write(
        &app_manifest,
        pinned_text.replace("\"version\": \"0.1.0\"", "\"version\": \"0.2.0\""),
    )
    .unwrap();
    let pinned_install = run_in(&["pkg", "install"]);
    assert!(
        !pinned_install.status.success(),
        "a moved pin must be fatal: {}",
        String::from_utf8_lossy(&pinned_install.stdout)
    );
    let pin_err = String::from_utf8_lossy(&pinned_install.stderr);
    assert!(pin_err.contains("pinned to 0.2.0"), "{pin_err}");
    assert!(pin_err.contains("mfb.lock records 0.1.0"), "{pin_err}");
    assert_eq!(
        std::fs::read(&installed).unwrap(),
        v1_bytes,
        "a refused install must not touch packages/"
    );

    // --- pin: false, same drift -> WARN, and the LOCKED version installs.
    std::fs::write(
        &app_manifest,
        pinned_text
            .replace("\"version\": \"0.1.0\"", "\"version\": \"0.2.0\"")
            .replace("\"pin\": true", "\"pin\": false"),
    )
    .unwrap();
    let floating_install = run_in(&["pkg", "install"]);
    assert!(
        floating_install.status.success(),
        "a moved ABI floor must warn and continue: {}",
        String::from_utf8_lossy(&floating_install.stderr)
    );
    let warn = String::from_utf8_lossy(&floating_install.stderr);
    assert!(warn.contains("warning:"), "{warn}");
    assert!(warn.contains("floating"), "{warn}");
    assert!(warn.contains("0.2.0"), "must name the new floor: {warn}");
    assert!(
        warn.contains("0.1.0"),
        "must name the locked version: {warn}"
    );

    // The load-bearing assertion: the LOCKED selection landed, not the
    // manifest's newer floor. Exit code alone cannot tell these apart.
    assert_eq!(
        std::fs::read(&installed).unwrap(),
        v1_bytes,
        "the warn path must install the LOCKED version, not the manifest's"
    );
    assert_ne!(std::fs::read(&installed).unwrap(), v2_bytes);
}

/// plan-60-C Phase 1 (spike): does a `file://`-added package whose header
/// carries an `owner#pkg` ident get resolved against the **registry** by
/// `mfb pkg update`?
///
/// This matters because `add_package_from_file` copies the ident out of the
/// `.mfp` header (`src/cli/pkg.rs:566`), not out of the URL, so a published
/// package added by file *does* carry a `#`. `resolve()` seeds its nodes with
/// `.filter(|dep| dep.ident.contains('#'))` (`src/cli/resolve.rs:253`), which
/// keys on the ident and ignores `source`. If that filter admits this
/// dependency, `mfb pkg update` silently replaces the user's local file with a
/// registry blob.
///
/// Publishes a *different* version to the registry than the one added locally,
/// so a substitution is unambiguous rather than a no-op byte-for-byte.
#[test]
fn spike_file_added_package_with_registry_ident_survives_update() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "alice"])
        .status
        .success());

    let package_dir = work.path().join("spike_pkg");
    let package_dir_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir_arg])
        .status
        .success());
    let manifest_path = package_dir.join("project.json");
    let seed = std::fs::read_to_string(&manifest_path).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#spike_pkg\",\n",
    );
    std::fs::write(&manifest_path, &seed).unwrap();

    // Build and sign 0.1.0 locally — this is the copy the consumer adds by file.
    assert!(run_mfb(
        &repo,
        home.path(),
        &["build", "--sign", "alice", package_dir_arg]
    )
    .status
    .success());
    let mfp_path = package_dir.join("spike_pkg.mfp");

    // Publish a DIFFERENT version (0.2.0) to the registry, so if resolution
    // swaps the local file the bytes must change. NOTE: publishing rebuilds
    // `spike_pkg.mfp` in place, so the consumer below adds the 0.2.0 artifact —
    // the baseline for the survival check must therefore be read from the
    // consumer's `packages/` *after* the add, not from this path before it.
    let bumped = seed.replace("\"version\": \"0.1.0\"", "\"version\": \"0.2.0\"");
    std::fs::write(&manifest_path, bumped).unwrap();
    let published = run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", package_dir_arg],
    );
    assert!(
        published.status.success(),
        "publish failed: {}",
        String::from_utf8_lossy(&published.stderr)
    );

    // Consumer adds the LOCAL 0.1.0 by file://.
    let app_dir = work.path().join("spike_consumer");
    let app_dir_arg = app_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_dir_arg]).status.success());
    assert!(run_mfb_in(
        &repo,
        home.path(),
        &app_dir,
        &["pkg", "add", &format!("file://{}", mfp_path.display())]
    )
    .status
    .success());

    // OBSERVATION 1: what did the add actually write?
    let consumer_manifest = std::fs::read_to_string(app_dir.join("project.json")).unwrap();
    eprintln!("SPIKE consumer project.json:\n{consumer_manifest}");
    assert!(
        consumer_manifest.contains("alice#spike_pkg"),
        "the spike's premise is that a file:// add records a registry-shaped \
         ident; if this fails the premise is void: {consumer_manifest}"
    );

    // The baseline: exactly what `add` put in the consumer's packages/.
    let installed = app_dir.join("packages/spike_pkg.mfp");
    let before_update = std::fs::read(&installed).unwrap();

    // OBSERVATION 2: does `update` resolve it, and do the local bytes survive?
    let update = run_mfb_in(&repo, home.path(), &app_dir, &["pkg", "update"]);

    // plan-60-E: a project whose only dependency is a `file://` package has no
    // registry dependencies, so `update` is a clean no-op rather than an error.
    // Before plan-60-C's fix it "succeeded" by corrupting the package; between
    // that fix and plan-60-E it exited 1 with "declares no registry
    // dependencies to resolve".
    assert!(
        update.status.success(),
        "update on a file://-only project must be a no-op, not an error: {}",
        String::from_utf8_lossy(&update.stderr)
    );
    assert!(
        String::from_utf8_lossy(&update.stdout).contains("No registry dependencies"),
        "{}",
        String::from_utf8_lossy(&update.stdout)
    );

    let after_update = std::fs::read(&installed).unwrap();
    assert_eq!(
        before_update, after_update,
        "REGRESSION (plan-60-C §5 defect branch): `mfb pkg update` replaced the \
         file://-added local package with a registry blob. A dependency whose \
         `source` is a file:// URL must not be a registry resolution node — the \
         ident alone is not enough, because `add_package_from_file` copies the \
         ident out of the .mfp header, so a published-then-file-added package \
         carries a registry-shaped ident."
    );

    // And the lock must not claim to have resolved it.
    let lock = std::fs::read_to_string(app_dir.join("mfb.lock")).unwrap_or_default();
    assert!(
        !lock.contains("spike_pkg"),
        "a file:// dependency must not appear in mfb.lock as a resolved \
         registry package: {lock}"
    );
}

#[test]
fn repo_resolver_selects_substitute_and_locks_deterministically() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "alice"])
        .status
        .success());

    // Publish dep@0.1.0 and dep@0.1.1 (a compatible patch, same ABI surface).
    let dep_dir = work.path().join("dep");
    let dep_arg = dep_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", dep_arg]).status.success());
    let dep_manifest = dep_dir.join("project.json");
    let base = std::fs::read_to_string(&dep_manifest).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#dep\",\n",
    );
    std::fs::write(&dep_manifest, &base).unwrap();
    assert!(
        run_mfb(&repo, home.path(), &["repo", "publish", "alice", dep_arg])
            .status
            .success()
    );
    let bumped = base.replace("\"version\": \"0.1.0\"", "\"version\": \"0.1.1\"");
    std::fs::write(&dep_manifest, &bumped).unwrap();
    assert!(
        run_mfb(&repo, home.path(), &["repo", "publish", "alice", dep_arg])
            .status
            .success()
    );

    // Consumer pins dep@0.1.0 via add, then relaxes to a floating dependency.
    let app_dir = work.path().join("consumer");
    let app_arg = app_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_arg]).status.success());
    let run_in = |args: &[&str]| {
        Command::new(mfb_exe())
            .args(args)
            .current_dir(&app_dir)
            .env("MFB_REPO_URL", &repo.url)
            .env("MFB_HOME", home.path().join(".mfb"))
            .output()
            .expect("run mfb in consumer")
    };
    assert!(run_in(&["pkg", "add", "alice#dep@0.1.0"]).status.success());
    let manifest_path = app_dir.join("project.json");
    let relaxed = std::fs::read_to_string(&manifest_path)
        .unwrap()
        .replace("\"pin\": true", "\"pin\": false");
    std::fs::write(&manifest_path, relaxed).unwrap();

    // Update resolves the floating dep up to the compatible patch 0.1.1.
    let update = run_in(&["pkg", "update"]);
    let stdout = String::from_utf8_lossy(&update.stdout);
    assert!(
        update.status.success(),
        "update failed: {stdout}\n{}",
        String::from_utf8_lossy(&update.stderr)
    );
    let lock = std::fs::read_to_string(app_dir.join("mfb.lock")).unwrap();
    assert!(lock.contains("\"selected\": \"0.1.1\""), "{lock}");
    assert!(lock.contains("\"requested\": \"0.1.0\""), "{lock}");
    assert!(lock.contains("\"repoFingerprint\":"), "{lock}");
    assert!(lock.contains("\"checkpoint\":"), "{lock}");

    // Re-resolving an unchanged project reproduces the lock byte-for-byte.
    assert!(run_in(&["pkg", "update"]).status.success());
    let lock2 = std::fs::read_to_string(app_dir.join("mfb.lock")).unwrap();
    assert_eq!(lock, lock2, "re-resolve must be byte-identical");

    // The locked install fetches by hash and installs the selected 0.1.1.
    assert!(std::fs::remove_file(app_dir.join("packages/dep.mfp")).is_ok());
    let install = run_in(&["pkg", "install"]);
    assert!(
        install.status.success(),
        "install failed: {}",
        String::from_utf8_lossy(&install.stderr)
    );
    let installed = mfb_repository::package::parse_mfp_package(
        &std::fs::read(app_dir.join("packages/dep.mfp")).unwrap(),
    )
    .unwrap();
    assert_eq!(installed.version, "0.1.1");

    // A pinned dependency bypasses the search and keeps its exact version.
    let pinned = std::fs::read_to_string(&manifest_path)
        .unwrap()
        .replace("\"pin\": false", "\"pin\": true");
    std::fs::write(&manifest_path, pinned).unwrap();
    assert!(run_in(&["pkg", "update"]).status.success());
    let lock = std::fs::read_to_string(app_dir.join("mfb.lock")).unwrap();
    assert!(
        lock.contains("\"selected\": \"0.1.0\""),
        "pinned select: {lock}"
    );
}

#[test]
fn repo_resolver_reports_diamond_conflict_naming_both_requirers() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "alice"])
        .status
        .success());

    // common@1.0.0 exports `shared()`; common@2.0.0 changes its signature, so
    // the two versions export `shared` with different ABI hashes.
    let common_dir = work.path().join("common");
    let common_arg = common_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", common_arg]).status.success());
    let common_manifest = common_dir.join("project.json");
    let common_base = std::fs::read_to_string(&common_manifest).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"1.0.0\",\n  \"ident\": \"alice#common\",\n",
    );
    std::fs::write(&common_manifest, &common_base).unwrap();
    std::fs::write(
        common_dir.join("src/lib.mfb"),
        "EXPORT FUNC shared() AS Integer\n  RETURN 1\nEND FUNC\n",
    )
    .unwrap();
    assert!(run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", common_arg]
    )
    .status
    .success());
    // Save common@1.0.0's blob before bumping so `user` can build against it.
    let common_v1 = work.path().join("common-1.0.0.mfp");
    std::fs::copy(common_dir.join("common.mfp"), &common_v1).unwrap();

    let common_v2 = common_base.replace("\"version\": \"1.0.0\"", "\"version\": \"2.0.0\"");
    std::fs::write(&common_manifest, &common_v2).unwrap();
    std::fs::write(
        common_dir.join("src/lib.mfb"),
        "EXPORT FUNC shared(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\n",
    )
    .unwrap();
    assert!(run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", common_arg]
    )
    .status
    .success());

    // user@1.0.0 imports common (compiled against common@1.0.0's `shared`).
    let user_dir = work.path().join("user");
    let user_arg = user_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", user_arg]).status.success());
    let user_manifest = user_dir.join("project.json");
    let user_base = std::fs::read_to_string(&user_manifest).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"1.0.0\",\n  \"ident\": \"alice#user\",\n",
    );
    std::fs::write(&user_manifest, &user_base).unwrap();
    let add_common = Command::new(mfb_exe())
        .args(["pkg", "add", &format!("file://{}", common_v1.display())])
        .current_dir(&user_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("add common to user");
    assert!(
        add_common.status.success(),
        "{}",
        String::from_utf8_lossy(&add_common.stderr)
    );
    std::fs::write(
        user_dir.join("src/lib.mfb"),
        "IMPORT common\nEXPORT FUNC callShared() AS Integer\n  RETURN common::shared()\nEND FUNC\n",
    )
    .unwrap();
    let publish_user = run_mfb(&repo, home.path(), &["repo", "publish", "alice", user_arg]);
    assert!(
        publish_user.status.success(),
        "publish user failed: {}\n{}",
        String::from_utf8_lossy(&publish_user.stdout),
        String::from_utf8_lossy(&publish_user.stderr)
    );

    // Consumer wants `user` (which needs common's old `shared`) AND
    // common@2.0.0 (whose `shared` has a different ABI) — a diamond conflict.
    let app_dir = work.path().join("diamond_consumer");
    let app_arg = app_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_arg]).status.success());
    let run_in = |args: &[&str]| {
        Command::new(mfb_exe())
            .args(args)
            .current_dir(&app_dir)
            .env("MFB_REPO_URL", &repo.url)
            .env("MFB_HOME", home.path().join(".mfb"))
            .output()
            .expect("run mfb in consumer")
    };
    assert!(run_in(&["pkg", "add", "alice#user@1.0.0"]).status.success());

    // plan-60-C: `add` now resolves BEFORE mutating, so the add that creates the
    // conflicting graph is itself refused — the conflict surfaces here rather
    // than being written to disk and discovered later by `update`. Assert the
    // diagnostic at this new, earlier point too.
    let conflicting_add = run_in(&["pkg", "add", "alice#common@2.0.0"]);
    assert!(
        !conflicting_add.status.success(),
        "resolve-first `add` must refuse a change that cannot resolve"
    );
    let add_stderr = String::from_utf8_lossy(&conflicting_add.stderr);
    assert!(add_stderr.contains("diamond conflict"), "{add_stderr}");
    // ...and it must have written nothing: the refused dependency is absent.
    let manifest_path = app_dir.join("project.json");
    let after_refusal = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(
        !after_refusal.contains("common"),
        "a refused add must leave project.json untouched: {after_refusal}"
    );

    // The original coverage — that `update` reports the conflict naming both
    // requirers — still has to hold. `add` will no longer produce the
    // conflicting manifest, so construct it directly: declare `common@2.0.0`
    // alongside `user`, floating, exactly as the old two-add-then-relax setup
    // produced.
    let conflicted = after_refusal
        .replace("\"pin\": true", "\"pin\": false")
        .replace(
            "\"packages\": [",
            "\"packages\": [\n    { \"name\": \"common\", \"ident\": \"alice#common\",              \"version\": \"2.0.0\", \"pin\": false, \"source\": \"alice#common\" },",
        );
    std::fs::write(&manifest_path, conflicted).unwrap();

    let update = run_in(&["pkg", "update"]);
    assert!(
        !update.status.success(),
        "diamond conflict must fail resolution"
    );
    let stderr = String::from_utf8_lossy(&update.stderr);
    assert!(stderr.contains("diamond conflict"), "{stderr}");
    assert!(
        stderr.contains("shared"),
        "must name the disagreeing symbol: {stderr}"
    );
    assert!(
        stderr.contains("user"),
        "must name the requirer package: {stderr}"
    );
}

/// plan-49 acceptance: `mfb-repo gc` reclaims a blob that nothing references
/// and leaves every live package installable.
///
/// The orphan is created the way a real one is — a `PUT /blob` whose publish
/// never lands (network failure, failed validation, `^C`) — rather than by
/// writing a row directly, because the whole point of the plan is that this
/// path can now produce bytes nothing will ever name.
#[test]
fn repo_gc_reclaims_an_orphaned_blob_and_leaves_live_packages_installable() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "alice"])
        .status
        .success());

    // Two live packages, one of which vendors a native library — so the sweep
    // has to spare both halves of the reachable set: `package_versions.hash`
    // (the `.mfp`) and `package_version_blobs.hash` (the vendor blob).
    let vendor_bytes: &[u8] = b"gc acceptance vendored library bytes";
    let plain_dir = work.path().join("gcplain");
    let plain_arg = plain_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", plain_arg]).status.success());
    let plain_manifest = std::fs::read_to_string(plain_dir.join("project.json"))
        .unwrap()
        .replace(
            "  \"version\": \"0.1.0\",\n",
            "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#gcplain\",\n",
        );
    std::fs::write(plain_dir.join("project.json"), plain_manifest).unwrap();

    let vendor_pkg = work.path().join("gcvendor");
    let vendor_arg = vendor_pkg.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", vendor_arg]).status.success());
    std::fs::write(
        vendor_pkg.join("project.json"),
        r#"{
  "name": "gcvendor",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "package",
  "description": "Test fixture package for repo gc with a vendored library.",
  "ident": "alice#gcvendor",
  "libraries": {
    "demo": [
      { "os": "macos", "type": "vendor", "source": "libgc.dylib" },
      { "os": "linux", "arch": "aarch64", "libc": "glibc", "type": "vendor", "source": "libgc-aarch64-glibc.so" }
    ]
  },
  "sources": [ { "root": "src", "role": "package", "include": ["**/*.mfb"] } ]
}
"#,
    )
    .unwrap();
    std::fs::write(
        vendor_pkg.join("src/lib.mfb"),
        r#"LINK "demo" AS demoLink
  FUNC ping() AS Integer
    SYMBOL "demo_ping"
    ABI (value OUT CInt32) AS status CInt32
    RETURN value
    SUCCESS_ON status = 0
  END FUNC
END LINK

EXPORT FUNC demoPing() AS Integer
  RETURN demoLink::ping()
END FUNC
"#,
    )
    .unwrap();
    std::fs::create_dir_all(vendor_pkg.join("vendor")).unwrap();
    std::fs::write(vendor_pkg.join("vendor/libgc.dylib"), vendor_bytes).unwrap();
    std::fs::write(
        vendor_pkg.join("vendor/libgc-aarch64-glibc.so"),
        b"gc acceptance linux vendored library bytes",
    )
    .unwrap();

    for pkg in [plain_arg, vendor_arg] {
        let published = run_mfb(&repo, home.path(), &["repo", "publish", "alice", pkg]);
        assert!(
            published.status.success(),
            "publish {pkg} failed: {}\n{}",
            String::from_utf8_lossy(&published.stdout),
            String::from_utf8_lossy(&published.stderr)
        );
    }

    // --- the orphan: an upload whose publish never lands -------------------
    let orphan_bytes = b"gc acceptance orphaned upload payload".to_vec();
    let orphan_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&orphan_bytes);
        format!("{:x}", hasher.finalize())
    };
    let session =
        std::fs::read_to_string(mfb_repo_home(&repo, home.path()).join("session/alice.ses"))
            .expect("alice's session token");
    mfb_repository::client::put_blob(
        &repo.url,
        &orphan_hash,
        orphan_bytes.clone(),
        session.trim(),
    )
    .expect("PUT /blob");
    let orphan_file = repo_dir.path().join(format!("packages/{orphan_hash}.bin"));
    assert!(orphan_file.is_file(), "the orphan blob should be stored");

    // Snapshot every live blob so the sweep can be proven to have spared them.
    let live_blobs: Vec<std::path::PathBuf> = std::fs::read_dir(repo_dir.path().join("packages"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path != &orphan_file)
        .collect();
    assert!(
        live_blobs.len() >= 3,
        "expected two .mfp blobs and a vendor blob: {live_blobs:?}"
    );

    let db_arg = repo_dir.path().join("meta.db");
    let data_arg = repo_dir.path().join("packages");
    let gc = |args: &[&str]| {
        let mut all = vec![
            "gc",
            "--dbpath",
            db_arg.to_str().unwrap(),
            "--datapath",
            data_arg.to_str().unwrap(),
        ];
        all.extend_from_slice(args);
        Command::new(repo_exe())
            .args(&all)
            .output()
            .expect("run mfb-repo gc")
    };

    // --- inside the grace window: nothing at all ---------------------------
    let fresh = gc(&[]);
    let fresh_out = String::from_utf8_lossy(&fresh.stdout);
    assert!(fresh.status.success(), "{fresh_out}");
    assert!(
        fresh_out.contains("No unreachable blobs older than 24h"),
        "a blob younger than the grace period must never be listed: {fresh_out}"
    );
    assert!(!fresh_out.contains(&orphan_hash), "{fresh_out}");

    // Age the orphan's row past the grace period. This is the one thing the
    // test cannot do through the product: the sweep is time-gated by design and
    // waiting 24h is not a test.
    {
        let conn = rusqlite::Connection::open(repo_dir.path().join("meta.db")).unwrap();
        let aged = conn
            .execute(
                "UPDATE package_blobs SET created_at = created_at - 172800 WHERE hash = ?1",
                rusqlite::params![orphan_hash],
            )
            .unwrap();
        assert_eq!(aged, 1, "the orphan's row should be the one aged");
    }

    // --- dry run: exactly the orphan, and it is still on disk --------------
    let dry = gc(&[]);
    let dry_out = String::from_utf8_lossy(&dry.stdout);
    assert!(dry.status.success(), "{dry_out}");
    assert!(dry_out.contains(&orphan_hash), "{dry_out}");
    assert!(dry_out.contains("1 unreachable blob,"), "{dry_out}");
    assert!(dry_out.contains("Run again with --delete"), "{dry_out}");
    assert!(orphan_file.is_file(), "a dry run must delete nothing");

    // --- --delete: the orphan goes, everything else stays ------------------
    let swept = gc(&["--delete", "--json"]);
    let swept_out = String::from_utf8_lossy(&swept.stdout);
    assert!(swept.status.success(), "{swept_out}");
    let report: serde_json::Value = serde_json::from_str(&swept_out).expect("gc --json report");
    assert_eq!(report["deletedCount"], 1, "{swept_out}");
    assert_eq!(report["deletedBytes"], orphan_bytes.len(), "{swept_out}");
    assert_eq!(report["unreachable"][0]["hash"], orphan_hash);
    assert_eq!(report["errors"].as_array().unwrap().len(), 0, "{swept_out}");
    assert!(!orphan_file.exists(), "the orphan's bytes should be gone");
    for path in &live_blobs {
        assert!(path.is_file(), "gc must not touch a live blob: {path:?}");
    }
    // The deleted hash now 404s; a reachable one still downloads.
    assert!(
        mfb_repository::client::fetch_blob(&repo.url, &orphan_hash).is_err(),
        "a collected blob must no longer be servable"
    );

    // --- every live package still installs and builds ----------------------
    let app_dir = work.path().join("gc_consumer");
    assert!(run_mfb_plain(&["init", app_dir.to_str().unwrap()])
        .status
        .success());
    let run_in_app = |args: &[&str]| {
        Command::new(mfb_exe())
            .args(args)
            .current_dir(&app_dir)
            .env("MFB_REPO_URL", &repo.url)
            .env("MFB_HOME", home.path().join(".mfb"))
            .output()
            .expect("run mfb in the consumer")
    };
    for ident in ["alice#gcplain", "alice#gcvendor"] {
        let add = run_in_app(&["pkg", "add", ident]);
        assert!(
            add.status.success(),
            "pkg add {ident} after the sweep failed: {}\n{}",
            String::from_utf8_lossy(&add.stdout),
            String::from_utf8_lossy(&add.stderr)
        );
    }
    // The vendored library came back down byte-for-byte — its blob survived.
    assert_eq!(
        std::fs::read(app_dir.join("packages/gcvendor.vendor/libgc.dylib")).unwrap(),
        vendor_bytes,
        "the surviving vendor blob must still serve its exact bytes"
    );
    let build = run_in_app(&["build"]);
    assert!(
        build.status.success(),
        "consumer build after the sweep failed: {}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    // --- a second sweep is a clean no-op -----------------------------------
    let again = gc(&["--delete"]);
    let again_out = String::from_utf8_lossy(&again.stdout);
    assert!(again.status.success(), "{again_out}");
    assert!(
        again_out.contains("No unreachable blobs older than 24h"),
        "{again_out}"
    );
    assert!(again_out.contains("Deleted 0 blobs"), "{again_out}");
}
