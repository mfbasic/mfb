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

/// Resolve the error-code set a body's text can raise: its own `FAIL` sites,
/// plus — for every call it makes — either the callee member's declared
/// `errors` (trusted, not re-derived) or, for a package-private helper, a
/// recursive inline scan of the helper's own block.
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

/// The result of sweeping every `Body::Mfb` member in the registry.
pub(crate) struct MfbSweepResult {
    pub(crate) failures: Vec<String>,
    pub(crate) mismatched: Vec<String>,
    pub(crate) measured_members: usize,
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

/// bug-698 Phase 1's measured baseline, taken verbatim from this guard's first
/// run (2026-10-03) and recorded in
/// `bugs/bug-698-builtin-descriptors-omit-errors-their-lowering-raises.md`'s
/// Phase 1 section. Every `Body::Mfb` member currently declares `errors:
/// vec![]`, so each entry's "declared but never raised" half is empty.
///
/// **A ratchet, not a target.** Phase 2 fills `errors`/prose member by member
/// and REMOVES each fixed entry here (never edits one in place, and never
/// adds a shrunk/renarrowed copy — that is the AGENTS.md re-baseline-a-golden
/// rule applied to this list). A `Body::Mfb` member NOT in this list must
/// match exactly what its source raises — same bar `declared_errors_all.rs`
/// holds every native member to — which is what keeps `cargo test` green
/// today without hiding a NEW gap this guard would otherwise catch. Some
/// entries overstate the true raise set (the guard's own doc comment notes
/// its function-level, not per-overload, propagation — e.g.
/// `collections::sort`'s `ErrNotFound` comes only from `collections::get`'s
/// unrelated map overload); Phase 2 must re-derive each one from the source,
/// not copy this list into a descriptor.
const PHASE_1_BASELINE: &[&str] = &[
    "astrings::addAttribute: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "astrings::removeAttribute: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "astrings::clearAttributes: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "astrings::getAttributes: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "astrings::toMarkdown: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "audio::render: undeclared [\"ErrFloatNaN\", \"ErrInvalidArgument\", \"ErrOutOfMemory\"], declared but never raised []",
    "audio::play: undeclared [\"ErrFloatInf\", \"ErrFloatNaN\", \"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\", \"ErrOverflow\"], declared but never raised []",
    "canvas::present: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrOverflow\", \"ErrResourceClosed\"], declared but never raised []",
    "canvas::getFrameStats: undeclared [\"ErrOverflow\"], declared but never raised []",
    "canvas::waitFrame: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "canvas::presentLayers: undeclared [\"ErrInvalidArgument\", \"ErrOutOfMemory\", \"ErrOverflow\"], declared but never raised []",
    "canvas::loadImage: undeclared [\"ErrAccessDenied\", \"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrInvalidFormat\", \"ErrInvalidPath\", \"ErrPathNotFound\", \"ErrReadFailed\", \"ErrTooLarge\"], declared but never raised []",
    "canvas::loadFont: undeclared [\"ErrAccessDenied\", \"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrInvalidFormat\", \"ErrInvalidPath\", \"ErrReadFailed\"], declared but never raised []",
    "canvas::listSystemFonts: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidFormat\"], declared but never raised []",
    "canvas::loadSystemFont: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidFormat\"], declared but never raised []",
    "canvas::measureText: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "color::toPacked: undeclared [\"ErrInvalidArgument\"], declared but never raised []",
    "color::fromPacked: undeclared [\"ErrInvalidArgument\"], declared but never raised []",
    "color::fromHex: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "color::toHex: undeclared [\"ErrIndexOutOfRange\"], declared but never raised []",
    "color::toHexAlpha: undeclared [\"ErrIndexOutOfRange\"], declared but never raised []",
    "color::hsl: undeclared [\"ErrOverflow\"], declared but never raised []",
    "color::hsla: undeclared [\"ErrOverflow\"], declared but never raised []",
    "color::toHsl: undeclared [\"ErrInvalidArgument\", \"ErrOverflow\"], declared but never raised []",
    "color::saturate: undeclared [\"ErrInvalidArgument\", \"ErrOverflow\"], declared but never raised []",
    "color::desaturate: undeclared [\"ErrInvalidArgument\", \"ErrOverflow\"], declared but never raised []",
    "color::rotateHue: undeclared [\"ErrInvalidArgument\", \"ErrOverflow\"], declared but never raised []",
    "color::fromName: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "color::nameOf: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "csv::parse: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "csv::stringify: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\"], declared but never raised []",
    "csv::parseStream: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\"], declared but never raised []",
    "csv::readRow: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "json::get: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "json::getOr: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "json::parse: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "json::stringify: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "regex::find: undeclared [\"ErrInvalidArgument\", \"ErrOutOfMemory\"], declared but never raised []",
    "regex::findAll: undeclared [\"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "regex::match: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "regex::replace: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "regex::findMatch: undeclared [\"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "regex::findAllMatches: undeclared [\"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "regex::count: undeclared [\"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "regex::split: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "datetime::toUtc: undeclared [\"ErrInvalidArgument\", \"ErrOverflow\"], declared but never raised []",
    "datetime::toLocal: undeclared [\"ErrOverflow\"], declared but never raised []",
    "datetime::format: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "datetime::parse: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\", \"ErrOverflow\"], declared but never raised []",
    "datetime::toIso: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "datetime::parseIso: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrOverflow\"], declared but never raised []",
    "datetime::formatDuration: undeclared [\"ErrInvalidArgument\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::utf16Encode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::utf16Decode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\"], declared but never raised []",
    "encoding::utf32Encode: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::hexEncode: undeclared [\"ErrIndexOutOfRange\"], declared but never raised []",
    "encoding::hexDecode: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::base32Encode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\"], declared but never raised []",
    "encoding::base32Decode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::base64Encode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\"], declared but never raised []",
    "encoding::base64Decode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::base64UrlEncode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\"], declared but never raised []",
    "encoding::base64UrlDecode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::percentEncode: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::percentDecode: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::htmlEscape: undeclared [\"ErrIndexOutOfRange\"], declared but never raised []",
    "encoding::htmlUnescape: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::formUrlEncode: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::formUrlDecode: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::punycodeEncode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::punycodeDecode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::uleb128Encode: undeclared [\"ErrInvalidArgument\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::uleb128Decode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\"], declared but never raised []",
    "encoding::sleb128Encode: undeclared [\"ErrInvalidArgument\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::sleb128Decode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\"], declared but never raised []",
    "encoding::varintEncode: undeclared [\"ErrInvalidArgument\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::varintDecode: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\"], declared but never raised []",
    "encoding::codepageDecode: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "encoding::codepageEncode: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "collections::sort: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "collections::sortBy: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "collections::take: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "collections::drop: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "collections::any: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\"], declared but never raised []",
    "collections::all: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\"], declared but never raised []",
    "collections::groupBy: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\"], declared but never raised []",
    "collections::mapValues: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "collections::flatten: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "collections::zip: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\"], declared but never raised []",
    "collections::chunks: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "collections::window: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "collections::distinct: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "collections::merge: undeclared [\"ErrIndexOutOfRange\"], declared but never raised []",
    "collections::partition: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\"], declared but never raised []",
    "collections::toSet: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "collections::union: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "collections::intersection: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "collections::difference: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "collections::symmetricDifference: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "crypto::uuid7: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\"], declared but never raised []",
    "crypto::ulid: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\"], declared but never raised []",
    "net::toUrl: undeclared [\"ErrIndexOutOfRange\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "net::percentDecode: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "net::parseQuery: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrInvalidFormat\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "http::read: undeclared [\"ErrAddressNotFound\", \"ErrConnectionClosed\", \"ErrInvalidArgument\", \"ErrNetworkFailed\", \"ErrOutOfMemory\", \"ErrResourceClosed\", \"ErrResourceMoved\", \"ErrTimeout\", \"ErrTlsFailed\"], declared but never raised []",
    "http::write: undeclared [\"ErrAddressNotFound\", \"ErrConnectionClosed\", \"ErrInvalidArgument\", \"ErrNetworkFailed\", \"ErrOutOfMemory\", \"ErrResourceClosed\", \"ErrResourceMoved\", \"ErrTimeout\", \"ErrTlsFailed\"], declared but never raised []",
    "http::startRead: undeclared [\"ErrAddressNotFound\", \"ErrConnectionClosed\", \"ErrInvalidArgument\", \"ErrNetworkFailed\", \"ErrOutOfMemory\", \"ErrResourceClosed\", \"ErrResourceMoved\", \"ErrTimeout\", \"ErrTlsFailed\"], declared but never raised []",
    "http::ready: undeclared [\"ErrInvalidArgument\", \"ErrNetworkFailed\", \"ErrOutOfMemory\", \"ErrResourceClosed\", \"ErrResourceMoved\", \"ErrTimeout\", \"ErrTlsFailed\"], declared but never raised []",
    "http::pump: undeclared [\"ErrConnectionClosed\", \"ErrInvalidArgument\", \"ErrNetworkFailed\", \"ErrOutOfMemory\", \"ErrResourceClosed\", \"ErrResourceMoved\", \"ErrTimeout\", \"ErrTlsFailed\"], declared but never raised []",
    "http::done: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrInvalidFormat\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "http::finish: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrInvalidFormat\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "http::server: undeclared [\"ErrAddressInvalid\", \"ErrNetworkFailed\", \"ErrOutOfMemory\"], declared but never raised []",
    "http::serverSSL: undeclared [\"ErrAddressInvalid\", \"ErrNetworkFailed\", \"ErrOutOfMemory\", \"ErrTlsFailed\"], declared but never raised []",
    "http::handleRequest: undeclared [\"ErrConnectionClosed\", \"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrInvalidFormat\", \"ErrNetworkFailed\", \"ErrNotFound\", \"ErrOutOfMemory\", \"ErrOverflow\", \"ErrResourceClosed\", \"ErrResourceMoved\", \"ErrTimeout\", \"ErrTlsFailed\"], declared but never raised []",
    "http::route: undeclared [\"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrNotFound\", \"ErrOutOfMemory\"], declared but never raised []",
    "http::responseDefault: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "http::ok: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "http::status: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "http::json: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "http::withHeader: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\"], declared but never raised []",
    "http::bytes: undeclared [\"ErrOutOfMemory\"], declared but never raised []",
    "http::respondFile: undeclared [\"ErrIndexOutOfRange\", \"ErrOutOfMemory\", \"ErrReadFailed\", \"ErrResourceClosed\", \"ErrResourceMoved\"], declared but never raised []",
    "http::respondPath: undeclared [\"ErrAccessDenied\", \"ErrIndexOutOfRange\", \"ErrInvalidArgument\", \"ErrInvalidPath\", \"ErrNotFound\", \"ErrOutOfMemory\", \"ErrPathNotFound\", \"ErrReadFailed\", \"ErrWriteFailed\"], declared but never raised []",
    "vector::length: undeclared [\"ErrFloatDomain\", \"ErrInvalidArgument\"], declared but never raised []",
    "vector::normalize: undeclared [\"ErrFloatDomain\", \"ErrOverflow\"], declared but never raised []",
    "vector::distance: undeclared [\"ErrFloatDomain\", \"ErrInvalidArgument\"], declared but never raised []",
    "vector::project: undeclared [\"ErrOverflow\"], declared but never raised []",
    "vector::angle: undeclared [\"ErrFloatDomain\", \"ErrOverflow\"], declared but never raised []",
    "vector::lerp: undeclared [\"ErrInvalidArgument\", \"ErrOverflow\"], declared but never raised []",
    "vector::lerp_unclamped: undeclared [\"ErrOverflow\"], declared but never raised []",
    "vector::slerp: undeclared [\"ErrFloatDomain\", \"ErrFloatNaN\", \"ErrOverflow\"], declared but never raised []",
    "vector::clamp_length: undeclared [\"ErrFloatDomain\", \"ErrOverflow\"], declared but never raised []",
    "vector::min: undeclared [\"ErrInvalidArgument\"], declared but never raised []",
    "vector::max: undeclared [\"ErrInvalidArgument\"], declared but never raised []",
    "vector::abs: undeclared [\"ErrOverflow\"], declared but never raised []",
    "vector::rotate_2d: undeclared [\"ErrFloatNaN\", \"ErrOverflow\"], declared but never raised []",
];

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
    let unexpected: Vec<&String> = result
        .mismatched
        .iter()
        .filter(|m| !PHASE_1_BASELINE.contains(&m.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "{} Body::Mfb member(s) declare errors that differ from what their source can raise, \
         beyond bug-698 Phase 1's recorded baseline (fix the descriptor `errors` in \
         src/codegen/builtins/<pkg>/func_*.rs, or extend the baseline if this is a genuine \
         Phase 1 addition):\n  {}",
        unexpected.len(),
        unexpected
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
