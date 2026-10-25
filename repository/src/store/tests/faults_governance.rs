//! `store/governance.rs` against a database that refuses. Every governance
//! write (org roles, publish tokens, transfer offers and accepts) is one
//! transaction ending in a transparency-log append: whichever step fails —
//! the begin, a statement, the log append, or the commit — the operation must
//! name that step and leave the registry, including its log, unchanged.

use super::*;

/// alice owns `alice#toolbox@1.0.0` and has carol in her org and one publish
/// token; bob holds a pending offer of the package.
struct Fixture {
    _temp: tempfile::TempDir,
    store: Store,
    token_fingerprint: String,
    new_token_public: Vec<u8>,
    new_token_proof: Vec<u8>,
    offer: (i64, String, Vec<u8>),
    accept: Vec<u8>,
}

const IDENT: &str = "alice#toolbox";

fn token_key(owner: &str) -> (Vec<u8>, Vec<u8>) {
    let (public, private) = crypto::generate_keypair();
    let proof = crypto::sign(
        &private,
        &crypto::registration_message(crypto::ROLE_AUTH, owner, &public),
    )
    .unwrap();
    (public, proof)
}

/// The fixture; with `offered`, bob's offer is already recorded (so the
/// accept can run), otherwise only signed (so the offer can run).
fn fixture(offered: bool) -> Fixture {
    let (temp, store) = test_store();
    let alice = register_keys(&store, "alice");
    let bob = register_keys(&store, "bob");
    register_keys(&store, "carol");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            alice_id,
            IDENT,
            "1.0.0",
            "hash-1.0.0",
            "data/1.0.0.mfp",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();
    store.grant_org_member("alice", "carol", "admin").unwrap();
    let (token_public, token_proof) = token_key("alice");
    store
        .issue_publish_token("alice", &token_public, &token_proof, IDENT, 600)
        .unwrap();
    let (new_token_public, new_token_proof) = token_key("alice");
    let offer = sign_offer_for_test(&store, IDENT, &alice, "bob");
    let mut accept = Vec::new();
    if offered {
        store
            .create_transfer_offer(IDENT, "alice", "bob", offer.0, &offer.1, &offer.2)
            .unwrap();
        accept = sign_accept_for_test(&store, IDENT, &bob, "bob");
    }
    Fixture {
        _temp: temp,
        store,
        token_fingerprint: crypto::fingerprint(&token_public),
        new_token_public,
        new_token_proof,
        offer,
        accept,
    }
}

type Op = fn(&Fixture) -> Result<(), String>;

/// Each governance write, whether it needs the offer recorded first, its
/// transaction's name in the begin error, and its commit error.
fn operations() -> Vec<(&'static str, bool, Op, &'static str, &'static str)> {
    vec![
        (
            "grant",
            false,
            |f| f.store.grant_org_member("alice", "bob", "publisher"),
            "failed to start org transaction",
            "failed to commit org member grant",
        ),
        (
            "remove",
            false,
            |f| f.store.remove_org_member("alice", "carol").map(|_| ()),
            "failed to start org transaction",
            "failed to commit org member removal",
        ),
        (
            "issue token",
            false,
            |f| {
                f.store
                    .issue_publish_token(
                        "alice",
                        &f.new_token_public,
                        &f.new_token_proof,
                        IDENT,
                        60,
                    )
                    .map(|_| ())
            },
            "failed to start token transaction",
            "failed to commit token issue",
        ),
        (
            "revoke token",
            false,
            |f| {
                f.store
                    .revoke_publish_token("alice", &f.token_fingerprint)
                    .map(|_| ())
            },
            "failed to start token revoke transaction",
            "failed to commit token revoke",
        ),
        (
            "offer",
            false,
            |f| {
                f.store
                    .create_transfer_offer(IDENT, "alice", "bob", f.offer.0, &f.offer.1, &f.offer.2)
            },
            "failed to start transfer transaction",
            "failed to commit transfer offer",
        ),
        (
            "accept",
            true,
            |f| f.store.accept_transfer(IDENT, "bob", &f.accept),
            "failed to start transfer transaction",
            "failed to commit transfer accept",
        ),
    ]
}

/// The log size, alice's org members, the package's owner, and the publish
/// token's scope/expiry/revocation: what a refused write must not touch.
type Snapshot = (
    i64,
    Vec<(String, String)>,
    Option<String>,
    Option<(String, i64, Option<i64>)>,
);

fn snapshot(f: &Fixture) -> Snapshot {
    let token_key = f
        .store
        .owner_auth_key_by_fingerprint("alice", &f.token_fingerprint)
        .unwrap()
        .map(|(_, key)| key.id);
    (
        f.store.log_size().unwrap(),
        f.store.list_org_members("alice").unwrap(),
        f.store
            .package_owner(IDENT)
            .unwrap()
            .map(|owner| owner.owner_display),
        token_key.and_then(|id| f.store.publish_token_for_key(id).unwrap()),
    )
}

#[test]
fn a_governance_write_whose_log_append_fails_changes_nothing() {
    for (name, offered, op, _begin, _commit) in operations() {
        let f = fixture(offered);
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
fn a_governance_write_whose_commit_fails_changes_nothing() {
    for (name, offered, op, _begin, commit) in operations() {
        let f = fixture(offered);
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
fn a_governance_write_that_cannot_begin_its_transaction_is_refused() {
    for (name, offered, op, begin, _commit) in operations() {
        let f = fixture(offered);
        wedge_transaction(&f.store);
        let err = op(&f).unwrap_err();
        assert!(err.starts_with(&format!("{begin}: ")), "{name}: {err}");
    }
}

#[test]
fn every_statement_of_a_token_revoke_and_a_transfer_accept_aborts_it() {
    for (offered, table, op, expected) in [
        (false, "publish_tokens", "UPDATE", "failed to revoke token"),
        (false, "keys", "UPDATE", "failed to revoke token key"),
        (
            false,
            "sessions",
            "UPDATE",
            "failed to close token sessions",
        ),
        (
            true,
            "transfer_history",
            "INSERT",
            "failed to record transfer history",
        ),
        (
            true,
            "packages",
            "UPDATE",
            "failed to re-bind package owner",
        ),
        (
            true,
            "transfer_offers",
            "UPDATE",
            "failed to close transfer offer",
        ),
    ] {
        let f = fixture(offered);
        // A session on the token, so closing its sessions has a row to update.
        if table == "sessions" {
            let key = f
                .store
                .owner_auth_key_by_fingerprint("alice", &f.token_fingerprint)
                .unwrap()
                .unwrap();
            f.store
                .insert_session(&NewSession {
                    owner_id: key.0.id,
                    key_id: key.1.id,
                    jwt_id: "jti-token".to_string(),
                    issued_at: now_unix(),
                    expires_at: now_unix() + 600,
                })
                .unwrap();
        }
        let before = snapshot(&f);
        refuse_writes(&f.store, table, op);
        let result = if offered {
            f.store.accept_transfer(IDENT, "bob", &f.accept)
        } else {
            f.store
                .revoke_publish_token("alice", &f.token_fingerprint)
                .map(|_| ())
        };
        assert_eq!(
            result.unwrap_err(),
            format!("{expected}: injected {op} failure")
        );
        assert_eq!(snapshot(&f), before, "{table}");
    }
}

#[test]
fn accepting_an_offer_of_an_unknown_package_finds_no_offer() {
    let f = fixture(true);
    assert_eq!(
        f.store
            .accept_transfer("alice#nothing", "bob", &f.accept)
            .unwrap_err(),
        "no pending transfer offer for this account"
    );
}

#[test]
fn a_transfer_with_an_owner_or_recipient_missing_its_ident_key_is_refused() {
    let f = fixture(true);
    let retire = |owner: &str| {
        tamper(
            &f.store,
            &format!(
                "UPDATE keys SET status = 'past' WHERE role = 'ident'
                   AND owner_id = (SELECT id FROM owners WHERE owner_folded = '{owner}')"
            ),
        )
    };
    retire("bob");
    assert_eq!(
        f.store
            .accept_transfer(IDENT, "bob", &f.accept)
            .unwrap_err(),
        "recipient has no current ident key"
    );
    retire("alice");
    assert_eq!(
        f.store.transfer_terms(IDENT).unwrap_err(),
        "package owner has no current ident key"
    );
}

#[test]
fn transfer_reads_report_a_missing_table() {
    let f = fixture(true);
    drop_tables(&f.store, &["transfer_history"]);
    assert_eq!(
        f.store.has_legacy_transfer(IDENT).unwrap_err(),
        "failed to read transfer history: no such table: transfer_history"
    );
    assert_eq!(
        f.store.transfer_history(IDENT).unwrap_err(),
        "failed to prepare transfer history query: no such table: transfer_history"
    );
    assert_eq!(
        f.store.transfer_terms(IDENT).unwrap_err(),
        "failed to count transfer history: no such table: transfer_history"
    );

    let f = fixture(true);
    drop_tables(
        &f.store,
        &[
            "package_version_docs",
            "package_version_blobs",
            "package_versions",
        ],
    );
    assert_eq!(
        f.store.transfer_terms(IDENT).unwrap_err(),
        "failed to prepare release digest query: no such table: package_versions"
    );

    let f = fixture(true);
    drop_tables(&f.store, &["packages"]);
    assert_eq!(
        f.store.transfer_terms(IDENT).unwrap_err(),
        "failed to load package owner: no such table: packages"
    );
}

#[test]
fn governance_rows_of_the_wrong_type_are_read_errors() {
    let f = fixture(true);
    f.store.accept_transfer(IDENT, "bob", &f.accept).unwrap();
    tamper(
        &f.store,
        "DROP TRIGGER transfer_history_no_update;
         UPDATE transfer_history SET offered_at = 'yesterday';",
    );
    let err = f.store.transfer_history(IDENT).unwrap_err();
    assert!(
        err.starts_with("failed to read transfer history: "),
        "{err}"
    );

    tamper(&f.store, "UPDATE package_versions SET hash = x'00'");
    let err = f.store.transfer_terms(IDENT).unwrap_err();
    assert!(err.starts_with("failed to read release: "), "{err}");

    tamper(&f.store, "UPDATE org_members SET role = x'00'");
    let err = f.store.list_org_members("alice").unwrap_err();
    assert!(err.starts_with("failed to read org member: "), "{err}");
}

#[test]
fn a_governance_listing_whose_name_cannot_be_bound_is_an_error_not_an_empty_list() {
    let f = fixture(false);
    limit_value_length(&f.store, 32);
    let long = "x".repeat(64);
    let err = f.store.list_org_members(&long).unwrap_err();
    assert!(err.starts_with("failed to list org members: "), "{err}");
    assert!(err.contains("too big"), "{err}");
    let err = f
        .store
        .transfer_history(&format!("alice#{long}"))
        .unwrap_err();
    assert!(
        err.starts_with("failed to load transfer history: "),
        "{err}"
    );
    assert!(err.contains("too big"), "{err}");
}
