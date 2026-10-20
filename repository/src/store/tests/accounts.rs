//! plan-176-I: `store::tests` — the accounts tests. The shared store harness and the helpers other\n//! hosts import (`register_keys`, `legacy_transfer`, `drop_tables`) live in `store/tests/mod.rs`.

use super::*;

#[test]
fn startup_creates_database_and_packages_dir() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("meta.db");
    let data_path = temp.path().join("data");
    let opened = Store::open_repository(&db_path, &data_path).unwrap();
    assert!(db_path.is_file());
    assert!(data_path.is_dir());
    opened.store.migrate().unwrap();
}

#[test]
fn registration_persists_owner_and_both_keys() {
    let (_temp, store) = test_store();
    let keys = register_keys(&store, "alice");
    let (owner, auth_key) = store.owner_with_auth_key("alice").unwrap().unwrap();
    assert_eq!(owner.owner_display, "alice");
    assert_eq!(auth_key.public_key, keys.auth_public);
    let (_owner, ident_key) = store.owner_with_ident_key("alice").unwrap().unwrap();
    assert_eq!(ident_key.public_key, keys.ident_public);
    assert_ne!(auth_key.fingerprint, ident_key.fingerprint);
    assert_eq!(store.count_owners().unwrap(), 1);
}

#[test]
fn poisoned_connection_lock_recovers_and_keeps_serving() {
    // bug-264 / REPO-09: a panic while the connection lock is held must not
    // permanently wedge the registry. Poison the mutex, then prove subsequent
    // reads and writes still succeed — the poison is recovered, not propagated
    // as "database lock poisoned" on every following request.
    let (_temp, store) = test_store();
    register_keys(&store, "alice");

    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
        let store = store.clone();
        move || {
            let _guard = store.conn.lock().unwrap();
            panic!("boom while holding the connection lock");
        }
    }));
    assert!(poisoned.is_err());
    assert!(store.conn.is_poisoned());

    // The service continues despite the poisoned lock.
    assert_eq!(store.count_owners().unwrap(), 1);
    register_keys(&store, "bob");
    assert_eq!(store.count_owners().unwrap(), 2);
}

#[test]
fn duplicate_registration_is_case_folded() {
    let (_temp, store) = test_store();
    register(&store, "alice");
    let (auth_public, auth_private) = crypto::generate_keypair();
    let (ident_public, ident_private) = crypto::generate_keypair();
    let auth_proof = crypto::sign(
        &auth_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "Alice", &auth_public),
    )
    .unwrap();
    let ident_proof = crypto::sign(
        &ident_private,
        &crypto::registration_message(crypto::ROLE_IDENT, "Alice", &ident_public),
    )
    .unwrap();
    let err = store
        .register_owner(
            "Alice",
            &auth_public,
            &auth_proof,
            &ident_public,
            &ident_proof,
        )
        .unwrap_err();
    assert!(err.contains("already in use"));
    assert_eq!(store.count_owners().unwrap(), 1);
}

#[test]
fn registration_rejects_bad_proof() {
    let (_temp, store) = test_store();
    let (auth_public, _auth_private) = crypto::generate_keypair();
    let (ident_public, ident_private) = crypto::generate_keypair();
    let (_other_public, other_private) = crypto::generate_keypair();
    let auth_proof = crypto::sign(
        &other_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &auth_public),
    )
    .unwrap();
    let ident_proof = crypto::sign(
        &ident_private,
        &crypto::registration_message(crypto::ROLE_IDENT, "alice", &ident_public),
    )
    .unwrap();
    let err = store
        .register_owner(
            "alice",
            &auth_public,
            &auth_proof,
            &ident_public,
            &ident_proof,
        )
        .unwrap_err();
    assert!(err.contains("invalid auth proof"));
}

#[test]
fn registration_rejects_role_replayed_proofs() {
    // A proof-of-possession signed for one role must not be accepted for
    // the other role, even with the same keypair (plan-23 Phase A1).
    let (_temp, store) = test_store();
    let (auth_public, auth_private) = crypto::generate_keypair();
    let (ident_public, ident_private) = crypto::generate_keypair();
    // Sign the ident proof with the IDENT key but over the AUTH role
    // message: replaying a role-mismatched proof must fail.
    let auth_proof = crypto::sign(
        &auth_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &auth_public),
    )
    .unwrap();
    let replayed_ident_proof = crypto::sign(
        &ident_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &ident_public),
    )
    .unwrap();
    let err = store
        .register_owner(
            "alice",
            &auth_public,
            &auth_proof,
            &ident_public,
            &replayed_ident_proof,
        )
        .unwrap_err();
    assert!(err.contains("invalid ident proof"));

    // And the mirror image: an ident-role proof replayed as the auth proof.
    let replayed_auth_proof = crypto::sign(
        &auth_private,
        &crypto::registration_message(crypto::ROLE_IDENT, "alice", &auth_public),
    )
    .unwrap();
    let ident_proof = crypto::sign(
        &ident_private,
        &crypto::registration_message(crypto::ROLE_IDENT, "alice", &ident_public),
    )
    .unwrap();
    let err = store
        .register_owner(
            "alice",
            &auth_public,
            &replayed_auth_proof,
            &ident_public,
            &ident_proof,
        )
        .unwrap_err();
    assert!(err.contains("invalid auth proof"));
}

#[test]
fn server_keypair_is_created_once_and_stable() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("meta.db");
    let data_path = temp.path().join("data");
    let opened = Store::open_repository(&db_path, &data_path).unwrap();
    let (public, private) = opened.store.server_keypair().unwrap();
    assert_eq!(public.len(), crypto::PUBLIC_KEY_LEN);
    assert_eq!(private.len(), crypto::PRIVATE_KEY_LEN);
    assert_eq!(crypto::public_from_private(&private).unwrap(), public);
    drop(opened);
    // Re-opening the repository must keep the same keypair.
    let reopened = Store::open_repository(&db_path, &data_path).unwrap();
    assert_eq!(reopened.store.server_public_key().unwrap(), public);
}

fn is_missing(fetch: &PairingFetch) -> bool {
    matches!(fetch, PairingFetch::Missing)
}

#[test]
fn pairing_blob_is_single_use_and_expires() {
    let (_temp, store) = test_store();
    register(&store, "alice");
    let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    let lookup = crypto::pairing_lookup("test-code");
    store
        .store_pairing_blob(owner_id, &lookup, b"ciphertext", b"salt", b"approval")
        .unwrap();

    // Wrong owner or wrong lookup yields nothing.
    assert!(is_missing(
        &store.take_pairing_blob("bob", &lookup, approved).unwrap()
    ));
    assert!(is_missing(
        &store
            .take_pairing_blob("alice", &crypto::pairing_lookup("other"), approved)
            .unwrap()
    ));

    // bug-583: a caller that fails the approval predicate is refused AND
    // does not consume the pairing — the honest fetch below still works.
    assert!(matches!(
        store
            .take_pairing_blob("alice", &lookup, |_key| false)
            .unwrap(),
        PairingFetch::Unapproved
    ));

    // First fetch succeeds; the second finds the blob consumed. The stored
    // approval key is exactly what the predicate is handed.
    let PairingFetch::Relayed { blob, salt } = store
        .take_pairing_blob("alice", &lookup, |key| key == b"approval")
        .unwrap()
    else {
        panic!("the approved fetch must relay the blob");
    };
    assert_eq!(blob, b"ciphertext");
    assert_eq!(salt, b"salt");
    assert!(is_missing(
        &store.take_pairing_blob("alice", &lookup, approved).unwrap()
    ));

    // An expired blob is never handed out.
    let expired_lookup = crypto::pairing_lookup("expired-code");
    store
        .store_pairing_blob(
            owner_id,
            &expired_lookup,
            b"ciphertext",
            b"salt",
            b"approval",
        )
        .unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE pairing_blobs SET expires_at = ?1 WHERE lookup = ?2",
            params![now_unix() - 1, expired_lookup],
        )
        .unwrap();
    assert!(is_missing(
        &store
            .take_pairing_blob("alice", &expired_lookup, approved)
            .unwrap()
    ));
}

#[test]
fn linked_machine_key_works_and_revocation_kills_sessions() {
    let (_temp, store) = test_store();
    let keys = register_keys(&store, "alice");

    // A second machine registers its own auth key.
    let (machine_public, machine_private) = crypto::generate_keypair();
    let proof = crypto::sign(
        &machine_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &machine_public),
    )
    .unwrap();
    let (owner, machine_key) = store
        .add_auth_key("alice", &machine_public, &proof)
        .unwrap();
    let machine_fingerprint = machine_key.fingerprint.clone();

    // Both machines' keys resolve by fingerprint; each can open a session.
    let first_fingerprint = crypto::fingerprint(&keys.auth_public);
    assert!(store
        .owner_auth_key_by_fingerprint("alice", &first_fingerprint)
        .unwrap()
        .is_some());
    let challenge = store
        .create_auth_challenge("alice", &machine_fingerprint)
        .unwrap();
    let signature = crypto::sign(
        &machine_private,
        &crypto::challenge_message(&challenge.id, &challenge.nonce),
    )
    .unwrap();
    let (_owner, key) = store.complete_challenge(&challenge.id, &signature).unwrap();
    store
        .insert_session(&NewSession {
            owner_id: owner.id,
            key_id: key.id,
            jwt_id: "machine-session".to_string(),
            issued_at: now_unix(),
            expires_at: now_unix() + 3600,
        })
        .unwrap();
    assert!(store.session_exists("machine-session").unwrap());

    // Revocation flips the key and closes its sessions.
    assert!(store
        .revoke_auth_key(owner.id, &machine_fingerprint)
        .unwrap());
    assert!(store
        .owner_auth_key_by_fingerprint("alice", &machine_fingerprint)
        .unwrap()
        .is_none());
    assert!(!store.session_exists("machine-session").unwrap());
    assert!(store
        .create_auth_challenge("alice", &machine_fingerprint)
        .is_err());
    // Revoking again reports nothing to revoke.
    assert!(!store
        .revoke_auth_key(owner.id, &machine_fingerprint)
        .unwrap());
    // The first machine is untouched.
    assert!(store
        .owner_auth_key_by_fingerprint("alice", &first_fingerprint)
        .unwrap()
        .is_some());

    // bug-493: it is now the account's last current machine key. Revoking
    // it would lock the account out permanently (no ident-authorized route
    // adds an auth key), so the store refuses and the key stays current. A
    // publish token does not count as a machine — it cannot enrol one
    // (bug-492) — so its presence does not unlock the revocation...
    let (token_public, token_private) = crypto::generate_keypair();
    let token_proof = crypto::sign(
        &token_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
    )
    .unwrap();
    store
        .issue_publish_token("alice", &token_public, &token_proof, "alice#toolbox", 3600)
        .unwrap();
    assert_eq!(
        store
            .revoke_auth_key(owner.id, &first_fingerprint)
            .unwrap_err(),
        "cannot revoke the account's last auth key; link another machine first",
    );
    assert!(store
        .owner_auth_key_by_fingerprint("alice", &first_fingerprint)
        .unwrap()
        .is_some());
    // ...while the token itself, not being a machine key, may still be
    // revoked through this path.
    assert!(store
        .revoke_auth_key(owner.id, &crypto::fingerprint(&token_public))
        .unwrap());
}

#[test]
fn revocation_challenge_requires_the_ident_key() {
    let (_temp, store) = test_store();
    let keys = register_keys(&store, "alice");
    let fingerprint = crypto::fingerprint(&keys.auth_public);

    // Signed with the AUTH key (auth session alone): refused.
    let challenge = store.create_ident_challenge("alice").unwrap();
    let nonce = challenge.nonce.clone();
    let bad = crypto::sign(
        &keys.auth_private,
        &crypto::revocation_message(&challenge.id, &nonce, &fingerprint),
    )
    .unwrap();
    assert!(store
        .complete_revocation_challenge(&challenge.id, &bad, &fingerprint)
        .is_err());

    // bug-493: a LOGIN challenge for the auth key, with the revocation
    // message signed by that auth key, is refused on *purpose* — not merely
    // on signature — and the refused attempt does not burn it.
    let login_challenge = store.create_auth_challenge("alice", &fingerprint).unwrap();
    let forged = crypto::sign(
        &keys.auth_private,
        &crypto::revocation_message(&login_challenge.id, &login_challenge.nonce, &fingerprint),
    )
    .unwrap();
    assert_eq!(
        store
            .complete_revocation_challenge(&login_challenge.id, &forged, &fingerprint)
            .unwrap_err(),
        "challenge was not issued for revocation",
    );
    let login = crypto::sign(
        &keys.auth_private,
        &crypto::challenge_message(&login_challenge.id, &login_challenge.nonce),
    )
    .unwrap();
    store
        .complete_challenge(&login_challenge.id, &login)
        .expect("the refused revocation did not burn the login challenge");
    // And the reverse: a revocation (ident) challenge cannot complete a login.
    let ident_challenge = store.create_ident_challenge("alice").unwrap();
    let ident_login = crypto::sign(
        &keys.ident_private,
        &crypto::challenge_message(&ident_challenge.id, &ident_challenge.nonce),
    )
    .unwrap();
    assert_eq!(
        store
            .complete_challenge(&ident_challenge.id, &ident_login)
            .unwrap_err(),
        "challenge was not issued for login",
    );

    // Signed with the ident key over a DIFFERENT fingerprint: refused
    // (the fingerprint is inside the signed bytes).
    let challenge = store.create_ident_challenge("alice").unwrap();
    let nonce = challenge.nonce.clone();
    let redirected = crypto::sign(
        &keys.ident_private,
        &crypto::revocation_message(&challenge.id, &nonce, "someone-else"),
    )
    .unwrap();
    assert!(store
        .complete_revocation_challenge(&challenge.id, &redirected, &fingerprint)
        .is_err());

    // Signed with the ident key over the right fingerprint: accepted.
    let challenge = store.create_ident_challenge("alice").unwrap();
    let nonce = challenge.nonce.clone();
    let good = crypto::sign(
        &keys.ident_private,
        &crypto::revocation_message(&challenge.id, &nonce, &fingerprint),
    )
    .unwrap();
    store
        .complete_revocation_challenge(&challenge.id, &good, &fingerprint)
        .unwrap();
}

#[test]
fn challenge_lifecycle_accepts_signature_once() {
    let (_temp, store) = test_store();
    let (_public, private) = register(&store, "alice");
    let challenge = store.create_challenge("alice").unwrap();
    let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
    let signature = crypto::sign(&private, &message).unwrap();
    let (owner, _key) = store.complete_challenge(&challenge.id, &signature).unwrap();
    assert_eq!(owner.owner_display, "alice");
    let err = store
        .complete_challenge(&challenge.id, &signature)
        .unwrap_err();
    assert!(err.contains("reused challenge"));
}

#[test]
fn challenge_rejects_bad_signature_and_unknown_owner() {
    let (_temp, store) = test_store();
    register(&store, "alice");
    assert!(store
        .create_challenge("bob")
        .unwrap_err()
        .contains("unknown owner"));
    let challenge = store.create_challenge("alice").unwrap();
    let (_public, private) = crypto::generate_keypair();
    let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
    let signature = crypto::sign(&private, &message).unwrap();
    assert!(store.complete_challenge(&challenge.id, &signature).is_err());
}

#[test]
fn challenge_rejects_expired_challenge() {
    let (_temp, store) = test_store();
    let (_public, private) = register(&store, "alice");
    let challenge = store.create_challenge("alice").unwrap();
    store.force_expire_challenge(&challenge.id).unwrap();
    let message = crypto::challenge_message(&challenge.id, &challenge.nonce);
    let signature = crypto::sign(&private, &message).unwrap();
    let err = store
        .complete_challenge(&challenge.id, &signature)
        .unwrap_err();
    assert!(err.contains("expired challenge"));
}

#[test]
fn complete_challenge_rejects_unknown_id() {
    let (_temp, store) = test_store();
    assert!(store
        .complete_challenge("no-such-id", &[0u8; 64])
        .unwrap_err()
        .contains("unknown challenge"));
}

/// bug-586: the metadata database holds the server signing key, the session
/// secret and the online snapshot/timestamp keys in plaintext. SQLite
/// creates it and its `-wal`/`-shm` sidecars with the process umask, so
/// under the ordinary `umask 022` they land at 0644 in a 0755 directory and
/// any other local account can copy the credentials.
///
/// The RED case is made deterministic by pre-creating the directory and an
/// empty database file at exposed modes rather than by setting the umask.
/// `umask` is process-global and libtest runs in parallel, so a test that
/// set it would race every other test that creates a file; pre-setting the
/// modes proves the same defect without that hazard, and additionally
/// covers the upgrade path — an ALREADY deployed volume whose files are
/// exposed today.
#[cfg(unix)]
#[test]
fn open_repository_makes_the_key_bearing_database_private() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;

    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("data");
    let dbpath = dir.join("meta.db");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    // A zero-length file is a valid empty SQLite database, so this is the
    // real "existing world-readable deployment" shape.
    fs::write(&dbpath, b"").unwrap();
    fs::set_permissions(&dbpath, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(mode(&dir), 0o755, "precondition: the directory is exposed");
    assert_eq!(
        mode(&dbpath),
        0o644,
        "precondition: the database is exposed"
    );

    let opened = Store::open_repository(&dbpath, &temp.path().join("blobs")).unwrap();

    assert_eq!(mode(&dir), 0o700, "the metadata directory must be private");
    assert_eq!(mode(&dbpath), 0o600, "the database must be private");
    // The sidecars carry the same rows; protecting only meta.db would leave
    // the secrets readable in the write-ahead log.
    for suffix in ["-wal", "-shm"] {
        let sidecar = dir.join(format!("meta.db{suffix}"));
        assert!(sidecar.exists(), "WAL mode must have created {suffix}");
        assert_eq!(
            mode(&sidecar),
            0o600,
            "the {suffix} sidecar must be private"
        );
    }

    // And the keys really are in there, so the modes above are guarding
    // something: this is what makes the bug HIGH-value rather than cosmetic.
    let (_public, private) = opened.store.server_keypair().unwrap();
    assert!(!private.is_empty());
}

/// POSITIVE (bug-586): hardening must not break an ordinary open, and must
/// never WIDEN an operator's deliberate choice.
///
/// Without this, "refuse every open" and "chmod everything to 0600
/// unconditionally" would both pass the test above. A `0400` database is a
/// legitimate read-only hardening; tightening is allowed to remove access,
/// never to grant it.
#[cfg(unix)]
#[test]
fn open_repository_leaves_an_already_private_database_alone_and_still_works() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;

    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("data");
    let dbpath = dir.join("meta.db");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();

    // A normal open on a fresh path: the store is fully functional.
    let opened = Store::open_repository(&dbpath, &temp.path().join("blobs")).unwrap();
    let store = opened.store;
    let keys = register_keys(&store, "alice");
    let owner_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            owner_id,
            "alice#toolbox",
            "1.0.0",
            "hash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    assert_eq!(
        store.list_package_versions("alice#toolbox").unwrap().len(),
        1
    );
    assert_eq!(mode(&dbpath), 0o600);
    assert_eq!(mode(&dir), 0o700);
    drop(store);

    // An operator who deliberately narrowed further keeps their choice:
    // re-opening must not relax 0400 back up to 0600.
    fs::set_permissions(&dbpath, fs::Permissions::from_mode(0o400)).unwrap();
    let reopened = Store::open_repository(&dbpath, &temp.path().join("blobs"));
    assert!(reopened.is_ok(), "a 0400 database must still open");
    assert_eq!(
        mode(&dbpath),
        0o400,
        "hardening may remove access, never grant it"
    );
    let _ = keys;
}

/// bug-580: an account's current ident key and every current auth key must
/// be DIFFERENT keys.
///
/// **This was never exploitable, and the report's premise was wrong.**
/// Measured on the pre-fix store, all five paths already refused a
/// role-colliding key — with `UNIQUE constraint failed: keys.fingerprint`.
/// `keys.fingerprint` is declared `NOT NULL UNIQUE` *globally*, so no two
/// rows anywhere can share a public key and the separation fell out of that.
///
/// What this test protects is that the guarantee stops being INCIDENTAL. It
/// came from an index whose purpose is not role separation, and bug-580's own
/// non-goals ask for that index to be loosened ("do not prohibit two
/// different accounts from independently choosing the same public key" — which
/// the global UNIQUE currently does prohibit). Whoever loosens it would remove
/// the role separation as a side effect, with nothing failing. Now
/// `insert_key_tx` enforces the account-scoped invariant directly, so the two
/// properties can move independently.
///
/// Every path is exercised because the point is that the list is complete.
/// The report named two; there are five, and the two it missed include
/// `issue_publish_token` — the one that matters most, since a publish token
/// is a delegated, exportable credential handed to CI.
#[test]
fn key_role_separation_is_enforced_at_every_creation_path() {
    let (_temp, store) = test_store();
    let (shared_public, shared_private) = crypto::generate_keypair();
    let proof = |role: &str, owner: &str, public: &[u8], private: &[u8]| {
        crypto::sign(private, &crypto::registration_message(role, owner, public)).unwrap()
    };

    // 1. Registration with one key in both roles.
    let err = store
        .register_owner(
            "alice",
            &shared_public,
            &proof(crypto::ROLE_AUTH, "alice", &shared_public, &shared_private),
            &shared_public,
            &proof(crypto::ROLE_IDENT, "alice", &shared_public, &shared_private),
        )
        .unwrap_err();
    assert!(err.contains("key role separation"), "{err}");
    // Nothing was persisted: the transaction rolled back whole.
    assert!(store.owner_with_ident_key("alice").unwrap().is_none());

    // A proper account, to test the remaining paths against.
    let keys = register_keys(&store, "alice");

    // 2. Linking a machine whose auth key IS the account ident key.
    let err = store
        .add_auth_key(
            "alice",
            &keys.ident_public,
            &proof(
                crypto::ROLE_AUTH,
                "alice",
                &keys.ident_public,
                &keys.ident_private,
            ),
        )
        .unwrap_err();
    assert!(err.contains("key role separation"), "{err}");

    // 3. A publish token minted on the ident key. The report missed this
    //    one, and it is the most dangerous: the token leaves the machine.
    let err = store
        .issue_publish_token(
            "alice",
            &keys.ident_public,
            &proof(
                crypto::ROLE_AUTH,
                "alice",
                &keys.ident_public,
                &keys.ident_private,
            ),
            "publish",
            3600,
        )
        .unwrap_err();
    assert!(err.contains("key role separation"), "{err}");

    // 4. Rotating the ident ONTO an existing machine auth key.
    let chain = crypto::sign(
        &keys.ident_private,
        &crypto::ident_rotation_message(
            "alice",
            &crypto::fingerprint(&keys.ident_public),
            &keys.auth_public,
        ),
    )
    .unwrap();
    let err = store
        .rotate_ident(
            "alice",
            &keys.auth_public,
            &chain,
            &proof(
                crypto::ROLE_IDENT,
                "alice",
                &keys.auth_public,
                &keys.auth_private,
            ),
        )
        .unwrap_err();
    assert!(err.contains("key role separation"), "{err}");

    // 5. Re-anchoring the ident onto an existing machine auth key.
    let err = store
        .reanchor_ident("alice", &keys.auth_public)
        .unwrap_err();
    assert!(err.contains("key role separation"), "{err}");

    // The account is intact after five refusals: each rolled back whole.
    let (_owner, ident) = store.owner_with_ident_key("alice").unwrap().unwrap();
    assert_eq!(ident.public_key, keys.ident_public);
}

/// POSITIVE (bug-580): distinct keys must keep working everywhere.
///
/// Without this, "reject every key" passes the test above. It pins the
/// normal shapes the invariant must NOT disturb — several machines, a
/// publish token, a rotation, and (the bug's explicit non-goal) two
/// different accounts independently choosing the same public key.
#[test]
fn distinct_keys_still_register_link_tokenize_and_rotate() {
    let (_temp, store) = test_store();
    let alice = register_keys(&store, "alice");
    let proof = |role: &str, owner: &str, public: &[u8], private: &[u8]| {
        crypto::sign(private, &crypto::registration_message(role, owner, public)).unwrap()
    };

    // Several distinct machines link to one account.
    for _ in 0..3 {
        let (machine_public, machine_private) = crypto::generate_keypair();
        store
            .add_auth_key(
                "alice",
                &machine_public,
                &proof(
                    crypto::ROLE_AUTH,
                    "alice",
                    &machine_public,
                    &machine_private,
                ),
            )
            .expect("a distinct machine key still links");
    }

    // A publish token on its own fresh key.
    let (token_public, token_private) = crypto::generate_keypair();
    store
        .issue_publish_token(
            "alice",
            &token_public,
            &proof(crypto::ROLE_AUTH, "alice", &token_public, &token_private),
            "publish",
            3600,
        )
        .expect("a distinct token key still issues");

    // A rotation onto a fresh ident.
    let (next_public, next_private) = crypto::generate_keypair();
    let chain = crypto::sign(
        &alice.ident_private,
        &crypto::ident_rotation_message(
            "alice",
            &crypto::fingerprint(&alice.ident_public),
            &next_public,
        ),
    )
    .unwrap();
    store
        .rotate_ident(
            "alice",
            &next_public,
            &chain,
            &proof(crypto::ROLE_IDENT, "alice", &next_public, &next_private),
        )
        .expect("a distinct ident still rotates");

    // The new check is per ACCOUNT (`owner_id = ?1`), so it does not itself
    // stop two accounts choosing one public key — bug-580's non-goals ask
    // for that to stay permitted.
    //
    // It is nonetheless refused today, by the GLOBAL `keys.fingerprint`
    // UNIQUE index. That is a pre-existing divergence from the stated
    // non-goal, measured rather than assumed, and it is deliberately left
    // alone here: allowing one key to authenticate as two accounts is a
    // policy decision with its own security argument, not something to
    // change while fixing an unrelated invariant. This asserts the CURRENT
    // behaviour so the divergence is visible and any future change to it is
    // a deliberate edit to this line.
    let (shared_public, shared_private) = crypto::generate_keypair();
    let (carol_ident, carol_ident_private) = crypto::generate_keypair();
    store
        .register_owner(
            "carol",
            &shared_public,
            &proof(crypto::ROLE_AUTH, "carol", &shared_public, &shared_private),
            &carol_ident,
            &proof(
                crypto::ROLE_IDENT,
                "carol",
                &carol_ident,
                &carol_ident_private,
            ),
        )
        .expect("the first account may use this key");
    let (dave_ident, dave_ident_private) = crypto::generate_keypair();
    let err = store
        .register_owner(
            "dave",
            &shared_public,
            &proof(crypto::ROLE_AUTH, "dave", &shared_public, &shared_private),
            &dave_ident,
            &proof(crypto::ROLE_IDENT, "dave", &dave_ident, &dave_ident_private),
        )
        .unwrap_err();
    assert!(
        err.contains("UNIQUE constraint failed: keys.fingerprint"),
        "a second account sharing a key is refused by the GLOBAL index, not \
             by the account-scoped role check: {err}"
    );
}

/// bug-580: `insert_key_tx` must stay the ONLY writer of a `keys` row.
///
/// The role-separation invariant is account-wide, so it has to hold at every
/// creation path — and the reason it did not before is that the paths were
/// enumerated by hand and the list was wrong (the report named two of five).
/// A per-site check would have been written from the same wrong list. This
/// asserts the structural property instead: one writer, so a new path cannot
/// be added without going through the check.
///
/// Same shape as `tests/gate_lock_covers_every_writer.rs` — a recogniser and
/// the thing it recognises are two lists, and they drift.
#[test]
fn key_insertion_has_exactly_one_writer() {
    let source = include_str!("../../store.rs");
    // The needle is the statement's full column list, not the bare table
    // name: a recogniser that matches its own diagnostic text counts itself
    // and is wrong by one before it has read a line of production code.
    let needle = concat!("INSERT INTO keys ", "(owner_id, role, public_key");
    let writers = source.matches(needle).count();
    assert_eq!(
        writers, 1,
        "every `keys` row must be created by `insert_key_tx`, which enforces \
             auth/ident role separation (bug-580); found {writers} insertion sites. \
             Route the new one through `insert_key_tx` rather than adding a second \
             check that will drift from this one."
    );
}

#[test]
fn open_repository_rejects_non_file_db_and_non_dir_data() {
    let temp = tempfile::tempdir().unwrap();
    // A directory where the DB file should be.
    let dir_as_db = temp.path().join("db-is-a-dir");
    fs::create_dir(&dir_as_db).unwrap();
    match Store::open_repository(&dir_as_db, &temp.path().join("data")) {
        Ok(_) => panic!("a directory must not be accepted as the DB file"),
        Err(err) => assert!(err.contains("is not a file"), "{err}"),
    }
    // A file where the data directory should be.
    let file_as_data = temp.path().join("data-is-a-file");
    fs::write(&file_as_data, b"x").unwrap();
    match Store::open_repository(&temp.path().join("meta.db"), &file_as_data) {
        Ok(_) => panic!("a file must not be accepted as the data dir"),
        Err(err) => assert!(err.contains("is not a directory"), "{err}"),
    }
}

#[test]
fn org_member_removal_and_listing() {
    let (_temp, store) = test_store();
    register_keys(&store, "acme");
    register_keys(&store, "alice");
    register_keys(&store, "bob");

    // Grant validation rejects a bad role and unknown accounts.
    assert!(store
        .grant_org_member("acme", "alice", "superuser")
        .is_err());
    assert!(store
        .grant_org_member("nosuchorg", "alice", "admin")
        .is_err());
    assert!(store.grant_org_member("acme", "nosuch", "admin").is_err());

    store.grant_org_member("acme", "alice", "admin").unwrap();
    store.grant_org_member("acme", "bob", "publisher").unwrap();
    // Update an existing member's role (ON CONFLICT path).
    store.grant_org_member("acme", "alice", "owner").unwrap();
    assert_eq!(
        store.org_member_role("acme", "alice").unwrap().as_deref(),
        Some("owner")
    );

    let members = store.list_org_members("acme").unwrap();
    assert_eq!(members.len(), 2);
    assert!(members.contains(&("alice".to_string(), "owner".to_string())));

    // Removal reports true, then false when there is nothing to remove.
    assert!(store.remove_org_member("acme", "bob").unwrap());
    assert!(!store.remove_org_member("acme", "bob").unwrap());
    assert!(store.remove_org_member("nosuchorg", "bob").is_err());
    assert!(store.remove_org_member("acme", "nosuch").is_err());
    assert!(store.org_member_role("acme", "carol").unwrap().is_none());
}

#[test]
fn publish_token_issue_validates_scope_and_ttl() {
    let (_temp, store) = test_store();
    let keys = register_keys(&store, "alice");
    let (token_public, token_private) = crypto::generate_keypair();
    let proof = crypto::sign(
        &token_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &token_public),
    )
    .unwrap();

    // Unknown owner.
    assert!(store
        .issue_publish_token("nosuch", &token_public, &proof, "nosuch#pkg", 60)
        .unwrap_err()
        .contains("unknown owner"));
    // Bad proof.
    assert!(store
        .issue_publish_token("alice", &token_public, &[0u8; 64], "alice#pkg", 60)
        .unwrap_err()
        .contains("invalid token proof"));
    // Empty and over-long scope.
    assert!(store
        .issue_publish_token("alice", &token_public, &proof, "", 60)
        .unwrap_err()
        .contains("scope"));
    assert!(store
        .issue_publish_token("alice", &token_public, &proof, &"a".repeat(300), 60)
        .unwrap_err()
        .contains("scope"));
    // Bad TTL bounds.
    assert!(store
        .issue_publish_token("alice", &token_public, &proof, "alice#pkg", 0)
        .unwrap_err()
        .contains("ttl"));
    assert!(store
        .issue_publish_token("alice", &token_public, &proof, "alice#pkg", 400 * 24 * 3600)
        .unwrap_err()
        .contains("ttl"));

    // A valid issue registers the token key + returns scope/expiry.
    let (owner, key, expires_at) = store
        .issue_publish_token("alice", &token_public, &proof, "alice#pkg", 60)
        .unwrap();
    assert_eq!(owner.owner_display, "alice");
    assert_eq!(key.fingerprint, crypto::fingerprint(&token_public));
    assert!(expires_at > now_unix());
    // publish_token_for_key reads it back.
    let (scope, exp, revoked) = store.publish_token_for_key(key.id).unwrap().unwrap();
    assert_eq!(scope, "alice#pkg");
    assert_eq!(exp, expires_at);
    assert!(revoked.is_none());
    // A non-token key yields None.
    let auth_key_id = store.owner_with_auth_key("alice").unwrap().unwrap().1.id;
    assert!(store.publish_token_for_key(auth_key_id).unwrap().is_none());

    // Revoke: true, then false; unknown owner errors.
    assert!(store
        .revoke_publish_token("alice", &key.fingerprint)
        .unwrap());
    assert!(!store
        .revoke_publish_token("alice", &key.fingerprint)
        .unwrap());
    assert!(store
        .revoke_publish_token("nosuch", &key.fingerprint)
        .is_err());
    let _ = keys;
}

#[test]
fn add_auth_key_and_rotate_reject_unknown_owner_and_bad_signatures() {
    let (_temp, store) = test_store();
    let (public, _private) = crypto::generate_keypair();
    assert!(store.add_auth_key("nosuch", &public, &[0u8; 64]).is_err());
    assert!(store
        .rotate_ident("nosuch", &public, &[0u8; 64], &[0u8; 64])
        .is_err());
    assert!(store.reanchor_ident("nosuch", &public).is_err());

    let keys = register_keys(&store, "alice");
    // A too-short re-anchor key is rejected.
    assert!(store
        .reanchor_ident("alice", &[0u8; 10])
        .unwrap_err()
        .contains("malformed ident public key"));
    // A bad chain signature for rotation is rejected.
    let (new_public, _new_private) = crypto::generate_keypair();
    assert!(store
        .rotate_ident("alice", &new_public, &[0u8; 64], &[0u8; 64])
        .unwrap_err()
        .contains("invalid ident chain"));
    let _ = keys;
}

#[test]
fn ident_chain_is_empty_before_rotation() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    assert!(store.ident_chain("alice").unwrap().is_empty());
}

#[test]
fn owner_name_validation_guards_every_key_and_challenge_entry_point() {
    // Every account-scoped entry point re-validates the owner name before it
    // reaches SQL or a signature check. The name is attacker-supplied on each
    // of these routes, so a single unguarded one would let an unvalidated
    // string through to `fold_owner`/the log payload.
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let (public, _private) = crypto::generate_keypair();
    let (auth_public, auth_proof, _ident_public, _ident_proof) = registration_payload("alice");

    for bad in ["bad name!", "1leading-digit", ""] {
        let (a_pub, a_proof, i_pub, i_proof) = registration_payload(bad);
        assert!(
            store
                .register_owner(bad, &a_pub, &a_proof, &i_pub, &i_proof)
                .is_err(),
            "register_owner accepted {bad:?}"
        );
        assert!(
            store.create_challenge(bad).is_err(),
            "create_challenge accepted {bad:?}"
        );
        assert!(
            store.create_auth_challenge(bad, "fp").is_err(),
            "create_auth_challenge accepted {bad:?}"
        );
        assert!(
            store.create_ident_challenge(bad).is_err(),
            "create_ident_challenge accepted {bad:?}"
        );
        assert!(
            store.add_auth_key(bad, &auth_public, &auth_proof).is_err(),
            "add_auth_key accepted {bad:?}"
        );
        assert!(
            store
                .rotate_ident(bad, &public, &[0u8; 64], &[0u8; 64])
                .is_err(),
            "rotate_ident accepted {bad:?}"
        );
        assert!(
            store.reanchor_ident(bad, &public).is_err(),
            "reanchor_ident accepted {bad:?}"
        );
    }

    // The reserved name is refused with its own message, not a charset one.
    assert!(store
        .create_ident_challenge("std")
        .unwrap_err()
        .contains("reserved owner name"));

    // Nothing above wrote a row: alice is still the only account.
    assert_eq!(store.count_owners().unwrap(), 1);
}

#[test]
fn ident_challenge_rejects_an_unknown_owner() {
    // The ident challenge is the gate in front of auth-key revocation, so a
    // name that resolves to no active account must fail closed.
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    assert_eq!(
        store.create_ident_challenge("nosuch").unwrap_err(),
        "unknown owner"
    );
    // And the valid owner still works, so the rejection is name-specific.
    assert!(store.create_ident_challenge("alice").is_ok());
}

#[test]
fn open_repository_reports_directories_it_cannot_create() {
    // Both paths are operator-supplied. When the parent of either is an
    // existing regular file the directory cannot be created, and the message
    // has to name which path failed — the alternative is an opaque io error
    // at first use.
    let temp = tempfile::tempdir().unwrap();
    let blocker = temp.path().join("blocker");
    fs::write(&blocker, b"not a directory").unwrap();

    let err = Store::open_repository(
        &blocker.join("nested").join("meta.db"),
        &temp.path().join("data"),
    )
    .map(|_| ())
    .unwrap_err();
    assert!(err.contains("failed to create database directory"), "{err}");

    let err = Store::open_repository(&temp.path().join("meta.db"), &blocker.join("data"))
        .map(|_| ())
        .unwrap_err();
    assert!(err.contains("failed to create data directory"), "{err}");
}
