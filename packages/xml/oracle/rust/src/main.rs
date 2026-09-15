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

use std::collections::BTreeSet;
use std::process::ExitCode;

use serde_json::{json, Value};

/// The namespace URI the `xml` prefix is bound to everywhere by the
/// specification. It is never written as a declaration.
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

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

fn refuse(id: &str, kind: &str, reason: impl std::fmt::Display) -> Value {
    json!({ "id": id, "ok": false, "kind": kind, "reason": reason.to_string() })
}

/// XML 1.0 §2.2 Char.
fn is_char(code: u32) -> bool {
    code == 0x9
        || code == 0xa
        || code == 0xd
        || (0x20..=0xd7ff).contains(&code)
        || (0xe000..=0xfffd).contains(&code)
        || (0x10000..=0x10ffff).contains(&code)
}

fn is_space_only(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c == ' ' || c == '\t' || c == '\r' || c == '\n')
}

/// Check the XML declaration ourselves rather than relying on roxmltree.
///
/// plan-138-C §2 records that whether the libraries refuse `version="1.1"` or a
/// non-UTF-8 encoding on their own is UNVERIFIED, and policy agreement between
/// the three sides may not rest on an unverified library behaviour.
fn check_declaration(text: &str) -> Option<(&'static str, String)> {
    let without_bom = text.strip_prefix('\u{feff}').unwrap_or(text);
    if !without_bom.starts_with("<?xml") {
        return None;
    }
    let end = without_bom.find("?>")?;
    let declaration = &without_bom[5..end];

    let version = pseudo_attribute(declaration, "version");
    match version.as_deref() {
        None => return Some(("parse", "the XML declaration has no version".into())),
        Some("1.0") => {}
        Some(other) => {
            return Some(("unsupported", format!("XML version {other} is not supported")))
        }
    }
    if let Some(encoding) = pseudo_attribute(declaration, "encoding") {
        if !encoding.eq_ignore_ascii_case("utf-8") {
            return Some((
                "unsupported",
                format!("encoding {encoding} is not supported"),
            ));
        }
    }
    None
}

/// `name="value"` or `name='value'` out of the declaration's inside.
fn pseudo_attribute(declaration: &str, name: &str) -> Option<String> {
    let at = declaration.find(name)?;
    let rest = declaration[at + name.len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

/// The raw `prefix:local` spelling of a name the parser has already resolved.
///
/// The package keeps names exactly as written and never resolves a prefix, so
/// the oracle has to put the prefix back to compare like with like.
fn qualified(node: roxmltree::Node, namespace: Option<&str>, local: &str) -> String {
    match namespace {
        None => local.to_string(),
        Some(uri) => match node.lookup_prefix(uri) {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}:{local}"),
            _ => local.to_string(),
        },
    }
}

/// The namespace declarations made ON this element.
///
/// `Node::namespaces()` yields everything IN SCOPE — a probe confirmed that an
/// element declaring nothing still reports its ancestors' declarations — so the
/// ones written here are the difference from the parent's scope. The package
/// keeps `xmlns` and `xmlns:p` as ordinary attributes, so they are added back.
fn declarations(node: roxmltree::Node) -> Vec<(String, String)> {
    let mine: Vec<(Option<&str>, &str)> =
        node.namespaces().map(|ns| (ns.name(), ns.uri())).collect();
    let parent: BTreeSet<(Option<&str>, &str)> = node
        .parent_element()
        .map(|p| p.namespaces().map(|ns| (ns.name(), ns.uri())).collect())
        .unwrap_or_default();

    let mut out = Vec::new();
    for (prefix, uri) in &mine {
        // The `xml` prefix is bound everywhere and is never declared.
        if *prefix == Some("xml") || *uri == XML_NS {
            continue;
        }
        if parent.contains(&(*prefix, *uri)) {
            continue;
        }
        out.push(match prefix {
            Some(prefix) => (format!("xmlns:{prefix}"), uri.to_string()),
            None => ("xmlns".to_string(), uri.to_string()),
        });
    }

    // A child may UNDECLARE the default namespace with xmlns="". That leaves no
    // entry in scope to diff against, so it is detected as a disappearance.
    let default_now = mine.iter().any(|(prefix, _)| prefix.is_none());
    let default_before = parent.iter().any(|(prefix, _)| prefix.is_none());
    if default_before && !default_now {
        out.push(("xmlns".to_string(), String::new()));
    }
    out
}

/// plan-138-A §4's content projection over one child list.
fn project(children: &[roxmltree::Node]) -> Vec<Value> {
    let has_element = children.iter().any(|child| child.is_element());
    let mut out: Vec<Value> = Vec::new();
    let mut pending = String::new();

    fn flush(out: &mut Vec<Value>, pending: &mut String, has_element: bool) {
        if pending.is_empty() {
            return;
        }
        if !(has_element && is_space_only(pending)) {
            out.push(json!(["t", pending]));
        }
        pending.clear();
    }

    for child in children {
        // A CDATA section is not a separate node kind here: `NodeType` has only
        // Root/Element/PI/Comment/Text, and the parser folds CDATA into text
        // (`roxmltree-0.21.1/src/parse.rs:1107 process_cdata`). The package
        // merges it into the surrounding Text too, so the two agree by
        // construction rather than by coincidence.
        if child.is_text() {
            pending.push_str(child.text().unwrap_or(""));
            continue;
        }
        if !child.is_element() {
            // A comment or PI is not content, and removing it lets the text on
            // either side of it merge.
            continue;
        }
        flush(&mut out, &mut pending, has_element);

        let name = qualified(*child, child.tag_name().namespace(), child.tag_name().name());
        let mut attributes: Vec<(String, String)> = declarations(*child);
        for attribute in child.attributes() {
            attributes.push((
                qualified(*child, attribute.namespace(), attribute.name()),
                attribute.value().to_string(),
            ));
        }
        attributes.sort_by(|left, right| left.0.cmp(&right.0));
        let attributes: Vec<Value> = attributes
            .into_iter()
            .map(|(name, value)| json!([name, value]))
            .collect();

        let grandchildren: Vec<roxmltree::Node> = child.children().collect();
        out.push(json!(["e", name, attributes, project(&grandchildren)]));
    }
    flush(&mut out, &mut pending, has_element);
    out
}

fn read_case(case: &Value) -> Value {
    let id = case.get("id").and_then(Value::as_str).unwrap_or("");
    let text = case.get("xml").and_then(Value::as_str).unwrap_or("");

    if let Some((kind, reason)) = check_declaration(text) {
        return refuse(id, kind, reason);
    }
    for character in text.chars() {
        if !is_char(character as u32) {
            return refuse(
                id,
                "parse",
                format!("U+{:04X} is not an XML 1.0 character", character as u32),
            );
        }
    }

    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        ..roxmltree::ParsingOptions::default()
    };
    let document = match roxmltree::Document::parse_with_options(text, options) {
        Ok(document) => document,
        Err(roxmltree::Error::DtdDetected) => {
            return refuse(id, "unsupported", "a DOCTYPE declaration is not supported")
        }
        Err(error) => return refuse(id, "parse", error),
    };

    let children: Vec<roxmltree::Node> = document.root().children().collect();
    json!({ "id": id, "ok": true, "content": ["doc", project(&children)] })
}

/// Answer one `write` case. Filled in by Phase 4.
fn write_case(case: &Value) -> Value {
    let id = case.get("id").and_then(Value::as_str).unwrap_or("");
    refuse(id, "parse", "write is not implemented yet")
}
