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
//! 5. The member's expected `errors` is the union of 1 and the propagated sets
//!    from 3/4; a mismatch against the descriptor's actual `errors` is
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

use crate::codegen::registry::{registry, Body};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::declared_errors_sweep::codes_by_value;

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

/// Every `pkg::name(` call in `text` (package, member name) that is not a
/// `CASE pkg::Variant(binding)` pattern match (see [`preceded_by_case`]).
fn qualified_calls(text: &str) -> Vec<(String, String)> {
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
                out.push((pkg, name));
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
    for (pkg, name) in qualified_calls(text) {
        match member_declared.get(&(pkg.clone(), name.clone())) {
            Some(declared) => out.extend(declared.iter().copied()),
            None => failures.push(format!(
                "call `{pkg}::{name}` does not resolve to a registered member"
            )),
        }
    }
    for sym in internal_calls(text) {
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
    let stripped = strip_func_header_and_footer(&strip_comment_lines(text));
    let lines: Vec<&str> = stripped.lines().collect();
    if let Some((trap_idx, end_idx)) = find_function_level_trap(&lines) {
        let normal_part = lines[..trap_idx].join("\n");
        let handler_part = lines[trap_idx + 1..end_idx].join("\n");
        let trapped_set = resolve_inline_traps(
            &normal_part,
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
    // audio
    ("audio", "render", "ErrInvalidArgument"),
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
    ("datetime", "format", "ErrInvalidArgument"),
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
    ("encoding", "codepageDecode", "ErrOutOfMemory"),
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

/// The mirror image of [`UNREACHABLE_VIA_CALLEE`]: a code this sweep's model
/// CANNOT see as reachable, but that really is — the `Body::Mfb` counterpart
/// of `declared_errors_all.rs`'s `IO_DECLARED_EXTRA`/`TERM_DECLARED_EXTRA`
/// ("a code this sweep's harness configuration cannot make reachable, but
/// that a real build does raise").
///
/// `general::toString`'s descriptor declares `errors: vec![]` (its own DESC:
/// "toString never fails"), but its `List OF Byte` overload decodes UTF-8 and
/// genuinely raises `ErrEncoding` on an ill-formed sequence (`.ai/compiler.md`
/// §11's `toInt`/`toString` census makes the same point for the native-sweep
/// side). Every `Mfb` body's `toString(<expr>)` call reaches whichever
/// overload the argument's static type selects, which this guard's qualified-
/// call scan cannot see — so `toString`'s own (wrong) `errors: vec![]` is what
/// propagates. Fixing `toString`'s descriptor directly is out of scope here
/// (it is a `general`-package, not `Mfb`-body, member, and correcting it
/// ripples into every OTHER caller across the whole registry, native and
/// `Mfb` alike — a separate, larger bug). `encoding::htmlUnescape`/
/// `punycodeDecode` measured the real raise directly: a compiled program
/// running `encoding::htmlUnescape("&#55296;")` (an unpaired UTF-16 surrogate
/// codepoint, which `toString(List OF Byte)` rejects) traps `77020004`
/// (`ErrEncoding`); same for `punycodeDecode("xn--ib9b")`.
const DECLARED_EXTRA: &[(&str, &str, &str)] = &[
    ("encoding", "htmlUnescape", "ErrEncoding"),
    ("encoding", "punycodeDecode", "ErrEncoding"),
    // `http::finish`'s `FAIL error(s.state.err, "http stream transport
    // failed")` (func_finish.rs) re-raises a CODE VALUE stored in a field —
    // `s.state.err`, not a digit literal — so `fail_codes`' `FAIL error(`
    // scan cannot see it (by design: it is a text scan, not an evaluator).
    // That field is set only by `__http_pump` (via `__http_readNet`/
    // `__http_readTls`'s TRAP handlers, which store `e.code` for every
    // propagated code except `ErrConnectionClosed`, which they turn into the
    // `closed` flag instead), by `__http_waitReadable` (`ErrTimeout`), or by
    // `__http_pump`'s own hard-coded `77050010` (`ErrOverflow`) once the
    // response exceeds `__HTTP_MAX_RESPONSE`. The real reachable set is
    // `tcp::read`/`tls::read`'s own declared errors minus `ErrConnectionClosed`
    // (diverted to `closed`), plus `ErrTimeout` and `ErrOverflow`.
    ("http", "finish", "ErrInvalidArgument"),
    ("http", "finish", "ErrNetworkFailed"),
    ("http", "finish", "ErrOutOfMemory"),
    ("http", "finish", "ErrOverflow"),
    ("http", "finish", "ErrResourceClosed"),
    ("http", "finish", "ErrResourceMoved"),
    ("http", "finish", "ErrTimeout"),
    ("http", "finish", "ErrTlsFailed"),
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

/// Sweep every `Body::Mfb` member across the whole registry (unlike
/// `declared_errors_sweep.rs`'s per-package entry points, one `Mfb` body can
/// call a sibling member in ANOTHER package — `collections::sort` calling
/// `collections::get`/`set` is same-package, but nothing in the source shape
/// guarantees that in general — so this guard is whole-registry, not
/// parameterized per package).
pub(crate) fn sweep() -> MfbSweepResult {
    let codes = codes_by_value();
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
                expected.extend(resolve(
                    body,
                    &codes,
                    &rewrite_to_owner,
                    &member_declared,
                    &helper_blocks,
                    &mut visiting,
                    &mut memo,
                    &mut failures,
                ));
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
