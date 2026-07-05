use mfb_repository::server;
use mfb_repository::store::Store;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process;

const USAGE: &str = "Usage: mfb-repo --dbpath <db_path> --datapath <data_path> [--listen <addr:port>]";

#[tokio::main]
async fn main() {
    let options = match parse_args(env::args().skip(1).collect()) {
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
