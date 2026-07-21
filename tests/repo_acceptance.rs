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

/// `run_mfb`, but from a chosen working directory — needed to exercise the
/// commands whose path argument defaults to `.` (plan-60-A §4.2).
fn run_mfb_in(
    repo: &RepoProcess,
    home: &std::path::Path,
    cwd: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    Command::new(mfb_exe())
        .args(args)
        .current_dir(cwd)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.join(".mfb"))
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
    // Re-registering from the machine that already holds alice's keys must fail
    // *and* leave those keys intact (bug-272).
    //
    // This assertion used to read the other way — that `Alice.*` did not exist
    // afterwards — which passed only because of the bug: register wrote the new
    // keypair with truncating writers and then deleted it on the server's error.
    // On a case-insensitive filesystem (macOS) `Alice.*` and `alice.*` are the
    // same files, so that sequence destroyed alice's real keys and the test
    // recorded it as success.
    //
    // Which layer refuses depends on the filesystem: case-insensitive, the local
    // guard sees the existing key and stops before any request; case-sensitive,
    // the names differ locally and the server's folded-name check rejects it.
    // Both are correct, so accept either message.
    let duplicate = run_mfb(&repo, home.path(), &["repo", "register", "Alice"]);
    assert!(!duplicate.status.success());
    let duplicate_err = String::from_utf8_lossy(&duplicate.stderr).to_string();
    assert!(
        duplicate_err.contains("already in use") || duplicate_err.contains("already exist locally"),
        "{duplicate_err}"
    );
    let repo_home = mfb_repo_home(&repo, home.path());
    assert!(repo_home.join("keys/alice.auth.prv").exists());
    assert!(repo_home.join("keys/alice.ident.prv").exists());

    // A *different* machine registering the same owner is still refused by the
    // server — the local guard cannot see keys it does not hold — and keeps no
    // keys from the refused attempt, which remains correct cleanup because it
    // created them itself.
    let other_home = tempfile::tempdir().unwrap();
    let remote_duplicate = run_mfb(&repo, other_home.path(), &["repo", "register", "alice"]);
    assert!(!remote_duplicate.status.success());
    assert!(
        String::from_utf8_lossy(&remote_duplicate.stderr).contains("already in use"),
        "{}",
        String::from_utf8_lossy(&remote_duplicate.stderr)
    );
    let other_repo_home = mfb_repo_home(&repo, other_home.path());
    assert!(!other_repo_home.join("keys/alice.auth.prv").exists());
    assert!(!other_repo_home.join("keys/alice.ident.prv").exists());

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
