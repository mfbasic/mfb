use std::process::Command;

#[path = "../common/mod.rs"]
mod common;

#[path = "cli_repo_install/remove_update_and_resolve.rs"]
mod remove_update_and_resolve;

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

/// plan-49 acceptance: `mfb-repo gc` reclaims a blob that nothing references
/// and leaves every live package installable.
///
/// The orphan is created the way a real one is — a `PUT /blob` whose publish
/// never lands (network failure, failed validation, `^C`) — rather than by
/// writing a row directly, because the whole point of the plan is that this
/// path can now produce bytes nothing will ever name.
/// bug-794: the installer deduplicated vendor blobs by content hash and, on a
/// repeat hash, skipped placing the file too. A package whose locators ship
/// identical bytes under different source names installed only the first name,
/// so a build needing another locator could not find its library. Every name
/// must be placed with the verified bytes. The setup is the GC test's, with
/// every vendored file given the same bytes.
#[test]
fn identical_vendor_blobs_keep_every_source_filename() {
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
    // (the `.mfp`) and `package_version_blobs.hash` (the vendor blobs).
    //
    // Each vendored file gets DISTINCT bytes: `install_vendor_blobs` dedups by
    // content hash and places one file per unique hash under that locator's
    // `source` name, so identical bytes would install only the first name and a
    // build that looks up each locator's file by name could not find the rest.
    let vendor_bytes_for = |_source: &str| b"identical vendor bytes".to_vec();
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
    // A vendor locator names one concrete build (there is no fat ELF), so on Linux
    // each entry must name an explicit (arch, libc) — a wildcard is rejected with
    // PROJECT_JSON_LIBRARY_INVALID. Enumerate every arch/libc the suite runs on (CI
    // is linux/x86_64/glibc; the test boxes add musl and other arches) plus macOS
    // and Windows.
    // A Linux executable build emits both libc flavors of its arch in one pass, so
    // the consumer build resolves — and must find installed — both the glibc and
    // musl `.so` for the host arch.
    let linux_flavors = [
        ("x86_64", "glibc"),
        ("x86_64", "musl"),
        ("aarch64", "glibc"),
        ("aarch64", "musl"),
        ("riscv64", "glibc"),
        ("riscv64", "musl"),
    ];
    let mut locators = String::from(
        "      { \"os\": \"macos\", \"type\": \"vendor\", \"source\": \"libgc.dylib\" }",
    );
    locators.push_str(
        ",\n      { \"os\": \"windows\", \"arch\": \"x86_64\", \"type\": \"vendor\", \"source\": \"libgc.dll\" }",
    );
    for (arch, libc) in linux_flavors {
        locators.push_str(&format!(
            ",\n      {{ \"os\": \"linux\", \"arch\": \"{arch}\", \"libc\": \"{libc}\", \"type\": \"vendor\", \"source\": \"libgc-{arch}-{libc}.so\" }}"
        ));
    }
    std::fs::write(
        vendor_pkg.join("project.json"),
        format!(
            r#"{{
  "name": "gcvendor",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "package",
  "description": "Test fixture package for repo gc with a vendored library.",
  "ident": "alice#gcvendor",
  "libraries": {{
    "demo": [
{locators}
    ]
  }},
  "sources": [ {{ "root": "src", "role": "package", "include": ["**/*.mfb"] }} ]
}}
"#
        ),
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
    std::fs::write(
        vendor_pkg.join("vendor/libgc.dylib"),
        vendor_bytes_for("libgc.dylib"),
    )
    .unwrap();
    std::fs::write(
        vendor_pkg.join("vendor/libgc.dll"),
        vendor_bytes_for("libgc.dll"),
    )
    .unwrap();
    for (arch, libc) in linux_flavors {
        let source = format!("libgc-{arch}-{libc}.so");
        std::fs::write(
            vendor_pkg.join(format!("vendor/{source}")),
            vendor_bytes_for(&source),
        )
        .unwrap();
    }

    for pkg in [plain_arg, vendor_arg] {
        let published = run_mfb(&repo, home.path(), &["repo", "publish", "alice", pkg]);
        assert!(
            published.status.success(),
            "publish {pkg} failed: {}\n{}",
            String::from_utf8_lossy(&published.stdout),
            String::from_utf8_lossy(&published.stderr)
        );
    }

    let app_dir = work.path().join("consumer");
    assert!(run_mfb_plain(&["init", app_dir.to_str().unwrap()])
        .status
        .success());
    let add = Command::new(mfb_exe())
        .args(["pkg", "add", "alice#gcvendor"])
        .current_dir(&app_dir)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    for source in ["libgc.dylib", "libgc.dll", "libgc-x86_64-glibc.so"] {
        let placed = app_dir.join("packages/gcvendor.vendor").join(source);
        assert_eq!(
            std::fs::read(&placed).unwrap_or_default(),
            b"identical vendor bytes",
            "a successful install must place vendor source {source}"
        );
    }
}

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
    // (the `.mfp`) and `package_version_blobs.hash` (the vendor blobs).
    //
    // Each vendored file gets DISTINCT bytes: `install_vendor_blobs` dedups by
    // content hash and places one file per unique hash under that locator's
    // `source` name, so identical bytes would install only the first name and a
    // build that looks up each locator's file by name could not find the rest.
    let vendor_bytes_for =
        |source: &str| format!("gc acceptance vendored library: {source}").into_bytes();
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
    // A vendor locator names one concrete build (there is no fat ELF), so on Linux
    // each entry must name an explicit (arch, libc) — a wildcard is rejected with
    // PROJECT_JSON_LIBRARY_INVALID. Enumerate every arch/libc the suite runs on (CI
    // is linux/x86_64/glibc; the test boxes add musl and other arches) plus macOS
    // and Windows.
    // A Linux executable build emits both libc flavors of its arch in one pass, so
    // the consumer build resolves — and must find installed — both the glibc and
    // musl `.so` for the host arch.
    let linux_flavors = [
        ("x86_64", "glibc"),
        ("x86_64", "musl"),
        ("aarch64", "glibc"),
        ("aarch64", "musl"),
        ("riscv64", "glibc"),
        ("riscv64", "musl"),
    ];
    let mut locators = String::from(
        "      { \"os\": \"macos\", \"type\": \"vendor\", \"source\": \"libgc.dylib\" }",
    );
    locators.push_str(
        ",\n      { \"os\": \"windows\", \"arch\": \"x86_64\", \"type\": \"vendor\", \"source\": \"libgc.dll\" }",
    );
    for (arch, libc) in linux_flavors {
        locators.push_str(&format!(
            ",\n      {{ \"os\": \"linux\", \"arch\": \"{arch}\", \"libc\": \"{libc}\", \"type\": \"vendor\", \"source\": \"libgc-{arch}-{libc}.so\" }}"
        ));
    }
    std::fs::write(
        vendor_pkg.join("project.json"),
        format!(
            r#"{{
  "name": "gcvendor",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "package",
  "description": "Test fixture package for repo gc with a vendored library.",
  "ident": "alice#gcvendor",
  "libraries": {{
    "demo": [
{locators}
    ]
  }},
  "sources": [ {{ "root": "src", "role": "package", "include": ["**/*.mfb"] }} ]
}}
"#
        ),
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
    std::fs::write(
        vendor_pkg.join("vendor/libgc.dylib"),
        vendor_bytes_for("libgc.dylib"),
    )
    .unwrap();
    std::fs::write(
        vendor_pkg.join("vendor/libgc.dll"),
        vendor_bytes_for("libgc.dll"),
    )
    .unwrap();
    for (arch, libc) in linux_flavors {
        let source = format!("libgc-{arch}-{libc}.so");
        std::fs::write(
            vendor_pkg.join(format!("vendor/{source}")),
            vendor_bytes_for(&source),
        )
        .unwrap();
    }

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
        "expected two .mfp blobs and the vendor blobs: {live_blobs:?}"
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
    // The vendored libraries came back down byte-for-byte — their blobs survived
    // the sweep. `install_vendor_blobs` places every locator's file (each a distinct
    // reachable blob), so verify each installed file serves exactly the bytes its
    // name was published with. This must find at least one file.
    let vendor_dir = app_dir.join("packages/gcvendor.vendor");
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&vendor_dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", vendor_dir.display()))
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert_eq!(
            std::fs::read(entry.path()).unwrap(),
            vendor_bytes_for(&name),
            "surviving vendor blob {name} must still serve its exact bytes"
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "no vendored library was installed in {}",
        vendor_dir.display()
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
