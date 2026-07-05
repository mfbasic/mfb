use mfb_repository::server;
use mfb_repository::store::Store;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process;

const USAGE: &str = "\
Usage: mfb-repo --dbpath <db_path> --datapath <data_path> [--listen <addr:port>]
       mfb-repo reanchor --dbpath <db_path> --datapath <data_path> --owner <owner> --ident-key <base64url>
       mfb-repo init-root --dbpath <db_path> --datapath <data_path> --registry-id <id> [--expires-days <n>]

`reanchor` is the registry-operator ceremony for a totally lost ident
(plan-23 §3.6): after out-of-band verification it binds <owner> to the given
fresh ident public key with NO chain link. Clients holding the old pin fail
hard with a re-anchor warning instead of silently following.";

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
                let expires_at = mfb_repository::store::now_unix() + expires_days * 24 * 3600;
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

    if let Err(err) = server::serve(opened.store, opened.packages_dir, options.listen).await {
        eprintln!("error: {err}");
        process::exit(1);
    }
}

fn parse_reanchor_args(
    args: Vec<String>,
) -> Result<(PathBuf, PathBuf, String, String), String> {
    let mut dbpath = None;
    let mut datapath = None;
    let mut owner = None;
    let mut ident_key = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dbpath" => dbpath = Some(PathBuf::from(iter.next().ok_or("--dbpath requires <db_path>")?)),
            "--datapath" => datapath = Some(PathBuf::from(iter.next().ok_or("--datapath requires <data_path>")?)),
            "--owner" => owner = Some(iter.next().ok_or("--owner requires <owner>")?),
            "--ident-key" => ident_key = Some(iter.next().ok_or("--ident-key requires <base64url>")?),
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
            "--dbpath" => dbpath = Some(PathBuf::from(iter.next().ok_or("--dbpath requires <db_path>")?)),
            "--datapath" => datapath = Some(PathBuf::from(iter.next().ok_or("--datapath requires <data_path>")?)),
            "--registry-id" => registry_id = Some(iter.next().ok_or("--registry-id requires <id>")?),
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

struct Options {
    dbpath: PathBuf,
    datapath: PathBuf,
    listen: SocketAddr,
}

fn parse_args(args: Vec<String>) -> Result<Options, String> {
    let mut dbpath = None;
    let mut datapath = None;
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
                datapath = Some(PathBuf::from(value));
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
                datapath = Some(PathBuf::from(arg.trim_start_matches("--datapath=")));
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
    Ok(Options { dbpath, datapath, listen })
}
