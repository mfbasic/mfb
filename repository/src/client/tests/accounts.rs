//! plan-176-I: `client::tests` — the accounts tests. The shared metadata fixtures and the\n//! in-process HTTP stub live in `client/tests/mod.rs`.

use super::*;

// --- /ident and the server-key pin -----------------------------------

/// First contact pins the served key; a body whose fingerprint disagrees
/// with its own key is refused before anything is pinned.
#[test]
fn ensure_server_key_pins_the_key_and_rejects_a_self_inconsistent_ident() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();

    let stub = registry.routes().serve();
    let fetched = ensure_server_key(&stub.url, &paths).expect("/ident pins on first contact");
    assert_eq!(fetched, registry.public);
    assert_eq!(
        local::read_pinned_server_key(&paths).unwrap(),
        registry.public,
        "the fetched key must be the one written to server.pub"
    );

    // A registry claiming a fingerprint that is not its key's own.
    let (_temp2, paths2) = temp_paths();
    let liar = Routes::new()
        .ok(
            "/ident",
            serde_json::json!({
                "serverKey": crypto::encode_bytes(&registry.public),
                "serverFingerprint": "0".repeat(64),
            })
            .to_string(),
        )
        .serve();
    let err = ensure_server_key(&liar.url, &paths2).unwrap_err();
    assert!(err.contains("fingerprint does not match its key"), "{err}");
    assert!(
        !paths2.server_key_path().exists(),
        "a self-inconsistent /ident must not pin anything"
    );
}

// --- register / auth --------------------------------------------------

/// A successful `register` posts role-separated proofs of possession for
/// the two keys it just generated, and keeps exactly those keys on disk.
#[test]
fn register_posts_role_separated_proofs_for_the_keys_it_stores() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let stub = registry
        .routes()
        .ok(
            "/accounts/register",
            serde_json::json!({
                "owner": "alice",
                "authFingerprint": "af",
                "identFingerprint": "if",
            })
            .to_string(),
        )
        .serve();

    let response = register(&stub.url, &paths, "alice").expect("registration succeeds");
    assert_eq!(response.owner, "alice");
    assert_eq!(response.auth_fingerprint, "af");

    // The keys the server was told about are the keys kept locally.
    let request = stub.request_to("/accounts/register");
    let stored_auth = local::read_auth_public_key(&paths, "alice").unwrap();
    let stored_ident = local::read_ident_public_key(&paths, "alice").unwrap();
    assert_eq!(
        crypto::decode_bytes(&field(&request, "authKey"), "authKey").unwrap(),
        stored_auth
    );
    assert_eq!(
        crypto::decode_bytes(&field(&request, "identKey"), "identKey").unwrap(),
        stored_ident
    );

    // Each proof is bound to its own role, so an auth proof can never be
    // replayed as an ident proof.
    let sent = json_body(&request);
    let proofs = sent.get("proofs").expect("proofs object");
    let auth_proof =
        crypto::decode_bytes(proofs.get("auth").unwrap().as_str().unwrap(), "auth").unwrap();
    let ident_proof =
        crypto::decode_bytes(proofs.get("ident").unwrap().as_str().unwrap(), "ident").unwrap();
    crypto::verify(
        &stored_auth,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &stored_auth),
        &auth_proof,
    )
    .expect("auth proof verifies under the auth key");
    crypto::verify(
        &stored_ident,
        &crypto::registration_message(crypto::ROLE_IDENT, "alice", &stored_ident),
        &ident_proof,
    )
    .expect("ident proof verifies under the ident key");
    assert!(crypto::verify(
        &stored_ident,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &stored_ident),
        &ident_proof,
    )
    .is_err());
}

/// `auth` signs the server's challenge with the local auth key and stores
/// the returned session token.
#[test]
fn auth_signs_the_challenge_and_stores_the_session() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let (auth_public, auth_private) = crypto::generate_keypair();
    local::write_auth_keypair(&paths, "alice", &auth_public, &auth_private).unwrap();

    let nonce = vec![7u8; 32];
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
            serde_json::json!({
                "sessionToken": "session-jwt",
                "owner": "alice",
                "expiresAt": 99,
            })
            .to_string(),
        )
        .serve();

    let login = auth(&stub.url, &paths, "alice").expect("auth succeeds");
    assert_eq!(login.session_token, "session-jwt");
    assert_eq!(
        local::read_session(&paths, "alice").unwrap(),
        "session-jwt",
        "the session token must be persisted for later calls"
    );

    // The challenge request identifies the key by fingerprint...
    let challenge = stub.request_to("/auth/challenge");
    assert_eq!(
        field(&challenge, "authFingerprint"),
        crypto::fingerprint(&auth_public)
    );
    // ...and the login proves possession of it over this exact challenge.
    assert_signed(
        &stub.request_to("/auth/login"),
        "signature",
        &auth_public,
        &crypto::challenge_message("chal-1", &nonce),
    );
}

/// A local private key that does not match the stored public key is a
/// tampered/mixed keypair; `auth` refuses rather than signing with it.
#[test]
fn auth_refuses_a_local_keypair_whose_halves_disagree() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let (_public_a, private_a) = crypto::generate_keypair();
    let (public_b, _private_b) = crypto::generate_keypair();
    // Public half of B stored beside the private half of A.
    local::write_auth_keypair(&paths, "alice", &public_b, &private_a).unwrap();

    let stub = registry.routes().serve();
    let err = auth(&stub.url, &paths, "alice").unwrap_err();
    assert_eq!(err, "mismatched local key fingerprint");
}

/// With no local key, `auth` probes the registry: an "unknown owner" answer
/// is the more useful diagnosis and replaces the local read error.
#[test]
fn auth_surfaces_unknown_owner_from_the_probe() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let stub = registry
        .routes()
        .reply(
            "/auth/challenge",
            404,
            "{\"error\":\"unknown owner 'alice'\"}",
        )
        .serve();

    let err = auth(&stub.url, &paths, "alice").unwrap_err();
    assert_eq!(err, "unknown owner 'alice'");

    // A probe failure that is *not* about the owner leaves the local error
    // in place, since it is the one the user can act on.
    let (_temp2, paths2) = temp_paths();
    let stub2 = registry
        .routes()
        .reply("/auth/challenge", 500, "{\"error\":\"database is down\"}")
        .serve();
    let err2 = auth(&stub2.url, &paths2, "alice").unwrap_err();
    assert!(err2.contains("missing local private key"), "{err2}");
}

// --- attestation ------------------------------------------------------

/// The attestation must verify under the *pinned* server key, so a swapped
/// registry cannot hand back paperwork consumers would later reject.
#[test]
fn request_attestation_verifies_the_attestation_under_the_pinned_key() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    seed_ident(&paths, "alice");

    let attestation = "{\"ident\":\"alice#pkg\",\"version\":\"1.0.0\"}";
    let signature = crypto::sign(
        &registry.private,
        &crypto::attestation_signing_input(attestation.as_bytes()),
    )
    .unwrap();
    let stub = registry
        .routes()
        .ok(
            "/signing",
            serde_json::json!({
                "owner": "alice",
                "attestation": attestation,
                "attestationSignature": crypto::encode_bytes(&signature),
            })
            .to_string(),
        )
        .serve();

    let response = request_attestation(&stub.url, &paths, "alice", "alice#pkg", "1.0.0", "sfp")
        .expect("a correctly signed attestation is accepted");
    assert_eq!(response.attestation, attestation);
    // The session token travels in the request body, not a header.
    assert_eq!(
        field(&stub.request_to("/signing"), "sessionToken"),
        "session-jwt"
    );

    // Same attestation, signature from some other key: refused.
    let (_temp2, paths2) = temp_paths();
    seed_ident(&paths2, "alice");
    let (_other_public, other_private) = crypto::generate_keypair();
    let forged = crypto::sign(
        &other_private,
        &crypto::attestation_signing_input(attestation.as_bytes()),
    )
    .unwrap();
    let stub2 = registry
        .routes()
        .ok(
            "/signing",
            serde_json::json!({
                "owner": "alice",
                "attestation": attestation,
                "attestationSignature": crypto::encode_bytes(&forged),
            })
            .to_string(),
        )
        .serve();
    let err =
        request_attestation(&stub2.url, &paths2, "alice", "alice#pkg", "1.0.0", "sfp").unwrap_err();
    assert!(
        err.contains("does not verify under the pinned server key"),
        "{err}"
    );
}

// --- machine link -----------------------------------------------------

/// The relayed blob is the ident keypair sealed under the pairing code, and
/// the server only ever sees the one-way lookup of that code.
#[test]
fn link_start_relays_only_a_blob_sealed_under_the_pairing_code() {
    // These reach `pin_server_key` through link's trust-on-first-use, which
    // reads the process-wide MFB_REPO_SERVER_FINGERPRINT. Take the same lock
    // `local::tests` takes, or a sibling that sets the variable makes this
    // assert on the wrong error.
    let _env = crate::local::tests::env_guard();
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let ident_public = seed_ident(&paths, "alice");
    let ident_private = local::read_ident_private_key(&paths, "alice").unwrap();

    let stub = registry
        .routes()
        .ok(
            "/machines/link",
            serde_json::json!({"owner": "alice", "expiresAt": 4242}).to_string(),
        )
        .serve();

    let (code, expires_at) = link_start(&stub.url, &paths, "alice").expect("link start");
    assert_eq!(expires_at, 4242);

    let request = stub.request_to("/machines/link");
    // The code itself is never sent — only its one-way lookup.
    assert!(!body_of(&request).contains(&code));
    assert_eq!(field(&request, "lookup"), crypto::pairing_lookup(&code));

    // The parked blob decrypts, with the displayed code, to exactly the
    // local ident keypair.
    let blob = crypto::decode_bytes(&field(&request, "blob"), "blob").unwrap();
    let salt = crypto::decode_bytes(&field(&request, "salt"), "salt").unwrap();
    let opened = crypto::open_pairing_blob(&code, &blob, &salt).expect("blob opens");
    let mut expected = ident_private;
    expected.extend_from_slice(&ident_public);
    assert_eq!(opened, expected);

    // bug-583: the parked row also carries the PUBLIC half of the
    // code-derived approval keypair — the verifier the fetch half is
    // checked against. The private half stays with the code.
    let (approval_public, _approval_private) = crypto::pairing_approval_keypair(&code).unwrap();
    assert_eq!(
        crypto::decode_bytes(&field(&request, "approvalKey"), "approvalKey").unwrap(),
        approval_public
    );
}

/// The new machine installs the relayed ident keypair alongside a fresh
/// auth keypair of its own.
#[test]
fn link_fetch_installs_the_relayed_ident_keypair() {
    // These reach `pin_server_key` through link's trust-on-first-use, which
    // reads the process-wide MFB_REPO_SERVER_FINGERPRINT. Take the same lock
    // `local::tests` takes, or a sibling that sets the variable makes this
    // assert on the wrong error.
    let _env = crate::local::tests::env_guard();
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let (ident_public, ident_private) = crypto::generate_keypair();
    let code = "abcde-fghij";
    let mut plaintext = ident_private.clone();
    plaintext.extend_from_slice(&ident_public);
    let (blob, salt) = crypto::seal_pairing_blob(code, &plaintext).unwrap();

    let stub = registry
        .routes()
        .ok(
            "/machines/link/fetch",
            serde_json::json!({
                "owner": "alice",
                "blob": crypto::encode_bytes(&blob),
                "salt": crypto::encode_bytes(&salt),
                "authFingerprint": "new-machine-af",
            })
            .to_string(),
        )
        .serve();

    // The typed code is trimmed, as it would be from a terminal paste.
    let response = link_fetch(&stub.url, &paths, "alice", "  abcde-fghij \n")
        .expect("link fetch installs the keypair");
    assert_eq!(response.auth_fingerprint, "new-machine-af");
    assert_eq!(
        response.ident_fingerprint,
        crypto::fingerprint(&ident_public)
    );
    assert_eq!(
        local::read_ident_private_key(&paths, "alice").unwrap(),
        ident_private
    );
    assert_eq!(
        local::read_ident_public_key(&paths, "alice").unwrap(),
        ident_public
    );

    // This machine registered its *own* auth key, proving possession of it.
    let request = stub.request_to("/machines/link/fetch");
    let auth_public = local::read_auth_public_key(&paths, "alice").unwrap();
    assert_eq!(
        crypto::decode_bytes(&field(&request, "authKey"), "authKey").unwrap(),
        auth_public
    );
    assert_signed(
        &request,
        "proof",
        &auth_public,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &auth_public),
    );
    assert_eq!(field(&request, "lookup"), crypto::pairing_lookup(code));

    // bug-583 cohesion pin: the approval this client sends is exactly what
    // the server verifies — the code-derived approval key signing THIS
    // pairing's lookup and THIS auth key. (`link_start`'s half of the pair
    // is pinned in the sibling test above, so the two together are the
    // end-to-end approval the server checks.)
    let (approval_public, _approval_private) = crypto::pairing_approval_keypair(code).unwrap();
    assert_signed(
        &request,
        "approval",
        &approval_public,
        &crypto::pairing_approval_message(&crypto::pairing_lookup(code), &auth_public),
    );
}

/// A blob that decrypts but is not a well-formed ident keypair is refused
/// before anything is written to disk.
#[test]
fn link_fetch_rejects_a_blob_that_is_not_an_ident_keypair() {
    // These reach `pin_server_key` through link's trust-on-first-use, which
    // reads the process-wide MFB_REPO_SERVER_FINGERPRINT. Take the same lock
    // `local::tests` takes, or a sibling that sets the variable makes this
    // assert on the wrong error.
    let _env = crate::local::tests::env_guard();
    let registry = Registry::new();
    let code = "abcde-fghij";

    let serve_blob = |plaintext: &[u8]| {
        let (blob, salt) = crypto::seal_pairing_blob(code, plaintext).unwrap();
        registry
            .routes()
            .ok(
                "/machines/link/fetch",
                serde_json::json!({
                    "owner": "alice",
                    "blob": crypto::encode_bytes(&blob),
                    "salt": crypto::encode_bytes(&salt),
                    "authFingerprint": "af",
                })
                .to_string(),
            )
            .serve()
    };

    // Wrong length.
    let (_temp, paths) = temp_paths();
    let stub = serve_blob(b"too short");
    let err = link_fetch(&stub.url, &paths, "alice", code).unwrap_err();
    assert_eq!(err, "pairing blob does not contain an ident keypair");
    assert!(!paths.ident_private_key_path("alice").exists());

    // Right length, but the halves are from different keypairs.
    let (_temp2, paths2) = temp_paths();
    let (_public_a, private_a) = crypto::generate_keypair();
    let (public_b, _private_b) = crypto::generate_keypair();
    let mut mixed = private_a;
    mixed.extend_from_slice(&public_b);
    let stub2 = serve_blob(&mixed);
    let err2 = link_fetch(&stub2.url, &paths2, "alice", code).unwrap_err();
    assert_eq!(err2, "pairing blob ident keypair is inconsistent");
    assert!(!paths2.ident_private_key_path("alice").exists());
}

// --- ident rotation ---------------------------------------------------

/// A committed rotation promotes the staged keypair and leaves no staging
/// behind; the chain link is signed by the *old* ident key.
#[test]
fn rotate_ident_promotes_the_staged_keypair_on_success() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let old_public = seed_ident(&paths, "alice");

    let stub = registry
        .routes()
        .ok(
            "/keys/rotate",
            serde_json::json!({"owner": "alice", "identFingerprint": "new-fp"}).to_string(),
        )
        .serve();

    let response = rotate_ident(&stub.url, &paths, "alice").expect("rotation commits");
    let new_public = local::read_ident_public_key(&paths, "alice").unwrap();
    assert_ne!(new_public, old_public, "the local ident key must advance");
    assert_eq!(response.ident_fingerprint, "new-fp");
    assert!(
        !paths.ident_pending_private_key_path("alice").exists(),
        "a committed rotation must clear the staging"
    );

    let request = stub.request_to("/keys/rotate");
    assert_eq!(
        crypto::decode_bytes(&field(&request, "newIdentKey"), "newIdentKey").unwrap(),
        new_public
    );
    // The chain link is signed by the old ident (the authority being
    // handed over) and possession is proven by the new one.
    assert_signed(
        &request,
        "chainSignature",
        &old_public,
        &crypto::ident_rotation_message("alice", &crypto::fingerprint(&old_public), &new_public),
    );
    assert_signed(
        &request,
        "possessionProof",
        &new_public,
        &crypto::registration_message(crypto::ROLE_IDENT, "alice", &new_public),
    );
}

/// `/keys/rotate` fails, and `/idents/<owner>` answers with whatever
/// `chain_reply` makes of the `newIdentKey` the client staged. This is the
/// lost-response scenario bug-276 R1 is about: the server may have
/// committed even though the client saw an error.
fn rotate_reconcile_stub(
    ident_body: String,
    chain_reply: impl Fn(Option<&str>) -> (u16, String) + Send + 'static,
) -> Stub {
    let staged: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    spawn_raw(move |request| {
        let path = path_of(request).to_string();
        let (status, body) = if path == "/ident" {
            (200, ident_body.clone())
        } else if path == "/keys/rotate" {
            *staged.lock().unwrap() = Some(field(request, "newIdentKey"));
            (503, "{\"error\":\"registry is unavailable\"}".to_string())
        } else if path.starts_with("/idents/") {
            let staged = staged.lock().unwrap().clone();
            chain_reply(staged.as_deref())
        } else {
            (404, "{\"error\":\"no route\"}".to_string())
        };
        http_reply(method_of(request), status, &body)
    })
}

fn ident_chain_body(ident_key: &str) -> String {
    serde_json::json!({
        "owner": "alice",
        "identKey": ident_key,
        "identFingerprint": "fp",
        "chain": [],
    })
    .to_string()
}

/// The rotation committed but the response was lost: the registry now holds
/// the staged key, so promoting it is the recovery — not discarding it.
#[test]
fn rotate_ident_promotes_when_the_registry_already_holds_the_staged_key() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let old_public = seed_ident(&paths, "alice");

    let stub = rotate_reconcile_stub(registry.ident_body(), |staged| {
        (200, ident_chain_body(staged.expect("rotate ran first")))
    });

    let response = rotate_ident(&stub.url, &paths, "alice")
        .expect("a committed-but-unacknowledged rotation is recovered");
    let now_local = local::read_ident_public_key(&paths, "alice").unwrap();
    assert_ne!(now_local, old_public);
    assert_eq!(response.ident_fingerprint, crypto::fingerprint(&now_local));
    assert!(!paths.ident_pending_private_key_path("alice").exists());
}

/// The registry demonstrably still holds the old ident: the rotation did
/// not happen, so the staging is discarded and the original error surfaces.
#[test]
fn rotate_ident_discards_the_staging_when_the_registry_kept_the_old_ident() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let old_public = seed_ident(&paths, "alice");

    let encoded_old = crypto::encode_bytes(&old_public);
    let stub = rotate_reconcile_stub(registry.ident_body(), move |_staged| {
        (200, ident_chain_body(&encoded_old))
    });

    let err = rotate_ident(&stub.url, &paths, "alice").unwrap_err();
    assert_eq!(err, "registry is unavailable");
    assert_eq!(
        local::read_ident_public_key(&paths, "alice").unwrap(),
        old_public,
        "an uncommitted rotation must leave the ident key alone"
    );
    assert!(
        !paths.ident_pending_private_key_path("alice").exists(),
        "a disproven rotation must clear its staging"
    );
}

/// The registry cannot be consulted at all: the staged key might be the
/// account authority, so it is kept and the error says where it is.
#[test]
fn rotate_ident_keeps_the_staging_when_the_registry_cannot_be_consulted() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let old_public = seed_ident(&paths, "alice");

    let stub = rotate_reconcile_stub(registry.ident_body(), |_staged| {
        (503, "{\"error\":\"chain unavailable\"}".to_string())
    });

    let err = rotate_ident(&stub.url, &paths, "alice").unwrap_err();
    assert!(err.contains("registry is unavailable"), "{err}");
    assert!(err.contains("staged ident key was left at"), "{err}");
    assert!(
        paths.ident_pending_private_key_path("alice").exists(),
        "a possibly-authoritative key must never be deleted on doubt"
    );
    assert_eq!(
        local::read_ident_public_key(&paths, "alice").unwrap(),
        old_public
    );
}

// --- machine revocation ----------------------------------------------

/// Revocation authority is the ident key alone: the request signs the
/// server's challenge together with the fingerprint being revoked.
#[test]
fn revoke_machine_signs_the_challenge_with_the_ident_key() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let ident_public = seed_ident(&paths, "alice");

    let nonce = vec![3u8; 32];
    let stub = registry
        .routes()
        .ok(
            "/machines/revoke/challenge",
            serde_json::json!({
                "challengeId": "rev-1",
                "nonce": crypto::encode_bytes(&nonce),
                "expiresAt": 1,
            })
            .to_string(),
        )
        .ok(
            "/machines/revoke",
            serde_json::json!({
                "owner": "alice",
                "authFingerprint": "lost-machine",
                "revoked": true,
            })
            .to_string(),
        )
        .serve();

    let response = revoke_machine(&stub.url, &paths, "alice", "lost-machine").expect("revocation");
    assert!(response.revoked);

    let request = stub.request_to("/machines/revoke");
    assert_eq!(field(&request, "challengeId"), "rev-1");
    assert_signed(
        &request,
        "identSignature",
        &ident_public,
        &crypto::revocation_message("rev-1", &nonce, "lost-machine"),
    );
    // No session token is required for a revocation.
    assert!(json_body(&request).get("sessionToken").is_none());
}

// --- org / token / transfer / release-state ---------------------------

/// The grant and the removal sign *different* messages: a removal signs the
/// literal role `removed`, so a captured grant signature cannot be replayed
/// as a removal.
#[test]
fn set_org_member_signs_grant_and_removal_distinctly() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let ident_public = seed_ident(&paths, "alice");
    let body = serde_json::json!({"org": "acme", "member": "bob", "role": "admin"}).to_string();

    let grant = registry.routes().ok("/orgs/members", body.clone()).serve();
    let response =
        set_org_member(&grant.url, &paths, "acme", "alice", "bob", "admin", false).expect("grant");
    assert_eq!(response.member, "bob");
    let request = grant.request_to("/orgs/members");
    assert_eq!(field(&request, "action"), "grant");
    assert_eq!(field(&request, "sessionToken"), "session-jwt");
    assert_signed(
        &request,
        "identSignature",
        &ident_public,
        &crypto::org_role_message("acme", "bob", "admin"),
    );

    let remove = registry.routes().ok("/orgs/members", body).serve();
    set_org_member(&remove.url, &paths, "acme", "alice", "bob", "admin", true).expect("remove");
    let request = remove.request_to("/orgs/members");
    assert_eq!(field(&request, "action"), "remove");
    assert_eq!(
        field(&request, "role"),
        "admin",
        "the wire request still names the role"
    );
    assert_signed(
        &request,
        "identSignature",
        &ident_public,
        &crypto::org_role_message("acme", "bob", "removed"),
    );
}

/// The token keypair is generated locally: only its public half and a
/// proof of possession leave the machine, and the private half is returned
/// to the operator.
#[test]
fn issue_publish_token_keeps_the_private_half_local() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let ident_public = seed_ident(&paths, "alice");

    let stub = registry
        .routes()
        .ok(
            "/tokens",
            serde_json::json!({
                "owner": "alice",
                "tokenFingerprint": "tfp",
                "scope": "alice#*",
                "expiresAt": 500,
            })
            .to_string(),
        )
        .serve();

    let (response, token_private) =
        issue_publish_token(&stub.url, &paths, "alice", "alice#*", 3600).expect("issue");
    assert_eq!(response.scope, "alice#*");

    let request = stub.request_to("/tokens");
    let token_public = crypto::decode_bytes(&field(&request, "tokenKey"), "tokenKey").unwrap();
    let private = crypto::decode_bytes(&token_private, "tokenPrivate").unwrap();
    assert_eq!(
        crypto::public_from_private(&private).unwrap(),
        token_public,
        "the returned private half must be the one the registry registered"
    );
    assert!(!body_of(&request).contains(&token_private));
    assert_eq!(
        json_body(&request).get("ttlSeconds").unwrap().as_i64(),
        Some(3600)
    );
    assert_signed(
        &request,
        "proof",
        &token_public,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
    );
    // The issuance itself is authorized by the account ident, over the
    // fingerprint of the key being issued.
    assert_signed(
        &request,
        "identSignature",
        &ident_public,
        &crypto::token_issue_message("alice", &crypto::fingerprint(&token_public), "alice#*"),
    );
}

#[test]
fn revoke_publish_token_signs_the_revocation_with_the_ident_key() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let ident_public = seed_ident(&paths, "alice");
    let stub = registry
        .routes()
        .ok(
            "/tokens/revoke",
            serde_json::json!({"owner": "alice", "tokenFingerprint": "tfp", "revoked": true})
                .to_string(),
        )
        .serve();

    let response = revoke_publish_token(&stub.url, &paths, "alice", "tfp").expect("revoke");
    assert!(response.revoked);
    assert_signed(
        &stub.request_to("/tokens/revoke"),
        "identSignature",
        &ident_public,
        &crypto::token_revoke_message("alice", "tfp"),
    );
}

/// Both halves of a transfer are ident-signed, each over its own message:
/// the v2 terms (plan-168-A) read from the verified `/index` — the next
/// sequence, the current owner and their bound key, the recipient's
/// registered name, and the digest of every served release.
#[test]
fn transfer_offer_and_accept_are_each_ident_signed() {
    let registry = Registry::new();
    let body =
        serde_json::json!({"ident": "alice#pkg", "toOwner": "bob", "accepted": false}).to_string();
    let bob_chain = serde_json::json!({
        "owner": "bob", "identKey": "", "identFingerprint": "", "chain": [],
    })
    .to_string();

    let (_temp, paths) = temp_paths();
    let alice_ident = seed_ident(&paths, "alice");
    let alice_key = format!("ed25519:{}", crypto::encode_bytes(&alice_ident));
    let alice_fingerprint = crypto::fingerprint(&alice_ident);
    let index = index_body(&registry, &alice_key, &alice_fingerprint);
    let digest = crypto::release_digest([("1.0.0", "h")]);
    let terms = crypto::TransferTermsRef {
        ident: "alice#pkg",
        sequence: 0,
        from_owner: "alice",
        from_ident_fingerprint: &alice_fingerprint,
        to_owner: "bob",
        release_digest: &digest,
    };
    let offer = registry
        .routes()
        .ok("/index/", index.clone())
        .ok("/idents/bob", bob_chain.clone())
        .ok("/packages/transfer/offer", body.clone())
        .serve();
    let response = transfer_offer(&offer.url, &paths, "alice#pkg", "bob").expect("offer");
    assert_eq!(response.to_owner, "bob");
    let request = offer.request_to("/packages/transfer/offer");
    assert_signed(
        &request,
        "identSignature",
        &alice_ident,
        &crypto::transfer_offer_message_v2(terms),
    );
    assert_eq!(field(&request, "fromOwner"), "alice");
    assert_eq!(field(&request, "releaseDigest"), digest);

    let (_temp2, paths2) = temp_paths();
    let bob_ident = seed_ident(&paths2, "bob");
    let accept = registry
        .routes()
        .ok("/index/", index)
        .ok("/idents/bob", bob_chain)
        .ok(
            "/packages/transfer/accept",
            serde_json::json!({"ident": "alice#pkg", "toOwner": "bob", "accepted": true})
                .to_string(),
        )
        .serve();
    let response = transfer_accept(&accept.url, &paths2, "alice#pkg", "bob").expect("accept");
    assert!(response.accepted);
    assert_signed(
        &accept.request_to("/packages/transfer/accept"),
        "identSignature",
        &bob_ident,
        &crypto::transfer_accept_message_v2(terms),
    );

    // A machine whose local alice key is not the bound current ident cannot
    // offer, and is told whose ident it lacks.
    let (_temp3, paths3) = temp_paths();
    seed_ident(&paths3, "alice");
    let err = transfer_offer(&offer.url, &paths3, "alice#pkg", "bob").unwrap_err();
    assert!(err.contains("does not hold alice's current ident"), "{err}");
}

/// A release-state change carries both credentials: the session (auth) and
/// an ident signature (authority) over the exact state being set.
#[test]
fn set_release_state_carries_both_session_and_ident_signature() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let ident_public = seed_ident(&paths, "alice");
    let stub = registry
        .routes()
        .ok(
            "/release-state",
            serde_json::json!({
                "ident": "alice#pkg",
                "version": "1.0.0",
                "state": "yanked",
                "logEntry": {"index": 4, "leafHash": "aa"},
            })
            .to_string(),
        )
        .serve();

    let response = set_release_state(&stub.url, &paths, "alice", "alice#pkg", "1.0.0", "yanked")
        .expect("state change");
    assert_eq!(response.state, "yanked");
    assert_eq!(response.log_entry.index, 4);

    let request = stub.request_to("/release-state");
    assert_eq!(field(&request, "sessionToken"), "session-jwt");
    assert_signed(
        &request,
        "identSignature",
        &ident_public,
        &crypto::release_state_message("alice#pkg", "1.0.0", "yanked"),
    );
}

/// The index's `identKey` is only pinnable because the name binding is
/// verified under the pinned server key and cross-checked against the
/// fingerprint it claims.
#[test]
fn fetch_index_verifies_the_name_binding_before_returning_the_ident_key() {
    let registry = Registry::new();
    let (ident_public, _) = crypto::generate_keypair();
    let fingerprint = crypto::fingerprint(&ident_public);

    // Happy path, with the metadata-form `ed25519:` prefix.
    let (_temp, paths) = temp_paths();
    let stub = registry
        .routes()
        .ok(
            "/index/",
            index_body(
                &registry,
                &format!("ed25519:{}", crypto::encode_bytes(&ident_public)),
                &fingerprint,
            ),
        )
        .serve();
    let response = fetch_index(&stub.url, &paths, "alice", "pkg").expect("index verifies");
    assert_eq!(response.ident, "alice#pkg");
    assert_eq!(response.versions.len(), 1);
    assert_eq!(
        target_of(&stub.request_to("/index/")),
        "/index/alice%23pkg",
        "the ident must be percent-encoded into the path"
    );

    // A key that is not the one the claimed fingerprint names. (Bare
    // base64url form, i.e. no `ed25519:` prefix.)
    let (_temp2, paths2) = temp_paths();
    let (other_public, _) = crypto::generate_keypair();
    let stub2 = registry
        .routes()
        .ok(
            "/index/",
            index_body(
                &registry,
                &crypto::encode_bytes(&other_public),
                &fingerprint,
            ),
        )
        .serve();
    let err = fetch_index(&stub2.url, &paths2, "alice", "pkg").unwrap_err();
    assert_eq!(
        err,
        "registry index identKey does not match its fingerprint"
    );

    // A consistent key/fingerprint pair, but the binding is not signed by
    // the pinned server key.
    let (_temp3, paths3) = temp_paths();
    let impostor = Registry::new();
    let mut body: serde_json::Value = serde_json::from_str(&index_body(
        &impostor,
        &crypto::encode_bytes(&ident_public),
        &fingerprint,
    ))
    .unwrap();
    body["serverFingerprint"] = serde_json::json!(crypto::fingerprint(&registry.public));
    let stub3 = registry.routes().ok("/index/", body.to_string()).serve();
    let err = fetch_index(&stub3.url, &paths3, "alice", "pkg").unwrap_err();
    assert_eq!(
        err,
        "registry name binding does not verify under the registry server key"
    );
}

/// bug-581: a signature over values the RESPONSE supplies cannot bind the
/// response to the REQUEST.
///
/// `fetch_index` verified `name_binding_message(response.owner,
/// response.ident_fingerprint)` and nothing else, so a registry holding its
/// online server key could answer any `/index/<ident>` with a validly signed
/// binding and version list for a package of its choosing. The client
/// returned it as the requested index, and `mfb pkg add` would pin that
/// ident key as the anchor for a dependency it never asked for.
///
/// All three substitutions below are *cryptographically valid*. Each fails
/// only on the route binding, and each defeats a different half-fix.
#[test]
fn fetch_index_rejects_a_validly_signed_response_for_another_ident() {
    let registry = Registry::new();
    let (ident_public, _) = crypto::generate_keypair();
    let ident_key = format!("ed25519:{}", crypto::encode_bytes(&ident_public));
    let fingerprint = crypto::fingerprint(&ident_public);

    let serve = |owner: &str, ident: &str| {
        registry
            .routes()
            .ok(
                "/index/",
                index_body_for(&registry, owner, ident, &ident_key, &fingerprint),
            )
            .serve()
    };

    // A wholly different package, under a different owner.
    let (_t1, p1) = temp_paths();
    let stub = serve("mallory", "mallory#other");
    let err = fetch_index(&stub.url, &p1, "alice", "pkg").unwrap_err();
    assert!(
        err.contains("different package"),
        "the response ident must be checked against the route, got: {err}"
    );

    // Same owner, different package. An owner-only check would accept this.
    let (_t2, p2) = temp_paths();
    let stub = serve("alice", "alice#other");
    let err = fetch_index(&stub.url, &p2, "alice", "pkg").unwrap_err();
    assert!(err.contains("different package"), "{err}");

    // The right ident echoed back, but signed for — and keyed to — a
    // different owner. An ident-only check would accept this, and it is the
    // attack in its purest form: the name binding verifies perfectly,
    // because it is a binding of the response to itself.
    let (_t3, p3) = temp_paths();
    let stub = serve("mallory", "alice#pkg");
    let err = fetch_index(&stub.url, &p3, "alice", "pkg").unwrap_err();
    assert!(
        err.contains("different owner"),
        "the signed owner must be checked against the route, got: {err}"
    );
}

/// POSITIVE (bug-581): the route binding must not refuse honest answers.
///
/// The registry resolves owners case-folded (`owner_folded`) and answers
/// with `owner_display`, so `Alice#pkg` is a legitimate request whose honest
/// response says `owner: "alice"`. An exact owner comparison would have
/// broken every mixed-case `mfb pkg add` — the precise shape of guard that
/// has shipped and rejected valid input four times in this repo.
#[test]
fn fetch_index_accepts_an_honest_response_whose_owner_case_differs() {
    let registry = Registry::new();
    let (ident_public, _) = crypto::generate_keypair();
    let ident_key = format!("ed25519:{}", crypto::encode_bytes(&ident_public));
    let fingerprint = crypto::fingerprint(&ident_public);

    // Requested `Alice#pkg`; the registry echoes the ident verbatim and
    // answers with the registered display form of the owner.
    let (_temp, paths) = temp_paths();
    let stub = registry
        .routes()
        .ok(
            "/index/",
            index_body_for(&registry, "alice", "Alice#pkg", &ident_key, &fingerprint),
        )
        .serve();
    let response =
        fetch_index(&stub.url, &paths, "Alice", "pkg").expect("a case-folded owner is honest");
    assert_eq!(response.owner, "alice");
    assert_eq!(response.versions.len(), 1);
}
