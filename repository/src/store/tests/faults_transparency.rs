//! `store/transparency.rs` against a database that refuses or holds damaged
//! rows: the root ceremony must name the step that failed and leave no root
//! configured; the index hash and the log readers must report a row they cannot
//! read; and the checkpoint memo must survive a poisoned lock (bug-264).

use super::*;

/// A fault to install on a fresh store.
type Fault = fn(&Store);

#[test]
fn a_root_ceremony_that_fails_at_any_step_configures_no_root() {
    let cases: [(Fault, &str); 3] = [
        (
            |store| refuse_writes(store, "registry_config", "INSERT"),
            "failed to store registry root: injected INSERT failure",
        ),
        (
            |store| refuse_writes(store, "log_entries", "INSERT"),
            "failed to append log entry: injected INSERT failure",
        ),
        (
            |store| refuse_commit_after(store, "log_entries", "INSERT"),
            "failed to commit root ceremony: FOREIGN KEY constraint failed",
        ),
    ];
    for (fault, expected) in cases {
        let (_temp, store) = test_store();
        let before = store.log_size().unwrap();
        fault(&store);
        assert_eq!(
            store
                .init_registry_root("reg-1", now_unix() + 3600)
                .unwrap_err(),
            expected
        );
        assert!(store.registry_config().unwrap().is_none(), "{expected}");
        assert_eq!(store.log_size().unwrap(), before, "{expected}");
    }

    let (_temp, store) = test_store();
    wedge_transaction(&store);
    let err = store
        .init_registry_root("reg-1", now_unix() + 3600)
        .unwrap_err();
    assert!(
        err.starts_with("failed to start root ceremony transaction: "),
        "{err}"
    );
}

#[test]
fn a_renewal_over_an_unreadable_root_row_is_refused() {
    let (_temp, store) = test_store();
    let root_private = store
        .init_registry_root("reg-1", now_unix() + 3600)
        .unwrap();
    tamper(&store, "UPDATE registry_config SET root_version = 'one'");
    let err = store
        .renew_registry_root("reg-1", now_unix() + 7200, &root_private)
        .unwrap_err();
    assert!(err.starts_with("failed to read registry config: "), "{err}");
}

#[test]
fn an_index_row_of_the_wrong_type_fails_the_index_hash() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();
    tamper(&store, "UPDATE package_versions SET state = x'00'");
    let err = store.index_canonical_hash().unwrap_err();
    assert!(err.starts_with("failed to read index row: "), "{err}");
}

#[test]
fn a_log_leaf_that_is_not_a_blob_is_a_read_error() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    tamper(&store, "UPDATE log_entries SET leaf_hash = 'not a hash'");
    let err = store.log_leaf_hashes(None).unwrap_err();
    assert!(err.starts_with("failed to read log leaf: "), "{err}");
}

#[test]
fn a_damaged_server_key_fails_the_checkpoint_rather_than_signing_with_it() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    tamper(&store, "UPDATE server_keys SET private_key = x'00'");
    assert_eq!(
        store.signed_checkpoint().unwrap_err(),
        "malformed local private key"
    );
}

/// bug-264 / REPO-09: a panic while the checkpoint memo's lock is held poisons
/// it; every later checkpoint must still be served (and re-memoised), not fail
/// forever.
#[test]
fn a_poisoned_checkpoint_memo_still_serves_checkpoints() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let memo = store.head_cache.clone();
    let poisoner = std::thread::spawn(move || {
        let _guard = memo.lock().unwrap();
        panic!("a request panicked while holding the checkpoint memo");
    });
    assert!(poisoner.join().is_err());
    assert!(store.head_cache.is_poisoned());

    let (size, root, signature) = store.signed_checkpoint().unwrap();
    assert_eq!(size, store.log_size().unwrap());
    assert_eq!(
        root,
        crate::log::root(&store.log_leaf_hashes(None).unwrap())
    );
    let (server_public, _private) = store.server_keypair().unwrap();
    crypto::verify(
        &server_public,
        &crate::log::checkpoint_signing_input(size as u64, &root),
        &signature,
    )
    .unwrap();
    // Served again from the recovered memo.
    assert_eq!(store.signed_checkpoint().unwrap(), (size, root, signature));
}
