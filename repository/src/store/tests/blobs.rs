//! plan-176-I: `store::tests` — the blobs tests. The shared store harness and the helpers other\n//! hosts import (`register_keys`, `legacy_transfer`, `drop_tables`) live in `store/tests/mod.rs`.

use super::*;

/// A native PUT whose bytes are already stored as a package blob must not
/// write a second, unreferenced `.bin` (bug-276 R5).
///
/// `package_blobs` is keyed by hash alone and `GET /blob/<hash>` picks the
/// file from that row's `kind`, so the existing object already serves these
/// exact bytes. `record_native_blob` reports whether a `.bin` promote is
/// warranted; on a kind collision it is not.
#[test]
fn recording_a_native_blob_over_a_package_blob_skips_the_bin_promote() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "sharedhash",
            "path",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();

    // The publish recorded `sharedhash` as a package blob.
    assert_eq!(
        store.blob_kind("sharedhash").unwrap().as_deref(),
        Some("package")
    );
    // A native upload of identical bytes must not promote a `.bin`, and must
    // leave the existing row's kind alone.
    assert!(!store
        .record_native_blob("sharedhash", "blobs/sharedhash.bin")
        .unwrap());
    assert_eq!(
        store.blob_kind("sharedhash").unwrap().as_deref(),
        Some("package"),
        "the existing row's kind must not be rewritten"
    );

    // A genuinely new native hash still records and promotes.
    assert!(store
        .record_native_blob("nativehash", "blobs/nativehash.bin")
        .unwrap());
    assert_eq!(
        store.blob_kind("nativehash").unwrap().as_deref(),
        Some("native")
    );
    // Idempotent re-upload of the same native blob still promotes.
    assert!(store
        .record_native_blob("nativehash", "blobs/nativehash.bin")
        .unwrap());
}

// --- Blob reachability (plan-49) ---------------------------------------

/// The reachable set is `package_versions.hash` **∪**
/// `package_version_blobs.hash` and **both halves are required**
/// (plan-49 §4.1).
///
/// A `.mfp` blob is not a vendor blob, so it never appears in
/// `package_version_blobs`; dropping that first half of the union would
/// report every published package as collectable garbage. The plan asks for
/// this sentence to live in a test rather than only in a comment, because
/// the failure it guards is silent and total.
#[test]
fn a_published_mfp_blob_is_never_a_candidate() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "mfphash",
            "data/mfphash.mfp",
            "{}",
            &[],
            &PublishMetadata::default(),
        )
        .unwrap();

    // The `.mfp` blob has no `package_version_blobs` edge at all...
    let edges: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM package_version_blobs WHERE hash = 'mfphash'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(edges, 0, "a .mfp is not a vendor blob");
    // ...and is nevertheless reachable, forever, via package_versions.hash.
    let ancient = now_unix() + 3650 * 86_400;
    assert!(store.unreachable_blobs(ancient, 86_400).unwrap().is_empty());
    assert_eq!(store.reachable_blobs().unwrap().len(), 1);
}

/// A vendor blob a live version references is never a candidate; an
/// unreferenced one is — but only once it is older than the grace period
/// (plan-49 §3.1/§4.2).
#[test]
fn candidates_are_unreferenced_blobs_past_the_grace_period() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .record_native_blob("vendorhash", "data/vendorhash.bin")
        .unwrap();
    store
        .record_native_blob("orphanhash", "data/orphanhash.bin")
        .unwrap();
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "mfphash",
            "data/mfphash.mfp",
            "{}",
            &[crate::abi::vendor_ref_for_hash("vendorhash")],
            &PublishMetadata::default(),
        )
        .unwrap();

    let now = now_unix();
    // Inside the grace window nothing is a candidate — the orphan may still
    // be an upload whose publish has not landed yet.
    assert!(store.unreachable_blobs(now, 86_400).unwrap().is_empty());
    // Outside it, exactly the unreferenced blob is.
    let candidates = store.unreachable_blobs(now + 2 * 86_400, 86_400).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].hash, "orphanhash");
    assert_eq!(candidates[0].kind, "native");
    // The `.mfp` and the referenced vendor blob are both reachable.
    let mut reachable: Vec<String> = store
        .reachable_blobs()
        .unwrap()
        .into_iter()
        .map(|row| row.hash)
        .collect();
    reachable.sort();
    assert_eq!(reachable, vec!["mfphash", "vendorhash"]);
}

/// plan-61-A Phase 2, gotcha 2 — the regression test the whole schema
/// decision exists for.
///
/// Two platforms shipping a byte-identical build under different `source`
/// filenames is legal (`PROJECT_JSON_LIBRARY_SOURCE_CONFLICT` forbids two
/// vendor locators sharing a *source*, not a *hash*) and collapses to one
/// entry under any dedupe-by-hash accumulation. A target row is a
/// *platform*, not a blob: dropping one here silently tells every reader
/// the package does not run on musl.
///
/// The neighbouring `package_version_blobs` edge is the opposite case and
/// this test pins both: reachability legitimately dedupes to one row.
#[test]
fn two_locators_sharing_one_blob_hash_write_two_target_rows() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    let shared_hash = "sharedbytes";
    store
        .record_native_blob(shared_hash, "data/sharedbytes.bin")
        .unwrap();
    let glibc = VendorBlobRef {
        logical: "snd".to_string(),
        source: "snd-glibc.a".to_string(),
        hash: shared_hash.to_string(),
        os: "linux".to_string(),
        arch: Some("x86_64".to_string()),
        libc: Some("glibc".to_string()),
        lib_type: "vendor".to_string(),
    };
    let musl = VendorBlobRef {
        source: "snd-musl.a".to_string(),
        libc: Some("musl".to_string()),
        ..glibc.clone()
    };
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "mfphash",
            "data/mfphash.mfp",
            "{}",
            &[glibc, musl],
            &PublishMetadata::default(),
        )
        .unwrap();

    let conn = store.conn();
    let mut stmt = conn
        .prepare(
            "SELECT source, libc, blob_hash FROM package_version_targets
                 ORDER BY source",
        )
        .unwrap();
    let rows: Vec<(String, Option<String>, Option<String>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();

    assert_eq!(
        rows.len(),
        2,
        "one blob under two names is two supported platforms, not one"
    );
    assert_eq!(rows[0].0, "snd-glibc.a");
    assert_eq!(rows[0].1.as_deref(), Some("glibc"));
    assert_eq!(rows[1].0, "snd-musl.a");
    assert_eq!(rows[1].1.as_deref(), Some("musl"));
    // Both rows point at the one blob that actually backs them.
    assert_eq!(rows[0].2.as_deref(), Some(shared_hash));
    assert_eq!(rows[1].2.as_deref(), Some(shared_hash));

    // The blob edge, by contrast, is about reachability and correctly
    // collapses: one blob is one thing to keep alive.
    let edges: i64 = conn
        .query_row("SELECT count(*) FROM package_version_blobs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(edges, 1, "reachability dedupes by hash on purpose");
}

/// The any-arch wildcard survives the round trip to SQL as NULL, and stays
/// distinguishable from a concrete arch (plan-61-A §3, gotcha 1).
#[test]
fn a_wildcard_arch_is_stored_as_null_and_stays_distinct() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;

    store
        .record_native_blob("machash", "data/machash.bin")
        .unwrap();
    store
        .record_native_blob("linuxhash", "data/linuxhash.bin")
        .unwrap();
    let any_arch = VendorBlobRef {
        logical: "snd".to_string(),
        source: "libsnd.dylib".to_string(),
        hash: "machash".to_string(),
        os: "macos".to_string(),
        arch: None,
        libc: None,
        lib_type: "vendor".to_string(),
    };
    let concrete = VendorBlobRef {
        source: "libsnd.a".to_string(),
        hash: "linuxhash".to_string(),
        os: "linux".to_string(),
        arch: Some("aarch64".to_string()),
        ..any_arch.clone()
    };
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "mfphash",
            "data/mfphash.mfp",
            "{}",
            &[any_arch, concrete],
            &PublishMetadata::default(),
        )
        .unwrap();

    let conn = store.conn();
    // A NULL arch is queryable as "any", which is the whole point of
    // storing it as NULL rather than ''.
    let wildcard: String = conn
        .query_row(
            "SELECT os FROM package_version_targets WHERE arch IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("the wildcard locator must land as arch IS NULL");
    assert_eq!(wildcard, "macos");
    let concrete_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM package_version_targets WHERE arch IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(concrete_count, 1);
    // NULL libc is "no constraint", never the empty string.
    let empty_string_libc: i64 = conn
        .query_row(
            "SELECT count(*) FROM package_version_targets WHERE libc = ''",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(empty_string_libc, 0);
}

/// A **yanked** version keeps its blobs (plan-49 §3.2). Yanking is a "do not
/// resolve this by default" signal, not a deletion: lockfiles pinning the
/// hash must keep installing. Only a version row that no longer exists
/// releases its blobs.
#[test]
fn a_yanked_versions_blobs_stay_reachable() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .record_native_blob("vendorhash", "data/vendorhash.bin")
        .unwrap();
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "mfphash",
            "data/mfphash.mfp",
            "{}",
            &[crate::abi::vendor_ref_for_hash("vendorhash")],
            &PublishMetadata::default(),
        )
        .unwrap();
    store
        .set_release_state(alice_id, "alice#toolbox", "1.0.0", "yanked")
        .unwrap();

    let ancient = now_unix() + 3650 * 86_400;
    assert!(
        store.unreachable_blobs(ancient, 86_400).unwrap().is_empty(),
        "yanking must not release a version's blobs"
    );
    assert_eq!(store.reachable_blobs().unwrap().len(), 2);
}

/// A blob shared by two versions survives the removal of one of them — the
/// case a refcount would get wrong and a recomputed scan gets right
/// (plan-49 §3.4).
#[test]
fn a_shared_blob_survives_removing_one_of_its_versions() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .record_native_blob("sharedvendor", "data/sharedvendor.bin")
        .unwrap();
    for version in ["1.0.0", "1.1.0"] {
        store
            .publish_package_version(
                alice_id,
                "alice#toolbox",
                version,
                &format!("mfp-{version}"),
                "data/mfp.mfp",
                "{}",
                &[crate::abi::vendor_ref_for_hash("sharedvendor")],
                &PublishMetadata::default(),
            )
            .unwrap();
    }

    let ancient = now_unix() + 3650 * 86_400;
    assert!(store.unreachable_blobs(ancient, 86_400).unwrap().is_empty());

    // Remove 1.0.0 the way a future version-deletion feature would. There is
    // no such API today (plan-49 "Open Decisions"), so this reaches into the
    // rows directly to prove the reachability rule, not the deletion path.
    let conn = store.conn();
    conn.execute(
        "DELETE FROM package_version_blobs WHERE package_version_id IN
               (SELECT id FROM package_versions WHERE version = '1.0.0')",
        [],
    )
    .unwrap();
    // plan-61-A added `package_version_targets`, a second child table
    // referencing `package_versions`. A real deletion feature must clear it
    // too, so the simulation does — the FK would otherwise reject the
    // parent delete. The reachability assertion below is unchanged.
    conn.execute(
        "DELETE FROM package_version_targets WHERE package_version_id IN
               (SELECT id FROM package_versions WHERE version = '1.0.0')",
        [],
    )
    .unwrap();
    conn.execute("DELETE FROM package_versions WHERE version = '1.0.0'", [])
        .unwrap();
    drop(conn);

    // The shared vendor blob is still referenced by 1.1.0 and must survive;
    // only 1.0.0's own `.mfp` becomes collectable.
    let candidates: Vec<String> = store
        .unreachable_blobs(ancient, 86_400)
        .unwrap()
        .into_iter()
        .map(|row| row.hash)
        .collect();
    assert_eq!(candidates, vec!["mfp-1.0.0"]);
}

/// `forget_blob` reports whether it actually removed a row, so a second
/// sweep over the same hash is a visible no-op. A row still referenced by a
/// `package_version_blobs` edge is refused by the foreign key rather than
/// stranding a live version's vendor blob.
#[test]
fn forget_blob_is_idempotent_and_fk_guarded() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .record_native_blob("vendorhash", "data/vendorhash.bin")
        .unwrap();
    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "mfphash",
            "data/mfphash.mfp",
            "{}",
            &[crate::abi::vendor_ref_for_hash("vendorhash")],
            &PublishMetadata::default(),
        )
        .unwrap();
    store
        .record_native_blob("orphanhash", "data/orphanhash.bin")
        .unwrap();

    assert!(store.forget_blob("orphanhash").unwrap());
    assert!(
        !store.forget_blob("orphanhash").unwrap(),
        "a second delete removes nothing"
    );
    // The referenced vendor blob cannot be deleted out from under its
    // version even if a caller asks.
    assert!(store.forget_blob("vendorhash").is_err());
}

/// `blob_is_reachable` answers the same question as the sweep's scan, one
/// hash at a time — it is what the collector re-asks immediately before each
/// delete, so a publish that lands mid-sweep spares its own blobs.
#[test]
fn blob_is_reachable_tracks_both_halves_of_the_union() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .record_native_blob("vendorhash", "data/vendorhash.bin")
        .unwrap();
    store
        .record_native_blob("orphanhash", "data/orphanhash.bin")
        .unwrap();

    // Before the publish, neither native blob is reachable.
    assert!(!store.blob_is_reachable("vendorhash").unwrap());
    assert!(!store.blob_is_reachable("orphanhash").unwrap());
    assert!(!store.blob_is_reachable("nosuchhash").unwrap());

    store
        .publish_package_version(
            alice_id,
            "alice#toolbox",
            "1.0.0",
            "mfphash",
            "data/mfphash.mfp",
            "{}",
            &[crate::abi::vendor_ref_for_hash("vendorhash")],
            &PublishMetadata::default(),
        )
        .unwrap();

    // The publish makes the `.mfp` (package_versions.hash) and its vendor
    // blob (package_version_blobs.hash) reachable; the orphan is untouched.
    assert!(store.blob_is_reachable("mfphash").unwrap());
    assert!(store.blob_is_reachable("vendorhash").unwrap());
    assert!(!store.blob_is_reachable("orphanhash").unwrap());

    // Yanking does not release them (§3.2).
    store
        .set_release_state(alice_id, "alice#toolbox", "1.0.0", "yanked")
        .unwrap();
    assert!(store.blob_is_reachable("mfphash").unwrap());
    assert!(store.blob_is_reachable("vendorhash").unwrap());
}

/// A negative or overflowing grace period is refused rather than wrapping
/// into the future and making the whole registry a candidate.
#[test]
fn unreachable_blobs_refuses_a_bad_grace_period() {
    let (_temp, store) = test_store();
    assert!(store
        .unreachable_blobs(0, -1)
        .unwrap_err()
        .contains("must not be negative"));
    assert!(store
        .unreachable_blobs(i64::MIN, 1)
        .unwrap_err()
        .contains("overflows the clock"));
}
