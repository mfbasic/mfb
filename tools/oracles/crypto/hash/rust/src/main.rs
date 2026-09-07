//! The judge for `crypto::hash` and `crypto::shake256`.
//!
//! Every algorithm here is computed by RustCrypto, not by hand. That is the
//! whole source of its authority: an oracle written by the same author who wrote
//! the implementation, by reading that implementation, ratifies it rather than
//! checking it. These crates are a different codebase maintained by people who
//! have never seen the MFBASIC core, and they are pinned exactly (see
//! `Cargo.toml`) so the thing that agreed cannot silently be replaced.
//!
//! Modes:
//!   hashref                                 list the algorithms it knows
//!   hashref run   <algo> <outlen> <in-hex>  one digest, for deriving a vector
//!   hashref batch                           stdin: "<algo> <outlen> <in-hex>"
//!                                           per line, one digest per line out
//!
//! `batch` exists because the differential harness has a few hundred cases and
//! one process per case is most of its wall clock. `run` is the same code path
//! with one line of input, kept so a human can derive a single value by hand.

use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};
use sha3::{Sha3_224, Sha3_256, Sha3_384, Sha3_512, Shake256};
use sha3::digest::{ExtendableOutput, Update, XofReader};
use std::io::{BufRead, Write};

/// The algorithm names the harness uses on the wire. These are the spellings
/// `mfb/src/main.mfb` prints; they are deliberately NOT the MFBASIC enum
/// spellings (`SHA2_256`), because the two sides agreeing on a label is not
/// something either side should be able to assume about the other.
const ALGOS: &[&str] = &[
    "sha1", "sha2-224", "sha2-256", "sha2-384", "sha2-512", "sha3-224", "sha3-256", "sha3-384",
    "sha3-512", "shake256",
];

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    // "-" is the harness's stand-in for empty: an empty field would shift every
    // later field left when the shell splits a line on spaces.
    if s == "-" {
        return Ok(Vec::new());
    }
    if s.len() % 2 != 0 {
        return Err(format!("odd-length hex: {s}"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("bad hex {s}: {e}")))
        .collect()
}

/// `out_len` is honoured only by the extendable-output function; for the fixed
/// hashes it is the caller's *claim* about the digest width, and a claim that
/// disagrees with the real width is an error rather than something to paper
/// over -- a silently truncated digest would compare equal to a truncated bug.
fn digest(algo: &str, out_len: usize, data: &[u8]) -> Result<Vec<u8>, String> {
    let fixed = |v: Vec<u8>| -> Result<Vec<u8>, String> {
        if v.len() != out_len {
            return Err(format!(
                "{algo} produces {} bytes, but {out_len} was requested",
                v.len()
            ));
        }
        Ok(v)
    };
    match algo {
        "sha1" => fixed(Sha1::digest(data).to_vec()),
        "sha2-224" => fixed(Sha224::digest(data).to_vec()),
        "sha2-256" => fixed(Sha256::digest(data).to_vec()),
        "sha2-384" => fixed(Sha384::digest(data).to_vec()),
        "sha2-512" => fixed(Sha512::digest(data).to_vec()),
        "sha3-224" => fixed(Sha3_224::digest(data).to_vec()),
        "sha3-256" => fixed(Sha3_256::digest(data).to_vec()),
        "sha3-384" => fixed(Sha3_384::digest(data).to_vec()),
        "sha3-512" => fixed(Sha3_512::digest(data).to_vec()),
        "shake256" => {
            let mut x = Shake256::default();
            x.update(data);
            let mut out = vec![0u8; out_len];
            x.finalize_xof().read(&mut out);
            Ok(out)
        }
        other => Err(format!("unknown algorithm: {other}")),
    }
}

/// Parse and answer one "<algo> <outlen> <in-hex>" request.
fn answer(line: &str) -> Result<String, String> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.len() != 3 {
        return Err(format!("expected 3 fields, got {}: {line}", f.len()));
    }
    let out_len: usize = f[1].parse().map_err(|e| format!("bad length {}: {e}", f[1]))?;
    Ok(hex(&digest(f[0], out_len, &unhex(f[2])?)?))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => {
            if args.len() != 5 {
                eprintln!("usage: hashref run <algo> <outlen> <input-hex>");
                std::process::exit(2);
            }
            match answer(&format!("{} {} {}", args[2], args[3], args[4])) {
                Ok(d) => println!("{d}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Some("batch") => {
            // One digest per input line, in order. The caller checks that the
            // number of lines back equals the number sent -- a reference that
            // dies halfway must not look like a short but passing run.
            let stdin = std::io::stdin();
            let mut out = std::io::BufWriter::new(std::io::stdout());
            for (n, line) in stdin.lock().lines().enumerate() {
                let line = match line {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("stdin line {}: {e}", n + 1);
                        std::process::exit(1);
                    }
                };
                if line.trim().is_empty() {
                    continue;
                }
                match answer(&line) {
                    Ok(d) => {
                        if writeln!(out, "{d}").is_err() {
                            std::process::exit(1);
                        }
                    }
                    Err(e) => {
                        eprintln!("line {}: {e}", n + 1);
                        std::process::exit(1);
                    }
                }
            }
            if out.flush().is_err() {
                std::process::exit(1);
            }
        }
        _ => {
            println!("hashref -- RustCrypto oracle for crypto::hash / crypto::shake256");
            println!("algorithms: {}", ALGOS.join(" "));
            println!("usage: hashref run <algo> <outlen> <input-hex>");
            println!("       hashref batch   < requests");
        }
    }
}
