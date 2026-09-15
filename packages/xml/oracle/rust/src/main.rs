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
    // roxmltree does not check the standalone value, and the suite tests it
    // (`standalone="YES"` is not well formed: §2.9 admits only yes and no).
    if let Some(standalone) = pseudo_attribute(declaration, "standalone") {
        if standalone != "yes" && standalone != "no" {
            return Some((
                "parse",
                format!("standalone must be yes or no, not {standalone}"),
            ));
        }
    }
    None
}

/// The source with comments, CDATA sections and processing instructions blanked
/// out, so a scan for markup cannot be fooled by text that merely looks like it.
///
/// The suite proves this is needed: `o-p16pass1` holds `&#c` inside a PI and
/// `o-p18pass1` holds it inside CDATA, and both are well-formed documents. A
/// raw scan reads those as malformed character references.
fn without_ignorable(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < bytes.len() {
        let rest = &text[at..];
        let skip = if rest.starts_with("<!--") {
            rest.find("-->").map(|end| end + 3)
        } else if rest.starts_with("<![CDATA[") {
            rest.find("]]>").map(|end| end + 3)
        } else if rest.starts_with("<?") {
            rest.find("?>").map(|end| end + 2)
        } else {
            None
        };
        match skip {
            Some(length) => {
                // Keep the length so any offset the caller reports still lines
                // up with the source.
                out.push_str(&" ".repeat(rest[..length].chars().count()));
                at += length;
            }
            None => {
                let character = rest.chars().next().expect("non-empty");
                out.push(character);
                at += character.len_utf8();
            }
        }
    }
    out
}

/// Refuse a character reference naming something XML 1.0 forbids.
///
/// roxmltree expands references itself and does not police §2.2 for them, so
/// `&#55298;` (a surrogate half) survives.
fn check_character_references(source: &str) -> Option<(&'static str, String)> {
    let text = &without_ignorable(source);
    let bytes = text.as_bytes();
    let mut at = 0;
    while let Some(found) = text[at..].find("&#") {
        let start = at + found + 2;
        let (digits, radix) = if bytes.get(start) == Some(&b'x') || bytes.get(start) == Some(&b'X') {
            (start + 1, 16)
        } else {
            (start, 10)
        };
        let end = match text[digits..].find(';') {
            Some(offset) => digits + offset,
            None => return Some(("parse", "a character reference is not terminated".into())),
        };
        match u32::from_str_radix(&text[digits..end], radix) {
            Ok(code) if is_char(code) => {}
            Ok(code) => {
                return Some((
                    "parse",
                    format!("a character reference names U+{code:04X}, which XML 1.0 does not allow"),
                ))
            }
            Err(_) => return Some(("parse", "a character reference is malformed".into())),
        }
        at = end;
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

/// The raw spelling of an ELEMENT's name.
///
/// Unlike an attribute, an element may take its namespace from the DEFAULT
/// declaration, and then it was written with no prefix at all. `lookup_prefix`
/// returns whichever prefix is bound to that URI, which need not be the
/// spelling used: in the suite's `rmt-ns10-039`, `xmlns` and `xmlns:a` are bound
/// to the SAME URI, so `<foo>` came back as `a:foo`. When the URI matches the
/// default in scope, the name was written bare.
fn qualified_element(node: roxmltree::Node) -> String {
    let local = node.tag_name().name();
    match node.tag_name().namespace() {
        None => local.to_string(),
        Some(uri) => {
            let default = node
                .namespaces()
                .find(|ns| ns.name().is_none())
                .map(|ns| ns.uri());
            if default == Some(uri) {
                return local.to_string();
            }
            qualified(node, Some(uri), local)
        }
    }
}

/// The namespace declarations made ON this element.
///
/// `Node::namespaces()` yields everything IN SCOPE — a probe confirmed that an
/// element declaring nothing still reports its ancestors' declarations — so the
/// ones written here are the difference from the parent's scope. The package
/// keeps `xmlns` and `xmlns:p` as ordinary attributes, so they are added back.
fn declarations(node: roxmltree::Node, source: &str) -> Vec<(String, String)> {
    let mine: Vec<(Option<&str>, &str)> =
        node.namespaces().map(|ns| (ns.name(), ns.uri())).collect();
    let parent: BTreeSet<(Option<&str>, &str)> = node
        .parent_element()
        .map(|p| p.namespaces().map(|ns| (ns.name(), ns.uri())).collect())
        .unwrap_or_default();

    let mut out = Vec::new();
    for (prefix, uri) in &mine {
        // The `xml` prefix is NOT skipped: roxmltree does not synthesise an
        // implicit binding for it (a probe confirmed an element using no
        // declarations reports none), so an entry here was written in the
        // source — and the package keeps it as an ordinary attribute. The
        // suite's `rmt-ns10-028` declares `xmlns:xml` explicitly and expects it
        // to survive.
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

    // An explicitly declared `xmlns:xml` never reaches namespaces(): roxmltree
    // treats the xml prefix as pre-bound and drops the declaration. The package
    // keeps it as an ordinary attribute (the suite's `rmt-ns10-028` declares it
    // and expects it to survive), so it is recovered from the element's OWN
    // start tag — Node::range() gives exactly that span, so a declaration on an
    // ancestor or a descendant cannot be mistaken for this element's.
    let range = node.range();
    let start_tag = source.get(range.start..range.end).unwrap_or("");
    let start_tag = &start_tag[..start_tag.find('>').map(|at| at + 1).unwrap_or(0)];
    if let Some(at) = start_tag.find("xmlns:xml") {
        let rest = &start_tag[at + "xmlns:xml".len()..];
        if rest.trim_start().starts_with('=') {
            if let Some(value) = rest
                .trim_start()
                .strip_prefix('=')
                .map(str::trim_start)
                .and_then(|value| {
                    let quote = value.chars().next()?;
                    let body = &value[quote.len_utf8()..];
                    body.find(quote).map(|end| body[..end].to_string())
                })
            {
                out.push(("xmlns:xml".to_string(), value));
            }
        }
    }
    out
}

/// Refuse the processing instructions roxmltree accepts but the policy does not.
///
/// The wrapper owns the policy checks — as it already does for version,
/// encoding and standalone — so agreement between the three sides never rests
/// on how permissive a particular library happens to be.
///
///   * a target matching `[Xx][Mm][Ll]` is reserved (§2.6), except the XML
///     declaration itself, which may appear only at the very start;
///   * a target may not hold a colon (Namespaces 1.0 erratum NE08);
///   * the target must be followed by whitespace or by `?>`.
fn check_processing_instructions(source: &str) -> Option<(&'static str, String)> {
    let text = source.strip_prefix('\u{feff}').unwrap_or(source);
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let rest = &text[at..];
        // Comments and CDATA may hold anything, including `<?`.
        if rest.starts_with("<!--") {
            match rest.find("-->") {
                Some(end) => {
                    at += end + 3;
                    continue;
                }
                None => break,
            }
        }
        if rest.starts_with("<![CDATA[") {
            match rest.find("]]>") {
                Some(end) => {
                    at += end + 3;
                    continue;
                }
                None => break,
            }
        }
        if !rest.starts_with("<?") {
            at += rest.chars().next().map(char::len_utf8).unwrap_or(1);
            continue;
        }

        let body = &rest[2..];
        let end = match body.find("?>") {
            Some(end) => end,
            None => return Some(("parse", "a processing instruction is not closed".into())),
        };
        // The target is a NAME, not "everything up to whitespace": the suite's
        // `o-p16fail3` is `<?pitarget+++?>`, where swallowing the `+++` into the
        // target would hide the fact that nothing separates it from the data.
        let target: String = body[..end]
            .chars()
            .take_while(|c| is_name_char(*c))
            .collect();
        let is_declaration = at == 0 && target == "xml";

        if !is_declaration && target.eq_ignore_ascii_case("xml") {
            return Some((
                "parse",
                format!("`{target}` is a reserved processing-instruction target"),
            ));
        }
        if target.contains(':') {
            return Some((
                "parse",
                format!("a processing-instruction target may not hold a colon (`{target}`)"),
            ));
        }
        let after = &body[target.len()..end];
        if !after.is_empty() && !after.starts_with(char::is_whitespace) {
            return Some((
                "parse",
                "a processing instruction's target must be followed by whitespace or `?>`".into(),
            ));
        }
        at += 2 + end + 2;
    }
    None
}

/// Refuse a name that is not a legal QName.
///
/// roxmltree accepts `:foo` (the suite's `rmt-ns10-015`, TYPE="not-wf"), keeping
/// the colon in the local part. Any colon surviving in a local name means the
/// name was not a well-formed QName.
fn check_names(document: &roxmltree::Document) -> Option<(&'static str, String)> {
    for node in document.descendants().filter(|n| n.is_element()) {
        let local = node.tag_name().name();
        if local.contains(':') || local.is_empty() {
            return Some(("parse", format!("`{local}` is not a valid QName")));
        }
        for attribute in node.attributes() {
            if attribute.name().contains(':') || attribute.name().is_empty() {
                return Some((
                    "parse",
                    format!("`{}` is not a valid QName", attribute.name()),
                ));
            }
        }
    }
    None
}

/// XML 1.0 §2.3 NameChar, which is what a processing-instruction target is made
/// of. Only the classes the checks here need to tell apart, not a full table:
/// anything outside ASCII is a NameChar in the Fifth Edition's ranges.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':' || !c.is_ascii()
}

/// Refuse a name written with an EMPTY prefix, such as `<:foo/>`.
///
/// roxmltree splits `:foo` into an empty prefix and the local name `foo`, so no
/// colon survives for `check_names` to find — the parsed tree looks exactly like
/// plain `<foo/>`. The source still shows it. (The suite's `rmt-ns10-015`,
/// TYPE="not-wf".)
fn check_empty_prefix(source: &str) -> Option<(&'static str, String)> {
    let text = without_ignorable(source);
    let bytes = text.as_bytes();
    for (at, _) in text.match_indices('<') {
        let mut next = at + 1;
        if bytes.get(next) == Some(&b'/') {
            next += 1;
        }
        if bytes.get(next) == Some(&b':') {
            return Some(("parse", "a name may not begin with a colon".into()));
        }
    }
    None
}

/// Refuse the namespace declarations Namespaces 1.0 forbids and roxmltree
/// hides: a non-default prefix may not be undeclared (`xmlns:a=""`, added only
/// in Namespaces 1.1), and `xmlns` may not itself be declared as a prefix.
fn check_declarations(source: &str) -> Option<(&'static str, String)> {
    let text = without_ignorable(source);
    for capture in text.match_indices("xmlns:") {
        let rest = &text[capture.0 + "xmlns:".len()..];
        let name: String = rest
            .chars()
            .take_while(|c| *c != '=' && !c.is_whitespace())
            .collect();
        let after = &rest[name.len()..];
        let value = after
            .trim_start()
            .strip_prefix('=')
            .map(str::trim_start)
            .and_then(|value| {
                let quote = value.chars().next()?;
                if quote != '"' && quote != '\'' {
                    return None;
                }
                let body = &value[quote.len_utf8()..];
                body.find(quote).map(|end| &body[..end])
            });
        if name == "xmlns" {
            return Some(("parse", "the prefix `xmlns` may not be declared".into()));
        }
        if value == Some("") {
            return Some((
                "parse",
                format!("namespace prefix `{name}` may not be bound to an empty URI"),
            ));
        }
    }
    None
}

/// plan-138-A §4's content projection over one child list.
fn project(children: &[roxmltree::Node], source: &str) -> Vec<Value> {
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

        let name = qualified_element(*child);
        let mut attributes: Vec<(String, String)> = declarations(*child, source);
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
        out.push(json!(["e", name, attributes, project(&grandchildren, source)]));
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
    if let Some((kind, reason)) = check_character_references(text) {
        return refuse(id, kind, reason);
    }
    if let Some((kind, reason)) = check_processing_instructions(text) {
        return refuse(id, kind, reason);
    }
    if let Some((kind, reason)) = check_declarations(text) {
        return refuse(id, kind, reason);
    }
    if let Some((kind, reason)) = check_empty_prefix(text) {
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

    if let Some((kind, reason)) = check_names(&document) {
        return refuse(id, kind, reason);
    }

    let children: Vec<roxmltree::Node> = document.root().children().collect();
    json!({ "id": id, "ok": true, "content": ["doc", project(&children, text)] })
}

/// Answer one `write` case: a tree in the §3 envelope shape, written back out.
///
/// quick-xml is the writer here, so the write direction has an implementation
/// independent of both the package and xmldom. `BytesText::new` escapes its
/// content and `from_escaped` does not — the difference matters, because
/// double-escaping would show up as a content disagreement that looks like a
/// package bug.
fn write_case(case: &Value) -> Value {
    let id = case.get("id").and_then(Value::as_str).unwrap_or("");
    let indent = case.get("indent").and_then(Value::as_str).unwrap_or("");
    let Some(tree) = case.get("tree") else {
        return refuse(id, "parse", "the case has no `tree`");
    };

    // quick-xml's own indent mode is NOT usable here: it indents inside every
    // element, including ones holding text, which CHANGES the content —
    // `[["t","line\nbreak"]]` came back as `[["t","\n      \n      line\nbreak"]]`.
    // The layout rule belongs to this writer (an element's children go on their
    // own lines only when it has an element child and no data text), and
    // quick-xml does the escaping and the compact serialization underneath.
    let body = match layout_tree(tree, indent) {
        Ok(body) => body,
        Err(error) => return refuse(id, "parse", error),
    };
    let separator = if indent.is_empty() { "" } else { "\n" };
    json!({
        "id": id,
        "ok": true,
        "xml": format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{separator}{body}"),
    })
}

/// Serialize one node compactly, with quick-xml doing the escaping.
fn compact(node: &Value) -> Result<String, String> {
    let mut writer = quick_xml::Writer::new(Vec::new());
    write_node(&mut writer, node)?;
    String::from_utf8(writer.into_inner()).map_err(|error| error.to_string())
}

fn is_xml_space(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\r' || c == '\n'
}

fn node_kind(node: &Value) -> Option<&str> {
    node.as_array()?.first()?.as_str()
}

/// A whole document laid out with `indent`, or compactly when it is empty.
fn layout_tree(tree: &Value, indent: &str) -> Result<String, String> {
    let parts = tree.as_array().ok_or("a tree must be an array")?;
    if parts.len() != 2 || parts[0].as_str() != Some("doc") {
        return Err("a tree must be [\"doc\", [children]]".into());
    }
    let children = parts[1].as_array().ok_or("a doc's children must be an array")?;
    let mut out = Vec::new();
    for child in children {
        out.push(layout_node(child, indent, 0)?);
    }
    Ok(out.join(if indent.is_empty() { "" } else { "\n" }))
}

/// One node at `level` indents deep.
///
/// An element's children go on their own lines only when it has an ELEMENT
/// child and no DATA text child. Anything else is written exactly as compact:
/// adding whitespace beside text changes the text, and adding it inside an
/// element whose only children are comments or PIs gives that element text it
/// never had (plan-138-A §4 step 3).
fn layout_node(node: &Value, indent: &str, level: usize) -> Result<String, String> {
    if indent.is_empty() || node_kind(node) != Some("e") {
        return compact(node);
    }
    let parts = node.as_array().ok_or("a node must be an array")?;
    let name = parts.get(1).and_then(Value::as_str).ok_or("an element needs a name")?;
    let attributes = parts.get(2).ok_or("an element needs attributes")?;
    let children = parts
        .get(3)
        .and_then(Value::as_array)
        .ok_or("an element needs a child list")?;
    if children.is_empty() {
        return compact(node);
    }

    let has_element = children.iter().any(|child| node_kind(child) == Some("e"));
    let has_data_text = children.iter().any(|child| {
        node_kind(child) == Some("t")
            && child
                .as_array()
                .and_then(|parts| parts.get(1))
                .and_then(Value::as_str)
                .map(|text| !text.is_empty() && !text.chars().all(is_xml_space))
                .unwrap_or(false)
    });
    if !has_element || has_data_text {
        return compact(node);
    }

    // The start tag, borrowed from the empty form so quick-xml still writes the
    // attributes and their escaping.
    let empty = compact(&json!(["e", name, attributes, []]))?;
    let open = empty
        .strip_suffix("/>")
        .map(|start| format!("{start}>"))
        .ok_or("quick-xml did not write an empty element as expected")?;

    let mut out = String::from(&open);
    for child in children {
        out.push('\n');
        out.push_str(&indent.repeat(level + 1));
        out.push_str(&layout_node(child, indent, level + 1)?);
    }
    out.push('\n');
    out.push_str(&indent.repeat(level));
    out.push_str(&format!("</{name}>"));
    Ok(out)
}

fn write_node(writer: &mut quick_xml::Writer<Vec<u8>>, node: &Value) -> Result<(), String> {
    use quick_xml::events::{BytesEnd, BytesPI, BytesStart, BytesText, Event};

    let parts = node.as_array().ok_or("a node must be an array")?;
    let kind = parts.first().and_then(Value::as_str).ok_or("a node needs a kind")?;
    let text_at = |index: usize| -> Result<&str, String> {
        parts
            .get(index)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("a `{kind}` node needs a string at {index}"))
    };

    match kind {
        "t" => {
            writer
                .write_event(Event::Text(BytesText::new(text_at(1)?)))
                .map_err(|error| error.to_string())?;
        }
        "c" => {
            writer
                .write_event(Event::Comment(BytesText::new(text_at(1)?)))
                .map_err(|error| error.to_string())?;
        }
        "p" => {
            let target = text_at(1)?;
            let data = text_at(2)?;
            let content = if data.is_empty() {
                target.to_string()
            } else {
                format!("{target} {data}")
            };
            writer
                .write_event(Event::PI(BytesPI::new(content)))
                .map_err(|error| error.to_string())?;
        }
        "e" => {
            let name = text_at(1)?;
            let mut start = BytesStart::new(name);
            for attribute in parts
                .get(2)
                .and_then(Value::as_array)
                .ok_or("an element needs an attribute list")?
            {
                let pair = attribute.as_array().ok_or("an attribute must be a pair")?;
                let key = pair.first().and_then(Value::as_str).ok_or("an attribute needs a name")?;
                let value = pair.get(1).and_then(Value::as_str).ok_or("an attribute needs a value")?;
                start.push_attribute((key, value));
            }
            let children = parts
                .get(3)
                .and_then(Value::as_array)
                .ok_or("an element needs a child list")?;
            if children.is_empty() {
                writer
                    .write_event(Event::Empty(start))
                    .map_err(|error| error.to_string())?;
                return Ok(());
            }
            writer
                .write_event(Event::Start(start))
                .map_err(|error| error.to_string())?;
            for child in children {
                write_node(writer, child)?;
            }
            writer
                .write_event(Event::End(BytesEnd::new(name)))
                .map_err(|error| error.to_string())?;
        }
        other => return Err(format!("unknown node kind `{other}`")),
    }
    Ok(())
}
