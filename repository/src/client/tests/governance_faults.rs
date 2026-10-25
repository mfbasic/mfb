//! The registry client's governance calls (`client/governance.rs`) refusing
//! before they act: a damaged local ident key signs nothing and sends nothing,
//! a server name binding that does not hold is refused by name, a transfer is
//! offered by the package's CURRENT (last-transfer) owner, and a server error
//! on token issuance is surfaced.

use super::*;
use crate::server::{IndexResponse, OwnerBinding, TransferLink};

/// Provision `owner` with an ident private key that decodes but is 16 bytes,
/// not an Ed25519 key: what a truncated key file holds.
fn seed_damaged_ident(paths: &LocalPaths, owner: &str) {
    let (public, _private) = crypto::generate_keypair();
    local::write_ident_keypair(paths, owner, &public, &[7u8; 16]).unwrap();
    local::write_session(paths, owner, "session-jwt").unwrap();
}

const DAMAGED: &str = "malformed local private key";

fn sent(stub: &Stub, path: &str) -> bool {
    stub.requests
        .lock()
        .unwrap()
        .iter()
        .any(|request| path_of(request) == path)
}

#[test]
fn a_damaged_local_ident_key_signs_and_sends_nothing() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    seed_damaged_ident(&paths, "alice");
    let stub = registry.routes().serve();

    assert_eq!(
        set_org_member(&stub.url, &paths, "acme", "alice", "bob", "admin", false).unwrap_err(),
        DAMAGED
    );
    assert_eq!(
        issue_publish_token(&stub.url, &paths, "alice", "alice#*", 60).unwrap_err(),
        DAMAGED
    );
    assert_eq!(
        revoke_publish_token(&stub.url, &paths, "alice", "tfp").unwrap_err(),
        DAMAGED
    );
    assert_eq!(
        set_release_state(&stub.url, &paths, "alice", "alice#pkg", "1.0.0", "yanked").unwrap_err(),
        DAMAGED
    );
    for path in [
        "/orgs/members",
        "/tokens",
        "/tokens/revoke",
        "/release-state",
    ] {
        assert!(!sent(&stub, path), "{path} must not be sent");
    }
}

#[test]
fn an_accept_signed_with_a_damaged_local_ident_key_is_not_sent() {
    let registry = Registry::new();
    let (_temp, alice_paths) = temp_paths();
    let alice_ident = seed_ident(&alice_paths, "alice");
    let index = index_body(
        &registry,
        &format!("ed25519:{}", crypto::encode_bytes(&alice_ident)),
        &crypto::fingerprint(&alice_ident),
    );
    let stub = registry
        .routes()
        .ok("/index/", index)
        .ok(
            "/idents/bob",
            serde_json::json!({"owner": "bob", "identKey": "", "identFingerprint": "", "chain": []})
                .to_string(),
        )
        .serve();
    let (_temp, bob_paths) = temp_paths();
    seed_damaged_ident(&bob_paths, "bob");
    assert_eq!(
        transfer_accept(&stub.url, &bob_paths, "alice#pkg", "bob").unwrap_err(),
        DAMAGED
    );
    assert!(!sent(&stub, "/packages/transfer/accept"));
}

#[test]
fn a_transfer_of_an_ident_with_no_owner_part_is_refused() {
    let (_temp, paths) = temp_paths();
    assert_eq!(
        transfer_offer("http://127.0.0.1:1", &paths, "alicepkg", "bob").unwrap_err(),
        "ident must use <owner>#<package>"
    );
}

#[test]
fn a_token_issuance_the_registry_refuses_is_reported() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    seed_ident(&paths, "alice");
    let stub = registry
        .routes()
        .reply(
            "/tokens",
            403,
            serde_json::json!({"error": "token scope not permitted"}).to_string(),
        )
        .serve();
    let err = issue_publish_token(&stub.url, &paths, "alice", "bob#*", 60).unwrap_err();
    assert!(err.contains("token scope not permitted"), "{err}");
}

/// A binding as the registry signs it, for `owner` holding `public`.
fn binding(registry: &Registry, owner: &str, public: &[u8]) -> OwnerBinding {
    let fingerprint = crypto::fingerprint(public);
    let signature = crypto::sign(
        &registry.private,
        &crypto::name_binding_message(owner, &fingerprint),
    )
    .unwrap();
    OwnerBinding {
        owner: owner.to_string(),
        ident_key: format!("ed25519:{}", crypto::encode_bytes(public)),
        ident_fingerprint: fingerprint,
        name_binding_signature: crypto::encode_bytes(&signature),
    }
}

#[test]
fn a_name_binding_that_does_not_hold_is_refused_by_name() {
    let registry = Registry::new();
    let (public, _private) = crypto::generate_keypair();
    let good = binding(&registry, "alice", &public);
    assert_eq!(
        verify_owner_binding(&registry.public, &good).unwrap(),
        public
    );

    let undecodable = OwnerBinding {
        ident_key: "ed25519:not base64!".to_string(),
        ..good.clone()
    };
    assert_eq!(
        verify_owner_binding(&registry.public, &undecodable).unwrap_err(),
        "malformed identKey"
    );

    let (other, _private) = crypto::generate_keypair();
    let mismatched = OwnerBinding {
        ident_key: format!("ed25519:{}", crypto::encode_bytes(&other)),
        ..good.clone()
    };
    assert_eq!(
        verify_owner_binding(&registry.public, &mismatched).unwrap_err(),
        "registry binding for alice names a key that does not match its fingerprint"
    );

    let impostor = Registry::new();
    assert_eq!(
        verify_owner_binding(&impostor.public, &good).unwrap_err(),
        "registry binding for alice does not verify under the registry server key"
    );
}

/// After alice handed the package to bob, the offer is bob's to make: the
/// client reads the current owner from the last transfer's binding and signs
/// the next terms (sequence 1) with bob's key.
#[test]
fn a_transferred_package_is_offered_by_its_last_recipient() {
    let registry = Registry::new();
    let (alice_public, _alice_private) = crypto::generate_keypair();
    let (_temp, paths) = temp_paths();
    let bob_ident = seed_ident(&paths, "bob");
    let mut index: IndexResponse = serde_json::from_str(&index_body(
        &registry,
        &format!("ed25519:{}", crypto::encode_bytes(&alice_public)),
        &crypto::fingerprint(&alice_public),
    ))
    .unwrap();
    index.transfers.push(TransferLink {
        sequence: 0,
        from_owner: "alice".to_string(),
        from_ident_key: None,
        to_owner: "bob".to_string(),
        to_ident_key: None,
        release_digest: None,
        offer_signature: None,
        accept_signature: None,
        offered_at: 0,
        accepted_at: 0,
        to_binding: binding(&registry, "bob", &bob_ident),
    });
    let stub = registry
        .routes()
        .ok("/index/", serde_json::to_string(&index).unwrap())
        .ok(
            "/idents/carol",
            serde_json::json!({"owner": "carol", "identKey": "", "identFingerprint": "", "chain": []})
                .to_string(),
        )
        .ok(
            "/packages/transfer/offer",
            serde_json::json!({"ident": "alice#pkg", "toOwner": "carol", "accepted": false})
                .to_string(),
        )
        .serve();
    transfer_offer(&stub.url, &paths, "alice#pkg", "carol").expect("bob offers");
    let request = stub.request_to("/packages/transfer/offer");
    assert_eq!(field(&request, "fromOwner"), "bob");
    let digest = crypto::release_digest([("1.0.0", "h")]);
    let bob_fingerprint = crypto::fingerprint(&bob_ident);
    assert_signed(
        &request,
        "identSignature",
        &bob_ident,
        &crypto::transfer_offer_message_v2(crypto::TransferTermsRef {
            ident: "alice#pkg",
            sequence: 1,
            from_owner: "bob",
            from_ident_fingerprint: &bob_fingerprint,
            to_owner: "carol",
            release_digest: &digest,
        }),
    );
}
