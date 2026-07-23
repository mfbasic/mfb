use mfb_repository::crypto;
use std::process::Command;

mod common;
use common::*;

#[test]
fn repo_signs_package_and_embeds_executable_metadata() {
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

    let package_dir = work.path().join("signed_pkg");
    let package_dir_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir_arg])
        .status
        .success());
    let output = run_mfb(
        &repo,
        home.path(),
        &["build", "--sign", "alice", package_dir_arg],
    );
    assert!(
        output.status.success(),
        "signed package build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let package = std::fs::read(package_dir.join("signed_pkg.mfp")).unwrap();
    let parsed = mfb_repository::package::parse_mfp_package(&package).expect("parse signed .mfp");

    // Header states v1.0 and carries the full trust chain (plan-23 §4).
    assert_eq!((parsed.container_major, parsed.container_minor), (1, 0));
    assert_eq!(parsed.signature_type, 1);
    assert_eq!(parsed.ident, "alice#signed_pkg");
    assert!(parsed.ident_key.starts_with("ed25519:"));
    assert!(parsed.signing_key.starts_with("ed25519:"));
    assert_ne!(parsed.ident_key, parsed.signing_key);

    // The proof verifies under the machine's local ident public key.
    let repo_home = mfb_repo_home(&repo, home.path());
    let ident_public = crypto::decode_bytes(
        std::fs::read_to_string(repo_home.join("keys/alice.ident.pub"))
            .unwrap()
            .trim(),
        "ident public key",
    )
    .unwrap();
    mfb_repository::package::verify_proof(&parsed, &ident_public).expect("proof verifies");

    // The attestation verifies under the pinned registry key.
    let server_key = crypto::decode_bytes(
        std::fs::read_to_string(repo_home.join("server.pub"))
            .unwrap()
            .trim(),
        "server key",
    )
    .unwrap();
    mfb_repository::package::verify_attestation(
        &parsed,
        &server_key,
        &crypto::fingerprint(&server_key),
    )
    .expect("attestation verifies");

    // The prefix signature verifies under the one-off signing key and the
    // payload hash welds header to payload.
    mfb_repository::package::verify_package_signature(&parsed).expect("package signature");
    mfb_repository::package::verify_payload_hash(&parsed).expect("payload hash");

    // The one-off signing key is exactly that: not the ident key, not the
    // auth key, and its private half is nowhere on disk — the only stored
    // private keys are the machine's auth and ident keys.
    let signing_public =
        mfb_repository::package::decode_metadata_key(&parsed.signing_key, "signingKey").unwrap();
    let auth_public = crypto::decode_bytes(
        std::fs::read_to_string(repo_home.join("keys/alice.auth.pub"))
            .unwrap()
            .trim(),
        "auth public key",
    )
    .unwrap();
    assert_ne!(signing_public, ident_public);
    assert_ne!(signing_public, auth_public);
    let stored_private_keys: Vec<_> = std::fs::read_dir(repo_home.join("keys"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".prv"))
        .collect();
    let mut sorted = stored_private_keys.clone();
    sorted.sort();
    assert_eq!(sorted, ["alice.auth.prv", "alice.ident.prv"]);

    let app_dir = work.path().join("signed_app");
    let app_dir_arg = app_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_dir_arg]).status.success());
    std::fs::write(
        app_dir.join("src/main.mfb"),
        "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
    )
    .unwrap();
    let output = run_mfb(
        &repo,
        home.path(),
        &["build", "--sign", "alice", app_dir_arg],
    );
    assert!(
        output.status.success(),
        "signed executable build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // plan-46-D §4.1: the build emits into the project's `build/` directory.
    let executable = std::fs::read_dir(app_dir.join("build"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(".out"))
                .unwrap_or(false)
        })
        .expect("signed executable");
    let executable = std::fs::read(executable).unwrap();
    assert!(executable
        .windows(b"mfb-signing-v1".len())
        .any(|window| window == b"mfb-signing-v1"));
    assert!(executable
        .windows(b"alice".len())
        .any(|window| window == b"alice"));
}

#[test]
fn repo_publishes_signed_package_and_rejects_duplicate_version() {
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

    let package_dir = work.path().join("publish_pkg");
    let package_dir_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir_arg])
        .status
        .success());
    let manifest_path = package_dir.join("project.json");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#publish_pkg\",\n",
    );
    std::fs::write(&manifest_path, manifest).unwrap();

    let output = run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", package_dir_arg],
    );
    assert!(
        output.status.success(),
        "publish failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Package validation report:"), "{stdout}");
    assert!(stdout.contains("valid: true"), "{stdout}");
    assert!(
        stdout.contains("Published alice#publish_pkg@0.1.0"),
        "{stdout}"
    );
    // The publish is logged and its inclusion proof verifies against the
    // signed, rollback-checked checkpoint (plan-23-B3).
    assert!(stdout.contains("Publish logged at index"), "{stdout}");
    assert!(
        stdout.contains("Inclusion verified against checkpoint"),
        "{stdout}"
    );
    let repo_home = mfb_repo_home(&repo, home.path());
    let checkpoint = std::fs::read_to_string(repo_home.join("checkpoint")).unwrap();
    let pinned_size: i64 = checkpoint
        .trim()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        pinned_size >= 3,
        "register+attestation+publish logged: {checkpoint}"
    );

    // Rollback rejection: poison the pinned checkpoint with a LARGER size —
    // the next checkpoint fetch must refuse the (apparently shrunken) log.
    let poisoned = format!("999999 {}", checkpoint.trim().split(' ').nth(1).unwrap());
    std::fs::write(repo_home.join("checkpoint"), &poisoned).unwrap();
    let package_dir2 = work.path().join("publish_pkg2");
    let package_dir2_arg = package_dir2.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir2_arg])
        .status
        .success());
    let manifest_path2 = package_dir2.join("project.json");
    let manifest2 = std::fs::read_to_string(&manifest_path2).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#publish_pkg2\",\n",
    );
    std::fs::write(&manifest_path2, manifest2).unwrap();
    let rollback = run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", package_dir2_arg],
    );
    assert!(!rollback.status.success());
    assert!(
        String::from_utf8_lossy(&rollback.stderr).contains("ROLLBACK"),
        "{}",
        String::from_utf8_lossy(&rollback.stderr)
    );
    // Restore the true pin: the next publish verifies again.
    std::fs::write(repo_home.join("checkpoint"), checkpoint.trim()).unwrap();
    let blobs = std::fs::read_dir(repo_dir.path().join("packages"))
        .unwrap()
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    assert_eq!(blobs.len(), 1);
    assert_eq!(
        blobs[0].path().extension().and_then(|ext| ext.to_str()),
        Some("mfp")
    );

    let duplicate = run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", package_dir_arg],
    );
    assert!(!duplicate.status.success());
    let duplicate_stdout = String::from_utf8_lossy(&duplicate.stdout);
    let duplicate_stderr = String::from_utf8_lossy(&duplicate.stderr);
    assert!(
        duplicate_stdout.contains("already published")
            || duplicate_stderr.contains("already published"),
        "stdout: {duplicate_stdout}\nstderr: {duplicate_stderr}"
    );
}

/// plan-60-A: the end-to-end proof of both halves of this letter — that
/// publishing dispatches from `mfb repo` at all, and that `publish`'s new
/// optional path really defaults to the current directory.
///
/// Publishes with `mfb repo publish alice` (no path argument, run from inside
/// the package directory) and then installs the result by ident from a separate
/// consumer project. The install can only succeed if the artifact actually
/// reached the registry index, so it is the artifact-appears-in-the-index check
/// rather than a stdout assertion about it.
#[test]
fn repo_publish_without_a_path_publishes_the_current_directory() {
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

    let package_dir = work.path().join("cwd_pkg");
    let package_dir_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir_arg])
        .status
        .success());
    let manifest_path = package_dir.join("project.json");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#cwd_pkg\",\n",
    );
    std::fs::write(&manifest_path, manifest).unwrap();

    // The whole point: no path argument, so `.` must be inferred.
    let output = run_mfb_in(
        &repo,
        home.path(),
        &package_dir,
        &["repo", "publish", "alice"],
    );
    assert!(
        output.status.success(),
        "pathless publish failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Published alice#cwd_pkg@0.1.0"), "{stdout}");

    // ...and the artifact is really in the registry index: a fresh consumer
    // resolves and installs it by ident.
    let app_dir = work.path().join("cwd_consumer");
    let app_dir_arg = app_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_dir_arg]).status.success());
    let add = run_mfb_in(
        &repo,
        home.path(),
        &app_dir,
        &["pkg", "add", "alice#cwd_pkg"],
    );
    assert!(
        add.status.success(),
        "add from index failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&add.stdout),
        String::from_utf8_lossy(&add.stderr)
    );

    // The old spelling is gone — same directory, same everything else.
    let moved = run_mfb_in(
        &repo,
        home.path(),
        &package_dir,
        &["pkg", "publish", "alice"],
    );
    assert_eq!(moved.status.code(), Some(2), "pkg publish must exit 2");
    let stderr = String::from_utf8_lossy(&moved.stderr);
    assert!(
        stderr.contains("mfb pkg publish has moved to mfb repo publish"),
        "{stderr}"
    );
}

#[test]
fn repo_registry_add_installs_and_verifies_from_index() {
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

    // Publish a signed package to the registry.
    let package_dir = work.path().join("addable_pkg");
    let package_dir_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir_arg])
        .status
        .success());
    let manifest_path = package_dir.join("project.json");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#addable_pkg\",\n",
    );
    std::fs::write(&manifest_path, manifest).unwrap();
    assert!(run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", package_dir_arg],
    )
    .status
    .success());

    // A fresh consumer installs it straight from the registry by ident.
    let app_dir = work.path().join("registry_consumer");
    let app_dir_arg = app_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_dir_arg]).status.success());
    let run_in_consumer = |args: &[&str]| {
        Command::new(mfb_exe())
            .args(args)
            .current_dir(&app_dir)
            .env("MFB_REPO_URL", &repo.url)
            .env("MFB_HOME", home.path().join(".mfb"))
            .output()
            .expect("run mfb in consumer")
    };

    let add = run_in_consumer(&["pkg", "add", "alice#addable_pkg"]);
    assert!(
        add.status.success(),
        "registry add failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&add.stdout),
        String::from_utf8_lossy(&add.stderr)
    );
    assert!(
        String::from_utf8_lossy(&add.stdout).contains("from alice#addable_pkg"),
        "{}",
        String::from_utf8_lossy(&add.stdout)
    );

    // The identKey was pinned from the registry-vouched index, the blob is
    // installed, and a build walks the full §3.5 chain to Verified.
    let manifest = std::fs::read_to_string(app_dir.join("project.json")).unwrap();
    assert!(
        manifest.contains("\"identKey\": \"ed25519:"),
        "registry add must pin the identKey: {manifest}"
    );
    assert!(app_dir.join("packages/addable_pkg.mfp").is_file());
    let build = run_in_consumer(&["build"]);
    assert!(
        build.status.success(),
        "consumer build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(
        String::from_utf8_lossy(&build.stdout).contains("uses addable_pkg - [Verified]"),
        "{}",
        String::from_utf8_lossy(&build.stdout)
    );

    // A version that does not exist is rejected with an actionable message, and
    // the refusal is ATOMIC: project.json and mfb.lock are byte-identical
    // afterwards.
    //
    // NOTE ON WHAT THIS DOES AND DOES NOT PROVE. This case fails inside
    // `select_index_version`, which runs BEFORE `apply_manifest_change` is
    // called at all — so it proves the pre-resolve validation path writes
    // nothing, but it does NOT exercise the resolve-first *ordering* inside the
    // pipeline. Verified by mutation: moving the `project.json` write above the
    // `resolve()` call in `apply_manifest_change` leaves this test green.
    //
    // The reorder-goes-red proof that plan-60-B Phase 3's acceptance requires is
    // `repo_resolver_reports_diamond_conflict_naming_both_requirers`, whose
    // failure occurs *inside* `resolve()`; that one does go red under the same
    // mutation. See plan-60-C Corrections #5.
    let app2 = work.path().join("registry_consumer2");
    let app2_arg = app2.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app2_arg]).status.success());
    // Give it a real dependency first, so there is a non-trivial lock to
    // preserve — an empty project would make the atomicity check vacuous.
    let run_in_app2 = |args: &[&str]| {
        Command::new(mfb_exe())
            .args(args)
            .current_dir(&app2)
            .env("MFB_REPO_URL", &repo.url)
            .env("MFB_HOME", home.path().join(".mfb"))
            .output()
            .expect("run mfb in app2")
    };
    assert!(run_in_app2(&["pkg", "add", "alice#addable_pkg"])
        .status
        .success());
    // plan-60-C's headline fix: `add` writes mfb.lock, so `install` runs
    // immediately WITHOUT an intervening `update`. Before this letter, add left
    // the lock stale and install hard-errored with "mfb.lock is stale".
    let install = run_in_app2(&["pkg", "install"]);
    assert!(
        install.status.success(),
        "install must run straight after add, with no `pkg update` in between: {}",
        String::from_utf8_lossy(&install.stderr)
    );

    // ...and a bare add records a FLOATING dependency (§4.1), where the old
    // behavior hardcoded pin: true.
    let bare_manifest = std::fs::read_to_string(app2.join("project.json")).unwrap();
    assert!(
        bare_manifest.contains("\"pin\": false"),
        "a bare `add` must record pin: false: {bare_manifest}"
    );

    let manifest_before = std::fs::read_to_string(app2.join("project.json")).unwrap();
    let lock_before = std::fs::read_to_string(app2.join("mfb.lock")).unwrap();
    assert!(
        !lock_before.is_empty(),
        "the atomicity check needs a real lock to preserve"
    );

    // The other half of §4.1's matrix, end to end: an explicit @version implies
    // a pin. Uses a fresh project because `project_json_with_package` refuses a
    // name that is already declared.
    let pinned_dir = work.path().join("registry_consumer_pinned");
    let pinned_arg = pinned_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init", pinned_arg]).status.success());
    let pinned_add = Command::new(mfb_exe())
        .args(["pkg", "add", "alice#addable_pkg@0.1.0"])
        .current_dir(&pinned_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("pinned add");
    assert!(
        pinned_add.status.success(),
        "pinned add failed: {}",
        String::from_utf8_lossy(&pinned_add.stderr)
    );
    let pinned_manifest = std::fs::read_to_string(pinned_dir.join("project.json")).unwrap();
    assert!(
        pinned_manifest.contains("\"pin\": true"),
        "an @version add must record pin: true: {pinned_manifest}"
    );
    assert!(
        String::from_utf8_lossy(&pinned_add.stdout).contains("(pinned)"),
        "{}",
        String::from_utf8_lossy(&pinned_add.stdout)
    );

    let missing = run_in_app2(&["pkg", "add", "alice#addable_pkg@9.9.9"]);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("no version `9.9.9`"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );
    assert_eq!(
        manifest_before,
        std::fs::read_to_string(app2.join("project.json")).unwrap(),
        "a failed add must leave project.json byte-identical (resolve-first)"
    );
    assert_eq!(
        lock_before,
        std::fs::read_to_string(app2.join("mfb.lock")).unwrap(),
        "a failed add must leave mfb.lock byte-identical (resolve-first)"
    );

    // A tampered server blob is rejected on download (hash mismatch): corrupt
    // the stored blob and a fresh add must refuse it.
    let blob = std::fs::read_dir(repo_dir.path().join("packages"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("mfp"))
        .expect("stored blob");
    let mut bytes = std::fs::read(&blob).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(&blob, &bytes).unwrap();
    let app3 = work.path().join("registry_consumer3");
    let app3_arg = app3.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app3_arg]).status.success());
    let tampered = Command::new(mfb_exe())
        .args(["pkg", "add", "alice#addable_pkg"])
        .current_dir(&app3)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("pkg add tampered");
    assert!(!tampered.status.success());
    assert!(
        String::from_utf8_lossy(&tampered.stderr).contains("does not match")
            || String::from_utf8_lossy(&tampered.stderr).contains("corruption"),
        "{}",
        String::from_utf8_lossy(&tampered.stderr)
    );
}

#[test]
fn repo_publish_rejects_non_package_and_missing_session() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());

    let app_dir = work.path().join("not_a_package");
    let app_dir_arg = app_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_dir_arg]).status.success());
    let non_package = run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", app_dir_arg],
    );
    assert!(!non_package.status.success());
    assert!(
        String::from_utf8_lossy(&non_package.stderr).contains("requires a package project"),
        "{}",
        String::from_utf8_lossy(&non_package.stderr)
    );

    let package_dir = work.path().join("missing_session_pkg");
    let package_dir_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir_arg])
        .status
        .success());
    let manifest_path = package_dir.join("project.json");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#missing_session_pkg\",\n",
    );
    std::fs::write(&manifest_path, manifest).unwrap();
    let missing_session = run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", package_dir_arg],
    );
    assert!(!missing_session.status.success());
    assert!(
        String::from_utf8_lossy(&missing_session.stderr).contains("failed to read session"),
        "{}",
        String::from_utf8_lossy(&missing_session.stderr)
    );
}

/// plan-48 end-to-end: a binding's vendored native libraries travel with the
/// package.
///
/// `repo publish` uploads each `vendor` locator's file as its own content-addressed
/// blob before the `.mfp`; `pkg add` downloads every blob the section-10 table
/// names and hash-verifies it into `packages/<name>.vendor/`; and a consumer
/// `mfb build` then finds the library with no file placed by hand. This is the
/// acceptance for the whole plan-46 + plan-48 arc.
#[test]
fn repo_vendored_native_libraries_publish_and_install_with_the_package() {
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

    // A binding package that vendors one native library per platform slot. The
    // bytes are arbitrary — nothing dlopens them here; what is under test is that
    // they travel, and that every hash matches on the way back down.
    let vendor_files: [(&str, &[u8]); 5] = [
        ("libdemo.dylib", b"macos-any-arch demo library bytes"),
        ("libdemo-aarch64-glibc.so", b"linux aarch64 glibc bytes"),
        ("libdemo-x86_64-glibc.so", b"linux x86_64 glibc bytes"),
        ("libdemo-aarch64-musl.so", b"linux aarch64 musl bytes"),
        ("libdemo-x86_64-musl.so", b"linux x86_64 musl bytes"),
    ];

    let package_dir = work.path().join("vendorbind");
    let package_dir_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir_arg])
        .status
        .success());
    let write_manifest = |version: &str| {
        std::fs::write(
            package_dir.join("project.json"),
            format!(
                r#"{{
  "name": "vendorbind",
  "version": "{version}",
  "mfb": "1.0",
  "kind": "package",
  "description": "Test fixture package for vendored native libraries.",
  "ident": "alice#vendorbind",
  "libraries": {{
    "demo": [
      {{ "os": "macos", "type": "vendor", "source": "libdemo.dylib" }},
      {{ "os": "linux", "arch": "aarch64", "libc": "glibc", "type": "vendor", "source": "libdemo-aarch64-glibc.so" }},
      {{ "os": "linux", "arch": "x86_64", "libc": "glibc", "type": "vendor", "source": "libdemo-x86_64-glibc.so" }},
      {{ "os": "linux", "arch": "aarch64", "libc": "musl", "type": "vendor", "source": "libdemo-aarch64-musl.so" }},
      {{ "os": "linux", "arch": "x86_64", "libc": "musl", "type": "vendor", "source": "libdemo-x86_64-musl.so" }}
    ]
  }},
  "sources": [ {{ "root": "src", "role": "package", "include": ["**/*.mfb"] }} ]
}}
"#
            ),
        )
        .unwrap();
    };
    write_manifest("0.1.0");
    std::fs::write(
        package_dir.join("src/lib.mfb"),
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
    let vendor_dir = package_dir.join("vendor");
    std::fs::create_dir_all(&vendor_dir).unwrap();
    for (name, bytes) in &vendor_files {
        std::fs::write(vendor_dir.join(name), bytes).unwrap();
    }

    // --- publish: blobs first, then the .mfp -------------------------------
    let published = run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", package_dir_arg],
    );
    assert!(
        published.status.success(),
        "publish failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&published.stdout),
        String::from_utf8_lossy(&published.stderr)
    );
    let publish_out = String::from_utf8_lossy(&published.stdout).into_owned();
    assert!(
        publish_out.contains("Vendor blobs: 5 uploaded, 0 already present"),
        "publish should upload every vendor blob exactly once: {publish_out}"
    );

    // Each vendored file is stored as its own `<hash>.bin` native blob, beside —
    // never inside — the package's own `<hash>.mfp`.
    let stored_bin: Vec<_> = std::fs::read_dir(repo_dir.path().join("packages"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("bin"))
        .collect();
    assert_eq!(
        stored_bin.len(),
        vendor_files.len(),
        "one native blob per vendored file, got {stored_bin:?}"
    );
    let hex = |bytes: &[u8]| -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() };
    for path in &stored_bin {
        let bytes = std::fs::read(path).unwrap();
        let expected = path.file_stem().unwrap().to_str().unwrap();
        assert_eq!(
            hex(&crypto::sha256(&bytes)),
            expected,
            "a native blob must be stored under its own content hash"
        );
    }

    // --- re-publish an unchanged library uploads no bytes ------------------
    write_manifest("0.2.0");
    let republished = run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", package_dir_arg],
    );
    assert!(
        republished.status.success(),
        "re-publish failed: {}",
        String::from_utf8_lossy(&republished.stderr)
    );
    let republish_out = String::from_utf8_lossy(&republished.stdout).into_owned();
    assert!(
        republish_out.contains("Vendor blobs: 0 uploaded, 5 already present"),
        "an unchanged library must upload once, ever: {republish_out}"
    );

    // --- install: the libraries arrive with the package --------------------
    let app_dir = work.path().join("vendor_consumer");
    let app_dir_arg = app_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app_dir_arg]).status.success());
    let run_in = |dir: &std::path::Path, args: &[&str]| {
        Command::new(mfb_exe())
            .args(args)
            .current_dir(dir)
            .env("MFB_REPO_URL", &repo.url)
            .env("MFB_HOME", home.path().join(".mfb"))
            .output()
            .expect("run mfb in consumer")
    };

    let add = run_in(&app_dir, &["pkg", "add", "alice#vendorbind"]);
    assert!(
        add.status.success(),
        "vendor add failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&add.stdout),
        String::from_utf8_lossy(&add.stderr)
    );

    // EVERY vendor blob is downloaded — not just the host target's — so a later
    // cross-compile and an offline build both work. They land per-package, never
    // in the consumer's own `vendor/`.
    let installed_vendor = app_dir.join("packages/vendorbind.vendor");
    for (name, bytes) in &vendor_files {
        let path = installed_vendor.join(name);
        assert!(
            path.is_file(),
            "{} should have been downloaded",
            path.display()
        );
        assert_eq!(&std::fs::read(&path).unwrap(), bytes, "{name} bytes differ");
    }
    assert!(
        !app_dir.join("vendor").exists(),
        "an imported binding must never write into the consumer's own vendor/"
    );

    // plan-60-F §4.5: `remove` deletes the package's vendor directory too, via
    // `imported_vendor_dir` — not just the `.mfp`. Leaving it behind would
    // accumulate hash-verified native libraries for packages the project no
    // longer declares.
    let removed = run_mfb_in(
        &repo,
        home.path(),
        &app_dir,
        &["pkg", "remove", "alice#vendorbind", "--yes"],
    );
    assert!(
        removed.status.success(),
        "remove failed: {}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(
        !installed_vendor.exists(),
        "packages/<name>.vendor/ must be deleted with the package: {}",
        installed_vendor.display()
    );
    assert!(!app_dir.join("packages/vendorbind.mfp").exists());

    // The build finds the library with no file placed by hand, verifies its hash,
    // and copies it into the output beside the executable.
    let build = run_in(&app_dir, &["build"]);
    assert!(
        build.status.success(),
        "consumer build failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    // --- a tampered blob fails closed and leaves nothing -------------------
    let victim = stored_bin
        .iter()
        .find(|path| {
            std::fs::read(path)
                .map(|bytes| bytes == b"macos-any-arch demo library bytes")
                .unwrap_or(false)
        })
        .cloned()
        .unwrap_or_else(|| stored_bin[0].clone());
    let mut corrupt = std::fs::read(&victim).unwrap();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0x01;
    std::fs::write(&victim, &corrupt).unwrap();

    let app2 = work.path().join("vendor_consumer_tampered");
    let app2_arg = app2.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app2_arg]).status.success());
    let tampered = run_in(&app2, &["pkg", "add", "alice#vendorbind"]);
    assert!(
        !tampered.status.success(),
        "a tampered vendor blob must fail the add"
    );
    let tampered_err = format!(
        "{}{}",
        String::from_utf8_lossy(&tampered.stdout),
        String::from_utf8_lossy(&tampered.stderr)
    );
    assert!(
        tampered_err.contains("PACKAGE_VENDOR_BLOB_HASH_MISMATCH")
            || tampered_err.contains("does not match"),
        "expected a hash-mismatch refusal, got: {tampered_err}"
    );
    // Nothing usable is left behind — not even a `.part`.
    let leftover = app2.join("packages/vendorbind.vendor");
    if leftover.exists() {
        let names: Vec<_> = std::fs::read_dir(&leftover)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("libdemo.dylib"))
            .collect();
        assert!(
            names.is_empty(),
            "a failed vendor download must leave no file (not even a .part): {names:?}"
        );
    }
}
