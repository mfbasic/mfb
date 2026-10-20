use super::*;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn init_root_expires_at_guards_range_and_overflow() {
    // A positive day count adds to `now`.
    assert_eq!(init_root_expires_at(365, 1000), Ok(1000 + 365 * 24 * 3600));
    // Zero and negative are rejected (bug-276 R10: no already-expired root).
    let err = init_root_expires_at(0, 1000).unwrap_err();
    assert!(err.contains("must be a positive"), "{err}");
    assert!(init_root_expires_at(-5, 1000).is_err());
    // A day count that overflows the seconds multiply is rejected.
    assert!(init_root_expires_at(i64::MAX, 1000).is_err());
    // Overflow of the `now + seconds` add is rejected too.
    assert!(init_root_expires_at(1, i64::MAX).is_err());
}

#[test]
fn parse_args_reads_space_and_equals_forms_and_defaults_listen() {
    let options = parse_args(args(&["--dbpath", "/db", "--datapath", "/data"])).unwrap();
    assert_eq!(options.dbpath, PathBuf::from("/db"));
    assert_eq!(options.datapath, PathBuf::from("/data"));
    assert_eq!(options.listen.to_string(), "127.0.0.1:7777");

    let options = parse_args(args(&[
        "--dbpath=/db2",
        "--datapath=/data2",
        "--listen=0.0.0.0:9000",
    ]))
    .unwrap();
    assert_eq!(options.dbpath, PathBuf::from("/db2"));
    assert_eq!(options.datapath, PathBuf::from("/data2"));
    assert_eq!(options.listen.to_string(), "0.0.0.0:9000");

    // The space form of --listen parses too.
    let options = parse_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/data",
        "--listen",
        "127.0.0.1:1234",
    ]))
    .unwrap();
    assert_eq!(options.listen.to_string(), "127.0.0.1:1234");
}

#[test]
fn parse_args_rejects_missing_values_bad_listen_and_unknown_options() {
    assert!(parse_args(args(&["--dbpath"]))
        .unwrap_err()
        .contains("--dbpath requires"));
    assert!(parse_args(args(&["--dbpath", "/db", "--datapath"]))
        .unwrap_err()
        .contains("--datapath requires"));
    assert!(parse_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/data",
        "--listen",
    ]))
    .unwrap_err()
    .contains("--listen requires"));
    assert!(parse_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/data",
        "--listen",
        "not-an-addr",
    ]))
    .unwrap_err()
    .contains("invalid listen address"));
    assert!(
        parse_args(args(&["--dbpath=/db", "--datapath=/data", "--listen=bad",]))
            .unwrap_err()
            .contains("invalid listen address")
    );
    assert!(parse_args(args(&["--datapath", "/data"]))
        .unwrap_err()
        .contains("--dbpath is required"));
    assert!(parse_args(args(&["--dbpath", "/db"]))
        .unwrap_err()
        .contains("--datapath is required"));
    assert!(parse_args(args(&["--bogus"]))
        .unwrap_err()
        .contains("unknown option"));
}

#[test]
fn parse_args_reads_s3_datapath_and_endpoint() {
    let options = parse_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "s3://my-bucket/packages",
        "--s3-endpoint",
        "https://minio.example:9000",
    ]))
    .unwrap();
    assert_eq!(
        options.blob_backend,
        BlobBackend::S3 {
            bucket: "my-bucket".to_string(),
            prefix: "packages/".to_string(),
            endpoint: Some("https://minio.example:9000".to_string()),
        }
    );

    // Real-AWS form: s3:// with no endpoint is valid.
    let options = parse_args(args(&["--dbpath=/db", "--datapath=s3://my-bucket"])).unwrap();
    assert_eq!(
        options.blob_backend,
        BlobBackend::S3 {
            bucket: "my-bucket".to_string(),
            prefix: String::new(),
            endpoint: None,
        }
    );
}

#[test]
fn parse_args_rejects_s3_endpoint_without_s3_datapath() {
    let err = parse_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/local/data",
        "--s3-endpoint",
        "https://minio.example",
    ]))
    .unwrap_err();
    assert!(err.contains("--s3-endpoint requires an s3://"));

    assert!(parse_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/d",
        "--s3-endpoint"
    ]))
    .unwrap_err()
    .contains("--s3-endpoint requires <url>"));
}

#[test]
fn parse_reanchor_args_reads_all_fields_and_reports_missing() {
    let (dbpath, datapath, owner, ident_key) = parse_reanchor_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/data",
        "--owner",
        "alice",
        "--ident-key",
        "KEY",
    ]))
    .unwrap();
    assert_eq!(dbpath, PathBuf::from("/db"));
    assert_eq!(datapath, PathBuf::from("/data"));
    assert_eq!(owner, "alice");
    assert_eq!(ident_key, "KEY");

    assert!(parse_reanchor_args(args(&["--owner", "alice"]))
        .unwrap_err()
        .contains("--dbpath is required"));
    assert!(parse_reanchor_args(args(&["--dbpath"]))
        .unwrap_err()
        .contains("--dbpath requires"));
    assert!(parse_reanchor_args(args(&["--nope", "x"]))
        .unwrap_err()
        .contains("unknown option"));
}

#[test]
fn parse_init_root_args_reads_fields_defaults_and_validates_expiry() {
    let (dbpath, datapath, registry_id, expires_days) = parse_init_root_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/data",
        "--registry-id",
        "reg-1",
    ]))
    .unwrap();
    assert_eq!(dbpath, PathBuf::from("/db"));
    assert_eq!(datapath, PathBuf::from("/data"));
    assert_eq!(registry_id, "reg-1");
    assert_eq!(expires_days, 365); // default

    let (_d, _p, _r, expires_days) = parse_init_root_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/data",
        "--registry-id",
        "reg-1",
        "--expires-days",
        "30",
    ]))
    .unwrap();
    assert_eq!(expires_days, 30);

    assert!(parse_init_root_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/data",
        "--registry-id",
        "reg-1",
        "--expires-days",
        "notnum",
    ]))
    .unwrap_err()
    .contains("--expires-days must be an integer"));
    assert!(parse_init_root_args(args(&["--dbpath"]))
        .unwrap_err()
        .contains("--dbpath requires"));
    assert!(parse_init_root_args(args(&["--registry-id", "reg"]))
        .unwrap_err()
        .contains("--dbpath is required"));
    assert!(parse_init_root_args(args(&["--nope", "x"]))
        .unwrap_err()
        .contains("unknown option"));
}

#[test]
fn parse_backfill_args_reads_both_forms_and_requires_both_paths() {
    let (dbpath, datapath, backend) =
        parse_backfill_args(args(&["--dbpath", "/db", "--datapath", "/data"])).unwrap();
    assert_eq!(dbpath, PathBuf::from("/db"));
    assert_eq!(datapath, "/data");
    assert_eq!(backend, BlobBackend::Local(PathBuf::from("/data")));

    // The `--flag=value` form parses identically.
    let (dbpath, datapath, _backend) =
        parse_backfill_args(args(&["--dbpath=/db2", "--datapath=/data2"])).unwrap();
    assert_eq!(dbpath, PathBuf::from("/db2"));
    assert_eq!(datapath, "/data2");

    // Both paths are required, and an unknown option is refused rather
    // than ignored — an operator ceremony must not silently do something
    // other than what was typed.
    assert!(parse_backfill_args(args(&["--datapath", "/data"]))
        .unwrap_err()
        .contains("--dbpath is required"));
    assert!(parse_backfill_args(args(&["--dbpath", "/db"]))
        .unwrap_err()
        .contains("--datapath is required"));
    assert!(
        parse_backfill_args(args(&["--dbpath", "/db", "--datapath", "/d", "--delete"]))
            .unwrap_err()
            .contains("unknown option")
    );
    assert!(parse_backfill_args(args(&["--dbpath"]))
        .unwrap_err()
        .contains("--dbpath requires"));
    assert!(parse_backfill_args(args(&["--datapath"]))
        .unwrap_err()
        .contains("--datapath requires"));
    assert!(parse_backfill_args(args(&["--s3-endpoint"]))
        .unwrap_err()
        .contains("--s3-endpoint requires"));
    // An S3 endpoint against a local data path is refused by the shared
    // backend parser, the same way `gc` refuses it.
    assert!(parse_backfill_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/data",
        "--s3-endpoint=https://example.invalid"
    ]))
    .is_err());
}

#[test]
fn parse_gc_args_defaults_to_a_dry_run_with_a_24h_grace() {
    let invocation = parse_gc_args(args(&["--dbpath", "/db", "--datapath", "/data"])).unwrap();
    assert_eq!(invocation.dbpath, PathBuf::from("/db"));
    assert_eq!(invocation.datapath, "/data");
    assert_eq!(
        invocation.blob_backend,
        BlobBackend::Local(PathBuf::from("/data"))
    );
    assert_eq!(
        invocation.options,
        GcOptions {
            grace_hours: 24,
            delete: false,
            json: false,
        },
        "gc must never delete unless asked"
    );

    let invocation = parse_gc_args(args(&[
        "--dbpath=/db",
        "--datapath=/data",
        "--grace-hours=72",
        "--delete",
        "--json",
    ]))
    .unwrap();
    assert_eq!(invocation.options.grace_hours, 72);
    assert!(invocation.options.delete);
    assert!(invocation.options.json);
}

/// An `s3://` datapath works in `gc` too — the metadata DB stays local
/// while the blobs live in the bucket (plan-49 §4.5).
#[test]
fn parse_gc_args_accepts_an_s3_datapath() {
    let invocation = parse_gc_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "s3://bucket/pkgs",
        "--s3-endpoint",
        "https://minio.example:9000",
    ]))
    .unwrap();
    assert_eq!(
        invocation.blob_backend,
        BlobBackend::S3 {
            bucket: "bucket".to_string(),
            prefix: "pkgs/".to_string(),
            endpoint: Some("https://minio.example:9000".to_string()),
        }
    );
}

/// `--grace-hours 0` is refused at the argument boundary, before anything
/// opens the store. It is the only thing protecting an in-flight publish
/// from the sweep (plan-49 §3.1), so it is not a value an operator can
/// stumble into.
#[test]
fn parse_gc_args_refuses_a_zero_or_bad_grace_period() {
    for grace in ["0", "-5"] {
        let err = parse_gc_args(args(&[
            "--dbpath",
            "/db",
            "--datapath",
            "/data",
            "--grace-hours",
            grace,
        ]))
        .unwrap_err();
        assert!(err.contains("must be a positive"), "{err}");
    }
    assert!(parse_gc_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/data",
        "--grace-hours",
        "notnum",
    ]))
    .unwrap_err()
    .contains("must be an integer"));
    assert!(parse_gc_args(args(&["--dbpath", "/db"]))
        .unwrap_err()
        .contains("--datapath is required"));
    assert!(parse_gc_args(args(&["--datapath", "/data"]))
        .unwrap_err()
        .contains("--dbpath is required"));
    assert!(parse_gc_args(args(&["--grace-hours"]))
        .unwrap_err()
        .contains("--grace-hours requires"));
    assert!(parse_gc_args(args(&["--nope"]))
        .unwrap_err()
        .contains("unknown option"));
}

/// Every `reanchor` flag that takes a value must reject a trailing bare
/// flag rather than silently binding the *next* flag as its value — an
/// operator typo here would otherwise re-anchor an owner named
/// `--ident-key`.
#[test]
fn parse_reanchor_args_rejects_every_flag_missing_its_value() {
    for (flag, expected) in [
        ("--dbpath", "--dbpath requires <db_path>"),
        ("--datapath", "--datapath requires <data_path>"),
        ("--owner", "--owner requires <owner>"),
        ("--ident-key", "--ident-key requires <base64url>"),
    ] {
        assert_eq!(parse_reanchor_args(args(&[flag])).unwrap_err(), expected);
    }
}

/// `reanchor` binds an owner to a brand-new ident with no chain link, so a
/// missing field must never be defaulted — each one is reported by name.
#[test]
fn parse_reanchor_args_names_each_missing_required_flag() {
    let complete = [
        "--dbpath",
        "/db",
        "--datapath",
        "/data",
        "--owner",
        "alice",
        "--ident-key",
        "KEY",
    ];
    for (drop_flag, expected) in [
        ("--dbpath", "--dbpath is required"),
        ("--datapath", "--datapath is required"),
        ("--owner", "--owner is required"),
        ("--ident-key", "--ident-key is required"),
    ] {
        let mut kept: Vec<&str> = Vec::new();
        for pair in complete.chunks(2) {
            if pair[0] != drop_flag {
                kept.extend_from_slice(pair);
            }
        }
        assert_eq!(
            parse_reanchor_args(args(&kept)).unwrap_err(),
            expected,
            "dropping {drop_flag}"
        );
    }
}

/// The same bare-flag rule for `init-root`. `--expires-days` is the one
/// with a default (365), so a bare `--expires-days` must still be an error
/// rather than quietly falling back to the default.
#[test]
fn parse_init_root_args_rejects_every_flag_missing_its_value() {
    for (flag, expected) in [
        ("--dbpath", "--dbpath requires <db_path>"),
        ("--datapath", "--datapath requires <data_path>"),
        ("--registry-id", "--registry-id requires <id>"),
        ("--expires-days", "--expires-days requires <n>"),
    ] {
        assert_eq!(parse_init_root_args(args(&[flag])).unwrap_err(), expected);
    }
}

#[test]
fn parse_init_root_args_names_each_missing_required_flag() {
    assert_eq!(
        parse_init_root_args(args(&["--dbpath", "/db", "--registry-id", "reg"])).unwrap_err(),
        "--datapath is required"
    );
    assert_eq!(
        parse_init_root_args(args(&["--dbpath", "/db", "--datapath", "/data"])).unwrap_err(),
        "--registry-id is required"
    );
}

/// `gc --delete` is irreversible, so every value-taking flag must fail
/// loudly when its value is missing rather than shifting the argument list
/// (a shifted `--delete` would be consumed as a path and the sweep would
/// run in a mode the operator did not ask for).
#[test]
fn parse_gc_args_rejects_every_flag_missing_its_value() {
    for (flag, expected) in [
        ("--dbpath", "--dbpath requires <db_path>"),
        ("--datapath", "--datapath requires <data_path>"),
        ("--s3-endpoint", "--s3-endpoint requires <url>"),
        ("--grace-hours", "--grace-hours requires <n>"),
    ] {
        assert_eq!(parse_gc_args(args(&[flag])).unwrap_err(), expected);
    }

    // A value-position flag is consumed as the value, not re-interpreted:
    // `--datapath --delete` yields a (nonsense) data path and, critically,
    // leaves the sweep in dry-run mode rather than enabling deletion.
    let invocation = parse_gc_args(args(&["--dbpath", "/db", "--datapath", "--delete"])).unwrap();
    assert_eq!(invocation.datapath, "--delete");
    assert!(
        !invocation.options.delete,
        "a swallowed --delete must not enable deletion"
    );
}

/// The `--flag=value` spellings must be equivalent to the space form for
/// `gc` too, including the S3 endpoint and the grace period.
#[test]
fn parse_gc_args_reads_the_equals_form_of_every_flag() {
    let invocation = parse_gc_args(args(&[
        "--dbpath=/db",
        "--datapath=s3://bucket/pkgs",
        "--s3-endpoint=https://minio.example:9000",
        "--grace-hours=48",
    ]))
    .unwrap();
    assert_eq!(invocation.dbpath, PathBuf::from("/db"));
    assert_eq!(invocation.datapath, "s3://bucket/pkgs");
    assert_eq!(
        invocation.blob_backend,
        BlobBackend::S3 {
            bucket: "bucket".to_string(),
            prefix: "pkgs/".to_string(),
            endpoint: Some("https://minio.example:9000".to_string()),
        }
    );
    assert_eq!(invocation.options.grace_hours, 48);

    assert!(parse_gc_args(args(&[
        "--dbpath=/db",
        "--datapath=/d",
        "--grace-hours=notnum"
    ]))
    .unwrap_err()
    .contains("--grace-hours must be an integer"));
}

/// `--s3-endpoint` only means anything against an `s3://` data path; `gc`
/// rejects the mismatch at the argument boundary, exactly as the server
/// path does, so a typo cannot silently sweep the local directory instead.
#[test]
fn parse_gc_args_rejects_an_s3_endpoint_without_an_s3_datapath() {
    let err = parse_gc_args(args(&[
        "--dbpath",
        "/db",
        "--datapath",
        "/local/data",
        "--s3-endpoint",
        "https://minio.example",
    ]))
    .unwrap_err();
    assert!(err.contains("--s3-endpoint requires an s3://"), "{err}");

    // The equals form is validated identically.
    let err = parse_gc_args(args(&[
        "--dbpath=/db",
        "--datapath=/local/data",
        "--s3-endpoint=https://minio.example",
    ]))
    .unwrap_err();
    assert!(err.contains("--s3-endpoint requires an s3://"), "{err}");
}

/// The server path's `--s3-endpoint=` spelling must reach `BlobBackend`
/// just like the space form — otherwise an S3 deployment configured with
/// `=` spellings would silently talk to real AWS instead of its MinIO/R2
/// endpoint.
#[test]
fn parse_args_reads_the_equals_form_of_s3_endpoint() {
    let options = parse_args(args(&[
        "--dbpath=/db",
        "--datapath=s3://my-bucket/packages",
        "--s3-endpoint=https://minio.example:9000",
    ]))
    .unwrap();
    assert_eq!(
        options.blob_backend,
        BlobBackend::S3 {
            bucket: "my-bucket".to_string(),
            prefix: "packages/".to_string(),
            endpoint: Some("https://minio.example:9000".to_string()),
        }
    );
    assert_eq!(options.datapath, PathBuf::from("s3://my-bucket/packages"));
}

/// The reanchor subcommand's core store operation (what `main` runs after
/// parsing) — decode the ident key and re-anchor the owner.
#[test]
fn reanchor_operation_binds_a_fresh_ident() {
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    // Register alice with proofs so an ident key exists to re-anchor.
    let (auth_public, auth_private) = mfb_repository::crypto::generate_keypair();
    let (ident_public, ident_private) = mfb_repository::crypto::generate_keypair();
    use mfb_repository::crypto;
    let auth_proof = crypto::sign(
        &auth_private,
        &crypto::registration_message(crypto::ROLE_AUTH, "alice", &auth_public),
    )
    .unwrap();
    let ident_proof = crypto::sign(
        &ident_private,
        &crypto::registration_message(crypto::ROLE_IDENT, "alice", &ident_public),
    )
    .unwrap();
    opened
        .store
        .register_owner(
            "alice",
            &auth_public,
            &auth_proof,
            &ident_public,
            &ident_proof,
        )
        .unwrap();

    let (fresh_public, _fresh_private) = crypto::generate_keypair();
    let ident_key = crypto::encode_bytes(&fresh_public);
    let decoded = crypto::decode_bytes(&ident_key, "identKey").unwrap();
    let key = opened.store.reanchor_ident("alice", &decoded).unwrap();
    assert_eq!(key.fingerprint, crypto::fingerprint(&fresh_public));
}

/// bug-584: `renew-root` parses the init-root fields plus the offline key
/// file, and the key is a PATH — never an argument value that would put the
/// offline root private key in the process table.
#[test]
fn parse_renew_root_args_requires_the_root_key_file() {
    let (dbpath, datapath, registry_id, expires_days, root_key_file) =
        parse_renew_root_args(args(&[
            "--dbpath",
            "/db",
            "--datapath",
            "/data",
            "--registry-id",
            "reg-1",
            "--root-key-file",
            "/keys/root",
            "--expires-days",
            "30",
        ]))
        .unwrap();
    assert_eq!(dbpath, PathBuf::from("/db"));
    assert_eq!(datapath, PathBuf::from("/data"));
    assert_eq!(registry_id, "reg-1");
    assert_eq!(expires_days, 30);
    assert_eq!(root_key_file, PathBuf::from("/keys/root"));

    assert_eq!(
        parse_renew_root_args(args(&[
            "--dbpath",
            "/db",
            "--datapath",
            "/data",
            "--registry-id",
            "reg-1",
        ]))
        .unwrap_err(),
        "--root-key-file is required",
    );
    assert_eq!(
        parse_renew_root_args(args(&["--root-key-file"])).unwrap_err(),
        "--root-key-file requires <path>",
    );
    // The shared fields keep their own diagnostics.
    assert_eq!(
        parse_renew_root_args(args(&["--root-key-file", "/keys/root"])).unwrap_err(),
        "--dbpath is required",
    );
    assert!(parse_renew_root_args(args(&["--nope", "x"]))
        .unwrap_err()
        .contains("unknown option"));
}

/// bug-584: the key file holds exactly what `init-root` printed; trailing
/// whitespace is tolerated, a missing or malformed file is a clear error.
#[test]
fn read_root_key_file_round_trips_what_init_root_printed() {
    use mfb_repository::crypto;
    let temp = tempfile::tempdir().unwrap();
    let (_public, private) = crypto::generate_keypair();
    let path = temp.path().join("root.key");
    std::fs::write(&path, format!("{}\n", crypto::encode_bytes(&private))).unwrap();
    assert_eq!(read_root_key_file(&path).unwrap(), private);

    let missing = temp.path().join("nope.key");
    assert!(read_root_key_file(&missing)
        .unwrap_err()
        .contains("failed to read root key file"));
    let garbage = temp.path().join("garbage.key");
    std::fs::write(&garbage, "not base64url!!").unwrap();
    assert!(read_root_key_file(&garbage)
        .unwrap_err()
        .contains("root key"));
}

/// bug-584 (positive pin at the CLI's store layer): the operator's honest
/// sequence — initialize once, renew later with the key that was printed —
/// works, while a second `init-root` is refused instead of silently
/// replacing the anchor.
#[test]
fn renew_root_operation_keeps_the_anchor_and_a_second_init_is_refused() {
    use mfb_repository::crypto;
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    let now = mfb_repository::store::now_unix();
    let root_private = opened
        .store
        .init_registry_root("reg-1", init_root_expires_at(365, now).unwrap())
        .unwrap();
    let anchor = opened.store.registry_config().unwrap().unwrap().root_public;

    assert!(opened
        .store
        .init_registry_root("reg-1", init_root_expires_at(365, now).unwrap())
        .unwrap_err()
        .contains("already initialized"));

    let key_path = temp.path().join("root.key");
    std::fs::write(&key_path, crypto::encode_bytes(&root_private)).unwrap();
    let version = opened
        .store
        .renew_registry_root(
            "reg-1",
            init_root_expires_at(30, now).unwrap(),
            &read_root_key_file(&key_path).unwrap(),
        )
        .unwrap();
    assert_eq!(version, 2);
    assert_eq!(
        opened.store.registry_config().unwrap().unwrap().root_public,
        anchor
    );
}

/// The init-root subcommand's core store operation: initialize the root of
/// trust and confirm the config + returned private key.
#[test]
fn init_root_operation_creates_config_and_returns_root_private_key() {
    let temp = tempfile::tempdir().unwrap();
    let opened =
        Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data")).unwrap();
    let expires_at = mfb_repository::store::now_unix() + 365 * 24 * 3600;
    let root_private = opened
        .store
        .init_registry_root("reg-1", expires_at)
        .unwrap();
    assert_eq!(root_private.len(), mfb_repository::crypto::PRIVATE_KEY_LEN);
    let config = opened.store.registry_config().unwrap().unwrap();
    assert_eq!(config.registry_id, "reg-1");
    // The returned private key derives the config's root public key.
    assert_eq!(
        mfb_repository::crypto::public_from_private(&root_private).unwrap(),
        config.root_public
    );
}
