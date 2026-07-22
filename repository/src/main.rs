use mfb_repository::blobstore::BlobBackend;
use mfb_repository::gc;
use mfb_repository::gc::GcOptions;
use mfb_repository::server;
use mfb_repository::store::Store;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process;

const USAGE: &str = "\
Usage: mfb-repo --dbpath <db_path> --datapath <data_path> [--listen <addr:port>] [--s3-endpoint <url>]
       mfb-repo reanchor --dbpath <db_path> --datapath <data_path> --owner <owner> --ident-key <base64url>
       mfb-repo init-root --dbpath <db_path> --datapath <data_path> --registry-id <id> [--expires-days <n>]
       mfb-repo gc --dbpath <db_path> --datapath <data_path> [--s3-endpoint <url>] [--grace-hours <n>] [--delete] [--json]

<data_path> is either a local directory or an `s3://<bucket>/<prefix>` URL for
S3 (or S3-compatible) blob storage. In S3 mode blob downloads are served as a
redirect to a short-lived presigned URL. `--s3-endpoint <url>` overrides the
endpoint for S3-compatible stores (MinIO/R2/Ceph) and requires an s3:// data
path; the package metadata database (--dbpath) always stays on local disk. S3
support must be compiled in (`cargo build -p mfb_repository --features s3`).

`reanchor` is the registry-operator ceremony for a totally lost ident
(plan-23 §3.6): after out-of-band verification it binds <owner> to the given
fresh ident public key with NO chain link. Clients holding the old pin fail
hard with a re-anchor warning instead of silently following.

`gc` reclaims package blobs that no live package version references — the
orphans a `PUT /blob` leaves when a publish is abandoned between the upload and
the commit. It is a DRY RUN unless `--delete` is given, and it never touches a
blob younger than the grace period (default 24h, `--grace-hours`) or one any
live version references, including a yanked one.";

// coverage:off — the async entrypoint binds a listener / spawns the server and
// calls process::exit on every error branch; it cannot run under a unit test.
// Its pure argument parsing is covered directly below via parse_* functions.
#[tokio::main]
async fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();

    // Operator subcommand: re-anchor an owner's ident (no server needed).
    if args.first().map(String::as_str) == Some("reanchor") {
        args.remove(0);
        match parse_reanchor_args(args) {
            Ok((dbpath, datapath, owner, ident_key)) => {
                let opened = match Store::open_repository(&dbpath, &datapath) {
                    Ok(opened) => opened,
                    Err(err) => {
                        eprintln!("error: {err}");
                        process::exit(1);
                    }
                };
                let public = match mfb_repository::crypto::decode_bytes(&ident_key, "identKey") {
                    Ok(public) => public,
                    Err(err) => {
                        eprintln!("error: {err}");
                        process::exit(2);
                    }
                };
                match opened.store.reanchor_ident(&owner, &public) {
                    Ok(key) => {
                        println!(
                            "Re-anchored owner {owner} to ident fingerprint {} (no chain link).",
                            key.fingerprint
                        );
                        println!("Consumers holding the old pin will fail hard until they re-verify out-of-band.");
                    }
                    Err(err) => {
                        eprintln!("error: {err}");
                        process::exit(1);
                    }
                }
                return;
            }
            Err(err) => {
                eprintln!("error: {err}\n\n{USAGE}");
                process::exit(2);
            }
        }
    }

    // Operator subcommand: initialize the signed-metadata root of trust
    // (plan-10-C2). Generates the offline root key + online snapshot/timestamp
    // keys, signs root.json, and prints the root PRIVATE key for the operator
    // to store offline (it is never persisted on the serving host).
    if args.first().map(String::as_str) == Some("init-root") {
        args.remove(0);
        match parse_init_root_args(args) {
            Ok((dbpath, datapath, registry_id, expires_days)) => {
                let opened = match Store::open_repository(&dbpath, &datapath) {
                    Ok(opened) => opened,
                    Err(err) => {
                        eprintln!("error: {err}");
                        process::exit(1);
                    }
                };
                // `--expires-days` is operator input with no range check: a huge
                // value overflowed (debug panic, release wrap) and a negative one
                // produced an already-expired registry root (bug-276 R10).
                let Some(expires_at) = expires_days
                    .checked_mul(24 * 3600)
                    .and_then(|seconds| mfb_repository::store::now_unix().checked_add(seconds))
                    .filter(|_| expires_days > 0)
                else {
                    eprintln!(
                        "error: --expires-days must be a positive number of days that does \
                         not overflow (got {expires_days})"
                    );
                    process::exit(1);
                };
                match opened.store.init_registry_root(&registry_id, expires_at) {
                    Ok(root_private) => {
                        let config = opened
                            .store
                            .registry_config()
                            .ok()
                            .flatten()
                            .expect("registry config exists after init");
                        println!(
                            "Initialized registry `{registry_id}` root of trust (expires in {expires_days} days)."
                        );
                        println!(
                            "Root fingerprint (pin this out of band): {}",
                            mfb_repository::crypto::fingerprint(&config.root_public)
                        );
                        println!(
                            "Root PRIVATE key (STORE OFFLINE, never on the server): {}",
                            mfb_repository::crypto::encode_bytes(&root_private)
                        );
                    }
                    Err(err) => {
                        eprintln!("error: {err}");
                        process::exit(1);
                    }
                }
                return;
            }
            Err(err) => {
                eprintln!("error: {err}\n\n{USAGE}");
                process::exit(2);
            }
        }
    }

    // Operator subcommand: reclaim unreferenced package blobs (plan-49).
    // Deliberately a subcommand rather than a hook in `reap_expired`: deleting
    // package content is irreversible and has no "it will be re-created"
    // fallback, so an operator decides when it runs and can see what it would
    // do first.
    if args.first().map(String::as_str) == Some("gc") {
        args.remove(0);
        let parsed = match parse_gc_args(args) {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("error: {err}\n\n{USAGE}");
                process::exit(2);
            }
        };
        let opened = match Store::open_repository(&parsed.dbpath, &PathBuf::from(&parsed.datapath))
        {
            Ok(opened) => opened,
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(1);
            }
        };
        let blob_store = match parsed.blob_backend.into_store().await {
            Ok(blob_store) => blob_store,
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(1);
            }
        };
        let report = match gc::run(
            &opened.store,
            &blob_store,
            &parsed.options,
            mfb_repository::store::now_unix(),
        )
        .await
        {
            Ok(report) => report,
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(1);
            }
        };
        if parsed.options.json {
            println!("{}", gc::render_json(&report));
        } else {
            print!("{}", gc::render_text(&report));
        }
        // Per-blob failures do not abandon the sweep, but they must not read as
        // success to a script either.
        if report.failed() {
            process::exit(1);
        }
        return;
    }

    let options = match parse_args(args) {
        Ok(options) => options,
        Err(err) => {
            eprintln!("error: {err}\n\n{USAGE}");
            process::exit(2);
        }
    };

    let opened = match Store::open_repository(&options.dbpath, &options.datapath) {
        Ok(opened) => opened,
        Err(err) => {
            eprintln!("error: {err}");
            process::exit(1);
        }
    };

    let blob_store = match options.blob_backend.into_store().await {
        Ok(blob_store) => blob_store,
        Err(err) => {
            eprintln!("error: {err}");
            process::exit(1);
        }
    };

    if let Err(err) = server::serve(opened.store, blob_store, options.listen).await {
        eprintln!("error: {err}");
        process::exit(1);
    }
}
// coverage:on

fn parse_reanchor_args(args: Vec<String>) -> Result<(PathBuf, PathBuf, String, String), String> {
    let mut dbpath = None;
    let mut datapath = None;
    let mut owner = None;
    let mut ident_key = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dbpath" => {
                dbpath = Some(PathBuf::from(
                    iter.next().ok_or("--dbpath requires <db_path>")?,
                ))
            }
            "--datapath" => {
                datapath = Some(PathBuf::from(
                    iter.next().ok_or("--datapath requires <data_path>")?,
                ))
            }
            "--owner" => owner = Some(iter.next().ok_or("--owner requires <owner>")?),
            "--ident-key" => {
                ident_key = Some(iter.next().ok_or("--ident-key requires <base64url>")?)
            }
            _ => return Err(format!("unknown option '{arg}'")),
        }
    }
    Ok((
        dbpath.ok_or("--dbpath is required")?,
        datapath.ok_or("--datapath is required")?,
        owner.ok_or("--owner is required")?,
        ident_key.ok_or("--ident-key is required")?,
    ))
}

/// A parsed `mfb-repo gc` invocation.
#[derive(Debug)]
struct GcInvocation {
    dbpath: PathBuf,
    /// Kept as the raw string: it may be an `s3://…` URL, which is not a path.
    datapath: String,
    blob_backend: BlobBackend,
    options: GcOptions,
}

/// Parse `mfb-repo gc`. `--grace-hours` is validated here — at the operator
/// boundary — so an out-of-range value is refused before anything opens the
/// store, and `0` is refused outright (plan-49 §4.5): it removes the only
/// protection the sweep has against an in-flight publish.
fn parse_gc_args(args: Vec<String>) -> Result<GcInvocation, String> {
    let mut dbpath = None;
    let mut datapath: Option<String> = None;
    let mut s3_endpoint: Option<String> = None;
    let mut options = GcOptions::default();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dbpath" => {
                dbpath = Some(PathBuf::from(
                    iter.next().ok_or("--dbpath requires <db_path>")?,
                ))
            }
            "--datapath" => datapath = Some(iter.next().ok_or("--datapath requires <data_path>")?),
            "--s3-endpoint" => {
                s3_endpoint = Some(iter.next().ok_or("--s3-endpoint requires <url>")?)
            }
            "--grace-hours" => {
                options.grace_hours = iter
                    .next()
                    .ok_or("--grace-hours requires <n>")?
                    .parse()
                    .map_err(|_| "--grace-hours must be an integer".to_string())?;
            }
            "--delete" => options.delete = true,
            "--json" => options.json = true,
            _ if arg.starts_with("--dbpath=") => {
                dbpath = Some(PathBuf::from(arg.trim_start_matches("--dbpath=")));
            }
            _ if arg.starts_with("--datapath=") => {
                datapath = Some(arg.trim_start_matches("--datapath=").to_string());
            }
            _ if arg.starts_with("--s3-endpoint=") => {
                s3_endpoint = Some(arg.trim_start_matches("--s3-endpoint=").to_string());
            }
            _ if arg.starts_with("--grace-hours=") => {
                options.grace_hours = arg
                    .trim_start_matches("--grace-hours=")
                    .parse()
                    .map_err(|_| "--grace-hours must be an integer".to_string())?;
            }
            _ => return Err(format!("unknown option '{arg}'")),
        }
    }
    let datapath = datapath.ok_or("--datapath is required")?;
    let blob_backend = BlobBackend::parse(&datapath, s3_endpoint)?;
    // Reject a bad grace period here rather than after the store is open.
    mfb_repository::gc::grace_seconds(options.grace_hours)?;
    Ok(GcInvocation {
        dbpath: dbpath.ok_or("--dbpath is required")?,
        datapath,
        blob_backend,
        options,
    })
}

fn parse_init_root_args(args: Vec<String>) -> Result<(PathBuf, PathBuf, String, i64), String> {
    let mut dbpath = None;
    let mut datapath = None;
    let mut registry_id = None;
    let mut expires_days = 365i64;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dbpath" => {
                dbpath = Some(PathBuf::from(
                    iter.next().ok_or("--dbpath requires <db_path>")?,
                ))
            }
            "--datapath" => {
                datapath = Some(PathBuf::from(
                    iter.next().ok_or("--datapath requires <data_path>")?,
                ))
            }
            "--registry-id" => {
                registry_id = Some(iter.next().ok_or("--registry-id requires <id>")?)
            }
            "--expires-days" => {
                expires_days = iter
                    .next()
                    .ok_or("--expires-days requires <n>")?
                    .parse()
                    .map_err(|_| "--expires-days must be an integer".to_string())?;
            }
            _ => return Err(format!("unknown option '{arg}'")),
        }
    }
    Ok((
        dbpath.ok_or("--dbpath is required")?,
        datapath.ok_or("--datapath is required")?,
        registry_id.ok_or("--registry-id is required")?,
        expires_days,
    ))
}

#[derive(Debug)]
struct Options {
    dbpath: PathBuf,
    datapath: PathBuf,
    blob_backend: BlobBackend,
    listen: SocketAddr,
}

fn parse_args(args: Vec<String>) -> Result<Options, String> {
    let mut dbpath = None;
    let mut datapath: Option<String> = None;
    let mut s3_endpoint: Option<String> = None;
    let mut listen = "127.0.0.1:7777".parse::<SocketAddr>().unwrap();
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dbpath" => {
                let Some(value) = iter.next() else {
                    return Err("--dbpath requires <db_path>".to_string());
                };
                dbpath = Some(PathBuf::from(value));
            }
            "--datapath" => {
                let Some(value) = iter.next() else {
                    return Err("--datapath requires <data_path>".to_string());
                };
                datapath = Some(value);
            }
            "--s3-endpoint" => {
                let Some(value) = iter.next() else {
                    return Err("--s3-endpoint requires <url>".to_string());
                };
                s3_endpoint = Some(value);
            }
            "--listen" => {
                let Some(value) = iter.next() else {
                    return Err("--listen requires <addr:port>".to_string());
                };
                listen = value
                    .parse()
                    .map_err(|_| format!("invalid listen address '{value}'"))?;
            }
            _ if arg.starts_with("--dbpath=") => {
                dbpath = Some(PathBuf::from(arg.trim_start_matches("--dbpath=")));
            }
            _ if arg.starts_with("--datapath=") => {
                datapath = Some(arg.trim_start_matches("--datapath=").to_string());
            }
            _ if arg.starts_with("--s3-endpoint=") => {
                s3_endpoint = Some(arg.trim_start_matches("--s3-endpoint=").to_string());
            }
            _ if arg.starts_with("--listen=") => {
                let value = arg.trim_start_matches("--listen=");
                listen = value
                    .parse()
                    .map_err(|_| format!("invalid listen address '{value}'"))?;
            }
            _ => return Err(format!("unknown option '{arg}'")),
        }
    }

    let Some(dbpath) = dbpath else {
        return Err("--dbpath is required".to_string());
    };
    let Some(datapath) = datapath else {
        return Err("--datapath is required".to_string());
    };
    let blob_backend = BlobBackend::parse(&datapath, s3_endpoint)?;
    Ok(Options {
        dbpath,
        datapath: PathBuf::from(datapath),
        blob_backend,
        listen,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
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
        let invocation =
            parse_gc_args(args(&["--dbpath", "/db", "--datapath", "--delete"])).unwrap();
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
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
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

    /// The init-root subcommand's core store operation: initialize the root of
    /// trust and confirm the config + returned private key.
    #[test]
    fn init_root_operation_creates_config_and_returns_root_private_key() {
        let temp = tempfile::tempdir().unwrap();
        let opened =
            Store::open_repository(&temp.path().join("meta.db"), &temp.path().join("data"))
                .unwrap();
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
}
