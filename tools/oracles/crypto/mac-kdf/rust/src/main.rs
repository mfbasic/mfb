//! The judge for `crypto::hmac`, `crypto::hkdf` and `crypto::pbkdf2`, over the
//! FULL `crypto::Hash` matrix -- all nine selectors, not just the ones `ring`
//! can reach.
//!
//! Nothing here is implemented by hand. The authority of this oracle is that
//! RustCrypto is a different codebase, maintained by people who never read the
//! MFBASIC cores; a transliteration written here would be a second
//! implementation by the same author as the first, which proves nothing.
//!
//! Modes:
//!   mackdfref                    list what it knows
//!   mackdfref batch              stdin: one request per line, one hex out
//!   mackdfref run <request…>     the same, as argv, for deriving one value
//!
//! Requests (hex arguments, "-" for empty):
//!   hmac   <hash> <key> <data>
//!   hkdf   <hash> <ikm> <salt> <info> <outlen>
//!   pbkdf2 <hash> <password> <salt> <iterations> <outlen>

use hmac::{Hmac, Mac};
use std::io::{BufRead, Write};

const HASHES: &[&str] = &[
    "sha1", "sha2-224", "sha2-256", "sha2-384", "sha2-512", "sha3-224", "sha3-256", "sha3-384",
    "sha3-512",
];

/// Expand `$body!(ConcreteHash)` once per selector. Writing the nine arms out
/// per member would be 27 copies of the same dispatch; getting one of them
/// wrong would silently check the wrong algorithm, which is exactly the failure
/// an oracle must not have.
macro_rules! by_hash {
    ($algo:expr, $body:ident) => {
        match $algo {
            "sha1" => $body!(sha1::Sha1),
            "sha2-224" => $body!(sha2::Sha224),
            "sha2-256" => $body!(sha2::Sha256),
            "sha2-384" => $body!(sha2::Sha384),
            "sha2-512" => $body!(sha2::Sha512),
            "sha3-224" => $body!(sha3::Sha3_224),
            "sha3-256" => $body!(sha3::Sha3_256),
            "sha3-384" => $body!(sha3::Sha3_384),
            "sha3-512" => $body!(sha3::Sha3_512),
            other => return Err(format!("unknown hash: {other}")),
        }
    };
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
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

fn do_hmac(algo: &str, key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    macro_rules! go {
        ($h:ty) => {{
            let mut mac = <Hmac<$h> as Mac>::new_from_slice(key)
                .map_err(|e| format!("hmac key: {e}"))?;
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        }};
    }
    Ok(by_hash!(algo, go))
}

fn do_hkdf(
    algo: &str,
    ikm: &[u8],
    salt: &[u8],
    info: &[u8],
    out_len: usize,
) -> Result<Vec<u8>, String> {
    macro_rules! go {
        ($h:ty) => {{
            // RFC 5869: an absent salt is HashLen zero bytes, and HMAC with an
            // empty key is HMAC with a zero-padded key -- so Some(&[]) and None
            // agree here, and passing the empty salt through keeps this a
            // faithful mirror of what the caller asked for.
            let hk = hkdf::Hkdf::<$h>::new(Some(salt), ikm);
            let mut out = vec![0u8; out_len];
            hk.expand(info, &mut out)
                .map_err(|e| format!("hkdf expand: {e}"))?;
            out
        }};
    }
    Ok(by_hash!(algo, go))
}

fn do_pbkdf2(
    algo: &str,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    out_len: usize,
) -> Result<Vec<u8>, String> {
    macro_rules! go {
        ($h:ty) => {{
            let mut out = vec![0u8; out_len];
            pbkdf2::pbkdf2_hmac::<$h>(password, salt, iterations, &mut out);
            out
        }};
    }
    Ok(by_hash!(algo, go))
}

/// Answer one whitespace-separated request.
fn answer(line: &str) -> Result<String, String> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.is_empty() {
        return Err("empty request".to_string());
    }
    let out = match f[0] {
        "hmac" => {
            if f.len() != 4 {
                return Err(format!("hmac needs 3 arguments, got {}", f.len() - 1));
            }
            do_hmac(f[1], &unhex(f[2])?, &unhex(f[3])?)?
        }
        "hkdf" => {
            if f.len() != 6 {
                return Err(format!("hkdf needs 5 arguments, got {}", f.len() - 1));
            }
            let len = f[5].parse().map_err(|e| format!("bad length {}: {e}", f[5]))?;
            do_hkdf(f[1], &unhex(f[2])?, &unhex(f[3])?, &unhex(f[4])?, len)?
        }
        "pbkdf2" => {
            if f.len() != 6 {
                return Err(format!("pbkdf2 needs 5 arguments, got {}", f.len() - 1));
            }
            let iters = f[4].parse().map_err(|e| format!("bad iterations {}: {e}", f[4]))?;
            let len = f[5].parse().map_err(|e| format!("bad length {}: {e}", f[5]))?;
            do_pbkdf2(f[1], &unhex(f[2])?, &unhex(f[3])?, iters, len)?
        }
        other => return Err(format!("unknown member: {other}")),
    };
    Ok(hex(&out))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => match answer(&args[2..].join(" ")) {
            Ok(d) => println!("{d}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        Some("batch") => {
            // One answer per input line, in order. The caller checks that the
            // number of lines back equals the number sent -- a reference that
            // dies halfway must not look like a short but passing run.
            let stdin = std::io::stdin();
            let mut out = std::io::BufWriter::new(std::io::stdout());
            for (n, line) in stdin.lock().lines().enumerate() {
                let line = line.unwrap_or_else(|e| {
                    eprintln!("stdin line {}: {e}", n + 1);
                    std::process::exit(1);
                });
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
            println!("mackdfref -- RustCrypto oracle for crypto::hmac / hkdf / pbkdf2");
            println!("hashes: {}", HASHES.join(" "));
            println!("usage: mackdfref run    hmac   <hash> <key> <data>");
            println!("       mackdfref run    hkdf   <hash> <ikm> <salt> <info> <outlen>");
            println!("       mackdfref run    pbkdf2 <hash> <password> <salt> <iters> <outlen>");
            println!("       mackdfref batch  < requests");
        }
    }
}
