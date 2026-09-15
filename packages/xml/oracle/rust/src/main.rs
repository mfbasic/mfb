//! xmloracle — the Rust side of the `packages/xml` differential oracle.
//!
//! It answers one JSON job file per invocation and writes one JSON document to
//! stdout, holding one result per case in the same order:
//!
//! ```text
//!   in   {"cases":[{"id":"c1","xml":"<a/>"}]}                        (read)
//!        {"cases":[{"id":"c1","tree":["doc",[...]],"indent":"  "}]}  (write)
//!
//!   out  {"results":[{"id":"c1","ok":true,"content":["doc",[...]]},
//!                    {"id":"c2","ok":false,"kind":"parse","reason":"..."}]}
//! ```
//!
//! A REFUSAL is a result, reported on stdout with exit 0. That leaves a
//! non-zero exit or an unparseable document meaning exactly one thing — the
//! oracle itself broke — which is what the runner's `mutate` mode checks for.
//!
//! This oracle and the Node one are EQUAL PEERS: neither is the reference. A
//! case passes only when the package, this, and Node all agree.

use std::process::ExitCode;

use serde_json::{json, Value};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(op), Some(path)) = (args.next(), args.next()) else {
        eprintln!("usage: xmloracle read|write <job.json>");
        return ExitCode::from(2);
    };

    let job = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("xmloracle: cannot read {path}: {error}");
            return ExitCode::from(1);
        }
    };
    let job: Value = match serde_json::from_str(&job) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("xmloracle: {path} is not JSON: {error}");
            return ExitCode::from(1);
        }
    };

    let cases = match job.get("cases").and_then(Value::as_array) {
        Some(cases) => cases.clone(),
        None => {
            eprintln!("xmloracle: the job has no `cases` array");
            return ExitCode::from(1);
        }
    };

    let results: Vec<Value> = match op.as_str() {
        "read" => cases.iter().map(read_case).collect(),
        "write" => cases.iter().map(write_case).collect(),
        other => {
            eprintln!("xmloracle: unknown operation `{other}` (expected read or write)");
            return ExitCode::from(2);
        }
    };

    println!("{}", json!({ "results": results }));
    ExitCode::SUCCESS
}

/// Answer one `read` case. Filled in by Phase 2; the envelope shape is already
/// fixed by plan-138-C §3, so the skeleton answers in it.
fn read_case(case: &Value) -> Value {
    let id = case.get("id").and_then(Value::as_str).unwrap_or("");
    json!({ "id": id, "ok": false, "kind": "parse", "reason": "read is not implemented yet" })
}

/// Answer one `write` case. Filled in by Phase 4.
fn write_case(case: &Value) -> Value {
    let id = case.get("id").and_then(Value::as_str).unwrap_or("");
    json!({ "id": id, "ok": false, "kind": "parse", "reason": "write is not implemented yet" })
}
