//! plan-176-I: `store::tests` — the registry_tables tests. The shared store harness and the helpers other\n//! hosts import (`register_keys`, `legacy_transfer`, `drop_tables`) live in `store/tests/mod.rs`.

use super::*;

#[test]
fn registry_config_absent_then_present_after_init() {
    let (_temp, store) = test_store();
    assert!(store.registry_config().unwrap().is_none());
    assert!(store
        .init_registry_root("", now_unix() + 3600)
        .unwrap_err()
        .contains("registry id"));
    let root_private = store
        .init_registry_root("reg-1", now_unix() + 3600)
        .unwrap();
    let config = store.registry_config().unwrap().unwrap();
    assert_eq!(config.registry_id, "reg-1");
    assert_eq!(
        crypto::public_from_private(&root_private).unwrap(),
        config.root_public
    );
    // Re-running is NOT delegation renewal: it minted a fresh anchor
    // (bug-584, measured). It is refused; renewal is `renew_registry_root`.
    assert!(store
        .init_registry_root("reg-1", now_unix() + 7200)
        .unwrap_err()
        .contains("already initialized"));
    assert!(store.registry_config().unwrap().is_some());
}

/// bug-584 (positive pin): an honest renewal — the operator supplies the
/// offline root key — bumps the version, re-delegates freshly generated
/// online keys under a new expiry, and leaves the ANCHOR untouched, so a
/// client pinned to the original fingerprint keeps verifying.
#[test]
fn renew_registry_root_keeps_the_anchor_and_rotates_the_delegated_keys() {
    let (_temp, store) = test_store();
    let expires = now_unix() + 3600;
    let root_private = store.init_registry_root("reg-1", expires).unwrap();
    let before = store.registry_config().unwrap().unwrap();

    let renewed_expires = expires + 86_400;
    let version = store
        .renew_registry_root("reg-1", renewed_expires, &root_private)
        .expect("a renewal under the configured root key is accepted");
    assert_eq!(version, 2);
    let after = store.registry_config().unwrap().unwrap();

    // The anchor — the only thing a client pins — is unchanged.
    assert_eq!(before.root_public, after.root_public);
    assert_eq!(
        crypto::fingerprint(&before.root_public),
        crypto::fingerprint(&after.root_public),
    );
    // The online delegated keys are fresh, and the new root.json verifies
    // under the same root key with the bumped version and new expiry.
    assert_ne!(before.snapshot_public, after.snapshot_public);
    assert_ne!(before.timestamp_public, after.timestamp_public);
    assert_ne!(before.root_json, after.root_json);
    crypto::verify(
        &after.root_public,
        &crypto::root_signing_input(after.root_json.as_bytes()),
        &after.root_signature,
    )
    .expect("the renewed root.json verifies under the pinned root key");
    assert!(after
        .root_json
        .contains(&format!("\"version\":2,\"expires\":{renewed_expires}")));
    assert!(after
        .root_json
        .contains(&crypto::encode_bytes(&after.snapshot_public)));
    // ...and the renewal is auditable in the transparency log.
    assert_eq!(log_kinds(&store), vec!["root-init", "root-renew"]);
}

/// bug-584: renewal is authenticated by possession of the offline root key.
/// A stranger's key, a malformed key, and an uninitialized registry are all
/// refused without touching a stored field.
#[test]
fn renew_registry_root_refuses_anything_but_the_configured_root_key() {
    let (_temp, store) = test_store();
    let (_stranger_public, stranger_private) = crypto::generate_keypair();
    assert!(store
        .renew_registry_root("reg-1", now_unix() + 3600, &stranger_private)
        .unwrap_err()
        .contains("not initialized"));

    let root_private = store
        .init_registry_root("reg-1", now_unix() + 3600)
        .unwrap();
    let before = store.registry_config().unwrap().unwrap();

    let err = store
        .renew_registry_root("reg-1", now_unix() + 7200, &stranger_private)
        .unwrap_err();
    assert!(
        err.contains("not this registry's offline root key"),
        "{err}"
    );
    assert!(store
        .renew_registry_root("reg-1", now_unix() + 7200, &[0u8; 3])
        .unwrap_err()
        .contains("malformed"));
    // A renewal aimed at a different registry id is refused too: the
    // registry id is part of what the pinned client checks.
    let err = store
        .renew_registry_root("reg-2", now_unix() + 7200, &root_private)
        .unwrap_err();
    assert!(err.contains("does not match the configured"), "{err}");

    let after = store.registry_config().unwrap().unwrap();
    assert_eq!(before.root_public, after.root_public);
    assert_eq!(before.root_json, after.root_json);
    assert_eq!(before.snapshot_public, after.snapshot_public);
    assert_eq!(before.timestamp_public, after.timestamp_public);
}

/// bug-584: replacing the anchor is still possible when the offline root
/// key is LOST, but only as its own explicitly selected ceremony, and it is
/// transparency-logged as a re-anchor rather than looking like a renewal.
#[test]
fn reanchor_registry_root_replaces_the_anchor_only_when_explicitly_selected() {
    let (_temp, store) = test_store();
    assert!(store
        .reanchor_registry_root("reg-1", now_unix() + 3600)
        .unwrap_err()
        .contains("not initialized"));
    store
        .init_registry_root("reg-1", now_unix() + 3600)
        .unwrap();
    let before = store.registry_config().unwrap().unwrap();

    let new_private = store
        .reanchor_registry_root("reg-1", now_unix() + 7200)
        .unwrap();
    let after = store.registry_config().unwrap().unwrap();
    assert_ne!(before.root_public, after.root_public);
    assert_eq!(
        crypto::public_from_private(&new_private).unwrap(),
        after.root_public
    );
    assert!(after.root_json.contains("\"version\":2"));
    assert_eq!(log_kinds(&store), vec!["root-init", "root-reanchor"]);
}

/// bug-584: `init-root` is one-time initialization. A second run used to
/// mint a brand-new offline root key and overwrite the sole config row,
/// silently replacing the anchor every pinned client verifies against —
/// indistinguishable from registry takeover. It must refuse, and must not
/// touch any configured field while refusing.
#[test]
fn init_registry_root_refuses_to_replace_a_configured_root() {
    let (_temp, store) = test_store();
    store
        .init_registry_root("reg-1", now_unix() + 3600)
        .unwrap();
    let before = store.registry_config().unwrap().unwrap();
    let err = store
        .init_registry_root("reg-1", now_unix() + 7200)
        .unwrap_err();
    assert!(err.contains("already initialized"), "{err}");
    let after = store.registry_config().unwrap().unwrap();
    assert_eq!(before.root_public, after.root_public);
    assert_eq!(before.root_json, after.root_json);
    assert_eq!(before.root_signature, after.root_signature);
    assert_eq!(before.snapshot_public, after.snapshot_public);
    assert_eq!(before.timestamp_public, after.timestamp_public);
}

// --- SQL failures must degrade to an error, never a panic ---------------
//
// Every store method returns `Result<_, String>` on purpose: this process
// serves HTTP, and the file's own `conn()` comment records that a single
// reachable panic in a critical section is a full-service DoS (bug-264 /
// REPO-09). The tests below remove a table out from under a live `Store` —
// a botched migration or a restored-from-a-bad-backup database — and assert
// that each reader/writer returns its documented operator-facing message
// instead of unwrapping. They are the regression guard against any of these
// `map_err` chains being "simplified" into an `unwrap`/`expect`.

/// Drop tables from a live store. Foreign keys are disabled for the drop so
/// the child rows do not block it, then re-enabled for the assertions.
/// Every transparency-log entry kind, in log order.
fn log_kinds(store: &Store) -> Vec<String> {
    let conn = store.conn();
    let mut statement = conn
        .prepare("SELECT kind FROM log_entries ORDER BY idx ASC")
        .unwrap();
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

#[test]
fn losing_the_owners_table_errors_every_account_lookup_instead_of_panicking() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let (public, _private) = crypto::generate_keypair();
    let (auth_public, auth_proof, ident_public, ident_proof) = registration_payload("carol");
    drop_tables(&store, &["owners"]);

    assert!(store
        .count_owners()
        .unwrap_err()
        .contains("failed to count owners"));
    assert!(store
        .owner_with_auth_key("alice")
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .owner_with_ident_key("alice")
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .owner_auth_key_by_fingerprint("alice", "fp")
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .org_member_role("acme", "alice")
        .unwrap_err()
        .contains("failed to load org member role"));

    // Challenge issuance fails closed rather than minting a challenge.
    assert!(store
        .create_challenge("alice")
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .create_auth_challenge("alice", "fp")
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .create_ident_challenge("alice")
        .unwrap_err()
        .contains("failed to load owner"));

    // Key management fails closed.
    assert!(store
        .add_auth_key("alice", &public, &[0u8; 64])
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .rotate_ident("alice", &public, &[0u8; 64], &[0u8; 64])
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .reanchor_ident("alice", &public)
        .unwrap_err()
        .contains("failed to load owner"));

    // Org / token / transfer entry points all resolve an owner first.
    assert!(store
        .grant_org_member("acme", "alice", "admin")
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .remove_org_member("acme", "alice")
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .issue_publish_token("alice", &public, &[0u8; 64], "alice#pkg", 60)
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .revoke_publish_token("alice", "fp")
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .accept_transfer("alice#toolbox", "bob", &[0u8; 64])
        .unwrap_err()
        .contains("failed to load owner"));
    assert!(store
        .package_owner("alice#toolbox")
        .unwrap_err()
        .contains("failed to load package owner"));
    assert!(store
        .create_transfer_offer("alice#toolbox", "alice", "bob", 0, "digest", &[0u8; 64])
        .unwrap_err()
        .contains("failed to load package owner"));
    assert!(store
        .take_pairing_blob("alice", "lookup", approved)
        .unwrap_err()
        .contains("failed to load pairing blob"));
    assert!(store
        .record_signing_request(1, "alice#toolbox", "1.0.0", "fp")
        .is_err());

    // A registration failure that is *not* a duplicate name reports the
    // generic message, not "already in use" — misreporting a schema fault as
    // a name collision would send the operator hunting the wrong problem.
    let err = store
        .register_owner(
            "carol",
            &auth_public,
            &auth_proof,
            &ident_public,
            &ident_proof,
        )
        .unwrap_err();
    assert!(err.contains("failed to register owner"), "{err}");
    assert!(!err.contains("already in use"), "{err}");
}

#[test]
fn losing_the_key_tables_errors_revocation_and_chain_reads() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    let (public, private) = crypto::generate_keypair();
    let proof = crypto::sign(
        &private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &public),
    )
    .unwrap();
    drop_tables(&store, &["ident_chain", "publish_tokens"]);

    assert!(store
        .ident_chain("alice")
        .unwrap_err()
        .contains("failed to prepare chain query"));
    assert!(store
        .publish_token_for_key(1)
        .unwrap_err()
        .contains("failed to load publish token"));
    // Issuing a token registers the key first, then records the token row;
    // losing the token table must fail the whole issuance, not leave a
    // registered auth key with no scope or expiry attached to it.
    assert!(store
        .issue_publish_token("alice", &public, &proof, "alice#pkg", 60)
        .unwrap_err()
        .contains("failed to record publish token"));
    assert!(store
        .revoke_publish_token("alice", "fp")
        .unwrap_err()
        .contains("failed to load token"));

    drop_tables(&store, &["keys"]);
    assert!(store
        .revoke_auth_key(owner, "fp")
        .unwrap_err()
        .contains("failed to load auth key"));
    assert!(store
        .issue_publish_token("alice", &public, &proof, "alice#pkg", 60)
        .unwrap_err()
        .contains("failed to register token key"));
}

#[test]
fn losing_the_package_tables_errors_publish_and_index_reads() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    drop_tables(&store, &["package_version_blobs", "package_versions"]);

    assert!(store
        .list_package_versions("alice#toolbox")
        .unwrap_err()
        .contains("failed to prepare version query"));
    assert!(store
        .package_version_exists("alice#toolbox", "1.0.0")
        .unwrap_err()
        .contains("failed to check package version"));
    assert!(store
        .owner_version_count(owner)
        .unwrap_err()
        .contains("failed to count owner versions"));
    assert!(store
        .index_canonical_hash()
        .unwrap_err()
        .contains("failed to prepare index query"));
    assert!(store
        .set_release_state(owner, "alice#toolbox", "1.0.0", "yanked")
        .unwrap_err()
        .contains("failed to load package version"));
    // The GC's reachability queries read both halves of the union, so both
    // sides must report rather than treat an unreadable table as "nothing is
    // reachable" — that would make every blob a deletion candidate.
    assert!(store
        .unreachable_blobs(now_unix(), 0)
        .unwrap_err()
        .contains("failed to prepare blob reachability query"));
    assert!(store
        .reachable_blobs()
        .unwrap_err()
        .contains("failed to prepare reachable blob query"));
    assert!(store
        .blob_is_reachable("h1")
        .unwrap_err()
        .contains("failed to re-check blob reachability"));
    // A publish that cannot record its version row reports the generic
    // failure, not the "already published" duplicate message.
    let err = store
        .publish_package_version(
            owner,
            "alice#toolbox",
            "1.0.0",
            "h1",
            "p1",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap_err();
    assert!(err.contains("failed to publish package version"), "{err}");
    assert!(!err.contains("already published"), "{err}");

    drop_tables(&store, &["package_blobs"]);
    assert!(store
        .blob_kind("h1")
        .unwrap_err()
        .contains("failed to load blob kind"));
    assert!(store
        .record_native_blob("h1", "p1")
        .unwrap_err()
        .contains("failed to load native blob metadata"));

    drop_tables(&store, &["packages"]);
    assert!(store
        .typosquat_candidates("alice#toolbox")
        .unwrap_err()
        .contains("failed to prepare typosquat query"));
    assert!(store
        .publish_package_version(
            owner,
            "alice#toolbox",
            "1.0.0",
            "h1",
            "p1",
            "{}",
            &[],
            &PublishMetadata::default()
        )
        .unwrap_err()
        .contains("failed to create package identity"));
}

#[test]
fn losing_the_log_table_errors_reads_and_aborts_state_changes() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    let (auth_public, auth_proof, ident_public, ident_proof) = registration_payload("carol");
    drop_tables(&store, &["log_entries"]);

    assert!(store
        .log_size()
        .unwrap_err()
        .contains("failed to size the log"));
    assert!(store
        .log_leaf_hashes(None)
        .unwrap_err()
        .contains("failed to prepare log query"));
    assert!(store
        .publish_log_entry("alice#toolbox", "1.0.0")
        .unwrap_err()
        .contains("failed to load publish log entry"));

    // Every state change appends its log entry inside the same transaction,
    // so an unappendable log must abort the change rather than commit an
    // unlogged one. `carol` must not exist afterwards.
    assert!(store
        .register_owner(
            "carol",
            &auth_public,
            &auth_proof,
            &ident_public,
            &ident_proof
        )
        .unwrap_err()
        .contains("failed to size the log"));
    assert!(store
        .publish_package_version(
            owner,
            "alice#toolbox",
            "1.0.0",
            "h1",
            "p1",
            "{}",
            &[],
            &PublishMetadata::default()
        )
        .unwrap_err()
        .contains("failed to size the log"));
    assert_eq!(store.count_owners().unwrap(), 1);
    assert!(!store
        .package_version_exists("alice#toolbox", "1.0.0")
        .unwrap());
}

#[test]
fn losing_the_session_and_challenge_tables_errors_the_auth_path() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let owner = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    drop_tables(&store, &["sessions"]);

    assert!(store
        .session_exists("jwt-1")
        .unwrap_err()
        .contains("failed to load session"));
    assert!(store
        .insert_session(&NewSession {
            owner_id: owner,
            key_id: 1,
            jwt_id: "jwt-1".to_string(),
            issued_at: now_unix(),
            expires_at: now_unix() + 60,
        })
        .unwrap_err()
        .contains("failed to store session"));
    assert!(store
        .reap_expired()
        .unwrap_err()
        .contains("failed to reap sessions"));

    drop_tables(&store, &["auth_challenges"]);
    assert!(store
        .create_challenge("alice")
        .unwrap_err()
        .contains("failed to create auth challenge"));
    assert!(store
        .complete_challenge("some-id", &[0u8; 64])
        .unwrap_err()
        .contains("failed to load auth challenge"));
    assert!(store
        .complete_revocation_challenge("some-id", &[0u8; 64], "fp")
        .unwrap_err()
        .contains("failed to load auth challenge"));
    assert!(store
        .force_expire_challenge("some-id")
        .unwrap_err()
        .contains("failed to expire challenge"));
    assert!(store
        .reap_expired()
        .unwrap_err()
        .contains("failed to reap challenges"));

    drop_tables(&store, &["pairing_blobs"]);
    assert!(store
        .store_pairing_blob(owner, "lookup", b"blob", b"salt", b"approval")
        .unwrap_err()
        .contains("failed to clear expired pairing blobs"));
}

#[test]
fn losing_the_server_key_and_config_tables_errors_signing_and_metadata() {
    let (_temp, store) = test_store();
    drop_tables(&store, &["registry_config"]);
    assert!(store
        .registry_config()
        .unwrap_err()
        .contains("failed to load registry config"));

    drop_tables(&store, &["server_secrets", "server_keys"]);
    assert!(store
        .server_secret()
        .unwrap_err()
        .contains("failed to load server signing secret"));
    assert!(store
        .server_keypair()
        .unwrap_err()
        .contains("failed to load server keypair"));
    assert!(store
        .server_public_key()
        .unwrap_err()
        .contains("failed to load server keypair"));
    // The root ceremony delegates the server's attestation key, so it must
    // refuse to sign a root that names a key it could not read.
    assert!(store
        .init_registry_root("reg-1", now_unix() + 3600)
        .unwrap_err()
        .contains("failed to load server keypair"));
}

#[test]
fn losing_the_org_and_transfer_tables_errors_membership_and_handover() {
    let (_temp, store) = test_store();
    register_keys(&store, "acme");
    let alice_keys = register_keys(&store, "alice");
    let bob_keys = register_keys(&store, "bob");
    let alice = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            alice,
            "alice#toolbox",
            "1.0.0",
            "h1",
            "p1",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    drop_tables(&store, &["org_members", "transfer_offers"]);

    assert!(store
        .list_org_members("acme")
        .unwrap_err()
        .contains("failed to prepare org query"));
    assert!(store
        .grant_org_member("acme", "alice", "admin")
        .unwrap_err()
        .contains("failed to record org member"));
    assert!(store
        .remove_org_member("acme", "alice")
        .unwrap_err()
        .contains("failed to remove org member"));

    let (sequence, digest, offer) =
        sign_offer_for_test(&store, "alice#toolbox", &alice_keys, "bob");
    assert!(store
        .create_transfer_offer("alice#toolbox", "alice", "bob", sequence, &digest, &offer)
        .unwrap_err()
        .contains("failed to record transfer offer"));
    let accept = sign_accept_for_test(&store, "alice#toolbox", &bob_keys, "bob");
    assert!(store
        .accept_transfer("alice#toolbox", "bob", &accept)
        .unwrap_err()
        .contains("failed to load transfer offer"));
    // Ownership never moved.
    assert_eq!(
        store
            .package_owner("alice#toolbox")
            .unwrap()
            .unwrap()
            .owner_display,
        "alice"
    );

    drop_tables(&store, &["release_state_changes"]);
    let alice_id = store.package_owner("alice#toolbox").unwrap().unwrap().id;
    assert!(store
        .set_release_state(alice_id, "alice#toolbox", "1.0.0", "yanked")
        .unwrap_err()
        .contains("failed to record release-state change"));
    // The aborted transition left the version available.
    assert_eq!(
        store.list_package_versions("alice#toolbox").unwrap()[0].state,
        "available"
    );
}
