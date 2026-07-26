use mfb_repository::crypto;
use std::process::{Command, Stdio};

mod common;
use common::*;

#[test]
fn repo_machine_link_makes_an_equal_and_revoke_cuts_it_off() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home_a = tempfile::tempdir().unwrap();
    let home_b = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    // Machine A registers and opens a session.
    assert!(
        run_mfb(&repo, home_a.path(), &["repo", "register", "alice"])
            .status
            .success()
    );
    assert!(run_mfb(&repo, home_a.path(), &["repo", "auth", "alice"])
        .status
        .success());

    // Machine A starts a link and displays the pairing code.
    let start = run_mfb(&repo, home_a.path(), &["repo", "link", "--start", "alice"]);
    assert!(
        start.status.success(),
        "link --start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    let stdout = String::from_utf8_lossy(&start.stdout);
    let code = stdout
        .lines()
        .map(str::trim)
        .find(|line| line.len() == 29 && line.bytes().filter(|byte| *byte == b'-').count() == 4)
        .expect("pairing code in output")
        .to_string();

    // Machine B links with the typed code (stdin) and becomes a full equal.
    let mut link = Command::new(mfb_exe())
        .args(["repo", "link", "alice"])
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home_b.path().join(".mfb"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn repo link");
    {
        use std::io::Write;
        link.stdin
            .as_mut()
            .unwrap()
            .write_all(format!("{code}\n").as_bytes())
            .unwrap();
    }
    let link = link.wait_with_output().expect("repo link");
    assert!(
        link.status.success(),
        "repo link failed: {}",
        String::from_utf8_lossy(&link.stderr)
    );

    // The ident keypair was copied; the auth keypair is machine B's own.
    let home_a_repo = mfb_repo_home(&repo, home_a.path());
    let home_b_repo = mfb_repo_home(&repo, home_b.path());
    let ident_a = std::fs::read_to_string(home_a_repo.join("keys/alice.ident.pub")).unwrap();
    let ident_b = std::fs::read_to_string(home_b_repo.join("keys/alice.ident.pub")).unwrap();
    assert_eq!(ident_a.trim(), ident_b.trim());
    let auth_a = std::fs::read_to_string(home_a_repo.join("keys/alice.auth.pub")).unwrap();
    let auth_b = std::fs::read_to_string(home_b_repo.join("keys/alice.auth.pub")).unwrap();
    assert_ne!(auth_a.trim(), auth_b.trim());

    // The pairing code is single use.
    let mut reuse = Command::new(mfb_exe())
        .args(["repo", "link", "alice"])
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", work.path().join("third/.mfb"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn reuse link");
    {
        use std::io::Write;
        reuse
            .stdin
            .as_mut()
            .unwrap()
            .write_all(format!("{code}\n").as_bytes())
            .unwrap();
    }
    let reuse = reuse.wait_with_output().expect("reuse link");
    assert!(!reuse.status.success());
    assert!(
        String::from_utf8_lossy(&reuse.stderr).contains("unknown, used, or expired pairing code"),
        "{}",
        String::from_utf8_lossy(&reuse.stderr)
    );

    // Machine B opens its own session and completes the FULL signed-build +
    // publish path with no involvement from machine A.
    assert!(run_mfb(&repo, home_b.path(), &["repo", "auth", "alice"])
        .status
        .success());
    let package_dir = work.path().join("linked_pkg");
    let package_dir_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir_arg])
        .status
        .success());
    let publish = run_mfb(
        &repo,
        home_b.path(),
        &["repo", "publish", "alice", package_dir_arg],
    );
    assert!(
        publish.status.success(),
        "linked-machine publish failed: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&publish.stdout),
        String::from_utf8_lossy(&publish.stderr)
    );

    // Machine A revokes machine B's auth key (lost machine): B's session is
    // dead and its auth key can no longer log in or fetch attestations.
    let auth_b_fingerprint = {
        let public = crypto::decode_bytes(auth_b.trim(), "auth key").unwrap();
        crypto::fingerprint(&public)
    };
    let revoke = run_mfb(
        &repo,
        home_a.path(),
        &["machine", "revoke", "alice", &auth_b_fingerprint],
    );
    assert!(
        revoke.status.success(),
        "machine revoke failed: {}",
        String::from_utf8_lossy(&revoke.stderr)
    );

    let auth_after = run_mfb(&repo, home_b.path(), &["repo", "auth", "alice"]);
    assert!(!auth_after.status.success());
    assert!(
        String::from_utf8_lossy(&auth_after.stderr).contains("mismatched local key fingerprint"),
        "{}",
        String::from_utf8_lossy(&auth_after.stderr)
    );
    // The revoked machine's existing session cannot request attestations.
    let build_after = run_mfb(
        &repo,
        home_b.path(),
        &["build", "--sign", "alice", package_dir_arg],
    );
    assert!(!build_after.status.success());
    assert!(
        String::from_utf8_lossy(&build_after.stderr).contains("unknown session token"),
        "{}",
        String::from_utf8_lossy(&build_after.stderr)
    );
    // Machine A is untouched.
    assert!(run_mfb(&repo, home_a.path(), &["repo", "auth", "alice"])
        .status
        .success());
}

#[test]
fn repo_ident_rotation_follows_pins_and_reanchor_hard_errors() {
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

    // Build + install a signed package; the consumer pins ident I0.
    let package_dir = work.path().join("rotating_pkg");
    let package_dir_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir_arg])
        .status
        .success());
    assert!(run_mfb(
        &repo,
        home.path(),
        &["build", "--sign", "alice", package_dir_arg]
    )
    .status
    .success());
    let mfp_path = package_dir.join("rotating_pkg.mfp");
    let app_dir = work.path().join("rotating_consumer");
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
    assert!(
        run_in_consumer(&["pkg", "add", &format!("file://{}", mfp_path.display())])
            .status
            .success()
    );
    let verify = run_in_consumer(&["pkg", "verify"]);
    assert!(String::from_utf8_lossy(&verify.stdout).contains("[Verified]"));

    // Rotate the ident (I0 -> I1). Packages published under I0 still verify:
    // the consumer's OFFLINE chain (old pin, old package) is untouched.
    let rotate = run_mfb(&repo, home.path(), &["key", "rotate", "alice"]);
    assert!(
        rotate.status.success(),
        "key rotate failed: {}",
        String::from_utf8_lossy(&rotate.stderr)
    );
    let build = run_in_consumer(&["build"]);
    assert!(
        String::from_utf8_lossy(&build.stdout).contains("uses rotating_pkg - [Verified]"),
        "old-ident package must still verify after rotation: {}",
        String::from_utf8_lossy(&build.stdout)
    );

    // Rebuild under I1 and reinstall: `pkg verify` follows the signed chain,
    // updates the pin with a notice, and the package verifies.
    assert!(run_mfb(
        &repo,
        home.path(),
        &["build", "--sign", "alice", package_dir_arg]
    )
    .status
    .success());
    std::fs::copy(&mfp_path, app_dir.join("packages/rotating_pkg.mfp")).unwrap();
    let manifest_before = std::fs::read_to_string(app_dir.join("project.json")).unwrap();
    let verify = run_in_consumer(&["pkg", "verify"]);
    let stdout = String::from_utf8_lossy(&verify.stdout);
    assert!(verify.status.success(), "{stdout}");
    assert!(
        stdout.contains("notice: owner `alice` rotated their ident"),
        "{stdout}"
    );
    assert!(stdout.contains("[Verified]"), "{stdout}");
    let manifest_after = std::fs::read_to_string(app_dir.join("project.json")).unwrap();
    assert_ne!(manifest_before, manifest_after, "the pin must be rewritten");
    // The follow is sticky: a plain build now verifies against the new pin
    // with no server contact.
    let build = run_in_consumer(&["build"]);
    assert!(
        String::from_utf8_lossy(&build.stdout).contains("uses rotating_pkg - [Verified]"),
        "{}",
        String::from_utf8_lossy(&build.stdout)
    );

    // Re-anchor (operator ceremony, NO chain link) to a fresh ident I2 and
    // hand alice's machine the new keypair out-of-band.
    let (anchor_public, anchor_private) = crypto::generate_keypair();
    let reanchor = Command::new(repo_exe())
        .args([
            "reanchor",
            "--dbpath",
            repo_dir.path().join("meta.db").to_str().unwrap(),
            "--datapath",
            repo_dir.path().join("packages").to_str().unwrap(),
            "--owner",
            "alice",
            "--ident-key",
            &crypto::encode_bytes(&anchor_public),
        ])
        .output()
        .expect("reanchor");
    assert!(
        reanchor.status.success(),
        "reanchor failed: {}",
        String::from_utf8_lossy(&reanchor.stderr)
    );
    let repo_home = mfb_repo_home(&repo, home.path());
    std::fs::write(
        repo_home.join("keys/alice.ident.pub"),
        crypto::encode_bytes(&anchor_public),
    )
    .unwrap();
    std::fs::write(
        repo_home.join("keys/alice.ident.prv"),
        crypto::encode_bytes(&anchor_private),
    )
    .unwrap();

    // A package signed by the re-anchored ident does NOT chain from the
    // consumer's pin: pkg verify hard-errors and leaves the pin alone.
    assert!(run_mfb(
        &repo,
        home.path(),
        &["build", "--sign", "alice", package_dir_arg]
    )
    .status
    .success());
    std::fs::copy(&mfp_path, app_dir.join("packages/rotating_pkg.mfp")).unwrap();
    let manifest_before = std::fs::read_to_string(app_dir.join("project.json")).unwrap();
    let verify = run_in_consumer(&["pkg", "verify"]);
    assert!(!verify.status.success());
    let stderr = String::from_utf8_lossy(&verify.stderr);
    assert!(
        stderr.contains("NO chain link"),
        "expected the re-anchor hard error, got: {stderr}"
    );
    let manifest_after = std::fs::read_to_string(app_dir.join("project.json")).unwrap();
    assert_eq!(
        manifest_before, manifest_after,
        "the pin must NOT be updated"
    );
}

#[test]
fn repo_check_abi_reports_superset_and_breaking_changes() {
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

    let package_dir = work.path().join("abi_pkg");
    let package_dir_arg = package_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", package_dir_arg])
        .status
        .success());
    let manifest_path = package_dir.join("project.json");
    let base_manifest = std::fs::read_to_string(&manifest_path).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#abi_pkg\",\n",
    );
    std::fs::write(&manifest_path, &base_manifest).unwrap();
    let src_path = package_dir.join("src/lib.mfb");

    // Publish the baseline (exports `answer`).
    assert!(run_mfb(
        &repo,
        home.path(),
        &["repo", "publish", "alice", package_dir_arg],
    )
    .status
    .success());

    let run_check = || {
        Command::new(mfb_exe())
            .args(["repo", "check-abi"])
            .current_dir(&package_dir)
            .env("MFB_REPO_URL", &repo.url)
            .env("MFB_HOME", home.path().join(".mfb"))
            .output()
            .expect("repo check-abi")
    };

    // Unchanged working tree: identical ABI, exit 0. This also proves the
    // registry stored and served a real (non-empty) abiIndex — an empty index
    // would have reported `answer` as dropped.
    let identical = run_check();
    let stdout = String::from_utf8_lossy(&identical.stdout);
    assert!(
        identical.status.success(),
        "check-abi failed: {stdout}\n{}",
        String::from_utf8_lossy(&identical.stderr)
    );
    assert!(stdout.contains("ABI is identical"), "{stdout}");

    // Adding an export is a backward-compatible superset (exit 0).
    std::fs::write(
        &src_path,
        "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n\
         EXPORT FUNC greet() AS Integer\n  RETURN 1\nEND FUNC\n",
    )
    .unwrap();
    let superset = run_check();
    let stdout = String::from_utf8_lossy(&superset.stdout);
    assert!(superset.status.success(), "{stdout}");
    assert!(stdout.contains("added:   greet"), "{stdout}");
    assert!(stdout.contains("superset"), "{stdout}");

    // Changing an exported signature is breaking: named + non-zero exit.
    std::fs::write(
        &src_path,
        "EXPORT FUNC answer(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\n",
    )
    .unwrap();
    let breaking = run_check();
    let stdout = String::from_utf8_lossy(&breaking.stdout);
    assert!(!breaking.status.success(), "{stdout}");
    assert!(stdout.contains("changed: answer"), "{stdout}");
}

#[test]
fn repo_signed_metadata_root_verifies_chain_and_gates_add() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();

    // Operator ceremony (offline, before serving): initialize the root of
    // trust. Prints the root fingerprint to pin.
    let init = Command::new(repo_exe())
        .args([
            "init-root",
            "--dbpath",
            repo_dir.path().join("meta.db").to_str().unwrap(),
            "--datapath",
            repo_dir.path().join("packages").to_str().unwrap(),
            "--registry-id",
            "test-registry",
        ])
        .output()
        .expect("init-root");
    assert!(
        init.status.success(),
        "init-root failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let init_stdout = String::from_utf8_lossy(&init.stdout);
    let root_fingerprint = init_stdout
        .lines()
        .find_map(|line| line.strip_prefix("Root fingerprint (pin this out of band): "))
        .expect("root fingerprint in init-root output")
        .trim()
        .to_string();

    let repo = start_repo(repo_dir.path());
    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "alice"])
        .status
        .success());

    // Pinning the correct root fingerprint verifies the whole chain and that
    // the pinned server key is root-delegated.
    let trust = run_mfb(
        &repo,
        home.path(),
        &["repo", "trust", "test-registry", &root_fingerprint],
    );
    assert!(
        trust.status.success(),
        "repo trust failed: {}",
        String::from_utf8_lossy(&trust.stderr)
    );
    assert!(
        String::from_utf8_lossy(&trust.stdout).contains("metadata chain verified"),
        "{}",
        String::from_utf8_lossy(&trust.stdout)
    );

    // A wrong root fingerprint is refused.
    let bad_trust = run_mfb(
        &repo,
        home.path(),
        &["repo", "trust", "test-registry", &"0".repeat(64)],
    );
    assert!(!bad_trust.status.success());

    // Publish a package, then a metadata-gated add must still succeed (the
    // chain verifies on the way in).
    let pkg_dir = work.path().join("meta_pkg");
    let pkg_arg = pkg_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", pkg_arg]).status.success());
    let manifest = pkg_dir.join("project.json");
    let base = std::fs::read_to_string(&manifest).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#meta_pkg\",\n",
    );
    std::fs::write(&manifest, &base).unwrap();
    assert!(
        run_mfb(&repo, home.path(), &["repo", "publish", "alice", pkg_arg])
            .status
            .success()
    );

    let app_dir = work.path().join("meta_consumer");
    assert!(run_mfb_plain(&["init", app_dir.to_str().unwrap()])
        .status
        .success());
    let add = Command::new(mfb_exe())
        .args(["pkg", "add", "alice#meta_pkg"])
        .current_dir(&app_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("metadata-gated add");
    assert!(
        add.status.success(),
        "metadata-gated add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    assert!(app_dir.join("packages/meta_pkg.mfp").is_file());
}

#[test]
fn repo_ownership_transfer_is_two_sided_and_rebinds_the_package() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    for owner in ["alice", "bob"] {
        assert!(run_mfb(&repo, home.path(), &["repo", "register", owner])
            .status
            .success());
        assert!(run_mfb(&repo, home.path(), &["repo", "auth", owner])
            .status
            .success());
    }

    // Alice publishes a package, then offers it to Bob.
    let pkg_dir = work.path().join("xfer_pkg");
    let pkg_arg = pkg_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", pkg_arg]).status.success());
    let manifest = pkg_dir.join("project.json");
    let base = std::fs::read_to_string(&manifest).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#xfer_pkg\",\n",
    );
    std::fs::write(&manifest, &base).unwrap();
    assert!(
        run_mfb(&repo, home.path(), &["repo", "publish", "alice", pkg_arg])
            .status
            .success()
    );

    let offer = run_mfb(
        &repo,
        home.path(),
        &["repo", "transfer", "alice#xfer_pkg", "bob"],
    );
    assert!(
        offer.status.success(),
        "transfer offer failed: {}",
        String::from_utf8_lossy(&offer.stderr)
    );

    // Bob accepts; the package is re-bound to bob.
    let accept = run_mfb(
        &repo,
        home.path(),
        &["repo", "transfer-accept", "alice#xfer_pkg@bob"],
    );
    assert!(
        accept.status.success(),
        "transfer accept failed: {}",
        String::from_utf8_lossy(&accept.stderr)
    );

    let opened = open_store(repo_dir.path());
    assert_eq!(
        opened
            .store
            .package_owner("alice#xfer_pkg")
            .unwrap()
            .unwrap()
            .owner_display,
        "bob"
    );
    // The already-published version is untouched.
    assert_eq!(
        opened
            .store
            .list_package_versions("alice#xfer_pkg")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn repo_release_state_yank_excludes_floating_but_allows_pin() {
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

    let pkg_dir = work.path().join("state_pkg");
    let pkg_arg = pkg_dir.to_str().unwrap();
    assert!(run_mfb_plain(&["init-pkg", pkg_arg]).status.success());
    let manifest = pkg_dir.join("project.json");
    let base = std::fs::read_to_string(&manifest).unwrap().replace(
        "  \"version\": \"0.1.0\",\n",
        "  \"version\": \"0.1.0\",\n  \"ident\": \"alice#state_pkg\",\n",
    );
    std::fs::write(&manifest, &base).unwrap();
    assert!(
        run_mfb(&repo, home.path(), &["repo", "publish", "alice", pkg_arg])
            .status
            .success()
    );

    let run_pkg = |args: &[&str]| {
        Command::new(mfb_exe())
            .args(args)
            .current_dir(&pkg_dir)
            .env("MFB_REPO_URL", &repo.url)
            .env("MFB_HOME", home.path().join(".mfb"))
            .output()
            .expect("run mfb pkg in package dir")
    };

    // A floating add succeeds while the version is available.
    let add_ok_dir = work.path().join("add_ok");
    assert!(run_mfb_plain(&["init", add_ok_dir.to_str().unwrap()])
        .status
        .success());
    let add_ok = Command::new(mfb_exe())
        .args(["pkg", "add", "alice#state_pkg"])
        .current_dir(&add_ok_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("floating add");
    assert!(
        add_ok.status.success(),
        "{}",
        String::from_utf8_lossy(&add_ok.stderr)
    );

    // Yank it (ident-signed, logged).
    let yank = run_pkg(&["repo", "release-state", "yanked"]);
    assert!(
        yank.status.success(),
        "yank failed: {}\n{}",
        String::from_utf8_lossy(&yank.stdout),
        String::from_utf8_lossy(&yank.stderr)
    );
    assert!(
        String::from_utf8_lossy(&yank.stdout).contains("to yanked"),
        "{}",
        String::from_utf8_lossy(&yank.stdout)
    );

    // A floating add now finds nothing install-eligible.
    let floating_dir = work.path().join("floating");
    assert!(run_mfb_plain(&["init", floating_dir.to_str().unwrap()])
        .status
        .success());
    let floating = Command::new(mfb_exe())
        .args(["pkg", "add", "alice#state_pkg"])
        .current_dir(&floating_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("floating add after yank");
    assert!(!floating.status.success());
    assert!(
        String::from_utf8_lossy(&floating.stderr).contains("install-eligible"),
        "{}",
        String::from_utf8_lossy(&floating.stderr)
    );

    // An exact pin still selects the yanked version.
    let pin_dir = work.path().join("pinned");
    assert!(run_mfb_plain(&["init", pin_dir.to_str().unwrap()])
        .status
        .success());
    let pinned = Command::new(mfb_exe())
        .args(["pkg", "add", "alice#state_pkg@0.1.0"])
        .current_dir(&pin_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("pin add after yank");
    assert!(
        pinned.status.success(),
        "pinned add of a yanked version must succeed: {}",
        String::from_utf8_lossy(&pinned.stderr)
    );
    assert!(pin_dir.join("packages/state_pkg.mfp").is_file());
}
