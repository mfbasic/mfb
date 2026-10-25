//! The registry client's identity calls (`client/identity.rs`) when the
//! registry refuses a step: a refused registration leaves no local keys, a
//! refused challenge/login/attestation/link/revocation is reported rather than
//! half-applied, and `auth` copes with a missing public key file. Plus the
//! chain source `registry_chains` hands the trust walk.

use super::*;

/// A registry that answers `/ident` and refuses `path` with 403 and `message`.
fn refusing(registry: &Registry, path: &str, message: &str) -> Stub {
    registry
        .routes()
        .reply(
            path,
            403,
            serde_json::json!({ "error": message }).to_string(),
        )
        .serve()
}

#[test]
fn a_refused_registration_leaves_no_local_keys() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let stub = refusing(
        &registry,
        "/accounts/register",
        "owner name 'alice' is already in use",
    );
    assert_eq!(
        register(&stub.url, &paths, "alice").unwrap_err(),
        "owner name 'alice' is already in use"
    );
    assert!(!paths.auth_private_key_path("alice").exists());
    assert!(!paths.ident_private_key_path("alice").exists());
}

#[test]
fn auth_without_a_local_key_reports_the_local_error_when_the_probe_fails_otherwise() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let stub = registry
        .routes()
        .reply(
            "/auth/challenge",
            500,
            serde_json::json!({"error": "database is locked"}).to_string(),
        )
        .serve();
    let err = auth(&stub.url, &paths, "alice").unwrap_err();
    assert!(
        err.starts_with(&format!(
            "missing local private key '{}'",
            paths.auth_private_key_path("alice").display()
        )),
        "{err}"
    );
    assert!(!err.contains("database is locked"), "{err}");
}

fn seed_auth(paths: &LocalPaths, owner: &str) -> Vec<u8> {
    let (public, private) = crypto::generate_keypair();
    local::write_auth_keypair(paths, owner, &public, &private).unwrap();
    public
}

#[test]
fn auth_without_a_public_key_file_logs_in_with_the_private_key() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let auth_public = seed_auth(&paths, "alice");
    std::fs::remove_file(paths.auth_public_key_path("alice")).unwrap();
    let nonce = [9u8; 32];
    let stub = registry
        .routes()
        .ok(
            "/auth/challenge",
            serde_json::json!({
                "challengeId": "chal-1",
                "nonce": crypto::encode_bytes(&nonce),
                "expiresAt": 99,
            })
            .to_string(),
        )
        .ok(
            "/auth/login",
            serde_json::json!({"sessionToken": "session-jwt", "owner": "alice", "expiresAt": 99})
                .to_string(),
        )
        .serve();
    let login = auth(&stub.url, &paths, "alice").expect("login");
    assert_eq!(login.session_token, "session-jwt");
    assert_eq!(local::read_session(&paths, "alice").unwrap(), "session-jwt");
    assert_eq!(
        field(&stub.request_to("/auth/challenge"), "authFingerprint"),
        crypto::fingerprint(&auth_public)
    );
}

#[test]
fn a_refused_challenge_or_login_is_reported_and_writes_no_session() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    seed_auth(&paths, "alice");
    let stub = refusing(&registry, "/auth/challenge", "unknown auth key");
    assert_eq!(
        auth(&stub.url, &paths, "alice").unwrap_err(),
        "unknown auth key"
    );

    let stub = registry
        .routes()
        .ok(
            "/auth/challenge",
            serde_json::json!({
                "challengeId": "chal-1",
                "nonce": crypto::encode_bytes(&[9u8; 32]),
                "expiresAt": 99,
            })
            .to_string(),
        )
        .reply(
            "/auth/login",
            403,
            serde_json::json!({"error": "expired challenge"}).to_string(),
        )
        .serve();
    assert_eq!(
        auth(&stub.url, &paths, "alice").unwrap_err(),
        "expired challenge"
    );
    assert!(local::read_session(&paths, "alice").is_err());
}

#[test]
fn refused_attestation_link_and_revocation_requests_are_reported() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    seed_ident(&paths, "alice");

    let stub = refusing(
        &registry,
        "/signing",
        "package is not owned by the session owner",
    );
    assert_eq!(
        request_attestation(
            &stub.url,
            &paths,
            "alice",
            "alice#pkg",
            "1.0.0",
            &"a".repeat(64)
        )
        .unwrap_err(),
        "package is not owned by the session owner"
    );

    let stub = refusing(
        &registry,
        "/machines/link",
        "a pairing with this code is already pending; generate a new code",
    );
    assert_eq!(
        link_start(&stub.url, &paths, "alice").unwrap_err(),
        "a pairing with this code is already pending; generate a new code"
    );

    let stub = refusing(&registry, "/machines/link/fetch", "pairing not found");
    let (_temp2, new_machine) = temp_paths();
    assert_eq!(
        link_fetch(&stub.url, &new_machine, "alice", "ABCD-EFGH-IJKL").unwrap_err(),
        "pairing not found"
    );
    assert!(!new_machine.ident_private_key_path("alice").exists());

    let stub = refusing(&registry, "/machines/revoke/challenge", "unknown owner");
    assert_eq!(
        revoke_machine(&stub.url, &paths, "alice", "fp").unwrap_err(),
        "unknown owner"
    );
}

#[test]
fn registry_chains_reads_each_owners_rotation_chain_from_the_registry() {
    let registry = Registry::new();
    let (old_public, old_private) = crypto::generate_keypair();
    let (new_public, _new_private) = crypto::generate_keypair();
    let signature = crypto::sign(
        &old_private,
        &crypto::ident_rotation_message("alice", &crypto::fingerprint(&old_public), &new_public),
    )
    .unwrap();
    let link = serde_json::json!({
        "oldKey": crypto::encode_bytes(&old_public),
        "newKey": crypto::encode_bytes(&new_public),
        "signature": crypto::encode_bytes(&signature),
        "issued": 5,
    });
    let stub = registry
        .routes()
        .ok(
            "/idents/alice",
            serde_json::json!({
                "owner": "alice",
                "identKey": format!("ed25519:{}", crypto::encode_bytes(&new_public)),
                "identFingerprint": crypto::fingerprint(&new_public),
                "chain": [link],
            })
            .to_string(),
        )
        .serve();
    let chains = registry_chains(&stub.url);
    let chain = chains("alice").expect("alice's chain");
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].new_key, crypto::encode_bytes(&new_public));
    assert_eq!(chain[0].issued, 5);
    assert!(
        chains("bob").is_err(),
        "an owner the registry does not serve"
    );
}
