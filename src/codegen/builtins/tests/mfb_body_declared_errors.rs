//! The `Body::Mfb` counterpart of `declared_errors_sweep.rs`'s native-lowering
//! guard (bug-698 Phase 1, the "second box": "the sweep does not see
//! `Body::Mfb` members ... Phase 1 adds a guard that derives their raise set
//! (their own `FAIL`s plus what the members they call can raise) and compares
//! it to the descriptor").
//!
//! An `Mfb` member carries an MFBASIC *source* body (`FUNC __pkg_name(...) ...
//! END FUNC`), not a native lowering `declared_errors_sweep.rs` can drive
//! through a backend — it raises only through an explicit `FAIL error(<code>,
//! …)` statement in its own source, or in a package-private helper
//! (`RegistryHelper`, no descriptor of its own) it calls. So this guard does
//! not lower anything: it scans the body TEXT.
//!
//! **The algorithm, per member (its implementations' bodies unioned, the same
//! granularity `declared_errors_sweep.rs` uses):**
//!
//! 1. Collect every `FAIL error(<digits>, …)` code in the member's own body
//!    text, mapped to its `errorCode` name via the same `codes_by_value()`
//!    table the native sweep uses.
//! 2. Collect every call the body makes: a qualified `pkg::name(` call (any
//!    package, including its own) or a bare `__pkg_name(` call (the internal
//!    rewrite-symbol spelling a member's own body uses to call a sibling
//!    `Mfb` member, e.g. `__datetime_parse2` calling `__datetime_parse3`).
//! 3. A qualified call always names a registered member — **trust that
//!    member's own declared `errors`** (already verified by this same guard
//!    for a sibling `Mfb` member, or by `declared_errors_all.rs`/
//!    `fs_declared_errors.rs` for a native one) rather than re-deriving it;
//!    this is exactly `declared_errors_sweep.rs`'s `Propagating::Member`
//!    shape, just reached by a source scan instead of a relocation scan.
//! 4. A bare `__name(` call is either another member's rewrite symbol (same
//!    treatment as 3) or a package-private helper with no descriptor — the
//!    sweep INLINES it: scans the helper's own `FUNC __name … END FUNC`
//!    block for further `FAIL`s and calls, recursively, memoized and guarded
//!    against a cycle.
//! 5. A bare call to a `general` member (`toString(`, `toInt(`, `toByte(`, …,
//!    bug-807) counts the codes of the OVERLOAD it reaches, read off
//!    `general_declared_errors.rs`'s measured per-overload rows: the argument's
//!    type comes from what the text fixes (a literal, a declared name or field,
//!    a conversion, a call with one concrete return type — [`infer_type`]), and
//!    an argument the text does not type counts every overload. Without this the
//!    guard either missed those calls entirely (before bug-807) or charged a
//!    `toInt(<Byte>)` with the `String` parse's errors. A call no row can raise
//!    for, from its arguments' ranges, is proven in [`UNREACHABLE_GENERAL_CALL`].
//! 6. The member's expected `errors` is the union of 1 and the propagated sets
//!    from 3/4/5; a mismatch against the descriptor's actual `errors` is
//!    reported by name, the same shape as `declared_errors_sweep.rs`'s
//!    `mismatched` list.
//!
//! This is deliberately NOT a full compile of the body: an implicit raise
//! from a plain source-level operator (e.g. `Integer` `+` overflow) inside an
//! `Mfb` body is outside what this guard measures — the owner's own framing
//! ("Mfb members ... raise through FAIL") scopes Phase 1 to explicit `FAIL`
//! sites and callee propagation, matching `declared_errors_sweep.rs`'s own
//! textual (not lowered) classification of a native body's `RAISES_NOTHING`/
//! `PROPAGATING` callees.
//!
//! **`TRAP` (spec `08_error-model.md`) is modeled, not ignored.** A function
//! or helper body routinely absorbs the very calls/`FAIL`s step 1/2 above
//! would otherwise count: a function-level `TRAP(e)` at the bottom of a
//! `FUNC` catches every error the normal body raises, and only what the
//! handler ITSELF does (its own `FAIL`/calls, plus the trapped set again if
//! the handler `PROPAGATE`s) escapes; a postfix inline `TRAP(e)` on one
//! expression does the same for just that expression, with `RECOVER`
//! (absorb and continue) as one more handler outcome alongside `RETURN`/
//! `FAIL`/`PROPAGATE`. Treating a trapped call's codes as unconditionally
//! reachable is wrong in exactly the way `net::percentDecode`
//! (`helper_percent_decode_impl.rs`'s function-level `TRAP` maps every
//! failure to one `ErrInvalidFormat` `FAIL`) and `canvas::loadImage`
//! (`__canvas_pngDecode`'s inline `TRAP(err) RETURN [] END TRAP` around
//! `compress::zlibDecode`) both demonstrated when this guard first ran
//! without `TRAP` awareness. See [`resolve`]/[`resolve_inline_traps`]/
//! [`scan_calls_and_fails`] for the three-layer implementation.

//!
//! **`Body::Rewrite` is covered too (bug-814).** A `Rewrite` implementation's
//! call becomes a call to a fixed `__`-symbol whose body is a package helper
//! `FUNC`; [`rewrite_sweep`] resolves that block exactly as above and compares it
//! with the one implementation's own `errors`. Qualified calls are narrowed where
//! the text fixes the answer ([`qualified_call_codes`]), and a code a helper's
//! call cannot raise from its arguments' ranges is proven per helper `FUNC` in
//! [`UNREACHABLE_QUALIFIED_CALL`] / [`UNREACHABLE_GENERAL_CALL`].

use crate::codegen::registry::{registry, Body};
use crate::types::ParameterType;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::declared_errors_sweep::codes_by_value;
use super::general_declared_errors::OVERLOADS;

/// The `general` package's member names (`toString`, `toInt`, `len`, …).
fn general_names() -> BTreeSet<String> {
    registry()
        .resolve_package("general")
        .expect("general package")
        .functions()
        .iter()
        .map(|function| function.name.to_string())
        .collect()
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Every `FAIL error(<digits>` code literal in `text`, in source order
/// (duplicates allowed; the caller only cares about the set).
fn fail_codes(text: &str) -> Vec<String> {
    let marker = "FAIL error(";
    let mut out = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = text[search_from..].find(marker) {
        let start = search_from + rel + marker.len();
        let digits: String = text[start..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        search_from = start + digits.len().max(1);
        if !digits.is_empty() {
            out.push(digits);
        }
    }
    out
}

/// Whether the identifier starting at `start` is a `CASE <Variant>(binding)`
/// union-variant pattern match rather than a call — `CASE tcp::Socket(p)` /
/// `CASE __regex_Range(rng)` both parse identically to a qualified/internal
/// call syntactically (`name(`), but the parens bind a pattern variable, not
/// an argument list, and neither raises anything.
fn preceded_by_case(chars: &[char], start: usize) -> bool {
    let mut i = start;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    let end = i;
    let mut begin = i;
    while begin > 0 && chars[begin - 1].is_ascii_alphabetic() {
        begin -= 1;
    }
    let word: String = chars[begin..end].iter().collect();
    word == "CASE"
}

/// Every `pkg::name(` call in `text` (package, member name, top-level argument
/// texts — empty when the parenthesis never closes in `text`) that is not a
/// `CASE pkg::Variant(binding)` pattern match (see [`preceded_by_case`]).
fn qualified_calls_with_arguments(text: &str) -> Vec<(String, String, Vec<String>)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 1 < chars.len() {
        if chars[i] == ':' && chars[i + 1] == ':' {
            let mut start = i;
            while start > 0 && is_ident_char(chars[start - 1]) {
                start -= 1;
            }
            let pkg: String = chars[start..i].iter().collect();
            let mut end = i + 2;
            while end < chars.len() && is_ident_char(chars[end]) {
                end += 1;
            }
            let name: String = chars[i + 2..end].iter().collect();
            if !pkg.is_empty()
                && !name.is_empty()
                && end < chars.len()
                && chars[end] == '('
                && !preceded_by_case(&chars, start)
            {
                let args = call_arguments(&chars, end).unwrap_or_default();
                out.push((pkg, name, args));
            }
            i = end.max(i + 2);
            continue;
        }
        i += 1;
    }
    out
}

/// Every bare `__name(` call in `text` — the internal rewrite-symbol spelling
/// a body uses for a sibling member or a package-private helper — that is not
/// a `CASE __pkg_Variant(binding)` pattern match (see [`preceded_by_case`]).
fn internal_calls(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 1 < chars.len() {
        if chars[i] == '_' && chars[i + 1] == '_' && (i == 0 || !is_ident_char(chars[i - 1])) {
            let mut end = i;
            while end < chars.len() && is_ident_char(chars[end]) {
                end += 1;
            }
            let name: String = chars[i..end].iter().collect();
            if end < chars.len() && chars[end] == '(' && !preceded_by_case(&chars, i) {
                out.push(name);
            }
            i = end.max(i + 1);
            continue;
        }
        i += 1;
    }
    out
}

/// Every bare call to a `general` member (`toString(`, `toInt(`, `toByte(`, …)
/// in `text`, with its top-level argument texts — the unqualified globals a
/// body writes with no package prefix, which [`qualified_calls_with_arguments`]/
/// [`internal_calls`] cannot see (bug-807). A name counts only as a call: not
/// part of a longer identifier, not qualified (`x.len`/`pkg::len`), not inside
/// a string literal, and not the `error(` of a `FAIL error(` statement, which is
/// the raise itself and is already read by [`fail_codes`].
fn general_calls(text: &str, general: &BTreeSet<String>) -> Vec<(String, Vec<String>)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut in_string = false;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            in_string = !in_string;
            i += 1;
            continue;
        }
        if c == '\n' {
            in_string = false;
        }
        if in_string || !c.is_ascii_alphabetic() || (i > 0 && is_ident_char(chars[i - 1])) {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < chars.len() && is_ident_char(chars[end]) {
            end += 1;
        }
        let name: String = chars[i..end].iter().collect();
        let qualified = i > 0 && matches!(chars[i - 1], ':' | '.');
        let fail_error = name == "error" && {
            let line_start = chars[..i]
                .iter()
                .rposition(|c| *c == '\n')
                .map_or(0, |p| p + 1);
            let before: String = chars[line_start..i].iter().collect();
            before.trim_end().ends_with("FAIL")
        };
        if end < chars.len()
            && chars[end] == '('
            && !qualified
            && !fail_error
            && general.contains(&name)
            && !preceded_by_case(&chars, i)
        {
            if let Some(args) = call_arguments(&chars, end) {
                out.push((name, args));
            }
        }
        i = end;
    }
    out
}

/// The top-level, comma-separated argument texts of the call whose `(` is at
/// `open`, or `None` when the parenthesis never closes in `chars`.
fn call_arguments(chars: &[char], open: usize) -> Option<Vec<String>> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut args = Vec::new();
    let mut current = String::new();
    for &c in &chars[open..] {
        if in_string {
            current.push(c);
            if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                current.push(c);
            }
            '(' | '[' => {
                depth += 1;
                if depth > 1 {
                    current.push(c);
                }
            }
            ')' | ']' => {
                depth -= 1;
                if depth == 0 {
                    if !current.trim().is_empty() {
                        args.push(current.trim().to_string());
                    }
                    return Some(args);
                }
                current.push(c);
            }
            ',' if depth == 1 => {
                args.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    None
}

/// What a body's text says about the static type of its names: every `FUNC`
/// parameter, `LET`/`MUT` binding and `FOR` counter declared `name AS Type`
/// anywhere in the chunk being resolved, plus the package the chunk belongs to
/// (for its record field types). Every declaration of a name is kept; a name
/// is typed only when all of them resolve to the same type (two `FUNC`s in one
/// `Body::Mfb` string, or two `FOR EACH` loops, can reuse a name).
struct Scope {
    package: String,
    func: String,
    names: HashMap<String, Vec<String>>,
}

impl Scope {
    fn of(text: &str) -> Scope {
        let mut names: HashMap<String, Vec<String>> = HashMap::new();
        let mut declare = |name: &str, ty: &str| {
            let ty = ty.trim().to_string();
            if name.is_empty() || ty.is_empty() || !name.chars().all(is_ident_char) {
                return;
            }
            let seen = names.entry(name.to_string()).or_default();
            if !seen.contains(&ty) {
                seen.push(ty);
            }
        };
        let mut package = String::new();
        let mut func = String::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("FUNC ") {
                let name_len = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
                if package.is_empty() {
                    package = package_of_symbol(&rest[..name_len]).unwrap_or_default();
                    func = rest[..name_len].to_string();
                }
                if let (Some(open), Some(close)) = (rest.find('('), rest.rfind(')')) {
                    let chars: Vec<char> = rest.chars().collect();
                    let open_chars = rest[..open].chars().count();
                    let close_chars = rest[..close].chars().count();
                    if open_chars < close_chars {
                        if let Some(params) = call_arguments(&chars, open_chars) {
                            for param in params {
                                let param =
                                    param.trim_start_matches("RES ").trim_start_matches("MUT ");
                                if let Some((name, ty)) = param.split_once(" AS ") {
                                    let ty = ty.split(" STATE ").next().unwrap_or(ty);
                                    declare(name.trim(), ty);
                                }
                            }
                        }
                    }
                }
                continue;
            }
            // `CASE Variant(binding)`: a union arm binds the variant's record.
            if let Some(rest) = trimmed.strip_prefix("CASE ") {
                if let (Some(open), true) = (rest.find('('), rest.ends_with(')')) {
                    let variant = rest[..open].trim();
                    let binding = rest[open + 1..rest.len() - 1].trim();
                    if variant.chars().all(|c| is_ident_char(c) || c == ':') {
                        declare(binding, variant);
                    }
                }
            }
            for keyword in ["LET ", "MUT "] {
                if let Some(rest) = trimmed.strip_prefix(keyword) {
                    if let Some((name, rest)) = rest.split_once(" AS ") {
                        let ty = rest.split(" = ").next().unwrap_or(rest);
                        declare(name.trim(), ty);
                    }
                }
            }
            if let Some(rest) = trimmed.strip_prefix("FOR ") {
                if let Some(each) = rest.strip_prefix("EACH ") {
                    // The element type of the iterable, resolved on use
                    // (`infer_type`), since it needs the helper table.
                    if let Some((name, iterable)) = each.split_once(" IN ") {
                        declare(name.trim(), &format!("{EACH_PREFIX}{}", iterable.trim()));
                    }
                } else if let Some((name, _)) = rest.split_once(" = ") {
                    declare(name.trim(), "Integer");
                }
            }
        }
        Scope {
            package,
            func,
            names,
        }
    }
}

/// Every package-level `LET`/`MUT __NAME AS Type` a built-in's injected source
/// declares (`__CANVAS_GROUP_MAX_DEPTH AS Integer`), by name. A name declared
/// twice with different types is left out.
fn package_globals() -> &'static HashMap<String, String> {
    static GLOBALS: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    GLOBALS.get_or_init(|| {
        let mut seen: HashMap<String, Option<String>> = HashMap::new();
        let mut texts: Vec<&'static str> = Vec::new();
        for package in registry().packages() {
            texts.extend(package.helpers().iter().filter_map(|h| h.body));
            for function in package.functions() {
                for implementation in function.implementations() {
                    if let Body::Mfb { body, .. } = &implementation.body {
                        texts.push(body);
                    }
                }
            }
        }
        for text in texts {
            for line in text.lines() {
                let Some(rest) = line
                    .strip_prefix("LET __")
                    .or_else(|| line.strip_prefix("MUT __"))
                else {
                    continue;
                };
                let Some((name, rest)) = rest.split_once(" AS ") else {
                    continue;
                };
                let ty = rest.split(" = ").next().unwrap_or(rest).trim().to_string();
                seen.entry(format!("__{name}"))
                    .and_modify(|t| {
                        if t.as_deref() != Some(ty.as_str()) {
                            *t = None;
                        }
                    })
                    .or_insert(Some(ty));
            }
        }
        seen.into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect()
    })
}

/// A [`Scope`] entry for a `FOR EACH name IN <iterable>` binding: the
/// iterable's text after this prefix, typed on use.
const EACH_PREFIX: &str = "@each ";

/// A registry type name in the `pkg::Type` spelling a body writes
/// (`ParameterType::name` renders an imported type as `color.Color`).
fn source_spelling(ty: &str) -> String {
    match ty.split_once('.') {
        Some((package, name)) if !ty.contains(' ') => format!("{package}::{name}"),
        _ => ty.to_string(),
    }
}

/// `__datetime_format` -> `datetime`: the package a rewrite symbol or private
/// helper belongs to, by the `__<package>_<name>` convention every body uses.
fn package_of_symbol(symbol: &str) -> Option<String> {
    let rest = symbol.strip_prefix("__")?;
    let (package, _) = rest.split_once('_')?;
    Some(package.to_string())
}

/// Strip one pair of parentheses that wraps all of `expr`.
fn strip_outer_parens(expr: &str) -> &str {
    let expr = expr.trim();
    if expr.starts_with('(') && expr.ends_with(')') {
        let chars: Vec<char> = expr.chars().collect();
        if let Some(args) = call_arguments(&chars, 0) {
            if args.len() == 1 && args[0].chars().count() + 2 == chars.len() {
                return &expr[1..expr.len() - 1];
            }
        }
    }
    expr
}

/// Whether `expr` has a binary operator at its top level (outside every
/// parenthesis, bracket and string), so it is an operator expression rather
/// than one name, literal, field read or call.
fn has_top_level_operator(expr: &str) -> bool {
    let mut depth = 0i32;
    let mut in_string = false;
    let chars: Vec<char> = expr.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if in_string {
            if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '+' | '*' | '/' | '&' | '<' | '>' | '=' | '^' if depth == 0 => return true,
            '-' if depth == 0 && i > 0 => return true,
            ' ' if depth == 0 => {
                let rest: String = chars[i + 1..].iter().collect();
                for word in ["MOD ", "DIV ", "AND ", "OR ", "XOR "] {
                    if rest.starts_with(word) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Whether `expr`'s top-level operators are all arithmetic (`+ - * / ^ MOD
/// DIV`) and there is at least one — its value is a number, never a String,
/// Boolean or list (`&` concatenates, comparisons and `AND`/`OR` are Boolean).
fn is_arithmetic(expr: &str) -> bool {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut seen = false;
    let chars: Vec<char> = expr.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if in_string {
            if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '&' | '<' | '>' | '=' if depth == 0 => return false,
            '+' | '*' | '/' | '^' if depth == 0 => seen = true,
            '-' if depth == 0 && i > 0 => seen = true,
            ' ' if depth == 0 => {
                let rest: String = chars[i + 1..].iter().collect();
                if ["AND ", "OR ", "XOR ", "NOT "]
                    .iter()
                    .any(|w| rest.starts_with(w))
                {
                    return false;
                }
                if ["MOD ", "DIV "].iter().any(|w| rest.starts_with(w)) {
                    seen = true;
                }
            }
            _ => {}
        }
    }
    seen
}

/// The static type `expr` provably has, from the body text alone, or `None`.
///
/// Only shapes whose type the text fixes are answered: a literal; a declared
/// name ([`Scope`]); a field read off a declared record of the body's own
/// package; a `general` conversion (its result type is its name); a
/// `collections::get`/`getOr` off a declared `List OF T`; a qualified member,
/// private helper or rewrite symbol whose every implementation returns the one
/// concrete type; and a record constructor `T[...]`. Anything else — an
/// operator expression, a generic call — is `None`, and its general call then
/// counts every overload's codes. The direction matters: an unknown type
/// over-reports, which a human then resolves; a wrong type would hide a code.
fn infer_type(
    expr: &str,
    scope: &Scope,
    helper_blocks: &HashMap<String, &'static str>,
) -> Option<String> {
    thread_local! {
        static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    // Two `FOR EACH` bindings typed through each other would recurse forever;
    // no real expression nests this deep.
    if DEPTH.get() > 16 {
        return None;
    }
    DEPTH.set(DEPTH.get() + 1);
    let ty = infer_type_inner(expr, scope, helper_blocks);
    DEPTH.set(DEPTH.get() - 1);
    ty
}

fn infer_type_inner(
    expr: &str,
    scope: &Scope,
    helper_blocks: &HashMap<String, &'static str>,
) -> Option<String> {
    let expr = strip_outer_parens(expr);
    if expr.is_empty() || has_top_level_operator(expr) {
        return None;
    }
    if expr.starts_with('"') && expr.ends_with('"') && expr.len() >= 2 {
        return Some("String".to_string());
    }
    if expr == "TRUE" || expr == "FALSE" {
        return Some("Boolean".to_string());
    }
    let numeric = expr.strip_prefix('-').unwrap_or(expr);
    if !numeric.is_empty() && numeric.chars().all(|c| c.is_ascii_digit() || c == '_') {
        return Some("Integer".to_string());
    }
    if numeric.chars().next().is_some_and(|c| c.is_ascii_digit())
        && numeric
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '_' | '.' | 'e' | 'E' | '-' | '+'))
        && numeric.contains(['.', 'e', 'E'])
    {
        return Some("Float".to_string());
    }
    // A call: `name(...)`, `pkg::name(...)`, or a constructor `T[...]`.
    let head_len = expr
        .find(|c: char| !(is_ident_char(c) || c == ':'))
        .unwrap_or(expr.len());
    let head = &expr[..head_len];
    let tail = &expr[head_len..];
    if !head.is_empty() && tail.starts_with('[') && tail.ends_with(']') {
        let chars: Vec<char> = tail.chars().collect();
        if call_arguments(&chars, 0).is_some_and(|_| true) {
            return Some(head.to_string());
        }
    }
    if !head.is_empty() && tail.starts_with('(') && tail.ends_with(')') {
        let chars: Vec<char> = tail.chars().collect();
        let args = call_arguments(&chars, 0)?;
        // The call must span the whole expression (`f(x).y` is a field read).
        let consumed: usize = 2 + args.iter().map(|a| a.chars().count()).sum::<usize>();
        if consumed > chars.len() {
            return None;
        }
        if let Some((package, member)) = head.split_once("::") {
            if package == "collections" && matches!(member, "get" | "getOr") {
                let list = infer_type(args.first()?, scope, helper_blocks)?;
                return list.strip_prefix("List OF ").map(str::to_string);
            }
            return single_return_type(package, member, &args, scope, helper_blocks);
        }
        let general_result = match head {
            "len" => Some("Integer"),
            "toString" | "typeName" => Some("String"),
            "toInt" => Some("Integer"),
            "toFloat" => Some("Float"),
            "toFixed" => Some("Fixed"),
            "toMoney" => Some("Money"),
            "toByte" => Some("Byte"),
            "toScalar" => Some("Scalar"),
            "isNumeric" | "isEven" | "isOdd" | "isPositive" | "isNegative" | "isZero"
            | "isEmpty" | "isNotEmpty" => Some("Boolean"),
            _ => None,
        };
        if let Some(result) = general_result {
            return Some(result.to_string());
        }
        if head.starts_with("__") {
            if let Some(block) = helper_blocks.get(head) {
                let header = block.lines().next()?;
                let close = header.rfind(')')?;
                let ty = header[close + 1..].trim().strip_prefix("AS ")?.trim();
                return Some(ty.to_string());
            }
        }
        return None;
    }
    // A name, or a field read `name.field.field`.
    let mut parts = expr.split('.');
    let base = parts.next()?;
    if !base.chars().all(is_ident_char) {
        return None;
    }
    if !scope.names.contains_key(base) {
        let mut ty = package_globals().get(base)?.clone();
        for field in parts {
            ty = record_field_type(&scope.package, &ty, field)?;
        }
        return Some(ty);
    }
    let mut ty: Option<String> = None;
    for declared in scope.names.get(base)? {
        let resolved = match declared.strip_prefix(EACH_PREFIX) {
            Some(iterable) => {
                // A `FOR EACH` over its own binding's field would recurse
                // forever; one level of iterable is all a body ever needs.
                if iterable.split('.').next() == Some(base) {
                    return None;
                }
                let collection = infer_type(iterable, scope, helper_blocks)?;
                collection
                    .strip_prefix("List OF ")
                    .or_else(|| collection.strip_prefix("Set OF "))?
                    .to_string()
            }
            None => declared.clone(),
        };
        match &ty {
            Some(seen) if *seen != resolved => return None,
            _ => ty = Some(resolved),
        }
    }
    let mut ty = ty?;
    for field in parts {
        if !field.chars().all(is_ident_char) {
            return None;
        }
        ty = record_field_type(&scope.package, &ty, field)?;
    }
    Some(ty)
}

/// The type of `field` on the record `record` declared by `package` (a bare
/// record name inside its own package's source), or `None`.
fn record_field_type(package: &str, record: &str, field: &str) -> Option<String> {
    let record = source_spelling(record);
    let (package, record) = match record.split_once("::") {
        Some((p, r)) => (p, r),
        None => (package, record.as_str()),
    };
    let package = registry().resolve_package(package)?;
    let record = package.records().iter().find(|r| r.name == record)?;
    let prop = record.props.iter().find(|p| p.name == field)?;
    Some(source_spelling(&prop.ty.name()))
}

/// The one concrete return type every implementation of `package::member`
/// shares for this call, or `None` (unregistered, overloaded on return type, or
/// generic). An implementation that returns `Arg(n)` ("argument `n`'s type",
/// `math::sqrt`) answers with that argument's inferred type.
fn single_return_type(
    package: &str,
    member: &str,
    args: &[String],
    scope: &Scope,
    helper_blocks: &HashMap<String, &'static str>,
) -> Option<String> {
    let function = registry()
        .resolve_package(package)?
        .functions()
        .iter()
        .find(|f| f.name == member)?;
    let mut found: Option<String> = None;
    for implementation in function.implementations() {
        let ty = match &implementation.return_type {
            ParameterType::Arg(n) => infer_type(args.get(*n)?, scope, helper_blocks)?,
            other => {
                let name = other.name().to_string();
                if name.len() == 1 || name.contains(" OF T") || name.contains("Arg") {
                    return None;
                }
                source_spelling(&name)
            }
        };
        match &found {
            Some(seen) if *seen != ty => return None,
            _ => found = Some(ty),
        }
    }
    found
}

/// Every concrete return type of the qualified member `expr` calls, when it is
/// a whole `pkg::name(...)` call and none of its implementations is generic.
fn overloaded_return_types(expr: &str) -> Option<BTreeSet<String>> {
    let expr = strip_outer_parens(expr);
    if has_top_level_operator(expr) || !expr.ends_with(')') {
        return None;
    }
    let head = &expr[..expr.find('(')?];
    let (package, member) = head.split_once("::")?;
    let function = registry()
        .resolve_package(package)?
        .functions()
        .iter()
        .find(|f| f.name == member)?;
    let mut types = BTreeSet::new();
    for implementation in function.implementations() {
        let name = implementation.return_type.name().to_string();
        if name.len() == 1 || name.contains(" OF T") || name.contains("Arg") {
            return None;
        }
        types.insert(source_spelling(&name));
    }
    Some(types)
}

/// The private helper a package registers as its override of the general
/// `name` for the argument's (inferred) type — `toString(c)` on a
/// `color::Color` is `__color_toString(c)` — or `None`.
fn general_override_helper(
    name: &str,
    args: &[String],
    scope: &Scope,
    helper_blocks: &HashMap<String, &'static str>,
) -> Option<&'static str> {
    let ty = infer_type(args.first()?, scope, helper_blocks)?;
    let (package, bare) = match ty.split_once("::") {
        Some((p, t)) => (p.to_string(), t.to_string()),
        None => (scope.package.clone(), ty),
    };
    registry()
        .resolve_package(&package)?
        .overrides()
        .iter()
        .find(|o| o.builtin == name && o.arg_type == bare)
        .map(|o| o.helper)
}

/// The codes one qualified `pkg::name(args)` call can raise, given the member's
/// whole declared set (bug-814). Two shapes the text fixes are narrowed, both
/// in the direction that can only drop a code the call provably cannot raise:
///
/// - **A literal shift count.** `bits::sl`/`sr`/`sra` raise ErrInvalidArgument
///   only for a count outside 0 .. 63, so a literal count inside it cannot.
/// - **Overload selection by the collection argument.** A member whose
///   implementations differ in the kind of their first parameter (`List`,
///   `Map`, `Set`, `String`) — `collections::get` raises ErrIndexOutOfRange for
///   a List index and ErrNotFound for a missing Map key — raises only what the
///   implementations of the first argument's inferred kind declare. An argument
///   [`infer_type`] cannot type counts every implementation.
fn qualified_call_codes(
    pkg: &str,
    name: &str,
    args: &[String],
    declared: &BTreeSet<&'static str>,
    scope: &Scope,
    helper_blocks: &HashMap<String, &'static str>,
) -> BTreeSet<&'static str> {
    if pkg == "bits"
        && matches!(name, "sl" | "sr" | "sra")
        && args.len() == 2
        && args[1]
            .replace('_', "")
            .parse::<i64>()
            .is_ok_and(|count| (0..=63).contains(&count))
    {
        let mut raised = declared.clone();
        raised.remove("ErrInvalidArgument");
        return raised;
    }
    let Some(function) = registry()
        .resolve_package(pkg)
        .and_then(|package| package.functions().iter().find(|f| f.name == name))
    else {
        return declared.clone();
    };
    let kind = |ty: &str| -> Option<&'static str> {
        ["List OF ", "Map OF ", "Set OF "]
            .into_iter()
            .find(|prefix| ty.starts_with(prefix))
            .or((ty == "String").then_some("String"))
    };
    let kinds: Vec<Option<&'static str>> = function
        .implementations()
        .iter()
        .map(|implementation| {
            implementation
                .params
                .first()
                .and_then(|param| kind(&param.ty.name()))
        })
        .collect();
    let distinct: BTreeSet<_> = kinds.iter().flatten().collect();
    if kinds.iter().any(Option::is_none) || distinct.len() < 2 {
        return declared.clone();
    }
    let Some(arg_kind) = args
        .first()
        .and_then(|arg| infer_type(arg, scope, helper_blocks))
        .and_then(|ty| kind(&ty))
    else {
        return declared.clone();
    };
    let selected: Vec<_> = function
        .implementations()
        .iter()
        .zip(&kinds)
        .filter(|(_, k)| **k == Some(arg_kind))
        .collect();
    if selected.is_empty() {
        return declared.clone();
    }
    selected
        .into_iter()
        .flat_map(|(implementation, _)| implementation.errors.iter().copied())
        .collect()
}

/// The codes one bare `general` call can raise: the matching per-overload row
/// of `general_declared_errors.rs`'s [`OVERLOADS`] when every argument's type is
/// known ([`infer_type`]), narrowed further for an in-range literal; otherwise
/// the member's whole declared set. `None` if `name` is not a general member.
fn general_call_codes(
    name: &str,
    args: &[String],
    scope: &Scope,
    member_declared: &HashMap<(String, String), BTreeSet<&'static str>>,
    helper_blocks: &HashMap<String, &'static str>,
) -> Option<BTreeSet<&'static str>> {
    let declared = member_declared
        .get(&("general".to_string(), name.to_string()))?
        .clone();
    // `toByte(bits::band(x, mask))` with a literal mask in 0 .. 255 is in
    // 0 .. mask, so it always fits a Byte.
    if name == "toByte" && args.len() == 1 {
        if let Some(inner) = args[0].strip_prefix("bits::band(") {
            let chars: Vec<char> = args[0].chars().collect();
            let open = "bits::band".chars().count();
            if let Some(band_args) = call_arguments(&chars, open) {
                // The band call must be the whole argument, not an operand.
                let whole = inner.ends_with(')') && !has_top_level_operator(&args[0]);
                if whole
                    && band_args.len() == 2
                    && band_args[1]
                        .parse::<i64>()
                        .is_ok_and(|m| (0..=255).contains(&m))
                {
                    return Some(BTreeSet::new());
                }
            }
        }
    }
    // A literal argument in the target's range converts at compile time and
    // cannot fail: `toByte(255)`, `toFixed(1.0)`, `toMoney(0)`.
    if args.len() == 1 {
        let literal = args[0].replace('_', "");
        let value = literal.parse::<f64>().ok();
        let fits = match name {
            "toByte" => literal.parse::<i64>().is_ok_and(|v| (0..=255).contains(&v)),
            "toFixed" => value.is_some_and(|v| v.abs() < 2_147_483_648.0),
            "toMoney" => value.is_some_and(|v| v.abs() < 92_233_720_368_547.0),
            "toFloat" => value.is_some(),
            _ => false,
        };
        if fits {
            return Some(BTreeSet::new());
        }
    }
    // A one-argument call whose argument is a qualified member overloaded on
    // its return type (`math::floor` over Float/Fixed/…) can only be one of
    // those return types: union the rows for each.
    if args.len() == 1 && infer_type(&args[0], scope, helper_blocks).is_none() {
        if let Some(candidates) = overloaded_return_types(&args[0]) {
            let mut raised = BTreeSet::new();
            for candidate in &candidates {
                let row = OVERLOADS.iter().find(|(member, row_args, _, _)| {
                    *member == name && row_args == &[candidate.as_str()]
                });
                match row {
                    Some((_, _, _, codes)) => raised.extend(codes.iter().copied()),
                    None => return Some(declared),
                }
            }
            return Some(raised);
        }
    }
    let types: Option<Vec<String>> = args
        .iter()
        .map(|arg| infer_type(arg, scope, helper_blocks))
        .collect();
    // An arithmetic expression (`+ - * / ^ MOD DIV` at its top level, nothing
    // else) is a number: the call is one of the numeric overloads, so it can
    // raise what any of them can, and nothing only a String or list form can.
    if args.len() == 1 && is_arithmetic(strip_outer_parens(&args[0])) {
        let numeric = ["Integer", "Float", "Fixed", "Money", "Byte"];
        return Some(
            OVERLOADS
                .iter()
                .filter(|(member, row_args, _, _)| {
                    *member == name && row_args.len() == 1 && numeric.contains(&row_args[0])
                })
                .flat_map(|(_, _, _, codes)| codes.iter().copied())
                .collect(),
        );
    }
    let Some(types) = types else {
        // `toString` raises ErrEncoding only for `List OF Byte`; an operator
        // expression's value is a number, a Boolean or a String, never a list.
        if name == "toString"
            && args
                .first()
                .is_some_and(|a| has_top_level_operator(strip_outer_parens(a)))
        {
            return Some(BTreeSet::from(["ErrOutOfMemory"]));
        }
        return Some(declared);
    };
    let is_enum = |ty: &str| {
        registry()
            .resolve_package(&scope.package)
            .is_some_and(|p| p.enums().iter().any(|e| e.name == ty))
    };
    for (member, row_args, _, codes) in OVERLOADS {
        if *member != name || row_args.len() != types.len() {
            continue;
        }
        let matches = row_args
            .iter()
            .zip(&types)
            .all(|(row, ty)| *row == ty.as_str() || (*row == "Color" && is_enum(ty)));
        if matches {
            return Some(codes.iter().copied().collect());
        }
    }
    if name == "toString" && types.first().is_some_and(|t| t != "List OF Byte") {
        // A type no row names (a record with a package `toString` override is
        // never a general call's argument here; any other renders like a scalar).
        return Some(BTreeSet::from(["ErrOutOfMemory"]));
    }
    Some(declared)
}

/// Split a `RegistryHelper`/`Body::Mfb` source chunk (which may declare more
/// than one `FUNC`, e.g. `helper_scalar_seam.rs`'s seven) into its individual
/// `FUNC <name> ... END FUNC` blocks, keyed by name. `FUNC`/`END FUNC` sit at
/// column 0 in every body in this tree (byte-significant source, never
/// reformatted — see each body const's own doc comment), so a plain line scan
/// finds every block boundary.
fn extract_func_blocks(text: &'static str) -> Vec<(&'static str, &'static str)> {
    let mut blocks = Vec::new();
    let mut offset = 0usize;
    let mut current: Option<(usize, &'static str)> = None;
    let mut lines = text.split('\n').peekable();
    while let Some(line) = lines.next() {
        let line_start = offset;
        offset += line.len() + 1;
        if let Some(rest) = line.strip_prefix("FUNC ") {
            let name_len = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
            current = Some((line_start, &rest[..name_len]));
        } else if line.trim_end() == "END FUNC" {
            if let Some((start, name)) = current.take() {
                let end = (line_start + line.len()).min(text.len());
                blocks.push((name, &text[start..end]));
            }
        }
    }
    blocks
}

/// Strip every MFBASIC line comment (`'` at the start of a line, after
/// trimming leading whitespace) from `text` — a prose line like
/// `helper_percent_decode_impl.rs`'s own "A function-level TRAP maps every
/// failure ..." would otherwise be mistaken for actual `TRAP`/`PROPAGATE`
/// source by [`find_function_level_trap`]/[`find_inline_trap_regions`]. Only
/// a whole-line comment is stripped (every comment observed in this tree
/// starts at column 0 or after only whitespace); an inline trailing comment
/// after code on the same line does not occur in this codebase's style.
fn strip_comment_lines(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('\''))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Strip the leading `FUNC name(...) AS ...` header line and the trailing
/// `END FUNC` line from a `Body::Mfb`/`RegistryHelper` block, so the header's
/// own `name(` is never mistaken for a self-call by [`internal_calls`] (the
/// identifier immediately follows `FUNC `, a non-identifier character, which
/// otherwise satisfies every check that distinguishes a real call).
fn strip_func_header_and_footer(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = if lines.first().is_some_and(|l| l.starts_with("FUNC ")) {
        1
    } else {
        0
    };
    let end = if lines.last().is_some_and(|l| l.trim_end() == "END FUNC") {
        lines.len().saturating_sub(1)
    } else {
        lines.len()
    };
    if start >= end {
        return String::new();
    }
    lines[start..end].join("\n")
}

/// Whether `word` appears in `text` as a standalone all-caps keyword token
/// (not a substring of a longer identifier) — used to detect `PROPAGATE`
/// inside a `TRAP` handler.
fn contains_keyword(text: &str, word: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let wchars: Vec<char> = word.chars().collect();
    if wchars.is_empty() || chars.len() < wchars.len() {
        return false;
    }
    for i in 0..=chars.len() - wchars.len() {
        if chars[i..i + wchars.len()] == wchars[..]
            && (i == 0 || !is_ident_char(chars[i - 1]))
            && (i + wchars.len() == chars.len() || !is_ident_char(chars[i + wchars.len()]))
        {
            return true;
        }
    }
    false
}

/// Find the body's one function-level `TRAP` (spec `08_error-model.md` §8.3:
/// "at most one function-level `TRAP` per function, at the bottom") — a line
/// that, once trimmed, is exactly `TRAP` or `TRAP(<ident>)` with nothing else
/// on it (column 0, like `FUNC`/`END FUNC`), paired with the next bare
/// `END TRAP` line. Returns `(trap_line_index, end_trap_line_index)`.
fn find_function_level_trap(lines: &[&str]) -> Option<(usize, usize)> {
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        // A function-level TRAP is indented like any other statement (it is
        // NOT a column-0 block keyword like FUNC/END FUNC -- confirmed against
        // helper_percent_decode_impl.rs's own `  TRAP(e)` at 2-space indent).
        // What distinguishes it from an inline `<expr> TRAP(e)` postfix is that
        // `TRAP`/`TRAP(e)` is the ENTIRE line once trimmed -- nothing precedes
        // it on the same line.
        let is_bare_trap_header =
            trimmed == "TRAP" || (trimmed.starts_with("TRAP(") && trimmed.ends_with(')'));
        if is_bare_trap_header {
            if let Some(end_offset) = lines[i + 1..].iter().position(|l| l.trim() == "END TRAP") {
                return Some((i, i + 1 + end_offset));
            }
        }
    }
    None
}

/// Find every **inline** `TRAP` region (spec §8.4): a postfix `TRAP`/`TRAP(e)`
/// following other content on the same line (the trapped expression), with
/// the handler on the following indented lines up to a matching `END TRAP`.
/// Returns `(start_line, end_line_inclusive, trapped_expr_text, handler_text)`
/// per region, in source order; regions are assumed non-nested (the language
/// scopes an inline trap to one expression, and this tree's style never nests
/// one inline `TRAP`'s handler inside another on one line).
fn find_inline_trap_regions(lines: &[&str]) -> Vec<(usize, usize, String, String)> {
    let mut regions = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        // The bare/function-level form starts the trimmed line with `TRAP`;
        // skip it here (handled by `find_function_level_trap`).
        if !trimmed.starts_with("TRAP") {
            let cut = if let Some(pos) = line.find(" TRAP(") {
                Some(pos)
            } else {
                let end = line.trim_end();
                end.strip_suffix(" TRAP").map(|_| end.len() - 5)
            };
            if let Some(pos) = cut {
                if let Some(end_offset) = lines[i + 1..].iter().position(|l| l.trim() == "END TRAP")
                {
                    let end = i + 1 + end_offset;
                    let trapped_expr = line[..pos].to_string();
                    let handler = lines[i + 1..end].join("\n");
                    regions.push((i, end, trapped_expr, handler));
                    i = end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    regions
}

/// Scan `text` for its own `FAIL` sites and every call it makes — a `pkg::name(`
/// call (trust the callee member's own declared `errors`) or a bare `__name(`
/// call (another member's rewrite symbol, same treatment, or a package-private
/// helper, inlined via [`resolve`] recursively). Does **not** itself understand
/// `TRAP`/`RECOVER`/`PROPAGATE` — [`resolve`] is the `TRAP`-aware entry point
/// that calls this on exactly the sub-texts a `TRAP`'s semantics expose (the
/// trapped region, the handler, and the untrapped remainder), per spec
/// `08_error-model.md` §8.3/§8.4.
#[allow(clippy::too_many_arguments)]
fn scan_calls_and_fails(
    text: &str,
    scope: &Scope,
    codes: &BTreeMap<&'static str, &'static str>,
    rewrite_to_owner: &HashMap<String, (String, String)>,
    member_declared: &HashMap<(String, String), BTreeSet<&'static str>>,
    helper_blocks: &HashMap<String, &'static str>,
    visiting: &mut BTreeSet<String>,
    memo: &mut HashMap<String, BTreeSet<&'static str>>,
    failures: &mut Vec<String>,
) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    for code in fail_codes(text) {
        match codes.get(code.as_str()).copied() {
            Some(name) => {
                out.insert(name);
            }
            None => failures.push(format!(
                "FAIL error({code}) does not name a registered errorCode value"
            )),
        }
    }
    for (pkg, name, args) in qualified_calls_with_arguments(text) {
        match member_declared.get(&(pkg.clone(), name.clone())) {
            Some(declared) => {
                let mut raised =
                    qualified_call_codes(&pkg, &name, &args, declared, scope, helper_blocks);
                let callee = format!("{pkg}::{name}");
                raised.retain(|code| {
                    !UNREACHABLE_QUALIFIED_CALL
                        .iter()
                        .any(|(func, member, unreachable)| {
                            *func == scope.func && *member == callee && unreachable == code
                        })
                });
                out.extend(raised)
            }
            None => failures.push(format!(
                "call `{pkg}::{name}` does not resolve to a registered member"
            )),
        }
    }
    let mut overrides = Vec::new();
    for (name, args) in general_calls(text, &general_names()) {
        // A package's own `toString(<its type>)` override is a call to that
        // package's private helper, resolved like any other below.
        if let Some(helper) = general_override_helper(&name, &args, scope, helper_blocks) {
            overrides.push(helper.to_string());
            continue;
        }
        match general_call_codes(&name, &args, scope, member_declared, helper_blocks) {
            Some(mut raised) => {
                raised.retain(|code| {
                    !UNREACHABLE_GENERAL_CALL
                        .iter()
                        .any(|(func, member, unreachable)| {
                            *func == scope.func && *member == name && unreachable == code
                        })
                });
                out.extend(raised)
            }
            None => failures.push(format!("bare call `{name}` is not a general member")),
        }
    }
    for sym in internal_calls(text).into_iter().chain(overrides) {
        if let Some(owner) = rewrite_to_owner.get(&sym) {
            if let Some(declared) = member_declared.get(owner) {
                out.extend(declared.iter().copied());
            }
            continue;
        }
        if let Some(block) = helper_blocks.get(&sym).copied() {
            if visiting.contains(&sym) {
                // A cycle through package-private helpers — nothing new to add on
                // the repeat visit; the first visit's scan already covers it.
                continue;
            }
            if let Some(cached) = memo.get(&sym) {
                out.extend(cached.iter().copied());
                continue;
            }
            visiting.insert(sym.clone());
            // Recurse through the TRAP-aware entry point, not back into this
            // plain scanner — a helper (e.g. `__net_percentDecodeImpl`) can
            // carry its own function-level `TRAP` just like a public member.
            let inner = resolve(
                block,
                codes,
                rewrite_to_owner,
                member_declared,
                helper_blocks,
                visiting,
                memo,
                failures,
            );
            visiting.remove(&sym);
            memo.insert(sym.clone(), inner.clone());
            out.extend(inner);
            continue;
        }
        failures.push(format!(
            "internal call `{sym}` does not resolve to a member's rewrite target or a \
             registered helper FUNC"
        ));
    }
    out
}

/// As [`scan_calls_and_fails`], but first carves out every **inline** `TRAP`
/// region (spec §8.4) in `text`: each one's trapped expression and handler are
/// scanned separately, the handler's escape (its own `FAIL`/calls, plus the
/// trapped expression's own codes too if the handler `PROPAGATE`s) replaces
/// whatever the trapped expression alone would have contributed, and the
/// untrapped remainder of `text` is scanned normally. `RECOVER` needs no
/// special case: it is simply the case where the handler contains no
/// `PROPAGATE` and no `FAIL`/call of its own, so nothing escapes for that
/// region — exactly `__net_decodeQueryComponent`'s `RECOVER s`.
#[allow(clippy::too_many_arguments)]
fn resolve_inline_traps(
    text: &str,
    scope: &Scope,
    codes: &BTreeMap<&'static str, &'static str>,
    rewrite_to_owner: &HashMap<String, (String, String)>,
    member_declared: &HashMap<(String, String), BTreeSet<&'static str>>,
    helper_blocks: &HashMap<String, &'static str>,
    visiting: &mut BTreeSet<String>,
    memo: &mut HashMap<String, BTreeSet<&'static str>>,
    failures: &mut Vec<String>,
) -> BTreeSet<&'static str> {
    let lines: Vec<&str> = text.lines().collect();
    let regions = find_inline_trap_regions(&lines);
    if regions.is_empty() {
        return scan_calls_and_fails(
            text,
            scope,
            codes,
            rewrite_to_owner,
            member_declared,
            helper_blocks,
            visiting,
            memo,
            failures,
        );
    }
    let mut out = BTreeSet::new();
    let mut consumed: BTreeSet<usize> = BTreeSet::new();
    for (start, end, trapped_expr, handler) in &regions {
        for l in *start..=*end {
            consumed.insert(l);
        }
        let trapped_set = scan_calls_and_fails(
            trapped_expr,
            scope,
            codes,
            rewrite_to_owner,
            member_declared,
            helper_blocks,
            visiting,
            memo,
            failures,
        );
        let handler_set = scan_calls_and_fails(
            handler,
            scope,
            codes,
            rewrite_to_owner,
            member_declared,
            helper_blocks,
            visiting,
            memo,
            failures,
        );
        out.extend(handler_set);
        if contains_keyword(handler, "PROPAGATE") {
            out.extend(trapped_set);
        }
    }
    let remaining: String = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| !consumed.contains(i))
        .map(|(_, l)| *l)
        .collect::<Vec<_>>()
        .join("\n");
    out.extend(scan_calls_and_fails(
        &remaining,
        scope,
        codes,
        rewrite_to_owner,
        member_declared,
        helper_blocks,
        visiting,
        memo,
        failures,
    ));
    out
}

/// Resolve the error-code set a body's text can raise, per spec
/// `08_error-model.md`: its own `FAIL` sites and calls (via
/// [`scan_calls_and_fails`]/[`resolve_inline_traps`]), but with every error a
/// function-level `TRAP` (§8.3) absorbs removed — only what the `TRAP`
/// handler itself raises (its own `FAIL`/calls, plus whatever the trapped
/// body could raise if the handler `PROPAGATE`s) escapes the function.
/// Comment lines and the `FUNC`/`END FUNC` wrapper are stripped first so
/// neither confuses the `TRAP`/call scan (see [`strip_comment_lines`]/
/// [`strip_func_header_and_footer`]).
#[allow(clippy::too_many_arguments)]
fn resolve(
    text: &str,
    codes: &BTreeMap<&'static str, &'static str>,
    rewrite_to_owner: &HashMap<String, (String, String)>,
    member_declared: &HashMap<(String, String), BTreeSet<&'static str>>,
    helper_blocks: &HashMap<String, &'static str>,
    visiting: &mut BTreeSet<String>,
    memo: &mut HashMap<String, BTreeSet<&'static str>>,
    failures: &mut Vec<String>,
) -> BTreeSet<&'static str> {
    let scope = Scope::of(text);
    let scope = &scope;
    let stripped = strip_func_header_and_footer(&strip_comment_lines(text));
    let lines: Vec<&str> = stripped.lines().collect();
    if let Some((trap_idx, end_idx)) = find_function_level_trap(&lines) {
        let normal_part = lines[..trap_idx].join("\n");
        let handler_part = lines[trap_idx + 1..end_idx].join("\n");
        let trapped_set = resolve_inline_traps(
            &normal_part,
            scope,
            codes,
            rewrite_to_owner,
            member_declared,
            helper_blocks,
            visiting,
            memo,
            failures,
        );
        let handler_set = resolve_inline_traps(
            &handler_part,
            scope,
            codes,
            rewrite_to_owner,
            member_declared,
            helper_blocks,
            visiting,
            memo,
            failures,
        );
        let mut escapes = handler_set;
        if contains_keyword(&handler_part, "PROPAGATE") {
            escapes.extend(trapped_set);
        }
        return escapes;
    }
    resolve_inline_traps(
        &stripped,
        scope,
        codes,
        rewrite_to_owner,
        member_declared,
        helper_blocks,
        visiting,
        memo,
        failures,
    )
}

/// The result of sweeping every `Body::Mfb` member in the registry.
pub(crate) struct MfbSweepResult {
    pub(crate) failures: Vec<String>,
    pub(crate) mismatched: Vec<String>,
    pub(crate) measured_members: usize,
}

/// A code this guard's own function-level (not per-overload, not
/// argument-sensitive) call propagation reports as reachable for `member`,
/// but which a human reading `member`'s own source has proven can never
/// actually fire — the `Body::Mfb` counterpart of
/// `declared_errors_sweep.rs`'s `unreachable_own_raise`/
/// `unreachable_via_callee` (same evidentiary bar: cite the exact loop
/// invariant or argument shape that makes the callee's check unreachable,
/// not "it seems unlikely"). Phase 2 adds an entry here instead of declaring
/// a code a member cannot actually raise, and instead of leaving a
/// permanently-red baseline entry for a false positive.
///
/// `astrings::toMarkdown` calls `astrings::getAttributes`/`strings::mid`
/// (via `__astrings_mdStateAt`) with an index `i` bounded by its own `WHILE i
/// < n` loop, where `n = astrings::scalarLen(a)` is the exact same bound
/// both callees range-check against (`getAttributes`'s `index < 0 OR index
/// >= n` and `mid`'s scalar-unit bound, confirmed matching units: both
/// `scalarLen` and `strings::mid` count Unicode scalars, not bytes or UTF-16
/// units) — so the index can never be out of range for either call.
///
/// **Keyed by `(package, member, code)`, never bare `member`** — this sweep
/// is whole-registry, and a bare member name collides across packages that
/// happen to share one (`datetime::parse`/`csv::parse`/`json::parse`): an
/// entry proven for one would silently also suppress a REAL gap on another
/// package's same-named member. Coordinator correction, 2026-10-03.
const UNREACHABLE_VIA_CALLEE: &[(&str, &str, &str)] = &[
    ("astrings", "toMarkdown", "ErrIndexOutOfRange"),
    // audio (`render`'s ErrInvalidArgument row is gone: since bug-810 render
    // raises it itself, for a note past the ten-minute limit)
    ("audio", "play", "ErrFloatInf"),
    ("audio", "play", "ErrFloatNaN"),
    ("audio", "play", "ErrOverflow"),
    ("audio", "play", "ErrIndexOutOfRange"),
    ("audio", "play", "ErrNotFound"),
    // canvas (the loadImage ErrInvalidFormat/ErrTooLarge and loadFont
    // ErrInvalidFormat findings the canvas agent reported are TRAP-absorbed
    // via __canvas_pngDecode's/__canvas_faceName's inline `TRAP(err) RETURN
    // ... END TRAP` -- the TRAP-aware resolve() now excludes them on its own,
    // so they are deliberately NOT listed here, per the coordinator's "prune
    // redundant entries" instruction)
    ("canvas", "present", "ErrIndexOutOfRange"),
    ("canvas", "present", "ErrInvalidArgument"),
    ("canvas", "present", "ErrResourceClosed"),
    ("canvas", "presentLayers", "ErrInvalidArgument"),
    ("canvas", "loadImage", "ErrIndexOutOfRange"),
    ("canvas", "loadFont", "ErrIndexOutOfRange"),
    ("canvas", "listSystemFonts", "ErrIndexOutOfRange"),
    ("canvas", "listSystemFonts", "ErrInvalidFormat"),
    ("canvas", "loadSystemFont", "ErrIndexOutOfRange"),
    ("canvas", "loadSystemFont", "ErrInvalidFormat"),
    ("canvas", "measureText", "ErrIndexOutOfRange"),
    // color
    ("color", "toPacked", "ErrInvalidArgument"),
    ("color", "fromPacked", "ErrInvalidArgument"),
    ("color", "fromHex", "ErrIndexOutOfRange"),
    ("color", "fromHex", "ErrNotFound"),
    ("color", "toHex", "ErrIndexOutOfRange"),
    ("color", "toHexAlpha", "ErrIndexOutOfRange"),
    ("color", "toHsl", "ErrInvalidArgument"),
    ("color", "toHsl", "ErrOverflow"),
    ("color", "saturate", "ErrInvalidArgument"),
    ("color", "saturate", "ErrOverflow"),
    ("color", "desaturate", "ErrInvalidArgument"),
    ("color", "desaturate", "ErrOverflow"),
    ("color", "rotateHue", "ErrInvalidArgument"),
    ("color", "rotateHue", "ErrOverflow"),
    ("color", "hsl", "ErrOverflow"),
    ("color", "hsla", "ErrOverflow"),
    ("color", "fromName", "ErrIndexOutOfRange"),
    ("color", "nameOf", "ErrIndexOutOfRange"),
    // csv
    ("csv", "parse", "ErrIndexOutOfRange"),
    ("csv", "parse", "ErrNotFound"),
    ("csv", "stringify", "ErrIndexOutOfRange"),
    ("csv", "stringify", "ErrNotFound"),
    ("csv", "stringify", "ErrInvalidArgument"),
    ("csv", "parseStream", "ErrIndexOutOfRange"),
    ("csv", "parseStream", "ErrNotFound"),
    ("csv", "readRow", "ErrNotFound"),
    // readRow (`__csv_next`) first FAILs ErrInvalidArgument unless `count =
    // len(chars)` and `0 <= index <= count`; every later read is
    // `collections::get(chars, i)` with `i` in `index .. count - 1` (the `WHILE
    // index < count` loop, `__csv_isDoubledQuote`/`__csv_separatorLength`'s own
    // `index + 1 < count` guards) or `__csv_decodeRange` over `fieldStart ..
    // index`, a sub-range of the same bounds.
    ("csv", "readRow", "ErrIndexOutOfRange"),
    // crypto
    ("crypto", "uuid7", "ErrIndexOutOfRange"),
    ("crypto", "uuid7", "ErrNotFound"),
    ("crypto", "uuid7", "ErrInvalidArgument"),
    ("crypto", "ulid", "ErrNotFound"),
    ("crypto", "ulid", "ErrInvalidArgument"),
    // json
    ("json", "get", "ErrIndexOutOfRange"),
    ("json", "getOr", "ErrIndexOutOfRange"),
    ("json", "getOr", "ErrNotFound"),
    ("json", "parse", "ErrIndexOutOfRange"),
    ("json", "parse", "ErrNotFound"),
    ("json", "stringify", "ErrIndexOutOfRange"),
    ("json", "stringify", "ErrInvalidArgument"),
    ("json", "stringify", "ErrNotFound"),
    // regex
    ("regex", "find", "ErrInvalidArgument"),
    ("regex", "findAll", "ErrInvalidArgument"),
    ("regex", "match", "ErrInvalidArgument"),
    ("regex", "findMatch", "ErrInvalidArgument"),
    ("regex", "findAllMatches", "ErrInvalidArgument"),
    ("regex", "findAll", "ErrNotFound"),
    ("regex", "match", "ErrNotFound"),
    ("regex", "replace", "ErrNotFound"),
    ("regex", "findMatch", "ErrNotFound"),
    ("regex", "findAllMatches", "ErrNotFound"),
    ("regex", "count", "ErrNotFound"),
    ("regex", "split", "ErrNotFound"),
    ("regex", "match", "ErrIndexOutOfRange"),
    ("regex", "replace", "ErrIndexOutOfRange"),
    ("regex", "split", "ErrIndexOutOfRange"),
    // `__encoding_fromCodepoint` encodes a surrogate (55296 .. 57343) as three
    // bytes `toString` rejects with ErrEncoding (bug-807); these callers never
    // pass one. csv's `__csv_decodeRange` and `utf32Decode` FAIL on a surrogate
    // first; `utf16Decode` passes either a paired scalar (>= 65536) or a unit it
    // has checked is not a surrogate; `codepageDecode` passes only `n < 128`.
    ("csv", "parse", "ErrEncoding"),
    ("csv", "readRow", "ErrEncoding"),
    ("encoding", "utf16Decode", "ErrEncoding"),
    ("encoding", "utf32Decode", "ErrEncoding"),
    ("encoding", "codepageDecode", "ErrEncoding"),
    // datetime
    // parseIso's own `__datetime_readOffset` reads every field through
    // `__datetime_peek`, which range-checks `start + count <= len(value)`
    // before ever calling `strings::mid` (helper_peek.rs's own doc comment:
    // "the ErrIndexOutOfRange a bare mid past the end would raise" is
    // exactly what the guard precedes) -- so the ErrIndexOutOfRange
    // `strings::mid` declares can never fire through this caller.
    ("datetime", "parseIso", "ErrIndexOutOfRange"),
    // parseIso calls `__datetime_fixedOffset1` (the ONE-argument overload of
    // `datetime::fixedOffset`, which alone declares only ErrInvalidArgument)
    // with a magnitude `readOffset` has already bounded under 24h (hours
    // 00-23, minutes/seconds 00-59) -- exactly fixedOffset1's own `<=
    // -86400 OR >= 86400` check, which can therefore never fail here. The
    // guard's function-level (not per-overload) union pulls in
    // ErrInvalidArgument/ErrOverflow from fixedOffset's UNRELATED
    // two-argument overload (`__datetime_fixedOffset2`), which parseIso
    // never calls.
    ("datetime", "parseIso", "ErrInvalidArgument"),
    ("datetime", "parseIso", "ErrOverflow"),
    ("datetime", "toUtc", "ErrInvalidArgument"),
    ("datetime", "toUtc", "ErrOverflow"),
    // format's only collections::get is `__datetime_monthName`'s
    // `names[month - 1]`, reached solely from `__datetime_formatToken`'s
    // month-name arm, which first FAILs ErrInvalidArgument for a `month`
    // outside 1 .. 12 -- so the index is always 0 .. 11 of a 12-element list.
    // (`__datetime_weekdayName`'s index is `__datetime_floorMod(_, 7)`, always
    // 0 .. 6 of a 7-element list.)
    ("datetime", "format", "ErrIndexOutOfRange"),
    ("datetime", "format", "ErrNotFound"),
    ("datetime", "parse", "ErrIndexOutOfRange"),
    ("datetime", "parse", "ErrNotFound"),
    ("datetime", "toIso", "ErrIndexOutOfRange"),
    ("datetime", "formatDuration", "ErrInvalidArgument"),
    // net
    ("net", "toUrl", "ErrIndexOutOfRange"),
    ("net", "toUrl", "ErrNotFound"),
    ("net", "parseQuery", "ErrIndexOutOfRange"),
    ("net", "parseQuery", "ErrInvalidArgument"),
    ("net", "parseQuery", "ErrNotFound"),
    // vector
    ("vector", "length", "ErrFloatDomain"),
    ("vector", "length", "ErrInvalidArgument"),
    ("vector", "distance", "ErrFloatDomain"),
    ("vector", "distance", "ErrInvalidArgument"),
    ("vector", "normalize", "ErrFloatDomain"),
    ("vector", "normalize", "ErrOverflow"),
    ("vector", "angle", "ErrFloatDomain"),
    ("vector", "angle", "ErrOverflow"),
    ("vector", "lerp", "ErrInvalidArgument"),
    ("vector", "slerp", "ErrFloatDomain"),
    ("vector", "slerp", "ErrFloatNaN"),
    ("vector", "clamp_length", "ErrFloatDomain"),
    ("vector", "clamp_length", "ErrOverflow"),
    ("vector", "min", "ErrInvalidArgument"),
    ("vector", "max", "ErrInvalidArgument"),
    ("vector", "rotate_2d", "ErrFloatNaN"),
    ("vector", "rotate_2d", "ErrOverflow"),
    // encoding (subset A)
    ("encoding", "utf16Encode", "ErrIndexOutOfRange"),
    ("encoding", "utf16Encode", "ErrInvalidArgument"),
    ("encoding", "utf16Encode", "ErrNotFound"),
    ("encoding", "utf16Decode", "ErrIndexOutOfRange"),
    ("encoding", "utf16Decode", "ErrInvalidArgument"),
    ("encoding", "utf16Decode", "ErrNotFound"),
    ("encoding", "utf32Encode", "ErrIndexOutOfRange"),
    ("encoding", "utf32Encode", "ErrNotFound"),
    ("encoding", "hexEncode", "ErrIndexOutOfRange"),
    ("encoding", "hexDecode", "ErrIndexOutOfRange"),
    ("encoding", "hexDecode", "ErrNotFound"),
    ("encoding", "base32Encode", "ErrIndexOutOfRange"),
    ("encoding", "base32Encode", "ErrInvalidArgument"),
    ("encoding", "base32Decode", "ErrIndexOutOfRange"),
    ("encoding", "base32Decode", "ErrInvalidArgument"),
    ("encoding", "base32Decode", "ErrNotFound"),
    ("encoding", "base64Encode", "ErrIndexOutOfRange"),
    ("encoding", "base64Encode", "ErrInvalidArgument"),
    ("encoding", "base64Decode", "ErrIndexOutOfRange"),
    ("encoding", "base64Decode", "ErrInvalidArgument"),
    ("encoding", "base64Decode", "ErrNotFound"),
    ("encoding", "base64UrlEncode", "ErrIndexOutOfRange"),
    ("encoding", "base64UrlEncode", "ErrInvalidArgument"),
    ("encoding", "base64UrlDecode", "ErrIndexOutOfRange"),
    ("encoding", "base64UrlDecode", "ErrInvalidArgument"),
    ("encoding", "base64UrlDecode", "ErrNotFound"),
    ("encoding", "percentEncode", "ErrIndexOutOfRange"),
    ("encoding", "percentDecode", "ErrIndexOutOfRange"),
    ("encoding", "percentDecode", "ErrNotFound"),
    // encoding (subset B)
    ("encoding", "htmlEscape", "ErrIndexOutOfRange"),
    ("encoding", "htmlUnescape", "ErrIndexOutOfRange"),
    ("encoding", "htmlUnescape", "ErrNotFound"),
    ("encoding", "formUrlEncode", "ErrIndexOutOfRange"),
    ("encoding", "formUrlDecode", "ErrIndexOutOfRange"),
    ("encoding", "formUrlDecode", "ErrNotFound"),
    ("encoding", "punycodeEncode", "ErrIndexOutOfRange"),
    ("encoding", "punycodeEncode", "ErrNotFound"),
    ("encoding", "punycodeEncode", "ErrInvalidArgument"),
    ("encoding", "punycodeDecode", "ErrIndexOutOfRange"),
    ("encoding", "punycodeDecode", "ErrNotFound"),
    ("encoding", "punycodeDecode", "ErrInvalidArgument"),
    ("encoding", "uleb128Encode", "ErrInvalidArgument"),
    ("encoding", "uleb128Decode", "ErrIndexOutOfRange"),
    ("encoding", "uleb128Decode", "ErrNotFound"),
    ("encoding", "uleb128Decode", "ErrInvalidArgument"),
    ("encoding", "sleb128Encode", "ErrInvalidArgument"),
    ("encoding", "sleb128Decode", "ErrIndexOutOfRange"),
    ("encoding", "sleb128Decode", "ErrNotFound"),
    ("encoding", "sleb128Decode", "ErrInvalidArgument"),
    ("encoding", "varintEncode", "ErrInvalidArgument"),
    ("encoding", "varintDecode", "ErrIndexOutOfRange"),
    ("encoding", "varintDecode", "ErrNotFound"),
    ("encoding", "varintDecode", "ErrInvalidArgument"),
    ("encoding", "codepageDecode", "ErrIndexOutOfRange"),
    ("encoding", "codepageDecode", "ErrNotFound"),
    ("encoding", "codepageEncode", "ErrIndexOutOfRange"),
    ("encoding", "codepageEncode", "ErrNotFound"),
    // collections (subset A)
    ("collections", "sort", "ErrIndexOutOfRange"),
    ("collections", "sort", "ErrNotFound"),
    ("collections", "sortBy", "ErrIndexOutOfRange"),
    ("collections", "sortBy", "ErrNotFound"),
    ("collections", "take", "ErrIndexOutOfRange"),
    ("collections", "take", "ErrNotFound"),
    ("collections", "drop", "ErrIndexOutOfRange"),
    ("collections", "drop", "ErrNotFound"),
    ("collections", "any", "ErrIndexOutOfRange"),
    ("collections", "any", "ErrNotFound"),
    ("collections", "all", "ErrIndexOutOfRange"),
    ("collections", "all", "ErrNotFound"),
    ("collections", "groupBy", "ErrIndexOutOfRange"),
    ("collections", "groupBy", "ErrNotFound"),
    ("collections", "mapValues", "ErrIndexOutOfRange"),
    ("collections", "flatten", "ErrIndexOutOfRange"),
    ("collections", "flatten", "ErrNotFound"),
    ("collections", "zip", "ErrIndexOutOfRange"),
    ("collections", "zip", "ErrNotFound"),
    // collections (subset B)
    ("collections", "chunks", "ErrIndexOutOfRange"),
    ("collections", "chunks", "ErrNotFound"),
    ("collections", "window", "ErrIndexOutOfRange"),
    ("collections", "window", "ErrNotFound"),
    ("collections", "distinct", "ErrIndexOutOfRange"),
    ("collections", "distinct", "ErrNotFound"),
    ("collections", "partition", "ErrIndexOutOfRange"),
    ("collections", "partition", "ErrNotFound"),
    ("collections", "merge", "ErrIndexOutOfRange"),
    // http
    ("http", "ok", "ErrIndexOutOfRange"),
    ("http", "status", "ErrIndexOutOfRange"),
    ("http", "json", "ErrIndexOutOfRange"),
    ("http", "withHeader", "ErrIndexOutOfRange"),
    ("http", "route", "ErrIndexOutOfRange"),
    ("http", "route", "ErrNotFound"),
    ("http", "respondFile", "ErrIndexOutOfRange"),
    ("http", "respondPath", "ErrIndexOutOfRange"),
    ("http", "respondPath", "ErrNotFound"),
];

/// A code a bare `general` call's overload can raise that no call of that
/// member in that one `FUNC` (a member body or a private helper) can actually
/// raise, keyed `(FUNC, general member, code)` and applied where the call is,
/// so every member that reaches the `FUNC` inherits the proof (bug-807). The
/// proof must cover every call of that member in the `FUNC`, from its
/// arguments' ranges; a proof that depends on the caller names the callers.
const UNREACHABLE_GENERAL_CALL: &[(&str, &str, &str)] = &[
    // Every toByte argument is in 0 .. 255: `value` after the `< 0` / `> 255`
    // guards above it, and the literals 0 and 255.
    ("__color_clampByte", "toByte", "ErrOverflow"),
    // `255 - toInt(<Byte channel>)` is in 0 .. 255.
    ("__color_invert", "toByte", "ErrOverflow"),
    // `lo` starts at 0 and `hi` at 255; `lo = mid + 1` runs only while `lo < hi`
    // with `mid < hi`, so `lo` ends in 0 .. 255.
    ("__color_fromLinear", "toByte", "ErrOverflow"),
    // Every `digits` entry is a `__color_hexValue` result checked `>= 0`, so
    // 0 .. 15: `d * 17` and `d * 16 + e` are at most 255.
    ("__color_fromHex", "toByte", "ErrOverflow"),
    // `math::floor` of a Float is an Integer, and toFloat of an Integer cannot
    // fail.
    ("__color_hslToColor", "toFloat", "ErrInvalidFormat"),
    ("__color_hslToColor", "toFloat", "ErrOverflow"),
    ("__audio_mmlSynth", "toFloat", "ErrInvalidFormat"),
    ("__audio_mmlSynth", "toFloat", "ErrOverflow"),
    // `t` is `__color_clampFraction(amount)`, so 0.0 .. 1.0: `aa + toInt((ba -
    // aa) * t)` lies between the two Byte alphas, and the product fits an
    // Integer.
    ("__color_mix", "toByte", "ErrOverflow"),
    ("__color_mix", "toInt", "ErrOverflow"),
    // The channel helpers' only callers (`__color_brighten`, `__color_darken`,
    // `__color_mix`) pass `amount` through `__color_clampFraction` (0.0 .. 1.0),
    // and `color::toLinear` is in 0 .. 65535, so each product is at most 65535.
    ("__color_brightenChannel", "toInt", "ErrOverflow"),
    ("__color_darkenChannel", "toInt", "ErrOverflow"),
    ("__color_mixChannel", "toInt", "ErrOverflow"),
    // `c` is a `color::Color` from a PolyLine's `colors`: toInt of a Byte channel.
    ("__canvas_hashItem", "toInt", "ErrInvalidFormat"),
    ("__canvas_hashItem", "toInt", "ErrOverflow"),
    // Every `rgba` channel from `__canvas_pngPixel` is in 0 .. 255: an 8-bit
    // sample, the high byte of a 16-bit one, a sub-byte sample `MOD 2^depth`
    // scaled by `255 / (2^depth - 1)`, or a palette/tRNS byte — the depth is one
    // `__canvas_pngHeader` accepts (1, 2, 4, 8, 16), and the sole caller,
    // `__canvas_pngDecode`, passes that checked header.
    ("__canvas_pngPass", "toByte", "ErrOverflow"),
    // `x`, `a`, `b`, `c` are bytes and Paeth picks one of `a`/`b`/`c`, so `value`
    // is in 0 .. 510 and `value MOD 256` in 0 .. 255.
    ("__canvas_pngUnfilter", "toByte", "ErrOverflow"),
    // The sole caller, `__canvas_extractFace`, passes `cursor` (12 plus padded
    // table sizes) and `size` (`__canvas_beU32`, two unsigned 16-bit halves):
    // both are non-negative, so each `(v / k) MOD 256` is in 0 .. 255.
    ("__canvas_beBytes32", "toByte", "ErrOverflow"),
    // `token` is 1 .. 18 ASCII digits with no leading zero (checked above it):
    // a well-formed Integer below 10^18.
    ("__json_arrayIndex", "toInt", "ErrInvalidFormat"),
    ("__json_arrayIndex", "toInt", "ErrOverflow"),
    // The exponent text is the digits after `json::sciParts`'s `e` for a finite,
    // non-zero magnitude (`__json_requireFiniteNumberText` runs first): an
    // optional sign and at most three digits.
    ("__json_stringifyNumber", "toInt", "ErrInvalidFormat"),
    ("__json_stringifyNumber", "toInt", "ErrOverflow"),
    // `n > 0` (the `n <= 0` early return), so `math::sqrt(toFloat(n))` is below
    // 3.1e9 and fits an Integer.
    ("__vector_isqrtFloor", "toInt", "ErrOverflow"),
    // `value` is checked `>= 0` first and each branch bounds it, so every toByte
    // argument is in 0 .. 244 (a lead byte) or 128 .. 191 (a continuation).
    ("__encoding_fromCodepoint", "toByte", "ErrOverflow"),
    // The `List OF Integer` overload checks each `unit` is in 0 .. 255 before
    // its toByte, and both overloads call toString only after
    // `__encoding_utf8Valid` accepted the bytes.
    ("__encoding_utf8Decode", "toByte", "ErrOverflow"),
    ("__encoding_utf8Decode", "toString", "ErrEncoding"),
    // Every caller passes ASCII: `__encoding_formUrlEncode`/`percentEncode` an
    // `__encoding_isAlphaNum`/`isUnreserved` byte, `__encoding_punyEncodeLabel` a
    // code point `< 128` or `__encoding_punyDigit` of a digit in 0 .. 35 (97 ..
    // 122 or 48 .. 57). One ASCII byte fits a Byte and is valid UTF-8.
    ("__encoding_byteChar", "toByte", "ErrOverflow"),
    ("__encoding_byteChar", "toString", "ErrEncoding"),
    // `c` is a byte, `hi`/`lo` are `__encoding_hexValue` results checked `>= 0`
    // (0 .. 15), and the bytes are decoded only after `__encoding_utf8Valid`.
    ("__encoding_percentDecodeBytes", "toByte", "ErrOverflow"),
    ("__encoding_percentDecodeBytes", "toString", "ErrEncoding"),
    // `bits::band(_, 255)`.
    ("__encoding_baseDecodeBits", "toByte", "ErrOverflow"),
    // `hi`/`lo` are `__encoding_hexValue` results checked `>= 0` (0 .. 15).
    ("__encoding_hexDecode", "toByte", "ErrOverflow"),
    // `chunk = bits::band(_, 127)`, so `chunk` and `chunk + 128` fit a Byte.
    ("__encoding_leb128Emit", "toByte", "ErrOverflow"),
    ("__encoding_sleb128Encode", "toByte", "ErrOverflow"),
    // `point` is a code point below 128 (checked), and `strings::find` in a
    // 128-scalar codepage table returns a scalar index 0 .. 127.
    ("__encoding_codepageEncode", "toByte", "ErrOverflow"),
    // bug-814. Each value is `bits::band(_, 255)` bound to a name first, a
    // decoded literal symbol `0 <= sym < 256`, `endPos MOD 256` of a
    // non-negative position, `bits::sr(c, 8)` of `0 <= c <= 65535`, a zlib header
    // byte of a value below 31000, or the gzip XFL 0/2/4.
    ("__compress_inflateCore", "toByte", "ErrOverflow"),
    ("__compress_deflateCore", "toByte", "ErrOverflow"),
    ("__compress_zlibEncode", "toByte", "ErrOverflow"),
    ("__compress_gzipEncode", "toByte", "ErrOverflow"),
    ("__crypto_appendBeWord", "toByte", "ErrOverflow"),
    ("__crypto_appendBeWord64", "toByte", "ErrOverflow"),
    ("__crypto_pad512", "toByte", "ErrOverflow"),
    // `bits::bxor`/`bits::bor` of two bytes, or of a byte and 128 / the pad 54/92,
    // stays a byte; `bits::sl(parity, 7)` of a parity bit is 0 or 128; `clampScalar`'s
    // `(b AND 127) OR 64` is at most 127; the sponge suffix is 6 or 31 at every call.
    ("__crypto_xorPad", "toByte", "ErrOverflow"),
    ("__crypto_xorBytes", "toByte", "ErrOverflow"),
    ("__crypto_keccakSponge", "toByte", "ErrOverflow"),
    ("__crypto_clampScalar", "toByte", "ErrOverflow"),
    ("__crypto_clampScalar448", "toByte", "ErrOverflow"),
    ("__crypto_ed448Prune", "toByte", "ErrOverflow"),
    ("__crypto_ed448Encode", "toByte", "ErrOverflow"),
    ("__crypto_packPoint", "toByte", "ErrOverflow"),
    // `counter <= ceil(length / L) <= 255`: `crypto::hkdf` rejects `length > 255
    // * L`, and HPKE's LabeledExpand asks at most 64 bytes of a 32/64-byte HMAC.
    ("__crypto_hkdfExpand", "toByte", "ErrOverflow"),
    // `uuid4`'s version/variant bytes are `(b AND 15) OR 64 <= 79` and `(b AND 63)
    // OR 128 <= 191`; `randomIntBig`'s top byte is a byte `MOD 2^topBits`, topBits
    // in 1 .. 8.
    ("__crypto_uuid4", "toByte", "ErrOverflow"),
    ("__crypto_randomIntBig", "toByte", "ErrOverflow"),
];

/// A code a qualified `pkg::member(` call can raise that no call of that member
/// in that one `FUNC` (a private helper or a rewrite target) can actually raise,
/// keyed `(FUNC, "pkg::member", code)` and applied where the call is, so every
/// member that reaches the `FUNC` inherits the proof — the qualified-call
/// counterpart of [`UNREACHABLE_GENERAL_CALL`] (bug-814), with the same bar: the
/// proof covers every call of that member in the `FUNC`, from its arguments'
/// ranges, and a proof that depends on the callers names them. One entry per
/// line (`rustfmt::skip`), so a FUNC's entries read as one block under its proof.
#[rustfmt::skip]
const UNREACHABLE_QUALIFIED_CALL: &[(&str, &str, &str)] = &[
    // --- compress: the inflate core (RFC 1951 decoder) ---
    // Every `data` read is `IF pos < n` guarded (`pos` starts at the caller's
    // `start`: 0, 2 after a zlib header checked `n >= 2`, or a gzip header end
    // checked `<= n`) or a stored-block copy after `pos + remaining > n` FAILs.
    // `litT`/`distT`/`clT` come from `__compress_buildTable` with roots 9/6/7, so
    // they hold at least 512/64/128 entries for the `bits::band(bitBuf, 511/63/127)`
    // lookups; a sub-table index is `off + (bits below 2^subBits)`, inside the
    // sub-table buildTable appended at `off`. `li` is checked `< 29` and `dsym <
    // 30` before indexing the 29-entry length and 30-entry distance tables;
    // `ci < hclen <= 19` indexes the 19-entry CL_ORDER; `lens` holds exactly
    // `hlit + hdist >= 258` entries when `lens[256]` is read and `have >= 1` when
    // `lens[have - 1]` is; `clens` is 19 long and CL_ORDER's values are 0 .. 18.
    // A copy reads `out[from + k]` with `1 <= dist <= outLen` checked, so `from`
    // is in `0 .. outLen - 1` and `from + k` below the list's growing length.
    // Shifts: a refill shifts a byte by `bitCount <= 48`; every drop is a code
    // length `entry MOD 16` of an entry checked `>= 0` (0 .. 15), an extra-bit
    // count from the length/distance tables (0 .. 5 / 0 .. 13), `drop = bitCount
    // MOD 8` of a non-negative count, or a literal; `bits::sl(1, sub)` takes a
    // `(entry - 1048576) MOD 16` of an entry checked `>= 1048576`.
    ("__compress_inflateCore", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_inflateCore", "collections::set", "ErrIndexOutOfRange"),
    ("__compress_inflateCore", "bits::sl", "ErrInvalidArgument"),
    ("__compress_inflateCore", "bits::sr", "ErrInvalidArgument"),
    // `bl`/`nextCode` are 16 long and indexed by a code length 0 .. 15 (every
    // caller's lengths are a fixed 5/7/8/9, or a decoded 3-bit or `cl < 16`
    // length) or by `bitLen`/`maxLen` in 0 .. 15. `lens[start + i]` with `i <
    // count` stays inside every caller's list: (0, 288) of 288, (0, 32) of 32,
    // (0, 19) of 19, (0, hlit) and (hlit, hdist) of `hlit + hdist`. The table
    // starts `2^root` long (root 9/6/7 from every caller); a direct entry is
    // written at `rev + k * 2^l < 2^root`, and a long code's sub-table entry at
    // `off + sj`, `sj < 2^subBits`, inside the sub-table appended at `off` — a
    // prefix-free code (over-subscription FAILs; an incomplete code is only
    // accepted with `maxLen = 1`, which has no long codes) never finds a direct
    // entry at a long code's root prefix. Shifts are by `root`, `l`, `b < l`,
    // `l - root` and `maxLen - root`, all in 0 .. 15 where evaluated.
    ("__compress_buildTable", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_buildTable", "collections::set", "ErrIndexOutOfRange"),
    ("__compress_buildTable", "bits::sl", "ErrInvalidArgument"),
    // The inflate core always appends its 8-byte end position, so `withEnd`
    // holds at least 8 bytes: `last - k` is in range and `len - 8 >= 0`.
    ("__compress_endPosition", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_stripEnd", "collections::mid", "ErrIndexOutOfRange"),
    ("__compress_inflate", "collections::mid", "ErrIndexOutOfRange"),
    // `data[0]`/`data[1]` after `n < 2` FAILs; `data[endPos + k]`, k < 4, after
    // `endPos + 4 > n` FAILs.
    ("__compress_zlibDecode", "collections::get", "ErrIndexOutOfRange"),
    // Every header read follows a `pos + 10 > n` / `p + 2 > n` / `p >= n` FAIL;
    // the header CRC covers `pos .. p` with `p <= n`; `__compress_le32` reads
    // `endPos .. endPos + 7` after `endPos + 8 > n` FAILs; `member[k]`, k < its
    // length.
    ("__compress_gzipDecode", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_gzipDecode", "collections::mid", "ErrIndexOutOfRange"),
    ("__compress_le32", "collections::get", "ErrIndexOutOfRange"),
    // `i + 7 <= n - 1` in the eight-byte loop and `i < n` in the tail; the
    // 2048-entry slicing table is read at `k * 256 + (a byte)`, k in 0 .. 7: a
    // data byte, `bits::band(_, 255)`, or `bits::sr(x, 24)` of `x`, a 32-bit
    // value (the running CRC is checked or derived in 0 .. 2^32 - 1, and every
    // table entry is a 32-bit CRC). Adler-32 reads `data[i]`, `i < n`.
    ("__compress_crc32", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_adler32", "collections::get", "ErrIndexOutOfRange"),
    // --- compress: the deflate encoder ---
    // Every caller rejects a level outside 0 .. 9 first, so the 10-entry
    // configuration lists are in range. `head` has 32768 entries indexed by a
    // 15-bit hash; `prev` has `min(n, 32768)` indexed by `bits::band(p, 32767)` of
    // a position `p < n`. A match compares `data[cand + k]`/`data[pos + k]` only
    // while `k < maxLen <= n - pos` (`AND` short-circuits) with `cand < pos`, and
    // `data[pos + bestLen]` while `bestLen < maxLen` (a match reaching `maxLen`
    // exits). `pos - 1` is read only after `pos` advanced. LEN_SYM (259 entries)
    // is indexed by a match length 3 .. 258, DIST_CODE (512) by `d1 < 256` or `256
    // + bits::sr(d1, 7)` with `d1 = dist - 1 <= 32767`; LEN/DIST tables by a
    // `li <= 28` / `dsym <= 29` read off them; `litFreq` (286) at a literal, 256 or
    // `257 + li`, `distFreq` (30) at `dsym`; the code lists (288/286 and 30) at
    // the same symbols. The bit writer flushes to `bitCount < 8` after every
    // symbol, so a shift count is at most 10 + 15 + 5 + 15 = 45.
    ("__compress_deflateCore", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_deflateCore", "collections::set", "ErrIndexOutOfRange"),
    ("__compress_deflateCore", "bits::sl", "ErrInvalidArgument"),
    // `freqs`/`lens` are equal-length (`__compress_codeLengths` returns one length
    // per frequency); `litFreq` (286) is shorter than FIXED_LIT (288); `i < len`.
    ("__compress_treeBits", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_fixedLitBits", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_headerBits", "collections::get", "ErrIndexOutOfRange"),
    // `lens[0]` and `lens[n + 1]` with `n + 1 < codes`, `codes` (hlit <= 286 /
    // hdist <= 30) at most the list's length.
    ("__compress_treeRle", "collections::get", "ErrIndexOutOfRange"),
    // `litLens[hlit - 1]`/`distLens[hdist - 1]` with hlit in 258 .. 286 and hdist
    // in 2 .. 30; `clFreq`/`clLens`/`clCodes` are 19 long and indexed by a
    // code-length symbol `entry MOD 32` in 0 .. 18 (zlib's send_tree invariant:
    // a repeat count is decremented only when it is >= 4, so every `count - 3`
    // and `count - 11` is >= 0), or by a CL_ORDER value 0 .. 18.
    ("__compress_dynamicHeader", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_dynamicHeader", "collections::set", "ErrIndexOutOfRange"),
    // Package-merge over `count` (19, 30 or 286) symbols: `f[node]`, node 0 .. 2;
    // the insertion sort reads `keys[j - 1]` only for `j > 0` (`AND`
    // short-circuits); pool indices are pool entries created earlier; `cur` holds
    // at least `2n - 2` items after `limit - 1` merges because `n <= 2^limit` (286 <
    // 2^15, 30 < 2^15, 19 < 2^7); the stack is read and removed at `top`, its
    // last index; `lens[sym]` with `sym < count`.
    ("__compress_codeLengths", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_codeLengths", "collections::set", "ErrIndexOutOfRange"),
    ("__compress_codeLengths", "collections::removeAt", "ErrIndexOutOfRange"),
    // Code lengths are 1 .. 15 (package-merge with limit 15 or 7), indexing the
    // 16-entry `blCount`/`nextCode`; `b - 1` is 0 .. 14.
    ("__compress_canonicalCodes", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_canonicalCodes", "collections::set", "ErrIndexOutOfRange"),
    // --- crypto: shared byte-list helpers ---
    // Each reads index `i < len` of its own argument (`concat`'s `b`,
    // `concatInt`'s `b`, `isAllZero`'s `b`, `xorPad`'s `data`), `0 .. len(data)`
    // (`copyBytes`), or the shorter length (`constantTimeEqual`).
    ("__crypto_concat", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_concatInt", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_isAllZero", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_xorPad", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_constantTimeEqual", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_copyBytes", "collections::mid", "ErrIndexOutOfRange"),
    // `xorBytes(a, b)` reads `b[i]`, `i < len(a)`: its callers pass equal-length
    // lists (`__crypto_gcmTag`: two 16-byte blocks; `__crypto_pbkdf2Block`: two
    // outputs of the same HMAC).
    ("__crypto_xorBytes", "collections::get", "ErrIndexOutOfRange"),
    // `slice(data, start, stop)` / `truncate(data, n)`: every caller passes `0 <=
    // start <= stop <= len(data)` / `0 <= n <= len(data)` — fixed digests and
    // seeds (a SHA-512 64, a SHAKE256-114 or ChaCha20 64-byte block cut at 32/57,
    // an HPKE key schedule's 44 bytes cut at 32/44, a length-checked 64/114-byte
    // signature), `__crypto_decrypt`'s box after `total < nenc + 16` FAILs, the
    // sponge's `off <= n`, and output loops that run until `len(okm) >= length`.
    ("__crypto_slice", "collections::mid", "ErrIndexOutOfRange"),
    ("__crypto_truncate", "collections::mid", "ErrIndexOutOfRange"),
    // --- crypto: SHA-1 / SHA-2 ---
    // `pad512`/`pad1024` leave a multiple of 64/128 bytes, and each block loop
    // runs `base < total`, so a schedule's `beWord`/`beWord64` reads `base .. base
    // + 63/127`; the other `beWord64` callers read a 16-byte GCM subkey block at
    // 0/8, GHASH data (a multiple of 16) at `o`/`o + 8`, `beWords64`'s
    // multiple-of-8 hex constants and the 192-byte Keccak RC table at `t * 8`, t
    // < 24. The schedules index their own 80/64-entry `w` at `t - 2 .. t - 16`
    // for `t >= 16`; the cores read `w`/K256 (64) or K512 (80) at `t` below the
    // round count and their 8-word IV at 0 .. 7.
    ("__crypto_beWord", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_beWord64", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha1Schedule", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha256Schedule", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha512Schedule", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha1_bytes", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha2_32", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha2_64", "collections::get", "ErrIndexOutOfRange"),
    // Shift counts `56 - i * 8` / `i * 8` for `i < 8`, and `__crypto_shr32`'s
    // count from its only callers, `ssig0` (3) and `ssig1` (10).
    ("__crypto_pad512", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_appendBeWord64", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_appendLeLane", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_shr32", "bits::sr", "ErrInvalidArgument"),
    // --- crypto: Keccak-f[1600] ---
    // The state is always 25 lanes (`__crypto_keccakZero`, then `set`s and
    // rounds that return 25); `lane`/`fl`/`sq < rateLanes <= 18`. An absorb
    // reads `off + rate <= n`; the padded tail is exactly `rate` bytes (the tail
    // is `n - off < rate` bytes plus the suffix byte), so `last = rate - 1` and
    // `fl * 8 + 7` are in it. A round reads `c` (5) at `MOD 5`, `a`/`t`/`b`/`out`
    // (25) at `x + 5y < 25`, and the 25-entry RHO/PI tables, whose PI values are
    // 0 .. 24; `keccakF` reads the 24 round constants at `round < 24`.
    // `leLane(data, o)` reads `o .. o + 7`: the sponge's checked offsets above,
    // BLAKE2b's 128-byte blocks at `base + i * 8`, i < 16, and Argon2's 1024-byte
    // H' output at `k * 8`, k < 128; its shift `i * 8` is at most 56.
    ("__crypto_keccakSponge", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_keccakSponge", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_keccakRound", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_keccakRound", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_keccakF", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_leLane", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_leLane", "bits::sl", "ErrInvalidArgument"),
    // --- crypto: Curve25519 / Ed25519 (16 limbs) ---
    // Every field element is 16 limbs (`gf0`/`gf1`/`unpack25519` build 16, and
    // `edA`/`edZ`/`edM`/`sel25519`/`car25519` map 16 to 16), so the limb loops'
    // `i, j, k < 16` and `car25519`'s `i + 1` (for `i < 15`) are in range. A point
    // is four elements (64 limbs; `gfAt(p, 0 .. 3)`), a ladder pair two
    // (128; `first64`/`last64`/`cswap128` read `i` and `64 + i`, i < 64).
    // `unpack25519`/`clampScalar` read 32 bytes; every caller passes 32: a key
    // length-checked by `__crypto_exchange`/`__crypto_hpkeRecipientPub`/
    // `__crypto_ed25519Verify`, `crypto::randomBytes(32)`, a 32-byte truncated
    // SHA-512, a `pack25519` output (32), the 32-byte X25519 base, or HPKE's
    // `enc` (`nenc` = 32). The ladders index the 32-byte scalar at `bits::sr(i, 3)`
    // / `i / 8` for `i <= 255`, and shift by `bits::band(i, 7)`.
    ("__crypto_sel25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_edA", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_edZ", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_edM", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_par25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gfAt", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_first64", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_last64", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_car25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_car25519", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_pack25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_pack25519", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_unpack25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_unpack25519", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_clampScalar", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_clampScalar", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_packPoint", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_packPoint", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_cswap128", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_cswap128", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_gf1", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_gf121665", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_x25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_x25519", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_scalarmult", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_scalarmult", "bits::sr", "ErrInvalidArgument"),
    // --- crypto: Curve448 / Ed448 (16 limbs of 28 bits) ---
    // As for Curve25519: every `gf448` value is 16 limbs, a product's `c` is 31
    // (`n + 16 <= 30` for `n <= 14`, `n + 24 <= 30` for `n <= 6`), a point 48 limbs
    // (`ed448PointAt(p, 0 .. 2)`) and a ladder pair 96 (`ed448Cswap` reads `i`,
    // `48 + i`; the ladder cuts it at 0/48). `gf448Unpack`/`clampScalar448` read 56
    // bytes: a length-checked 56-byte key, `crypto::randomBytes(56)`, a 56-byte
    // SHAKE256 output, a `gf448Pack` output (56), the 56-byte base, a 57-byte Ed448
    // key cut to 56, or the two 56-byte hex constants of the Ed448 base point.
    // `ed448Prune` reads 0/55/56 of a 57-byte truncated SHAKE256 output, and
    // `ed448Encode` byte 0 of a 56-byte pack. The ladders index the 56/57-byte
    // scalar at `bits::sr(i, 3)` for `i <= 447`; shifts are `i * 8`/`k * 8` for `i,
    // k < 7` and `bits::band(i, 7)`.
    ("__crypto_gf448Add", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Sub", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Mul", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448MulSmall", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Select", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_ed448Encode", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Carry", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Carry", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_clampScalar448", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_clampScalar448", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_ed448Prune", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_ed448Prune", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_ed448Cswap", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_ed448Cswap", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_gf448Unpack", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Unpack", "bits::sl", "ErrInvalidArgument"),
    ("__crypto_gf448Pack", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Pack", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_x448", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_x448", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_ed448Scalarmult", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_ed448Scalarmult", "collections::mid", "ErrIndexOutOfRange"),
    ("__crypto_ed448Scalarmult", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_ed448PointAt", "collections::mid", "ErrIndexOutOfRange"),
    ("__crypto_ed448PubToX448", "collections::mid", "ErrIndexOutOfRange"),
    // --- crypto: BLAKE2b and Argon2id ---
    // BLAKE2b's chain `h` is 8 words and the IV 8; the working vector `v` 16 and
    // the message `m` 16, read at SIGMA values 0 .. 15; SIGMA holds 12 rounds of 16
    // (`s + 15 <= 191`); `blake2bG`'s four indices are literals 0 .. 15 at every
    // call; the digest loop reads `h[i / 8]`, `i < outLen <= 64` (H0 asks 64, H'
    // asks `outLen <= 64` or 64, or its last `outLen - 32r` in 33 .. 64), shifting
    // by `(i MOD 8) * 8 <= 56`. `gather3` reads each of its three lists below that
    // list's length, from `i < stop <= n`.
    ("__crypto_blake2b3", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_blake2b3", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_blake2b3", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_blake2bCompress", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_blake2bCompress", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_blake2bG", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_blake2bG", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_gather3", "collections::get", "ErrIndexOutOfRange"),
    // Argon2id: `mem` holds `blocks + 4` blocks of 128 words (`blocks >= 8 *
    // parallelism` after the range checks), and every offset is a block number
    // times 128 plus `k < 128`: a lane's first two blocks, the current block
    // `curr` and its predecessor `prev` (both inside the current lane), the
    // reference block (`refLane = (rnd >> 32) MOD parallelism`, `refIdx` reduced
    // `MOD laneLen`), the last column, or the four scratch blocks past the matrix
    // (`inputOff + 6`, `addrOff + idx MOD 128`). The fill reads three such 128-word
    // blocks, its `r`/`tmp` are 128 words (`2g + 16k + 1 <= 127`), and the
    // permutation takes and returns 16 words. `argon2Le32` shifts by `i * 8`, i < 4.
    ("__crypto_argon2id", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_argon2id", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_argon2Fill", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_argon2Fill", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_argon2Fill", "collections::mid", "ErrIndexOutOfRange"),
    ("__crypto_argon2P", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_argon2P", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_argon2P", "collections::mid", "ErrIndexOutOfRange"),
    ("__crypto_argon2Le32", "bits::sr", "ErrInvalidArgument"),
    // --- crypto: random draws ---
    // `crypto::randomBytes(8)`/`(16)` is a literal count inside 0 .. 16 MiB, and
    // the result is exactly that many bytes: `rand62`/`rand63` read 0 .. 7,
    // `uuid4` reads and writes 6 and 8 and cuts its 32-character hex string at
    // 0 .. 32. `randomIntBig` draws `byteCount = (width + 7) / 8 >= 1` bytes
    // (`max > min`, so the span is at least 2) and reads/writes `byteCount - 1`.
    ("__crypto_rand62", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_rand62", "crypto::randomBytes", "ErrInvalidArgument"),
    ("__crypto_rand63", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_rand63", "crypto::randomBytes", "ErrInvalidArgument"),
    ("__crypto_uuid4", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_uuid4", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_uuid4", "strings::mid", "ErrIndexOutOfRange"),
    ("__crypto_uuid4", "crypto::randomBytes", "ErrInvalidArgument"),
    ("__crypto_randomIntBig", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_randomIntBig", "collections::set", "ErrIndexOutOfRange"),
];

/// The mirror image of [`UNREACHABLE_VIA_CALLEE`]: a code this sweep's model
/// CANNOT see as reachable, but that really is — the `Body::Mfb` counterpart
/// of `declared_errors_all.rs`'s `IO_DECLARED_EXTRA`/`TERM_DECLARED_EXTRA`
/// ("a code this sweep's harness configuration cannot make reachable, but
/// that a real build does raise"). `encoding::htmlUnescape`/`punycodeDecode`'s
/// ErrEncoding used to be carried here; since bug-807 this guard sees the bare
/// `toString(<List OF Byte>)` call in `__encoding_fromCodepoint` that raises it.
/// `http::finish`'s eight transport codes used to be carried here too, because
/// it re-raised the code VALUE in `s.state.err`; since bug-808 it raises each
/// one from a literal `FAIL error(<code>, …)`, which this guard sees.
const DECLARED_EXTRA: &[(&str, &str, &str)] = &[
    // Pre-existing, correct `ErrOverflow`/`ErrOutOfMemory`/etc. declarations
    // on members whose only raise site is an IMPLICIT checked-arithmetic
    // operator (`+`/`-`/`*` on Integer, a MOD/divide) in their own Mfb
    // source, never a literal `FAIL error(...)` -- this guard's own top-of-
    // file doc comment scopes it to explicit FAILs and callee propagation,
    // so an implicit operator raise is a documented blind spot, not a wrong
    // declaration. Found by the whole-registry sweep (none of these were in
    // Phase 1's original 136-member list; they predate bug-698 and were
    // simply never measured until this guard's first whole-registry run).
    ("audio", "render", "ErrInvalidFormat"),
    ("audio", "render", "ErrOverflow"),
    ("canvas", "getSize", "ErrOutOfMemory"),
    ("canvas", "getSize", "ErrResourceClosed"),
    ("datetime", "instant", "ErrOverflow"),
    ("datetime", "duration", "ErrOverflow"),
    ("datetime", "inZone", "ErrOverflow"),
    ("datetime", "resolve", "ErrOverflow"),
    ("datetime", "add", "ErrOverflow"),
    ("datetime", "subtract", "ErrOverflow"),
    ("datetime", "between", "ErrOverflow"),
    ("datetime", "negate", "ErrOverflow"),
    ("datetime", "plus", "ErrOverflow"),
    ("datetime", "minus", "ErrOverflow"),
    ("datetime", "weekday", "ErrOverflow"),
    ("datetime", "dayOfYear", "ErrOverflow"),
    ("datetime", "toMillis", "ErrOverflow"),
    ("datetime", "toNanos", "ErrOverflow"),
    ("datetime", "format", "ErrOverflow"),
    ("datetime", "formatDuration", "ErrOverflow"),
];

/// The whole-registry lookup tables both sweeps resolve calls against.
struct SweepTables {
    /// Every member's declared `errors`, unioned across its implementations.
    member_declared: HashMap<(String, String), BTreeSet<&'static str>>,
    /// A `Body::Mfb` rewrite symbol -> the `(package, member)` that owns it.
    rewrite_to_owner: HashMap<String, (String, String)>,
    /// Every `Body::Mfb` member's body strings.
    mfb_bodies: HashMap<(String, String), Vec<&'static str>>,
    /// Every `FUNC` block in a `RegistryHelper` or `Body::Mfb` string, by name.
    helper_blocks: HashMap<String, &'static str>,
}

fn sweep_tables() -> SweepTables {
    let reg = registry();
    let mut member_declared: HashMap<(String, String), BTreeSet<&'static str>> = HashMap::new();
    let mut rewrite_to_owner: HashMap<String, (String, String)> = HashMap::new();
    let mut mfb_bodies: HashMap<(String, String), Vec<&'static str>> = HashMap::new();
    let mut helper_blocks: HashMap<String, &'static str> = HashMap::new();

    for package in reg.packages() {
        let pkg_name = package.import_name();
        for helper in package.helpers() {
            if let Some(text) = helper.body {
                for (name, block) in extract_func_blocks(text) {
                    helper_blocks.insert(name.to_string(), block);
                }
            }
        }
        for function in package.functions() {
            let mut errs: BTreeSet<&'static str> = BTreeSet::new();
            for implementation in function.implementations() {
                errs.extend(implementation.errors.iter().copied());
                if let Body::Mfb { body, rewrite, .. } = &implementation.body {
                    rewrite_to_owner.insert(
                        rewrite.to_string(),
                        (pkg_name.to_string(), function.name.to_string()),
                    );
                    mfb_bodies
                        .entry((pkg_name.to_string(), function.name.to_string()))
                        .or_default()
                        .push(body);
                    // A Body::Mfb chunk can declare more than one `FUNC` (the
                    // public rewrite target plus package-private helpers in the
                    // SAME string, e.g. `canvas::getFrameStats`'s `__canvas_rate`,
                    // `color::brighten`'s `__color_brightenChannel`) — register
                    // every block so a call from the main body to one of them
                    // resolves. The main block itself (keyed by `rewrite`) is
                    // already handled via `rewrite_to_owner`, taking priority
                    // over `helper_blocks` in [`resolve`], so re-adding it here
                    // too is harmless.
                    for (name, block) in extract_func_blocks(body) {
                        helper_blocks.insert(name.to_string(), block);
                    }
                }
            }
            member_declared.insert((pkg_name.to_string(), function.name.to_string()), errs);
        }
    }
    SweepTables {
        member_declared,
        rewrite_to_owner,
        mfb_bodies,
        helper_blocks,
    }
}

/// Sweep every `Body::Mfb` member across the whole registry (unlike
/// `declared_errors_sweep.rs`'s per-package entry points, one `Mfb` body can
/// call a sibling member in ANOTHER package — `collections::sort` calling
/// `collections::get`/`set` is same-package, but nothing in the source shape
/// guarantees that in general — so this guard is whole-registry, not
/// parameterized per package).
pub(crate) fn sweep() -> MfbSweepResult {
    let codes = codes_by_value();
    let reg = registry();
    let SweepTables {
        member_declared,
        rewrite_to_owner,
        mfb_bodies,
        helper_blocks,
    } = sweep_tables();

    let mut failures = Vec::new();
    let mut mismatched = Vec::new();
    let mut measured_members = 0usize;
    let mut visiting = BTreeSet::new();
    let mut memo = HashMap::new();

    for package in reg.packages() {
        let pkg_name = package.import_name();
        for function in package.functions() {
            let key = (pkg_name.to_string(), function.name.to_string());
            let Some(bodies) = mfb_bodies.get(&key) else {
                continue;
            };
            measured_members += 1;
            let mut expected = BTreeSet::new();
            for body in bodies {
                // One `FUNC` at a time: a `Body::Mfb` string can hold the rewrite
                // target plus private helper FUNCs, and each has its own
                // parameter names (`Scope`) and its own function-level `TRAP`.
                for (_, block) in extract_func_blocks(body) {
                    expected.extend(resolve(
                        block,
                        &codes,
                        &rewrite_to_owner,
                        &member_declared,
                        &helper_blocks,
                        &mut visiting,
                        &mut memo,
                        &mut failures,
                    ));
                }
            }
            expected.retain(|code| {
                !UNREACHABLE_VIA_CALLEE
                    .iter()
                    .any(|(pkg, member, unreachable)| {
                        *pkg == pkg_name && *member == function.name && unreachable == code
                    })
            });
            for (pkg, member, extra) in DECLARED_EXTRA {
                if *pkg == pkg_name && *member == function.name {
                    expected.insert(extra);
                }
            }
            let declared = member_declared.get(&key).cloned().unwrap_or_default();
            if declared != expected {
                let missing: Vec<_> = expected.difference(&declared).collect();
                let extra: Vec<_> = declared.difference(&expected).collect();
                mismatched.push(format!(
                    "{pkg_name}::{}: undeclared {missing:?}, declared but never raised {extra:?}",
                    function.name
                ));
            }
        }
    }

    MfbSweepResult {
        failures,
        mismatched,
        measured_members,
    }
}

#[test]
fn every_body_mfb_member_declares_exactly_the_errors_its_source_can_raise() {
    let result = sweep();
    assert!(
        result.failures.is_empty(),
        "the Body::Mfb sweep could not classify every call:\n  {}",
        result.failures.join("\n  ")
    );
    assert!(
        result.measured_members >= 1,
        "the sweep found no Body::Mfb members; it should have found >= 211 when this was written"
    );
    assert!(
        result.mismatched.is_empty(),
        "{} Body::Mfb member(s) declare errors that differ from what their source can raise \
         (fix the descriptor `errors` in src/codegen/builtins/<pkg>/func_*.rs):\n  {}",
        result.mismatched.len(),
        result.mismatched.join("\n  ")
    );
}

/// A `Body::Rewrite` implementation's call becomes a call to a fixed `__`-symbol
/// whose MFBASIC body is a package `RegistryHelper` `FUNC` block (bug-814). The
/// block is resolved exactly as a `Body::Mfb` body is ([`resolve`]: its own
/// `FAIL`s, its calls, `TRAP` modeled), and compared with that one
/// implementation's own `errors` — not the member's union, because a member can
/// mix a `Rewrite` overload with a native one (`crypto::hash`'s `String` form
/// next to its `List OF Byte` `AbiFunction`), and `declared_errors_all.rs`
/// answers for the native overloads.
///
/// A call back into the member itself (`__crypto_hashText` re-entering
/// `crypto::hash` over the encoded bytes) counts the member's OTHER
/// implementations only: counting the implementation under test would let any
/// code it declares justify itself.
pub(crate) struct RewriteSweepResult {
    pub(crate) failures: Vec<String>,
    pub(crate) mismatched: Vec<String>,
    pub(crate) measured_implementations: usize,
}

/// A code a `Body::Rewrite` member's resolved body reaches that a human reading
/// the source has proven can never fire, keyed `(package, member, code)` and
/// applied to every `Rewrite` implementation of that member — the `Rewrite`
/// counterpart of [`UNREACHABLE_VIA_CALLEE`], with the same evidentiary bar.
const REWRITE_UNREACHABLE: &[(&str, &str, &str)] = &[];

/// The mirror image of [`REWRITE_UNREACHABLE`], the `Rewrite` counterpart of
/// [`DECLARED_EXTRA`]: a code the text scan cannot see (an implicit
/// checked-arithmetic raise) but that the member really raises.
const REWRITE_DECLARED_EXTRA: &[(&str, &str, &str)] = &[];

pub(crate) fn rewrite_sweep() -> RewriteSweepResult {
    let codes = codes_by_value();
    let reg = registry();
    let tables = sweep_tables();
    let mut failures = Vec::new();
    let mut mismatched = Vec::new();
    let mut measured_implementations = 0usize;

    for package in reg.packages() {
        let pkg_name = package.import_name();
        for function in package.functions() {
            for (index, implementation) in function.implementations().iter().enumerate() {
                let Body::Rewrite(target) = implementation.body else {
                    continue;
                };
                measured_implementations += 1;
                let Some(block) = tables.helper_blocks.get(target).copied() else {
                    failures.push(format!(
                        "{pkg_name}::{}: rewrite target `{target}` is not a registered helper FUNC",
                        function.name
                    ));
                    continue;
                };
                let key = (pkg_name.to_string(), function.name.to_string());
                let others: BTreeSet<&'static str> = function
                    .implementations()
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != index)
                    .flat_map(|(_, other)| other.errors.iter().copied())
                    .collect();
                let mut member_declared = tables.member_declared.clone();
                member_declared.insert(key, others);
                let mut expected = resolve(
                    block,
                    &codes,
                    &tables.rewrite_to_owner,
                    &member_declared,
                    &tables.helper_blocks,
                    &mut BTreeSet::new(),
                    &mut HashMap::new(),
                    &mut failures,
                );
                expected.retain(|code| {
                    !REWRITE_UNREACHABLE
                        .iter()
                        .any(|(pkg, member, unreachable)| {
                            *pkg == pkg_name && *member == function.name && unreachable == code
                        })
                });
                for (pkg, member, extra) in REWRITE_DECLARED_EXTRA {
                    if *pkg == pkg_name && *member == function.name {
                        expected.insert(extra);
                    }
                }
                let declared: BTreeSet<&'static str> =
                    implementation.errors.iter().copied().collect();
                if declared != expected {
                    let missing: Vec<_> = expected.difference(&declared).collect();
                    let extra: Vec<_> = declared.difference(&expected).collect();
                    mismatched.push(format!(
                        "{pkg_name}::{} ({target}): undeclared {missing:?}, declared but never \
                         raised {extra:?}",
                        function.name
                    ));
                }
            }
        }
    }

    RewriteSweepResult {
        failures,
        mismatched,
        measured_implementations,
    }
}

#[test]
fn every_body_rewrite_implementation_declares_exactly_the_errors_its_target_can_raise() {
    let result = rewrite_sweep();
    assert!(
        result.failures.is_empty(),
        "the Body::Rewrite sweep could not classify every call:\n  {}",
        result.failures.join("\n  ")
    );
    assert!(
        result.measured_implementations >= 34,
        "the sweep found only {} Body::Rewrite implementations; there were 34 (29 members) \
         when this was written",
        result.measured_implementations
    );
    assert!(
        result.mismatched.is_empty(),
        "{} of {} Body::Rewrite implementation(s) declare errors that differ from what their \
         target can raise (fix the implementation's `errors` in \
         src/codegen/builtins/<pkg>/func_*.rs):\n  {}",
        result.mismatched.len(),
        result.measured_implementations,
        result.mismatched.join("\n  ")
    );
}
