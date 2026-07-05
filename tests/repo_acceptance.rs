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
    home.join(".mfb").join(crypto::fingerprint(repo.url.as_bytes()))
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
    assert_eq!(u16::from_le_bytes([package[20], package[21]]), 1);
    assert_eq!(
        u32::from_le_bytes([package[22], package[23], package[24], package[25]]),
        64
    );
    assert!(package
        .windows(b"alice".len())
        .any(|window| window == b"alice"));

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
    let executable = std::fs::read_dir(&app_dir)
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
