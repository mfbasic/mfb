//! plan-176-H: what a call in MFBASIC source can raise — call scanning, scope and source
//! spelling, type inference, and error-code derivation. The sweeps that use it live in
//! `mfb_body_declared_errors/mod.rs`.

use super::*;

/// The `general` package's member names (`toString`, `toInt`, `len`, …).
pub(super) fn general_names() -> BTreeSet<String> {
    registry()
        .resolve_package("general")
        .expect("general package")
        .functions()
        .iter()
        .map(|function| function.name.to_string())
        .collect()
}

pub(super) fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Every `FAIL error(<digits>` code literal in `text`, in source order
/// (duplicates allowed; the caller only cares about the set).
pub(super) fn fail_codes(text: &str) -> Vec<String> {
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
pub(super) fn qualified_calls_with_arguments(text: &str) -> Vec<(String, String, Vec<String>)> {
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
pub(super) fn internal_calls(text: &str) -> Vec<String> {
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
pub(super) fn general_calls(text: &str, general: &BTreeSet<String>) -> Vec<(String, Vec<String>)> {
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
pub(super) struct Scope {
    package: String,
    pub(super) func: String,
    names: HashMap<String, Vec<String>>,
}

impl Scope {
    pub(super) fn of(text: &str) -> Scope {
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
pub(super) fn general_override_helper(
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

/// Whether `expr`'s outermost node is an arithmetic operator (`+ - * / ^`,
/// `MOD`, `DIV`) or it is a numeric literal: either way its value is a number,
/// never a collection or `String` — MFBASIC has no infix operator on `List`,
/// `Map` or `Set`, and `&` (the one `String` operator) is not counted. Scans
/// outside parentheses, brackets and string literals only.
fn is_arithmetic_expression(expr: &str) -> bool {
    let expr = expr.trim();
    if expr
        .trim_start_matches('-')
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
        && expr
            .trim_start_matches('-')
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    {
        return true;
    }
    let bytes = expr.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    for (index, &byte) in bytes.iter().enumerate() {
        match byte {
            b'"' => in_string = !in_string,
            _ if in_string => {}
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'+' | b'*' | b'/' | b'^' if depth == 0 => return true,
            // A binary minus has an operand before it; a leading one is a
            // negation, which is arithmetic too.
            b'-' if depth == 0 => return true,
            b' ' if depth == 0 => {
                let rest = &expr[index + 1..];
                if rest.starts_with("MOD ") || rest.starts_with("DIV ") {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
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
pub(super) fn qualified_call_codes(
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
    // bug-928: a member with both collection-kind and scalar first parameters
    // (`math::sqrt(List OF Float)` beside `math::sqrt(Float)`) called with an
    // argument proven to be a scalar raises only what its scalar overloads
    // declare — a scalar cannot match a `List`/`Map`/`Set`/`String` parameter, so
    // dropping the collection forms' codes (the result list's ErrOutOfMemory) is
    // sound. An argument of any other or unknown type keeps the rule below.
    let has_scalar_form = kinds.iter().any(Option::is_none);
    let has_collection_form = kinds.iter().any(Option::is_some);
    if has_scalar_form && has_collection_form {
        let scalar_arg = args.first().is_some_and(|arg| {
            is_arithmetic_expression(arg)
                || infer_type(arg, scope, helper_blocks).is_some_and(|ty| {
                    matches!(
                        ty.as_str(),
                        "Integer" | "Float" | "Fixed" | "Money" | "Byte" | "Boolean"
                    )
                })
        });
        if scalar_arg {
            return function
                .implementations()
                .iter()
                .zip(&kinds)
                .filter(|(_, k)| k.is_none())
                .flat_map(|(implementation, _)| implementation.errors.iter().copied())
                .collect();
        }
    }
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
pub(super) fn general_call_codes(
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
