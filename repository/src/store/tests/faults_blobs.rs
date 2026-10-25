//! `store/blobs.rs` against a database that refuses or holds damaged rows: a
//! native blob that cannot be recorded must say so, and the GC's reachability
//! readers must report a row they cannot read instead of returning a short set —
//! a short reachable set would make a live blob look like garbage.

use super::*;

#[test]
fn a_native_blob_that_cannot_be_recorded_is_reported() {
    let (_temp, store) = test_store();
    refuse_writes(&store, "package_blobs", "INSERT");
    assert_eq!(
        store.record_native_blob("h-snd", "data/h-snd").unwrap_err(),
        "failed to store native blob metadata: injected INSERT failure"
    );
    assert_eq!(store.blob_kind("h-snd").unwrap(), None);
}

#[test]
fn a_blob_row_of_the_wrong_type_fails_both_reachability_readers() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    // One reachable row (the published artifact) and one unreachable (an
    // uploaded native blob no version references).
    publish_as(&store, "alice", "alice#toolbox", "1.0.0", None).unwrap();
    store
        .record_native_blob("h-orphan", "data/h-orphan")
        .unwrap();
    tamper(&store, "UPDATE package_blobs SET path = x'00'");

    let err = store.reachable_blobs().unwrap_err();
    assert!(err.starts_with("failed to read blob row: "), "{err}");
    let err = store.unreachable_blobs(now_unix() + 60, 0).unwrap_err();
    assert!(err.starts_with("failed to read blob row: "), "{err}");
}
