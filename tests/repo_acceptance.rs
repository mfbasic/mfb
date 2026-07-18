use mfb_repository::crypto;
use mfb_repository::store::Store;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Once;
use tinyjson::JsonValue;

static BUILD_REPO: Once = Once::new();

struct RepoProcess {
    child: Child,
    url: String,
}

impl Drop for RepoProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn mfb_exe() -> String {
    std::env::var("CARGO_BIN_EXE_mfb").unwrap_or_else(|_| "target/debug/mfb".to_string())
}

fn repo_exe() -> String {
    BUILD_REPO.call_once(|| {
        let status = Command::new("cargo")
            .args([
                "build",
                "--manifest-path",
                "repository/Cargo.toml",
                "--bin",
                "mfb-repo",
            ])
            .status()
            .expect("build mfb-repo");
        assert!(status.success(), "mfb-repo build failed");
    });
    "repository/target/debug/mfb-repo".to_string()
}

fn start_repo(repo_dir: &std::path::Path) -> RepoProcess {
    let mut child = Command::new(repo_exe())
        .args([
            "--dbpath",
            repo_dir.join("meta.db").to_str().unwrap(),
            "--datapath",
            repo_dir.join("packages").to_str().unwrap(),
            "--listen",
            "127.0.0.1:0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start mfb-repo");

    let stdout = child.stdout.take().expect("repo stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("repo listen line");
    let address = line
        .trim()
        .strip_prefix("MFB_REPO_LISTEN=")
        .expect("repo listen prefix");
    RepoProcess {
        child,
        url: format!("http://{address}"),
    }
}

fn open_store(repo_dir: &std::path::Path) -> mfb_repository::store::OpenedRepository {
    Store::open_repository(&repo_dir.join("meta.db"), &repo_dir.join("packages"))
        .expect("open repository store")
}

/// The local key/session store the CLI uses for this repository: MFB_HOME
/// scoped by the SHA-256 of the repository URL (`~/.mfb/<repo-hash>/`).
fn mfb_repo_home(repo: &RepoProcess, home: &std::path::Path) -> std::path::PathBuf {
    home.join(".mfb")
        .join(crypto::fingerprint(repo.url.as_bytes()))
}

fn run_mfb(repo: &RepoProcess, home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(mfb_exe())
        .args(args)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.join(".mfb"))
        .output()
        .expect("run mfb")
}

fn run_mfb_plain(args: &[&str]) -> std::process::Output {
    Command::new(mfb_exe())
        .args(args)
        .output()
        .expect("run mfb")
}

#[test]
fn repo_register_and_authenticate_owner() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(repo_dir.path().join("meta.db").is_file());
    assert!(repo_dir.path().join("packages").is_dir());

    let output = run_mfb(&repo, home.path(), &["repo", "register", "alice"]);
    assert!(
        output.status.success(),
        "register failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let repo_home = mfb_repo_home(&repo, home.path());
    assert!(repo_home.join("keys/alice.auth.pub").is_file());
    assert!(repo_home.join("keys/alice.auth.prv").is_file());
    assert!(repo_home.join("keys/alice.ident.pub").is_file());
    assert!(repo_home.join("keys/alice.ident.prv").is_file());
    // The registry public key is pinned on first contact.
    let pinned = std::fs::read_to_string(repo_home.join("server.pub")).unwrap();
    assert!(!pinned.trim().is_empty());

    let opened = open_store(repo_dir.path());
    assert_eq!(opened.store.count_owners().unwrap(), 1);
    // The server never stores a user private key: only the two public keys.
    let (_owner, auth_key) = opened.store.owner_with_auth_key("alice").unwrap().unwrap();
    let (_owner, ident_key) = opened.store.owner_with_ident_key("alice").unwrap().unwrap();
    assert_ne!(auth_key.fingerprint, ident_key.fingerprint);

    let output = run_mfb(&repo, home.path(), &["repo", "auth", "alice"]);
    assert!(
        output.status.success(),
        "auth failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let session_path = repo_home.join("session/alice.ses");
    assert!(session_path.is_file());
    let token = std::fs::read_to_string(session_path).unwrap();
    let payload = token.split('.').nth(1).unwrap();
    let payload = crypto::decode_bytes(payload, "jwt payload").unwrap();
    let claims: JsonValue = String::from_utf8(payload).unwrap().parse().unwrap();
    assert_eq!(claims["sub"].get::<String>().unwrap(), "alice");
    let iat = *claims["iat"].get::<f64>().unwrap() as i64;
    let exp = *claims["exp"].get::<f64>().unwrap() as i64;
    assert!(exp - iat <= 3600);
}

#[test]
fn repo_rejects_duplicate_and_missing_owner_auth() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    let duplicate = run_mfb(&repo, home.path(), &["repo", "register", "Alice"]);
    assert!(!duplicate.status.success());
    assert!(
        String::from_utf8_lossy(&duplicate.stderr).contains("already in use"),
        "{}",
        String::from_utf8_lossy(&duplicate.stderr)
    );
    let repo_home = mfb_repo_home(&repo, home.path());
    assert!(!repo_home.join("keys/Alice.auth.pub").exists());
    assert!(!repo_home.join("keys/Alice.auth.prv").exists());
    assert!(!repo_home.join("keys/Alice.ident.pub").exists());
    assert!(!repo_home.join("keys/Alice.ident.prv").exists());
    let opened = open_store(repo_dir.path());
    assert_eq!(opened.store.count_owners().unwrap(), 1);

    let missing = run_mfb(&repo, home.path(), &["repo", "auth", "missing_owner"]);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("unknown owner"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );
}

#[test]
fn repo_auth_requires_local_private_key_and_keeps_sessions_per_owner() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());
    assert!(run_mfb(&repo, home.path(), &["repo", "register", "bob"])
        .status
        .success());
    let repo_home = mfb_repo_home(&repo, home.path());
    std::fs::remove_file(repo_home.join("keys/alice.auth.prv")).unwrap();
    let missing_key = run_mfb(&repo, home.path(), &["repo", "auth", "alice"]);
    assert!(!missing_key.status.success());
    assert!(
        String::from_utf8_lossy(&missing_key.stderr).contains("missing local private key"),
        "{}",
        String::from_utf8_lossy(&missing_key.stderr)
    );

    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "bob"])
        .status
        .success());
    assert!(repo_home.join("session/bob.ses").is_file());
    assert!(!repo_home.join("session/alice.ses").exists());
}

#[test]
fn repo_refuses_a_changed_server_key() {
    let repo_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let repo = start_repo(repo_dir.path());

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"])
        .status
        .success());

    // Poison the pinned server key: every later contact must refuse to
    // proceed rather than silently trusting the new key.
    let repo_home = mfb_repo_home(&repo, home.path());
    let (other_key, _other_private) = crypto::generate_keypair();
    std::fs::write(
        repo_home.join("server.pub"),
        crypto::encode_bytes(&other_key),
    )
    .unwrap();

    let auth = run_mfb(&repo, home.path(), &["repo", "auth", "alice"]);
    assert!(!auth.status.success());
    assert!(
        String::from_utf8_lossy(&auth.stderr).contains("does not match the pinned key"),
        "{}",
        String::from_utf8_lossy(&auth.stderr)
    );
}

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
        &["pkg", "publish", "alice", package_dir_arg],
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
        &["pkg", "publish", "alice", package_dir_arg],
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
        &["pkg", "publish", "alice", package_dir2_arg],
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
        &["pkg", "publish", "alice", package_dir_arg],
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
        &["pkg", "publish", "alice", package_dir_arg],
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

    // A version that does not exist is rejected with an actionable message.
    let app2 = work.path().join("registry_consumer2");
    let app2_arg = app2.to_str().unwrap();
    assert!(run_mfb_plain(&["init", app2_arg]).status.success());
    let missing = Command::new(mfb_exe())
        .args(["pkg", "add", "alice#addable_pkg@9.9.9"])
        .current_dir(&app2)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.path().join(".mfb"))
        .output()
        .expect("pkg add missing version");
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("no version `9.9.9`"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
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
        &["pkg", "publish", "alice", package_dir_arg],
    )
    .status
    .success());

    let run_check = || {
        Command::new(mfb_exe())
            .args(["pkg", "check-abi"])
            .current_dir(&package_dir)
            .env("MFB_REPO_URL", &repo.url)
            .env("MFB_HOME", home.path().join(".mfb"))
            .output()
            .expect("pkg check-abi")
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
        run_mfb(&repo, home.path(), &["pkg", "publish", "alice", dep_arg])
            .status
            .success()
    );
    let bumped = base.replace("\"version\": \"0.1.0\"", "\"version\": \"0.1.1\"");
    std::fs::write(&dep_manifest, &bumped).unwrap();
    assert!(
        run_mfb(&repo, home.path(), &["pkg", "publish", "alice", dep_arg])
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
        run_mfb(&repo, home.path(), &["pkg", "publish", "alice", pkg_arg])
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
        run_mfb(&repo, home.path(), &["pkg", "publish", "alice", pkg_arg])
            .status
            .success()
    );

    let offer = run_mfb(
        &repo,
        home.path(),
        &["pkg", "transfer", "alice#xfer_pkg", "bob"],
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
        &["pkg", "transfer-accept", "alice#xfer_pkg@bob"],
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
        run_mfb(&repo, home.path(), &["pkg", "publish", "alice", pkg_arg])
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
    let yank = run_pkg(&["pkg", "release-state", "yanked"]);
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
    assert!(
        run_mfb(&repo, home.path(), &["pkg", "publish", "alice", common_arg])
            .status
            .success()
    );
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
    assert!(
        run_mfb(&repo, home.path(), &["pkg", "publish", "alice", common_arg])
            .status
            .success()
    );

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
    let publish_user = run_mfb(&repo, home.path(), &["pkg", "publish", "alice", user_arg]);
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
    assert!(run_in(&["pkg", "add", "alice#common@2.0.0"])
        .status
        .success());
    let manifest_path = app_dir.join("project.json");
    let relaxed = std::fs::read_to_string(&manifest_path)
        .unwrap()
        .replace("\"pin\": true", "\"pin\": false");
    std::fs::write(&manifest_path, relaxed).unwrap();

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
        &["pkg", "publish", "alice", app_dir_arg],
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
        &["pkg", "publish", "alice", package_dir_arg],
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
/// `pkg publish` uploads each `vendor` locator's file as its own content-addressed
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
        &["pkg", "publish", "alice", package_dir_arg],
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
        &["pkg", "publish", "alice", package_dir_arg],
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
