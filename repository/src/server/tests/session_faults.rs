//! The session cross-checks of the authenticated account and governance
//! routes (`server/accounts.rs`, `server/governance.rs`), and the server's own
//! plumbing (`server/mod.rs`, `server/limits.rs`): a validly signed session
//! whose key was retired, or whose claims name another account than its key,
//! is refused by every route; a publisher with no current ident key cannot
//! attest or change a release; the accept loop reports a port it cannot bind;
//! the background reaper sweeps; and a poisoned rate-limiter lock recovers.

use super::*;
use crate::store::tests::{refuse_writes, tamper};

const NOT_CURRENT: &str = "session key is not a current auth key";
const WRONG_OWNER: &str = "session owner does not match requested owner";
const NO_MATCH: &str = "session key does not match current auth key";

/// alice's live session, plus a second, validly signed session whose subject
/// and key are alice's but whose owner id is bob's.
struct Sessions {
    h: Harness,
    alice: TestOwnerKeys,
    token: String,
    forged: String,
}

fn sessions() -> Sessions {
    let h = harness();
    let alice = register_owner_with_all_keys(&h.store, "alice");
    let token = open_session(&h.store, "alice", &alice.auth_private);
    register_owner_with_all_keys(&h.store, "bob");
    let (alice_owner, alice_key) = h.store.owner_with_auth_key("alice").unwrap().unwrap();
    let bob_id = h.store.owner_with_ident_key("bob").unwrap().unwrap().0.id;
    let now = now_unix();
    let forged = session_with_claims(
        &h.store,
        alice_owner.id,
        alice_key.id,
        SessionClaims {
            sub: "alice".to_string(),
            owner_id: bob_id,
            auth_fingerprint: alice_key.fingerprint.clone(),
            iat: now,
            exp: now + 600,
            jti: Uuid::new_v4().to_string(),
            iss: SESSION_TOKEN_ISSUER.to_string(),
            aud: SESSION_TOKEN_AUDIENCE.to_string(),
        },
    );
    Sessions {
        h,
        alice,
        token,
        forged,
    }
}

fn retire_auth_keys(h: &Harness) {
    tamper(
        &h.store,
        "UPDATE keys SET status = 'past' WHERE role = 'auth'",
    );
}

fn retire_ident_keys(h: &Harness) {
    tamper(
        &h.store,
        "UPDATE keys SET status = 'past' WHERE role = 'ident'",
    );
}

async fn rotate(h: &Harness, token: &str) -> (StatusCode, String) {
    err_of(
        rotate_ident(
            State(h.state.clone()),
            Json(RotateRequest {
                owner: "alice".to_string(),
                new_ident_key: String::new(),
                chain_signature: String::new(),
                possession_proof: String::new(),
                session_token: token.to_string(),
            }),
        )
        .await,
    )
}

fn link_request(token: &str, approval_key: &[u8]) -> LinkStartRequest {
    LinkStartRequest {
        owner: "alice".to_string(),
        lookup: "a".repeat(64),
        blob: crypto::encode_bytes(b"ciphertext"),
        salt: crypto::encode_bytes(b"salt"),
        approval_key: crypto::encode_bytes(approval_key),
        session_token: token.to_string(),
    }
}

async fn link(h: &Harness, request: LinkStartRequest) -> (StatusCode, String) {
    err_of(link_start(State(h.state.clone()), Json(request)).await)
}

async fn attest(h: &Harness, token: &str) -> (StatusCode, String) {
    err_of(
        signing(
            State(h.state.clone()),
            peer("127.0.0.1"),
            Json(SigningRequest {
                owner: "alice".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
                signing_fingerprint: "a".repeat(64),
                session_token: token.to_string(),
            }),
        )
        .await,
    )
}

async fn set_state(h: &Harness, token: &str) -> (StatusCode, String) {
    err_of(
        release_state(
            State(h.state.clone()),
            Json(ReleaseStateRequest {
                owner: "alice".to_string(),
                ident: "alice#toolbox".to_string(),
                version: "1.0.0".to_string(),
                state: "yanked".to_string(),
                session_token: token.to_string(),
                ident_signature: String::new(),
            }),
        )
        .await,
    )
}

fn bad(message: &str) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, message.to_string())
}

#[tokio::test]
async fn a_session_whose_claims_name_another_account_is_refused_by_every_route() {
    let s = sessions();
    assert_eq!(rotate(&s.h, &s.forged).await, bad(WRONG_OWNER));
    assert_eq!(
        link(&s.h, link_request(&s.forged, &[0; 32])).await,
        bad(WRONG_OWNER)
    );
    assert_eq!(attest(&s.h, &s.forged).await, bad(NO_MATCH));
    assert_eq!(set_state(&s.h, &s.forged).await, bad(WRONG_OWNER));
}

#[tokio::test]
async fn a_session_whose_key_was_retired_is_refused_by_every_route() {
    let s = sessions();
    retire_auth_keys(&s.h);
    assert_eq!(rotate(&s.h, &s.token).await, bad(NOT_CURRENT));
    assert_eq!(
        link(&s.h, link_request(&s.token, &[0; 32])).await,
        bad(NOT_CURRENT)
    );
    assert_eq!(attest(&s.h, &s.token).await, bad(NO_MATCH));
    assert_eq!(set_state(&s.h, &s.token).await, bad(NOT_CURRENT));
}

#[tokio::test]
async fn a_pairing_approval_key_that_is_not_an_ed25519_public_key_is_refused() {
    let s = sessions();
    assert_eq!(
        link(&s.h, link_request(&s.token, &[0; 16])).await,
        bad("malformed pairing approval key")
    );
}

#[tokio::test]
async fn a_publisher_with_no_current_ident_key_cannot_attest_or_change_a_release() {
    let s = sessions();
    publish_valid_package(&s.h.state, &s.alice, &s.token, "1.0.0").await;
    retire_ident_keys(&s.h);
    assert_eq!(
        attest(&s.h, &s.token).await,
        bad("owner has no current ident key")
    );
    assert_eq!(
        set_state(&s.h, &s.token).await,
        bad("owner has no current ident key")
    );
}

#[tokio::test]
async fn a_revocation_the_store_cannot_write_is_a_server_error_and_revokes_nothing() {
    let s = sessions();
    // A second machine, so the revocation is not refused as the last key.
    let (second_public, second_private) = crypto::generate_keypair();
    let proof = crypto::sign(
        &second_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &second_public),
    )
    .unwrap();
    s.h.store
        .add_auth_key("alice", &second_public, &proof)
        .unwrap();
    let fingerprint = crypto::fingerprint(&second_public);
    let challenge = revoke_challenge(
        State(s.h.state.clone()),
        Json(RevokeChallengeRequest {
            owner: "alice".to_string(),
        }),
    )
    .await
    .unwrap()
    .0;
    let nonce = crypto::decode_bytes(&challenge.nonce, "nonce").unwrap();
    let signature = crypto::sign(
        &s.alice.ident_private,
        &crypto::revocation_message(&challenge.challenge_id, &nonce, &fingerprint),
    )
    .unwrap();
    refuse_writes(&s.h.store, "keys", "UPDATE");
    assert_eq!(
        err_of(
            revoke_machine(
                State(s.h.state.clone()),
                Json(RevokeRequest {
                    challenge_id: challenge.challenge_id,
                    auth_fingerprint: fingerprint.clone(),
                    ident_signature: crypto::encode_bytes(&signature),
                }),
            )
            .await
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to revoke auth key: injected UPDATE failure".to_string()
        )
    );
    assert!(s
        .h
        .store
        .owner_auth_key_by_fingerprint("alice", &fingerprint)
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn an_offer_of_an_unknown_package_is_refused_by_the_store() {
    let s = sessions();
    assert_eq!(
        err_of(
            transfer_offer(
                State(s.h.state.clone()),
                Json(TransferOfferRequest {
                    ident: "alice#nothing".to_string(),
                    from_owner: "alice".to_string(),
                    to_owner: "bob".to_string(),
                    sequence: 0,
                    release_digest: String::new(),
                    session_token: s.token.clone(),
                    ident_signature: crypto::encode_bytes(&[0; 64]),
                }),
            )
            .await
        ),
        bad("unknown package")
    );
}

#[tokio::test]
async fn serve_reports_an_address_it_cannot_bind() {
    let h = harness();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap();
    let err = serve(
        h.store.clone(),
        BlobStore::local(h.packages_dir.clone()),
        addr,
    )
    .await
    .unwrap_err();
    assert!(
        err.starts_with(&format!("failed to bind {addr}: ")),
        "{err}"
    );
}

/// The reaper `serve` spawns sweeps once a minute: with the clock paused, a
/// minute passes instantly and an expired challenge is gone (not merely
/// refused as expired).
#[tokio::test(start_paused = true)]
async fn the_background_reaper_sweeps_expired_challenges_every_minute() {
    let h = harness();
    register_owner_with_all_keys(&h.store, "alice");
    let challenge = h.store.create_challenge("alice").unwrap();
    h.store.force_expire_challenge(&challenge.id).unwrap();
    assert_eq!(
        h.store
            .complete_challenge(&challenge.id, &[0; 64])
            .unwrap_err(),
        "expired challenge"
    );
    // The reaper is spawned before the bind, so it runs even though this
    // `serve` fails to bind.
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap();
    assert!(serve(
        h.store.clone(),
        BlobStore::local(h.packages_dir.clone()),
        addr
    )
    .await
    .is_err());
    tokio::time::sleep(std::time::Duration::from_secs(61)).await;
    assert_eq!(
        h.store
            .complete_challenge(&challenge.id, &[0; 64])
            .unwrap_err(),
        "unknown challenge"
    );
}

/// bug-264 / REPO-09: a panic while the rate limiter's lock is held must not
/// wedge rate limiting for the whole process.
#[test]
fn a_poisoned_rate_limiter_keeps_counting_and_pruning() {
    let limiter = RateLimiter::new();
    let hits = limiter.hits.clone();
    let poisoner = std::thread::spawn(move || {
        let _guard = hits.lock().unwrap();
        panic!("a request panicked while holding the rate limiter");
    });
    assert!(poisoner.join().is_err());
    assert!(limiter.hits.is_poisoned());
    assert!(limiter.allow("k", 1, 60));
    assert!(!limiter.allow("k", 1, 60), "the cap still holds");
    limiter.prune(0);
    assert!(limiter.allow("k", 1, 60), "pruned windows start over");
}
