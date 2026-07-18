use mfb_repository::blobstore::BlobBackend;
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

<data_path> is either a local directory or an `s3://<bucket>/<prefix>` URL for
S3 (or S3-compatible) blob storage. In S3 mode blob downloads are served as a
redirect to a short-lived presigned URL. `--s3-endpoint <url>` overrides the
endpoint for S3-compatible stores (MinIO/R2/Ceph) and requires an s3:// data
path; the package metadata database (--dbpath) always stays on local disk. S3
support must be compiled in (`cargo build -p mfb_repository --features s3`).

`reanchor` is the registry-operator ceremony for a totally lost ident
(plan-23 §3.6): after out-of-band verification it binds <owner> to the given
fresh ident public key with NO chain link. Clients holding the old pin fail
hard with a re-anchor warning instead of silently following.";

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
