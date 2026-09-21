//! plan-142-A: every self-update-shaped builtin overload has a runtime case.
//!
//! A **self-update** is `x = f(x, …)` with `x` a `List`, `Map` or `Set`. plan-142
//! guarantees each such `f` either mutates `x`'s block in place or provably never
//! copies it, and `tests/runtime/rt_inplace_self_update.rs` measures that from
//! `tests/runtime/inplace_self_update/cases.tsv`. This guard is what makes the
//! guarantee cover a builtin added later: it reads the documented surface through
//! the `mfb` binary under test (`mfb man`), finds every overload whose first
//! parameter is a collection and whose return type can equal it, and fails if one
//! has no line in `cases.tsv` — or if `cases.tsv` names a signature `mfb man` no
//! longer documents.
//!
//! The registry-side twin is `self_update_census_covers_every_registry_overload`
//! in `src/codegen/collection/assign/self_update.rs`; it ties the registry to the
//! table of arms, which this black-box test cannot see.

#[path = "../common/mod.rs"]
mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

fn man(args: &[&str]) -> String {
    let output = Command::new(common::mfb_exe())
        .arg("man")
        .args(args)
        .output()
        .expect("run mfb man");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The builtin package names listed on the `mfb man` index.
fn packages() -> Vec<String> {
    let top = man(&[]);
    let start = top
        .find("Builtin packages")
        .expect("`mfb man` lists no `Builtin packages` section");
    let mut out = Vec::new();
    for line in top[start..].lines() {
        let Some(rest) = line.strip_prefix("│ ") else {
            continue;
        };
        let Some((cell, _)) = rest.split_once('│') else {
            continue;
        };
        let name = cell.trim_end();
        if !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphabetic())
            && cell.len() > name.len()
            && name != "Package"
        {
            out.push(name.to_string());
        }
    }
    out
}

/// The function names on a package page's `Functions` table, in order.
fn functions(pkg: &str) -> Vec<String> {
    let page = man(&[pkg]);
    let Some(start) = page.find("\nFunctions\n") else {
        return Vec::new();
    };
    let needle = format!("│ {pkg}::");
    let mut out: Vec<String> = Vec::new();
    let mut rest = &page[start..];
    while let Some(at) = rest.find(&needle) {
        rest = &rest[at + needle.len()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Every rendered signature (`pkg::f(a AS T, …) AS R`) on a function page's
/// `Overloads` / `Declaration` section, whitespace collapsed.
fn signatures(pkg: &str, function: &str) -> Vec<String> {
    let page = man(&[pkg, function]);
    let lines: Vec<&str> = page.lines().collect();
    let Some(i) = lines
        .iter()
        .position(|l| matches!(l.trim(), "Overloads" | "Declaration"))
    else {
        return Vec::new();
    };
    // The section runs until the next heading: a non-blank line underlined by `─`.
    let mut block = Vec::new();
    let mut j = i + 2;
    while j < lines.len() {
        let heading = !lines[j].trim().is_empty()
            && j + 1 < lines.len()
            && !lines[j + 1].trim().is_empty()
            && lines[j + 1].trim().chars().all(|c| c == '─');
        if heading {
            break;
        }
        block.push(lines[j].trim());
        j += 1;
    }
    let text = block.join(" ");
    let prefix = format!("`{pkg}::");
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find(&prefix) {
        rest = &rest[at + 1..];
        let Some(end) = rest.find('`') else { break };
        let sig = rest[..end].split_whitespace().collect::<Vec<_>>().join(" ");
        rest = &rest[end + 1..];
        out.push(sig);
    }
    out
}

/// A type, parsed just far enough to unify collection shapes.
#[derive(Clone, Debug, PartialEq)]
enum Ty {
    List(Box<Ty>),
    Set(Box<Ty>),
    Map(Box<Ty>, Box<Ty>),
    /// A single-letter type variable (`T`, `U`, `K`, `V`).
    Var(String),
    /// Anything else, compared by spelling.
    Atom(String),
}

fn parse_ty(text: &str) -> Ty {
    fn go(tokens: &[&str], pos: &mut usize) -> Option<Ty> {
        let tok = *tokens.get(*pos)?;
        *pos += 1;
        match tok {
            "List" | "Set" if tokens.get(*pos) == Some(&"OF") => {
                *pos += 1;
                let inner = Box::new(go(tokens, pos)?);
                Some(if tok == "List" {
                    Ty::List(inner)
                } else {
                    Ty::Set(inner)
                })
            }
            "Map" if tokens.get(*pos) == Some(&"OF") => {
                *pos += 1;
                let key = go(tokens, pos)?;
                if tokens.get(*pos) != Some(&"TO") {
                    return None;
                }
                *pos += 1;
                let value = go(tokens, pos)?;
                Some(Ty::Map(Box::new(key), Box::new(value)))
            }
            t if t.len() == 1 && t.chars().all(|c| c.is_ascii_uppercase()) => {
                Some(Ty::Var(t.to_string()))
            }
            t => Some(Ty::Atom(t.to_string())),
        }
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut pos = 0;
    match go(&tokens, &mut pos) {
        Some(ty) if pos == tokens.len() => ty,
        _ => Ty::Atom(text.to_string()),
    }
}

fn resolve(ty: &Ty, bindings: &BTreeMap<String, Ty>) -> Ty {
    let mut ty = ty.clone();
    while let Ty::Var(name) = &ty {
        match bindings.get(name) {
            Some(bound) => ty = bound.clone(),
            None => break,
        }
    }
    ty
}

fn occurs(name: &str, ty: &Ty, bindings: &BTreeMap<String, Ty>) -> bool {
    match resolve(ty, bindings) {
        Ty::Var(other) => other == name,
        Ty::List(e) | Ty::Set(e) => occurs(name, &e, bindings),
        Ty::Map(k, v) => occurs(name, &k, bindings) || occurs(name, &v, bindings),
        Ty::Atom(_) => false,
    }
}

/// Can `a` and `b` be made equal by one substitution of their type variables?
/// The occurs check matters: without it `get(List OF T) AS T` would "unify" by
/// `T = List OF T`, an infinite type.
fn coincide(a: &Ty, b: &Ty, bindings: &mut BTreeMap<String, Ty>) -> bool {
    let (a, b) = (resolve(a, bindings), resolve(b, bindings));
    match (&a, &b) {
        (Ty::Var(x), Ty::Var(y)) if x == y => true,
        (Ty::Var(x), other) | (other, Ty::Var(x)) => {
            if occurs(x, other, bindings) {
                return false;
            }
            bindings.insert(x.clone(), other.clone());
            true
        }
        (Ty::List(x), Ty::List(y)) | (Ty::Set(x), Ty::Set(y)) => coincide(x, y, bindings),
        (Ty::Map(k1, v1), Ty::Map(k2, v2)) => {
            coincide(k1, k2, bindings) && coincide(v1, v2, bindings)
        }
        (x, y) => x == y,
    }
}

/// `(first parameter type, return type)` of a rendered signature.
fn first_param_and_return(sig: &str) -> Option<(String, String)> {
    let open = sig.find('(')?;
    let close = sig.rfind(") AS ")?;
    let params = &sig[open + 1..close];
    let ret = sig[close + ") AS ".len()..].to_string();
    let first = params.trim_start_matches('[');
    let (_, ty) = first.split_once(" AS ")?;
    // The first parameter's type runs to the next `, name AS ` (or `, [name AS `).
    let mut end = ty.len();
    let mut search = 0;
    while let Some(at) = ty[search..].find(", ") {
        let at = search + at;
        let next = ty[at + 2..].trim_start_matches('[');
        let word: String = next
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !word.is_empty() && next[word.len()..].starts_with(" AS ") {
            end = at;
            break;
        }
        search = at + 2;
    }
    Some((ty[..end].trim_end_matches(']').to_string(), ret))
}

fn self_update_shaped(sig: &str) -> bool {
    let Some((first, ret)) = first_param_and_return(sig) else {
        return false;
    };
    let (first, ret) = (parse_ty(&first), parse_ty(&ret));
    let mut bindings = BTreeMap::new();
    coincide(&first, &ret, &mut bindings)
        && matches!(
            resolve(&first, &bindings),
            Ty::List(_) | Ty::Set(_) | Ty::Map(_, _)
        )
}

fn cases_tsv_signatures() -> BTreeSet<String> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/runtime/inplace_self_update/cases.tsv"
    );
    let text = std::fs::read_to_string(path).expect("read cases.tsv");
    // An operator self-update (`s = s & t`) has no `mfb man` function page, so its
    // line (no `::`) is outside this census.
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').next().unwrap_or("").to_string())
        .filter(|sig| sig.contains("::"))
        .collect()
}

#[test]
fn every_self_update_shaped_overload_has_a_runtime_case() {
    let mut shaped = BTreeSet::new();
    let mut overloads = 0usize;
    for pkg in packages() {
        for function in functions(&pkg) {
            for sig in signatures(&pkg, &function) {
                overloads += 1;
                if self_update_shaped(&sig) {
                    shaped.insert(sig);
                }
            }
        }
    }
    assert!(
        overloads > 500,
        "the `mfb man` census read only {overloads} overloads — the page format changed \
         and this guard is no longer reading it"
    );
    let cases = cases_tsv_signatures();
    let missing: Vec<&String> = shaped.difference(&cases).collect();
    let stale: Vec<&String> = cases.difference(&shaped).collect();
    assert!(
        missing.is_empty(),
        "self-update-shaped overload(s) documented by `mfb man` with no line in \
         tests/runtime/inplace_self_update/cases.tsv — give each an in-place arm (or a \
         proven exemption) and a case:\n  {}",
        missing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert!(
        stale.is_empty(),
        "cases.tsv line(s) naming a signature `mfb man` does not document as \
         self-update-shaped:\n  {}",
        stale
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
