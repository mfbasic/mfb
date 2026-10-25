//! `store/mod.rs` — opening a repository the store cannot safely or fully open:
//! an exposed directory it cannot make private, a journal path it cannot
//! inspect, a file that is not a database, key tables replaced by views, and a
//! path SQLite refuses; plus the reaper and the pre-epoch clock reading.

use super::*;
use std::time::Duration;

#[test]
fn a_clock_before_the_epoch_reads_as_zero_and_after_it_as_whole_seconds() {
    assert_eq!(unix_seconds(UNIX_EPOCH - Duration::from_secs(5)), 0);
    assert_eq!(unix_seconds(UNIX_EPOCH + Duration::from_millis(42_900)), 42);
}

/// bug-586: an exposed metadata directory that is not ours to tighten — here
/// the shared `/tmp`, world-writable and owned by root — must stop the open with
/// operator guidance, never serve with the keys readable.
#[cfg(unix)]
#[test]
fn an_exposed_directory_owned_by_another_account_refuses_the_open() {
    use std::os::unix::fs::MetadataExt;
    let shared = Path::new("/tmp");
    let probe = tempfile::NamedTempFile::new().unwrap();
    let our_uid = fs::metadata(probe.path()).unwrap().uid();
    let shared_meta = fs::metadata(shared).unwrap();
    // Run only where the scenario exists: a directory someone else owns. As
    // root (or on a host whose /tmp we own) the chmod would succeed — and
    // change the real /tmp — so there is nothing to test.
    if shared_meta.uid() == our_uid || our_uid == 0 || shared_meta.mode() & 0o077 == 0 {
        eprintln!("skipped: /tmp is ours to chmod on this host");
        return;
    }
    let dbpath = shared.join(format!("mfb-repo-test-{}.db", Uuid::new_v4()));
    let data = tempfile::tempdir().unwrap();
    let err = match Store::open_repository(&dbpath, data.path()) {
        Ok(_) => panic!("an exposed foreign directory must refuse the open"),
        Err(err) => err,
    };
    assert!(
        err.starts_with(
            "database directory '/tmp' is accessible to other accounts (mode 777) and its \
             permissions could not be tightened to 700: "
        ),
        "{err}"
    );
    assert!(err.contains("chmod 700 <dir>"), "{err}");
    assert!(!dbpath.exists(), "nothing is created before the refusal");
}

/// A journal path that cannot even be inspected (here a symlink through a
/// regular file, so `stat` fails with ENOTDIR) is not "absent": the open stops.
#[cfg(unix)]
#[test]
fn a_journal_path_that_cannot_be_inspected_refuses_the_open() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("meta");
    fs::create_dir(&dir).unwrap();
    let file = temp.path().join("plain-file");
    fs::write(&file, b"").unwrap();
    let wal = dir.join("meta.db-wal");
    std::os::unix::fs::symlink(file.join("child"), &wal).unwrap();
    let err = match Store::open_repository(&dir.join("meta.db"), &temp.path().join("blobs")) {
        Ok(_) => panic!("an uninspectable journal must refuse the open"),
        Err(err) => err,
    };
    assert!(
        err.starts_with(&format!(
            "failed to inspect database journal '{}': ",
            wal.display()
        )),
        "{err}"
    );

    // The database path itself, the same way.
    fs::remove_file(&wal).unwrap();
    let dbpath = dir.join("meta.db");
    std::os::unix::fs::symlink(file.join("child"), &dbpath).unwrap();
    let err = match Store::open_repository(&dbpath, &temp.path().join("blobs")) {
        Ok(_) => panic!("an uninspectable database must refuse the open"),
        Err(err) => err,
    };
    assert!(
        err.starts_with(&format!(
            "failed to inspect database '{}': ",
            dbpath.display()
        )),
        "{err}"
    );
}

#[test]
fn a_remote_data_path_opens_the_metadata_store_without_a_local_directory() {
    let temp = tempfile::tempdir().unwrap();
    let opened = Store::open_repository(
        &temp.path().join("meta.db"),
        Path::new("s3://bucket/packages"),
    )
    .unwrap();
    assert_eq!(opened.packages_dir, Path::new("s3://bucket/packages"));
    assert!(!Path::new("s3:").exists(), "no local directory is created");
    assert_eq!(opened.store.count_owners().unwrap(), 0);
}

#[test]
fn a_file_that_is_not_a_database_refuses_the_open() {
    let temp = tempfile::tempdir().unwrap();
    let dbpath = temp.path().join("meta.db");
    fs::write(&dbpath, vec![b'x'; 4096]).unwrap();
    let err = match Store::open_repository(&dbpath, &temp.path().join("blobs")) {
        Ok(_) => panic!("a non-database file must refuse the open"),
        Err(err) => err,
    };
    assert_eq!(err, "failed to enable WAL: file is not a database");
}

/// Unix-only: the 512-byte limit is the unix VFS's, and Windows refuses a path
/// this long at directory creation, before SQLite is involved.
#[cfg(unix)]
#[test]
fn a_path_sqlite_cannot_open_refuses_the_open() {
    // Longer than SQLite's unix VFS pathname limit (512), in components short
    // enough for the filesystem to create.
    let temp = tempfile::tempdir().unwrap();
    let mut dir = temp.path().to_path_buf();
    for _ in 0..6 {
        dir.push("d".repeat(100));
    }
    let dbpath = dir.join("meta.db");
    let err = match Store::open_repository(&dbpath, &temp.path().join("blobs")) {
        Ok(_) => panic!("an unopenable path must refuse the open"),
        Err(err) => err,
    };
    assert!(
        err.starts_with(&format!("failed to open '{}': ", dbpath.display())),
        "{err}"
    );
}

/// A pre-existing database whose key table is a VIEW: `CREATE TABLE IF NOT
/// EXISTS` passes over it, and the open must then fail naming the key it could
/// not check or create instead of serving without one.
fn open_with_view(view: &str) -> String {
    let temp = tempfile::tempdir().unwrap();
    let dbpath = temp.path().join("meta.db");
    Connection::open(&dbpath)
        .unwrap()
        .execute_batch(view)
        .unwrap();
    match Store::open_repository(&dbpath, &temp.path().join("blobs")) {
        Ok(_) => panic!("{view}: the open must fail"),
        Err(err) => err,
    }
}

#[test]
fn key_tables_that_are_views_refuse_the_open_by_key() {
    let err = open_with_view("CREATE VIEW server_secrets AS SELECT 1 AS other;");
    assert!(
        err.starts_with("failed to check server signing secret: no such column: id"),
        "{err}"
    );
    let err = open_with_view("CREATE VIEW server_secrets AS SELECT 1 AS id WHERE 0;");
    assert!(
        err.starts_with("failed to create server signing secret: "),
        "{err}"
    );
    let err = open_with_view("CREATE VIEW server_keys AS SELECT 1 AS other;");
    assert!(
        err.starts_with("failed to check server keypair: no such column: id"),
        "{err}"
    );
    let err = open_with_view("CREATE VIEW server_keys AS SELECT 1 AS id WHERE 0;");
    assert!(
        err.starts_with("failed to create server keypair: "),
        "{err}"
    );
}

#[test]
fn a_reaper_that_cannot_delete_an_expired_pairing_reports_it() {
    let (_temp, store) = test_store();
    register_keys(&store, "alice");
    let alice_id = store.owner_with_ident_key("alice").unwrap().unwrap().0.id;
    store
        .store_pairing_blob(alice_id, "lookup-1", b"blob", b"salt", b"approval")
        .unwrap();
    tamper(&store, "UPDATE pairing_blobs SET expires_at = 0");
    refuse_writes(&store, "pairing_blobs", "DELETE");
    assert_eq!(
        store.reap_expired().unwrap_err(),
        "failed to reap pairing blobs: injected DELETE failure"
    );
}
