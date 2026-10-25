use mfb_repository::backfill;
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
       mfb-repo renew-root --dbpath <db_path> --datapath <data_path> --registry-id <id> --root-key-file <path> [--expires-days <n>]
       mfb-repo reanchor-root --dbpath <db_path> --datapath <data_path> --registry-id <id> [--expires-days <n>]
       mfb-repo gc --dbpath <db_path> --datapath <data_path> [--s3-endpoint <url>] [--grace-hours <n>] [--delete] [--json]
       mfb-repo backfill-metadata --dbpath <db_path> --datapath <data_path> [--s3-endpoint <url>]

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

`init-root` is ONE-TIME initialization and refuses to run against a registry
that already has a root of trust (bug-584): overwriting it would swap the anchor
every pinned client verifies against, which from the client's side is
indistinguishable from a takeover.

`renew-root` is the routine ceremony: it re-signs root.json with a bumped
version, a fresh expiry and freshly generated online snapshot/timestamp keys,
under the SAME offline root key, which it reads from --root-key-file (the
base64url key `init-root` printed, one line, nothing else) and never persists.
The root fingerprint does not change, so already-pinned clients keep verifying
with no out-of-band step.

`reanchor-root` is the recovery ceremony for a LOST offline root key: it mints a
new root key and replaces the anchor. Every client that pinned the old
fingerprint fails hard until it re-pins out of band, so it is a separate,
explicitly selected, transparency-logged mode.

`gc` reclaims package blobs that no live package version references — the
orphans a `PUT /blob` leaves when a publish is abandoned between the upload and
the commit. It is a DRY RUN unless `--delete` is given, and it never touches a
blob younger than the grace period (default 24h, `--grace-hours`) or one any
live version references, including a yanked one.

`backfill-metadata` populates the author, url, native-target and documentation
records for versions published before the server recorded them, by re-parsing
each stored package blob. No republish and no publisher action is needed. It is
idempotent, skips (rather than aborts on) a blob it cannot parse, and reports a
version whose header and signed manifest disagree under its own count — that
is a transparency finding, not a parse failure, and it is left untouched. A doc
section that is present but does not decode is likewise counted on its own line
and not recorded. It exits non-zero if anything was skipped.";

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
                let expires_at =
                    match init_root_expires_at(expires_days, mfb_repository::store::now_unix()) {
                        Ok(at) => at,
                        Err(err) => {
                            eprintln!("error: {err}");
                            process::exit(1);
                        }
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

    // Operator subcommand: renew the root of trust under the SAME offline root
    // key (bug-584). Keeps the pinned anchor; rotates the delegated online keys
    // and the root expiry. The offline key is read from a file rather than an
    // argument so it never appears in the process table.
    if args.first().map(String::as_str) == Some("renew-root") {
        args.remove(0);
        let (dbpath, datapath, registry_id, expires_days, root_key_file) =
            match parse_renew_root_args(args) {
                Ok(parsed) => parsed,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    process::exit(2);
                }
            };
        let root_private = match read_root_key_file(&root_key_file) {
            Ok(key) => key,
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(2);
            }
        };
        let opened = match Store::open_repository(&dbpath, &datapath) {
            Ok(opened) => opened,
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(1);
            }
        };
        let expires_at = match init_root_expires_at(expires_days, mfb_repository::store::now_unix())
        {
            Ok(at) => at,
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(1);
            }
        };
        match opened
            .store
            .renew_registry_root(&registry_id, expires_at, &root_private)
        {
            Ok(version) => {
                let config = opened
                    .store
                    .registry_config()
                    .ok()
                    .flatten()
                    .expect("registry config exists after renewal");
                println!(
                    "Renewed registry `{registry_id}` root of trust to version {version} (expires in {expires_days} days)."
                );
                println!(
                    "Root fingerprint is UNCHANGED — pinned clients keep verifying: {}",
                    mfb_repository::crypto::fingerprint(&config.root_public)
                );
            }
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(1);
            }
        }
        return;
    }

    // Operator subcommand: replace a LOST root of trust (bug-584). Mints a new
    // offline root key; every pinned client fails hard until it re-pins.
    if args.first().map(String::as_str) == Some("reanchor-root") {
        args.remove(0);
        let (dbpath, datapath, registry_id, expires_days) = match parse_init_root_args(args) {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("error: {err}\n\n{USAGE}");
                process::exit(2);
            }
        };
        let opened = match Store::open_repository(&dbpath, &datapath) {
            Ok(opened) => opened,
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(1);
            }
        };
        let expires_at = match init_root_expires_at(expires_days, mfb_repository::store::now_unix())
        {
            Ok(at) => at,
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(1);
            }
        };
        match opened
            .store
            .reanchor_registry_root(&registry_id, expires_at)
        {
            Ok(root_private) => {
                let config = opened
                    .store
                    .registry_config()
                    .ok()
                    .flatten()
                    .expect("registry config exists after re-anchor");
                println!(
                    "RE-ANCHORED registry `{registry_id}` root of trust (expires in {expires_days} days)."
                );
                println!(
                    "New root fingerprint (publish it; every pinned client must re-pin out of band): {}",
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

    // Operator subcommand: populate the plan-61-A metadata for versions
    // published before the server captured it.
    if args.first().map(String::as_str) == Some("backfill-metadata") {
        args.remove(0);
        let (dbpath, datapath, blob_backend) = match parse_backfill_args(args) {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("error: {err}\n\n{USAGE}");
                process::exit(2);
            }
        };
        // Same shape as `gc`: the raw datapath goes to the store (which only
        // uses it to place the packages dir) and the parsed backend does the
        // blob I/O, so an `s3://` path needs no special case here.
        let opened = match Store::open_repository(&dbpath, &PathBuf::from(&datapath)) {
            Ok(opened) => opened,
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(1);
            }
        };
        let blob_store = match blob_backend.into_store().await {
            Ok(blob_store) => blob_store,
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(1);
            }
        };
        let report = match backfill::run(&opened.store, &blob_store).await {
            Ok(report) => report,
            Err(err) => {
                eprintln!("error: {err}");
                process::exit(1);
            }
        };
        print!("{}", backfill::render_text(&report));
        // Per-version skips do not abandon the sweep, but they must not read as
        // a clean run to a script either — same contract as `gc`.
        if report.skipped() {
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

    if let Err(err) =
        server::serve(opened.store, blob_store, options.listen, shutdown_signal()).await
    {
        eprintln!("error: {err}");
        process::exit(1);
    }
}

/// Resolves when the process is asked to stop: SIGINT (`Ctrl-C`) or, on Unix,
/// SIGTERM, which is what a container runtime or service manager sends. `serve`
/// then stops accepting and lets in-flight requests finish before returning.
///
/// A signal that cannot be listened for is reported and never fires, rather
/// than resolving at once, which would shut a healthy server down at startup.
async fn shutdown_signal() {
    let interrupt = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            eprintln!("warning: cannot listen for SIGINT ({err})");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(err) => {
                eprintln!("warning: cannot listen for SIGTERM ({err})");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
    eprintln!("mfb-repo: shutting down; finishing in-flight requests");
}
// coverage:on

/// Absolute expiry timestamp for a new registry root of trust.
///
/// `--expires-days` is operator input with no range check: a huge value
/// overflowed (debug panic, release wrap) and a non-positive one produced an
/// already-expired root (bug-276 R10). Rejects both. Pure so it is unit-tested
/// apart from the integration-only `main`; `main` supplies `now = now_unix()`.
fn init_root_expires_at(expires_days: i64, now: i64) -> Result<i64, String> {
    expires_days
        .checked_mul(24 * 3600)
        .and_then(|seconds| now.checked_add(seconds))
        .filter(|_| expires_days > 0)
        .ok_or_else(|| {
            format!(
                "--expires-days must be a positive number of days that does \
                 not overflow (got {expires_days})"
            )
        })
}

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

/// Parse `mfb-repo backfill-metadata`. It takes only the two paths every
/// operator ceremony takes, plus the S3 endpoint override — the sweep has no
/// dry-run mode because it is idempotent and rewrites only columns that were
/// previously unpopulated or already derived from the same bytes.
fn parse_backfill_args(args: Vec<String>) -> Result<(PathBuf, String, BlobBackend), String> {
    let mut dbpath = None;
    let mut datapath: Option<String> = None;
    let mut s3_endpoint: Option<String> = None;
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
            _ if arg.starts_with("--dbpath=") => {
                dbpath = Some(PathBuf::from(arg.trim_start_matches("--dbpath=")));
            }
            _ if arg.starts_with("--datapath=") => {
                datapath = Some(arg.trim_start_matches("--datapath=").to_string());
            }
            _ if arg.starts_with("--s3-endpoint=") => {
                s3_endpoint = Some(arg.trim_start_matches("--s3-endpoint=").to_string());
            }
            _ => return Err(format!("unknown option '{arg}'")),
        }
    }
    let datapath = datapath.ok_or("--datapath is required")?;
    let blob_backend = BlobBackend::parse(&datapath, s3_endpoint)?;
    Ok((
        dbpath.ok_or("--dbpath is required")?,
        datapath,
        blob_backend,
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

/// `renew-root` takes the same fields as `init-root` plus the offline root key
/// file. The key is a path, never an argument value: an argument would put the
/// registry's offline root private key in the process table (bug-584).
fn parse_renew_root_args(
    args: Vec<String>,
) -> Result<(PathBuf, PathBuf, String, i64, PathBuf), String> {
    let mut rest = Vec::new();
    let mut root_key_file = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--root-key-file" {
            root_key_file = Some(PathBuf::from(
                iter.next().ok_or("--root-key-file requires <path>")?,
            ));
        } else {
            rest.push(arg);
        }
    }
    // The shared fields are parsed FIRST so an unknown option keeps its own
    // diagnostic rather than being reported as a missing key file.
    let (dbpath, datapath, registry_id, expires_days) = parse_init_root_args(rest)?;
    let root_key_file = root_key_file.ok_or("--root-key-file is required")?;
    Ok((dbpath, datapath, registry_id, expires_days, root_key_file))
}

/// Read the base64url offline root private key from `path` (the exact string
/// `init-root` printed, one line). Decoding here — rather than inside the store
/// — keeps the "malformed file" and "wrong key" diagnostics apart.
fn read_root_key_file(path: &PathBuf) -> Result<Vec<u8>, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read root key file {}: {err}", path.display()))?;
    mfb_repository::crypto::decode_bytes(raw.trim(), "root key")
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
#[path = "main/tests/mod.rs"]
mod tests;
