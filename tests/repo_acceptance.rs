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
            "--path",
            repo_dir.to_str().unwrap(),
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

fn run_mfb(repo: &RepoProcess, home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(mfb_exe())
        .args(args)
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
    assert!(home.path().join(".mfb/keys/alice.pub").is_file());
    assert!(home.path().join(".mfb/keys/alice.prv").is_file());

    let opened = Store::open_repository(repo_dir.path()).unwrap();
    assert_eq!(opened.store.count_owners().unwrap(), 1);

    let output = run_mfb(&repo, home.path(), &["repo", "auth", "alice"]);
    assert!(
        output.status.success(),
        "auth failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let session_path = home.path().join(".mfb/session/alice.ses");
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

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"]).status.success());
    let duplicate = run_mfb(&repo, home.path(), &["repo", "register", "Alice"]);
    assert!(!duplicate.status.success());
    assert!(
        String::from_utf8_lossy(&duplicate.stderr).contains("already in use"),
        "{}",
        String::from_utf8_lossy(&duplicate.stderr)
    );
    assert!(!home.path().join(".mfb/keys/Alice.pub").exists());
    assert!(!home.path().join(".mfb/keys/Alice.prv").exists());
    let opened = Store::open_repository(repo_dir.path()).unwrap();
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

    assert!(run_mfb(&repo, home.path(), &["repo", "register", "alice"]).status.success());
    assert!(run_mfb(&repo, home.path(), &["repo", "register", "bob"]).status.success());
    std::fs::remove_file(home.path().join(".mfb/keys/alice.prv")).unwrap();
    let missing_key = run_mfb(&repo, home.path(), &["repo", "auth", "alice"]);
    assert!(!missing_key.status.success());
    assert!(
        String::from_utf8_lossy(&missing_key.stderr).contains("missing local private key"),
        "{}",
        String::from_utf8_lossy(&missing_key.stderr)
    );

    assert!(run_mfb(&repo, home.path(), &["repo", "auth", "bob"]).status.success());
    assert!(home.path().join(".mfb/session/bob.ses").is_file());
    assert!(!home.path().join(".mfb/session/alice.ses").exists());
}
