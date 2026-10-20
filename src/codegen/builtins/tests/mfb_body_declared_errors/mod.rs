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

mod exception_tables;
mod source_scan;
use exception_tables::*;
use source_scan::*;

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
