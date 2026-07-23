use mfb_repository::crypto;
use tinyjson::JsonValue;

mod common;
use common::*;

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
