//! `store/accounts.rs` against a database that refuses. Every account write
//! (registration, attestation records, machine links, ident rotation and
//! re-anchor, machine revocation, login, pairing) is one transaction; whichever
//! step fails, the write must name it and leave the account and the
//! transparency log exactly as they were.

use super::*;

/// alice with two machine keys (so one can be revoked) and every input the
/// account writes need, signed in advance.
struct Fixture {
    _temp: tempfile::TempDir,
    store: Store,
    alice_id: i64,
    second_machine: String,
    erin: (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>),
    machine: (Vec<u8>, Vec<u8>),
    rotation: (Vec<u8>, Vec<u8>, Vec<u8>),
}

fn auth_key(owner: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (public, private) = crypto::generate_keypair();
    let proof = crypto::sign(
        &private,
        &crypto::registration_message(crypto::ROLE_AUTH, owner, &public),
    )
    .unwrap();
    (public, private, proof)
}

fn fixture() -> Fixture {
    let (temp, store) = test_store();
    let alice = register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    let (second_public, _private, second_proof) = auth_key("alice");
    store
        .add_auth_key("alice", &second_public, &second_proof)
        .unwrap();
    let (machine_public, _private, machine_proof) = auth_key("alice");
    let (new_ident, new_private) = crypto::generate_keypair();
    let chain = crypto::sign(
        &alice.ident_private,
        &crypto::ident_rotation_message(
            "alice",
            &crypto::fingerprint(&alice.ident_public),
            &new_ident,
        ),
    )
    .unwrap();
    let possession = crypto::sign(
        &new_private,
        &crypto::registration_message(crypto::ROLE_IDENT, "alice", &new_ident),
    )
    .unwrap();
    Fixture {
        _temp: temp,
        store,
        alice_id,
        second_machine: crypto::fingerprint(&second_public),
        erin: registration_payload("erin"),
        machine: (machine_public, machine_proof),
        rotation: (new_ident, chain, possession),
    }
}

type Op = fn(&Fixture) -> Result<(), String>;

/// Each logged account write, its transaction's name in the begin error, and
/// its commit error.
fn operations() -> Vec<(&'static str, Op, &'static str, &'static str)> {
    vec![
        (
            "register",
            |f| {
                let (auth, auth_proof, ident, ident_proof) = &f.erin;
                f.store
                    .register_owner("erin", auth, auth_proof, ident, ident_proof)
                    .map(|_| ())
            },
            "failed to start registration transaction",
            "failed to commit registration",
        ),
        (
            "signing request",
            |f| {
                f.store
                    .record_signing_request(f.alice_id, "alice#toolbox", "1.0.0", "fp")
            },
            "failed to start signing transaction",
            "failed to commit signing request",
        ),
        (
            "link",
            |f| {
                f.store
                    .add_auth_key("alice", &f.machine.0, &f.machine.1)
                    .map(|_| ())
            },
            "failed to start link transaction",
            "failed to commit link",
        ),
        (
            "rotate",
            |f| {
                let (new_ident, chain, possession) = &f.rotation;
                f.store
                    .rotate_ident("alice", new_ident, chain, possession)
                    .map(|_| ())
            },
            "failed to start rotation transaction",
            "failed to commit ident rotation",
        ),
        (
            "re-anchor",
            |f| f.store.reanchor_ident("alice", &f.rotation.0).map(|_| ()),
            "failed to start re-anchor transaction",
            "failed to commit re-anchor",
        ),
        (
            "revoke",
            |f| {
                f.store
                    .revoke_auth_key(f.alice_id, &f.second_machine)
                    .map(|_| ())
            },
            "failed to start revocation transaction",
            "failed to commit revocation",
        ),
    ]
}

/// What a refused account write must not have touched.
fn snapshot(f: &Fixture) -> (i64, i64, String, bool, usize) {
    (
        f.store.log_size().unwrap(),
        f.store.count_owners().unwrap(),
        f.store
            .owner_with_ident_key("alice")
            .unwrap()
            .unwrap()
            .1
            .fingerprint,
        f.store
            .owner_auth_key_by_fingerprint("alice", &f.second_machine)
            .unwrap()
            .is_some(),
        f.store.ident_chain("alice").unwrap().len(),
    )
}

#[test]
fn an_account_write_whose_log_append_fails_changes_nothing() {
    for (name, op, _begin, _commit) in operations() {
        let f = fixture();
        let before = snapshot(&f);
        refuse_writes(&f.store, "log_entries", "INSERT");
        assert_eq!(
            op(&f).unwrap_err(),
            "failed to append log entry: injected INSERT failure",
            "{name}"
        );
        assert_eq!(snapshot(&f), before, "{name}");
    }
}

#[test]
fn an_account_write_whose_commit_fails_changes_nothing() {
    for (name, op, _begin, commit) in operations() {
        let f = fixture();
        let before = snapshot(&f);
        refuse_commit_after(&f.store, "log_entries", "INSERT");
        assert_eq!(
            op(&f).unwrap_err(),
            format!("{commit}: FOREIGN KEY constraint failed"),
            "{name}"
        );
        assert_eq!(snapshot(&f), before, "{name}");
    }
}

#[test]
fn an_account_write_that_cannot_begin_its_transaction_is_refused() {
    for (name, op, begin, _commit) in operations() {
        let f = fixture();
        wedge_transaction(&f.store);
        let err = op(&f).unwrap_err();
        assert!(err.starts_with(&format!("{begin}: ")), "{name}: {err}");
    }
}

#[test]
fn every_statement_of_a_rotation_re_anchor_and_revocation_aborts_it() {
    let rotate: Op = |f| {
        let (new_ident, chain, possession) = &f.rotation;
        f.store
            .rotate_ident("alice", new_ident, chain, possession)
            .map(|_| ())
    };
    let reanchor: Op = |f| f.store.reanchor_ident("alice", &f.rotation.0).map(|_| ());
    let revoke: Op = |f| {
        f.store
            .revoke_auth_key(f.alice_id, &f.second_machine)
            .map(|_| ())
    };
    for (op, table, verb, expected) in [
        (rotate, "keys", "UPDATE", "failed to retire ident key"),
        (
            rotate,
            "ident_chain",
            "INSERT",
            "failed to record ident chain link",
        ),
        (reanchor, "keys", "UPDATE", "failed to retire ident key"),
        (revoke, "keys", "UPDATE", "failed to revoke auth key"),
        (revoke, "sessions", "UPDATE", "failed to revoke sessions"),
    ] {
        let f = fixture();
        // A session on the machine, so closing its sessions has a row to update.
        if table == "sessions" {
            let (owner, key) = f
                .store
                .owner_auth_key_by_fingerprint("alice", &f.second_machine)
                .unwrap()
                .unwrap();
            f.store
                .insert_session(&NewSession {
                    owner_id: owner.id,
                    key_id: key.id,
                    jwt_id: "jti-machine".to_string(),
                    issued_at: now_unix(),
                    expires_at: now_unix() + 600,
                })
                .unwrap();
        }
        let before = snapshot(&f);
        refuse_writes(&f.store, table, verb);
        assert_eq!(
            op(&f).unwrap_err(),
            format!("{expected}: injected {verb} failure")
        );
        assert_eq!(snapshot(&f), before, "{expected}");
    }
}

#[test]
fn a_revocation_that_cannot_classify_the_key_is_refused() {
    let f = fixture();
    drop_tables(&f.store, &["publish_tokens"]);
    assert_eq!(
        f.store
            .revoke_auth_key(f.alice_id, &f.second_machine)
            .unwrap_err(),
        "failed to classify auth key: no such table: publish_tokens"
    );
}

#[test]
fn an_owner_name_of_the_wrong_type_fails_the_writes_that_log_it() {
    let f = fixture();
    let before = f.store.log_size().unwrap();
    tamper(&f.store, "UPDATE owners SET owner_display = x'00'");
    let err = f
        .store
        .record_signing_request(f.alice_id, "alice#toolbox", "1.0.0", "fp")
        .unwrap_err();
    assert!(err.starts_with("failed to load owner: "), "{err}");
    let err = f
        .store
        .revoke_auth_key(f.alice_id, &f.second_machine)
        .unwrap_err();
    assert!(err.starts_with("failed to load owner: "), "{err}");
    assert_eq!(f.store.log_size().unwrap(), before);
}

#[test]
fn an_ident_chain_row_of_the_wrong_type_is_a_read_error() {
    let f = fixture();
    let (new_ident, chain, possession) = &f.rotation;
    f.store
        .rotate_ident("alice", new_ident, chain, possession)
        .unwrap();
    tamper(&f.store, "UPDATE ident_chain SET created_at = 'now'");
    let err = f.store.ident_chain("alice").unwrap_err();
    assert!(err.starts_with("failed to read ident chain: "), "{err}");
}

#[test]
fn an_unreadable_ident_key_row_fails_the_transfer_terms() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "hash-1.0.0",
            "data/1.0.0.mfp",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    tamper(
        &store,
        "UPDATE keys SET public_key = 'not a key' WHERE role = 'ident'",
    );
    let err = store.transfer_terms("alice#toolbox").unwrap_err();
    assert!(err.starts_with("failed to load ident key: "), "{err}");
}

#[test]
fn link_and_rotation_refuse_a_proof_made_by_another_key() {
    let f = fixture();
    let (public, _private, _proof) = auth_key("alice");
    let (_other_public, _other_private, other_proof) = auth_key("alice");
    assert_eq!(
        f.store
            .add_auth_key("alice", &public, &other_proof)
            .unwrap_err(),
        "invalid auth proof-of-possession signature"
    );
    let (new_ident, chain, _possession) = &f.rotation;
    assert_eq!(
        f.store
            .rotate_ident("alice", new_ident, chain, &other_proof)
            .unwrap_err(),
        "invalid ident proof-of-possession signature"
    );
}

/// Sign `challenge` with `private` for a login.
fn login_signature(challenge: &ChallengeRecord, private: &[u8]) -> Vec<u8> {
    crypto::sign(
        private,
        &crypto::challenge_message(&challenge.id, &challenge.nonce),
    )
    .unwrap()
}

#[test]
fn a_challenge_presented_to_the_wrong_completer_or_bound_to_the_wrong_key_is_refused() {
    let (_temp, store) = test_store();
    let alice = register_keys(&store, "alice");
    // A login challenge presented as a revocation.
    let login = store.create_challenge("alice").unwrap();
    assert_eq!(
        store
            .complete_revocation_challenge(&login.id, &[0u8; 64], "fp")
            .unwrap_err(),
        "challenge was not issued for revocation"
    );
    // A login challenge whose row names the ident key instead of an auth key.
    tamper(
        &store,
        &format!(
            "UPDATE auth_challenges SET key_id =
               (SELECT id FROM keys WHERE role = 'ident' AND status = 'current')
             WHERE id = '{}'",
            login.id
        ),
    );
    assert_eq!(
        store
            .complete_challenge(&login.id, &login_signature(&login, &alice.ident_private))
            .unwrap_err(),
        "challenge key is not the account's auth key"
    );
}

#[test]
fn a_login_that_cannot_burn_its_challenge_is_refused_and_the_challenge_stays_usable() {
    let (_temp, store) = test_store();
    let alice = register_keys(&store, "alice");
    let challenge = store.create_challenge("alice").unwrap();
    let signature = login_signature(&challenge, &alice.auth_private);

    refuse_writes(&store, "auth_challenges", "UPDATE");
    assert_eq!(
        store
            .complete_challenge(&challenge.id, &signature)
            .unwrap_err(),
        "failed to mark challenge used: injected UPDATE failure"
    );
    tamper(&store, "DROP TRIGGER refuse_UPDATE_auth_challenges;");

    refuse_commit_after(&store, "auth_challenges", "UPDATE");
    assert_eq!(
        store
            .complete_challenge(&challenge.id, &signature)
            .unwrap_err(),
        "failed to commit login: FOREIGN KEY constraint failed"
    );
    tamper(&store, "DROP TRIGGER trap_UPDATE_auth_challenges;");

    wedge_transaction(&store);
    let err = store
        .complete_challenge(&challenge.id, &signature)
        .unwrap_err();
    assert!(
        err.starts_with("failed to start login transaction: "),
        "{err}"
    );
    store.conn().execute_batch("ROLLBACK").unwrap();

    // None of the refusals burned it.
    let (owner, _key) = store.complete_challenge(&challenge.id, &signature).unwrap();
    assert_eq!(owner.owner_display, "alice");
}

#[test]
fn a_pairing_code_already_pending_is_refused_by_name() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .store_pairing_blob(alice_id, "lookup-1", b"blob", b"salt", b"approval")
        .unwrap();
    assert_eq!(
        store
            .store_pairing_blob(alice_id, "lookup-1", b"blob", b"salt", b"approval")
            .unwrap_err(),
        "a pairing with this code is already pending; generate a new code"
    );
    refuse_writes(&store, "pairing_blobs", "INSERT");
    assert_eq!(
        store
            .store_pairing_blob(alice_id, "lookup-2", b"blob", b"salt", b"approval")
            .unwrap_err(),
        "failed to store pairing blob: injected INSERT failure"
    );
}

#[test]
fn a_pairing_fetch_that_cannot_consume_the_blob_hands_nothing_out() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .store_pairing_blob(alice_id, "lookup-1", b"blob", b"salt", b"approval")
        .unwrap();

    refuse_writes(&store, "pairing_blobs", "UPDATE");
    assert_eq!(
        store
            .take_pairing_blob("alice", "lookup-1", |_| true)
            .unwrap_err(),
        "failed to consume pairing blob: injected UPDATE failure"
    );
    tamper(&store, "DROP TRIGGER refuse_UPDATE_pairing_blobs;");

    refuse_commit_after(&store, "pairing_blobs", "UPDATE");
    assert_eq!(
        store
            .take_pairing_blob("alice", "lookup-1", |_| true)
            .unwrap_err(),
        "failed to commit pairing fetch: FOREIGN KEY constraint failed"
    );
    tamper(&store, "DROP TRIGGER trap_UPDATE_pairing_blobs;");

    wedge_transaction(&store);
    let err = store
        .take_pairing_blob("alice", "lookup-1", |_| true)
        .unwrap_err();
    assert!(
        err.starts_with("failed to start pairing transaction: "),
        "{err}"
    );
    store.conn().execute_batch("ROLLBACK").unwrap();

    // The blob is still there, whole, for the honest machine.
    match store
        .take_pairing_blob("alice", "lookup-1", |_| true)
        .unwrap()
    {
        PairingFetch::Relayed { blob, salt } => {
            assert_eq!(
                (blob.as_slice(), salt.as_slice()),
                (&b"blob"[..], &b"salt"[..])
            );
        }
        _ => panic!("the blob survives every refused fetch"),
    }
}

#[test]
fn an_ident_chain_whose_owner_cannot_be_bound_is_an_error_not_an_empty_chain() {
    let f = fixture();
    limit_value_length(&f.store, 32);
    let err = f.store.ident_chain(&"x".repeat(64)).unwrap_err();
    assert!(err.starts_with("failed to load ident chain: "), "{err}");
    assert!(err.contains("too big"), "{err}");
}
