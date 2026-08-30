//! plan-111's ratchet gate: **no type strings after the AST**.
//!
//! `ParameterType` (`src/types.rs`) is meant to be the compiler's only type
//! currency from `hir::elaborate` to the emitted byte. A type *spelling* —
//! `"List OF Integer"`, `"Money"`, `"RES File STATE Cursor"` — is a rendering,
//! and rendering is legitimate at exactly three sinks: a diagnostic message,
//! a mangled symbol, and a wire encode. What is never legitimate is a spelling
//! flowing *into* a decision: re-parsed, matched on, compared against, taken as
//! a `&str` parameter, taken apart with `split_once`, or used as a map key.
//!
//! This file scans the tree for the seven needle classes that spell exactly
//! that, and asserts each per-directory count is at or below a hardcoded
//! budget. **plan-111 lowers a budget in the same commit as the work that
//! cleared it**, so "mostly migrated" is a number CI enforces rather than a
//! claim in a document. Letter G sets every budget to 0 and this file becomes a
//! **hard floor of 0** — mirroring `builtins_no_hand_picked_vreg` in
//! `tests/architecture_guards.rs`, whose migration is likewise complete.
//!
//! The budget table is asserted **tight in both directions**: a count above its
//! budget fails (a regression), and a budget above the live count also fails
//! with "lower this budget to N" (a silent allowance that would let a
//! regression hide inside slack already spent).
//!
//! Like `tests/architecture_guards.rs` this lives in `tests/` — an integration
//! crate — so neither the scan roots nor the self-exemption have to reason
//! about this file's own needles.
//!
//! See `planning/plan-111-A-vocabulary-and-ratchet-gate.md` (the lead document
//! for plan-111) for the design, the letter-by-letter roadmap, and the census
//! commands that seeded these budgets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Scan scope
// ---------------------------------------------------------------------------

/// Paths under `src/` that are outside the scan entirely.
///
/// - `src/ast/` — **the AST *is* the string domain.** It is the output of the
///   parser and the input to `hir::elaborate`; a type there is a source token
///   that has not been given a meaning yet. plan-111's whole statement is
///   "after the AST", so banning spellings inside it would ban the thing.
/// - `src/lexer.rs` — likewise: it produces the tokens `src/ast` consumes.
/// - `src/docs/` — the embedded `mfb spec` / `mfb man` corpus. Its type
///   spellings are prose and examples shown to the user, not decisions.
fn is_excluded_from_scan(rel: &str) -> bool {
    rel.starts_with("src/ast/") || rel == "src/lexer.rs" || rel.starts_with("src/docs/")
}

/// Test-only files: fixture corpora, round-trip needle lists and scan helpers
/// legitimately spell types as strings, because a test's *input* is a spelling.
/// Mirrors the census globs in plan-111-A §2
/// (`!**/tests*`, `!**/*_tests.rs`, `!src/testutil.rs`).
fn is_test_file(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or("");
    rel.contains("/tests/")
        || name == "tests.rs"
        || name == "testutil.rs"
        || name == "test_support.rs"
        || name.ends_with("_tests.rs")
        || name.starts_with("tests")
}

/// The non-test lines of a source file, as `(zero-based line number, line)`.
///
/// `tests/architecture_guards.rs` does this by truncating at the first
/// `#[cfg(test)]`, which works there because its two scan roots keep their test
/// code in a trailing module. It is **wrong across `src/` as a whole**: this
/// tree also uses `#[cfg(test)]` on individual mid-file items — a probe field
/// (`src/ir/shape.rs:158` `bound_types`), a test-only entry point
/// (`src/resolver/mod.rs:105` `resolve_hir_project`) — and truncating there
/// discards the rest of the file. Measured: with truncation the scanner saw 4
/// `parse_sites` in `ir` where 19 exist, and missed `src/resolver/mod.rs:275`
/// entirely.
///
/// So instead: strip each `#[cfg(test)]`-attributed item by brace depth, and
/// keep everything else. A single-line item (a struct field, a `mod tests;`
/// declaration) suppresses just its own line; one that opens a block is
/// suppressed until depth returns to where it started.
fn test_free_lines(src: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut depth: isize = 0;
    let mut pending_cfg_test = false;
    let mut skip_to: Option<isize> = None;

    for (n, line) in src.lines().enumerate() {
        let start_depth = depth;
        depth += line.matches('{').count() as isize - line.matches('}').count() as isize;

        if let Some(target) = skip_to {
            if depth <= target {
                skip_to = None;
            }
            continue;
        }
        let t = line.trim();
        if t == "#[cfg(test)]" {
            pending_cfg_test = true;
            continue;
        }
        if pending_cfg_test {
            pending_cfg_test = false;
            if depth > start_depth {
                skip_to = Some(start_depth);
            }
            continue;
        }
        if t.starts_with("mod tests {") || t == "mod tests;" {
            if depth > start_depth {
                skip_to = Some(start_depth);
            }
            continue;
        }
        out.push((n, line));
    }
    out
}

/// Recursively collect every `.rs` file under `roots`.
fn rs_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read source dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The five sanctioned boundaries (plan-111-A §2 "The five boundaries")
// ---------------------------------------------------------------------------

/// A type spelling legitimately *exists* in exactly five places, all of them
/// converting between the string world and the type world, none of them making
/// a decision. These are the only files `ParameterType::parse` may appear in.
///
/// Only the `parse_sites` class honours this list. The other six classes are
/// decisions, not conversions, and are exempt nowhere — a boundary file that
/// wants to `match` on a spelling has stopped being a boundary.
const BOUNDARY_FILES: &[(&str, &str)] = &[
    // 1 — the parser's own recursion. `parse` calls itself for every payload.
    ("src/types.rs", "the type grammar itself; `parse` recurses"),
    // 2 — IR wire/JSON decode: a serialized type arrives as text by definition.
    (
        "src/ir/binary.rs",
        "IR wire/JSON decode reads types as text",
    ),
    // 3 — AST -> HIR elaborate: the AST boundary, where spellings become types.
    (
        "src/hir/mod.rs",
        "AST->HIR elaborate: the string/type boundary",
    ),
    (
        "src/hir/build.rs",
        "AST->HIR elaborate: the string/type boundary",
    ),
    // 4 — `.mfp` wire codec: the package type table is text on disk.
    ("src/binary_repr/writer.rs", ".mfp wire codec"),
    ("src/binary_repr/sections.rs", ".mfp type-table codec"),
    // Same codec, decode side: `builder` resolves a wire `type_id` through the
    // package string table into the public `BinaryRepr*` view. plan-111-A named
    // the two files that HAD parses when it was written; this one acquired its
    // in plan-111-B, when the export signature stopped being decoded as text and
    // started being decoded as a type — which is what a boundary is for.
    (
        "src/binary_repr/builder.rs",
        ".mfp wire codec (decode side)",
    ),
    // 5 — manifest entry decode: project.json carries type spellings.
    ("src/manifest/package.rs", "manifest entry decode"),
];

fn is_boundary_file(rel: &str) -> bool {
    BOUNDARY_FILES.iter().any(|(p, _)| *p == rel)
}

// ---------------------------------------------------------------------------
// Needle vocabularies
// ---------------------------------------------------------------------------

/// Identifiers that name a *type* in a signature. A `&str` here is a type
/// spelling reaching a decision through the front door.
const TYPE_PARAM_NAMES: &[&str] = &[
    "type_",
    "type_name",
    "element_type",
    "value_type",
    "key_type",
    "field_type",
    "return_type",
    "declared_type",
    "target_type",
    "source_type",
    "state_type",
    "param_type",
    "arg_type",
    "base_type",
    "union_type",
    "member_type",
    "collection_type",
    "scrutinee_type",
    // plan-111-D Correction D1. The list was hand-seeded and missed these; a
    // sweep for `*type*: &str` across `src/` found them all at once. Deliberately
    // NOT added: `ctype` / `socktype` / `abi_return_ctype` (the C FFI type
    // vocabulary — `CInt8`, `CBool`, a genuinely different grammar, LINK's, not
    // MFBASIC's) and `type_code` (a numeric collection type-code rendered as a
    // string immediate, not a type name).
    "record_type",
    "result_type",
    "payload_type",
    "success_type",
    "ret_type",
    "resource_type",
    "function_type",
    "stride_type",
    "block_type",
    "type_str",
];

/// The language's built-in scalar spellings, as they appear in a `match` arm.
const SPELLINGS_MATCH: &[&str] = &[
    "Integer",
    "String",
    "Boolean",
    "Float",
    "Fixed",
    "Byte",
    "Money",
    "Nothing",
    "AttributeString",
    "Scalar",
    "Unknown",
    "Error",
];

/// The same, plus `Result` — which is compared against but never a bare match
/// arm (it is always `Result OF ...`).
const SPELLINGS_COMPARE: &[&str] = &[
    "Integer",
    "String",
    "Boolean",
    "Float",
    "Fixed",
    "Byte",
    "Money",
    "Nothing",
    "AttributeString",
    "Scalar",
    "Unknown",
    "Error",
    "Result",
];

/// `str` methods that take a type apart. Pairing them with a grammar token is
/// what makes them a second, hand-rolled copy of `ParameterType::parse`.
const GRAMMAR_METHODS: &[&str] = &[
    "split_once",
    "strip_prefix",
    "strip_suffix",
    "starts_with",
    "ends_with",
    "contains",
];

/// Grammar tokens of the type language. A `str` method applied to one of these
/// is re-implementing the parser.
const GRAMMAR_TOKENS: &[&str] = &[
    " STATE ",
    " TO ",
    " OF ",
    "List OF",
    "Set OF",
    "Map OF",
    "Result OF",
    "MapEntry OF",
    "RES ",
    "Thread OF",
    "ThreadWorker OF",
    "FUNC(",
    "ISOLATED FUNC(",
];

/// Container prefixes a `format!` uses to *build* a type spelling. Building one
/// is the inverse of parsing one, and equally a second grammar.
const CONSTRUCTED_PREFIXES: &[&str] = &[
    "List OF",
    "Set OF",
    "Map OF",
    "Result OF",
    "MapEntry OF",
    "Thread OF",
    "ThreadWorker OF",
    "RES ",
];

/// Maps and sets whose **key is a type name**, spelled `String`.
///
/// This class cannot be a bare regex: `HashMap<String, _>` appears 1209 times
/// in `src/` and almost all of them are keyed by a *symbol* — a function name,
/// a binding name, a package alias — which is legitimately a string. Keyed by a
/// type, it is a spelling making a decision (two spellings of one type miss
/// each other; `ParameterType` equality would not).
///
/// So the population is enumerated, `(file, identifier)`, each entry read and
/// confirmed type-keyed. Re-keying one by `ParameterType` (letter C) changes
/// its declaration and drops the count. Nearby non-type-keyed lookalikes that
/// are deliberately NOT listed, so the next reader does not "fix" them:
/// `ir/lower.rs`'s `binding_types` (keyed by variable name),
/// `monomorph`'s and `codegen`'s `function_types` / `package_return_types`
/// (function name), `ir/resource_escape.rs`'s `decl_type` (declaration name),
/// `data_objects.rs`'s local `types` (parameter name), `ir::verify`'s and
/// `monomorph`'s `globals` (binding name), `ir/shape.rs`'s `state_dropped`
/// (local name), codegen's and NIR's `resource_owners` / `owner_collections`
/// (binding name), and `function_lowering.rs`'s `union_extract_reads` (local
/// name).
///
/// **How this list was built, and how to extend it.** Two earlier attempts
/// under-reported, so the third was exhaustive and is recorded here so a fourth
/// is not needed. Every `HashMap`/`BTreeMap`/`HashSet`/`BTreeSet` declaration
/// in `src/` whose key is `String`, `&str` or `(String, String)` was extracted
/// with its doc comment — 251 of them — and each was classified by one
/// question: **is the KEY a type name?** 58 were candidates (identifier or doc
/// mentioning type/record/union/enum/resource/variant/nominal); each was read.
///
/// The two failed attempts are worth knowing about. The first grepped
/// identifiers containing `type`, which cannot see `record_fields`,
/// `union_names` or `enum_members` — most of `TypeModel`. The second fixed that
/// but used `\b`-delimited words, which do not match inside snake_case, and was
/// then run over the wrong subset of directories.
///
/// Three kinds of lookalike are deliberately EXCLUDED, so they are not "fixed"
/// later. Keyed by a **binding or local name**: `ir/lower.rs`'s
/// `binding_types` / `mutable_locals`, `ir::verify`'s and `monomorph`'s
/// `globals`, `ir/resource_escape.rs`'s `decl_type`, `ir/shape.rs`'s
/// `state_dropped`, codegen's and NIR's `resource_owners` /
/// `owner_collections` / `promoted_vector_locals`,
/// `function_lowering.rs`'s `union_extract_reads`, and `data_objects.rs`'s
/// local `types`. Keyed by a **function or symbol name**: `function_types`,
/// `package_return_types`, `imported_overloads`, `concrete_symbol_keys`.
/// **Wire type-table companions**: `binary_repr`'s `TypeTable::foreign_types`
/// sits beside `TypeTable::ids` — the `.mfp` type section's own name -> id
/// table — and both are keyed by the spelling the section ENCODES. Converting
/// one without the other would be incoherent, and converting `ids` would stop
/// the wire codec speaking wire names, which is the opposite of what boundary
/// #4 is for. Same reasoning as `ir/binary.rs`'s `seen_types`.
/// **Synthetic instantiation keys**: `monomorph`'s `emitted_type_keys` and
/// `emitted_function_keys` hold `"Pair<Integer,String>"` — a `name<args>` key
/// minted to be unambiguous where the mangled symbol is lossy (bug-226 /
/// bug-400). That is not a spelling in the type grammar (which would be
/// `Pair OF Integer, String`), so it is a key, not a type. Listed here because
/// the identifier reads as if it were type-keyed and an earlier census pass
/// classified it that way.
/// **Declaration indexes** — name → the AST/HIR node that declared it, built
/// beside a `funcs` index of the identical shape, so the key is an identifier
/// and the value is a declaration rather than type information:
/// `ir/docs.rs`'s and `resolver/resolution.rs`'s `types` / `resources`,
/// `src/doc/`'s `type_meta` / `resource_meta`, `ir/shape.rs`'s `union_decls`,
/// and `ir/binary.rs`'s `seen_types` (a duplicate-NAME detector on the wire).
const TYPE_KEYED_TABLES: &[(&str, &str)] = &[
    // `TypeModel` — codegen's whole picture of the module's declared types,
    // keyed by rendered name. plan-111-C re-keys these.
    ("src/codegen/engine/builder/mod.rs", "enum_members"),
    ("src/codegen/engine/builder/mod.rs", "record_fields"),
    ("src/codegen/engine/builder/mod.rs", "union_names"),
    ("src/codegen/engine/builder/mod.rs", "union_variants"),
    ("src/codegen/engine/builder/mod.rs", "union_variant_unions"),
    ("src/codegen/engine/builder/mod.rs", "union_variant_tags"),
    ("src/codegen/engine/builder/mod.rs", "union_variant_fields"),
    ("src/codegen/engine/builder/mod.rs", "resource_names"),
    ("src/codegen/engine/builder/mod.rs", "resource_closers"),
    // Two more codegen tables outside `TypeModel`, both keyed by a resource
    // TYPE name (plan-111-B Correction 12).
    (
        "src/codegen/link/thunk/link_thunk.rs",
        "record_native_resources",
    ),
    (
        "src/codegen/engine/validation/validation.rs",
        "native_resources",
    ),
    // A resource UNION type -> its variants' close helpers.
    (
        "src/target/shared/runtime/usage.rs",
        "resource_union_closes",
    ),
    // The IR shape checker's type registry.
    ("src/ir/shape.rs", "resource_types"),
    ("src/ir/shape.rs", "types"),
    // `ir::lower`'s `TypeIndex` — the lowering-side twin of `TypeModel`.
    ("src/ir/lower.rs", "records"),
    ("src/ir/lower.rs", "enums"),
    ("src/ir/lower.rs", "variants"),
    ("src/ir/lower.rs", "variant_unions"),
    ("src/ir/lower.rs", "variant_fields"),
    // `ir::verify`'s `TypeEnv` — nine tables, every one keyed by a type name.
    // `globals` is deliberately absent: it is keyed by a BINDING name.
    ("src/ir/verify/mod.rs", "records"),
    ("src/ir/verify/mod.rs", "unions"),
    ("src/ir/verify/mod.rs", "resource_closers"),
    ("src/ir/verify/mod.rs", "resource_sendable"),
    ("src/ir/verify/mod.rs", "field_types"),
    ("src/ir/verify/mod.rs", "record_field_lists"),
    ("src/ir/verify/mod.rs", "enums"),
    ("src/ir/verify/mod.rs", "type_decl_info"),
    ("src/ir/verify/mod.rs", "private_fields"),
    // A union's variant SET — a membership test over type names, the same
    // operation (and the same shape) as codegen's `union_names`.
    ("src/ir/verify/mod.rs", "variants"),
    // Monomorphization's template and instantiation tables.
    ("src/monomorph/mod.rs", "type_templates"),
    ("src/monomorph/mod.rs", "concrete_types"),
    ("src/monomorph/mod.rs", "type_instantiations"),
    ("src/monomorph/mod.rs", "record_fields"),
    // The resolver's set of declared type names.
    ("src/resolver/mod.rs", "types"),
];

// ---------------------------------------------------------------------------
// Low-level matching helpers
// ---------------------------------------------------------------------------

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Byte index just past `run of spaces` starting at `i`.
fn skip_spaces(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    i
}

/// Every start index at which `needle` occurs in `hay` **at a word boundary on
/// its left** (so `type_` does not match inside `sub_type_`).
fn word_start_matches(hay: &str, needle: &str) -> Vec<usize> {
    let bytes = hay.as_bytes();
    hay.match_indices(needle)
        .filter(|(i, _)| *i == 0 || !is_word_byte(bytes[i - 1]))
        .map(|(i, _)| i)
        .collect()
}

/// Is `hay[i..]` a quoted one of `names`, ending at a `"`? Returns the index
/// just past the closing quote.
fn quoted_name_end(hay: &str, i: usize, names: &[&str]) -> Option<usize> {
    let bytes = hay.as_bytes();
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    for name in names {
        let end = i + 1 + name.len();
        if hay.len() > end && &hay[i + 1..end] == *name && bytes[end] == b'"' {
            return Some(end + 1);
        }
    }
    None
}

/// A `file:line` hit, with the needle that produced it.
#[derive(Clone)]
struct Hit {
    line: usize,
    what: String,
}

fn hit(line: usize, what: impl Into<String>) -> Hit {
    Hit {
        line: line + 1,
        what: what.into(),
    }
}

/// Lines of `src` that are code: outside every `#[cfg(test)]` item, and not a
/// `//` comment. Yields `(zero-based line number, line)`.
fn code_lines(src: &str) -> impl Iterator<Item = (usize, &str)> {
    test_free_lines(src)
        .into_iter()
        .filter(|(_, l)| !l.trim_start().starts_with("//"))
}

// ---------------------------------------------------------------------------
// The seven scan classes
// ---------------------------------------------------------------------------

/// 1 — `ParameterType::parse(...)`: turning a spelling back into a type below
/// the boundary that already had one.
fn parse_sites(src: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for (n, line) in code_lines(src) {
        for _ in line.match_indices("ParameterType::parse(") {
            out.push(hit(n, "ParameterType::parse("));
        }
    }
    out
}

/// 2 — a parameter whose *name* says "type" and whose *type* is `&str`.
fn str_type_params(src: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for (n, line) in code_lines(src) {
        let bytes = line.as_bytes();
        for name in TYPE_PARAM_NAMES {
            for start in word_start_matches(line, name) {
                let mut i = skip_spaces(bytes, start + name.len());
                if bytes.get(i) != Some(&b':') {
                    continue;
                }
                i = skip_spaces(bytes, i + 1);
                if bytes.get(i) != Some(&b'&') {
                    continue;
                }
                i += 1;
                // optional `'lifetime `
                if bytes.get(i) == Some(&b'\'') {
                    let mut j = i + 1;
                    while j < bytes.len() && bytes[j].is_ascii_lowercase() {
                        j += 1;
                    }
                    if bytes.get(j) != Some(&b' ') {
                        continue;
                    }
                    i = j + 1;
                }
                if line[i..].starts_with("str")
                    && !bytes.get(i + 3).copied().is_some_and(is_word_byte)
                {
                    out.push(hit(n, format!("{name}: &str")));
                }
            }
        }
    }
    out
}

/// 3 — a `match` arm on a type spelling: `"Integer" | "Float" => ...`.
fn spelling_match_arms(src: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for (n, line) in code_lines(src) {
        // The arm PATTERN is everything left of the arrow. Scanning the pattern
        // (rather than requiring the line to *begin* with a quoted spelling)
        // is what catches a tuple arm — `("sin", "Float") => …` — which
        // dispatches on a spelling exactly as much as a bare arm does but was
        // invisible to this scanner until plan-111-D Correction D1.
        let Some(arrow) = line.find("=>") else {
            continue;
        };
        // The pattern ends at a match GUARD if there is one: `_ if t == "X" =>`
        // makes its decision by COMPARING, which is class 4's needle, not this
        // one. Counting it here would both double-count and mislabel it.
        let head = &line[..arrow];
        let pattern = match head.find(" if ") {
            Some(g) => &head[..g],
            None => head,
        };
        let mut found: Vec<&str> = Vec::new();
        let mut i = 0;
        while let Some(q) = pattern[i..].find('"') {
            let at = i + q;
            match quoted_name_end(pattern, at, SPELLINGS_MATCH) {
                Some(end) => {
                    found.push(&pattern[at..end]);
                    i = end;
                }
                None => {
                    // Skip the whole quoted run so a non-spelling literal
                    // (`"sin"`, `"someKey"`) cannot desynchronize the scan.
                    match pattern[at + 1..].find('"') {
                        Some(close) => i = at + 1 + close + 1,
                        None => break,
                    }
                }
            }
        }
        if !found.is_empty() {
            out.push(hit(n, found.join(" ")));
        }
    }
    out
}

/// 4 — `== "Integer"` / `!= "String"`: a decision made by comparing spellings.
fn spelling_compares(src: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for (n, line) in code_lines(src) {
        let bytes = line.as_bytes();
        for (i, _) in line.match_indices("= \"") {
            if i == 0 || !matches!(bytes[i - 1], b'=' | b'!') {
                continue;
            }
            if let Some(end) = quoted_name_end(line, i + 2, SPELLINGS_COMPARE) {
                out.push(hit(n, line[i - 1..end].to_string()));
            }
        }
        // plan-111-D Correction D1: the same decision through a wrapper —
        // `== Some("Integer")`, `!= Ok("Float")`. The comparison is against a
        // spelling either way; only the `Option`/`Result` shell differs, and
        // hiding behind one was enough to stay invisible through letters A-C.
        for (i, _) in line.match_indices("= Some(\"") {
            if i == 0 || !matches!(bytes[i - 1], b'=' | b'!') {
                continue;
            }
            if let Some(end) = quoted_name_end(line, i + 7, SPELLINGS_COMPARE) {
                out.push(hit(n, line[i - 1..end].to_string()));
            }
        }
    }
    out
}

/// 5 — a `str` method applied to a grammar token: a second, hand-rolled parser.
fn hand_rolled_grammar(src: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for (n, line) in code_lines(src) {
        for method in GRAMMAR_METHODS {
            let open = format!("{method}(\"");
            for (i, _) in line.match_indices(open.as_str()) {
                let rest = &line[i + open.len()..];
                if let Some(tok) = GRAMMAR_TOKENS.iter().find(|t| rest.starts_with(**t)) {
                    out.push(hit(n, format!("{method}(\"{tok}")));
                }
            }
        }
    }
    out
}

/// 6 — `format!("List OF {…}")`: constructing a spelling instead of a type.
fn format_type_construction(src: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for (n, line) in code_lines(src) {
        for (i, _) in line.match_indices("format!(\"") {
            let rest = &line[i + 9..];
            if let Some(p) = CONSTRUCTED_PREFIXES.iter().find(|p| rest.starts_with(**p)) {
                out.push(hit(n, format!("format!(\"{p}")));
            }
        }
    }
    out
}

/// 7 — a type-keyed map whose key is `String` (see `TYPE_KEYED_TABLES`).
fn string_keyed_type_maps(rel: &str, src: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for (n, line) in code_lines(src) {
        let bytes = line.as_bytes();
        for (file, ident) in TYPE_KEYED_TABLES {
            if *file != rel {
                continue;
            }
            for start in word_start_matches(line, ident) {
                let mut i = skip_spaces(bytes, start + ident.len());
                if bytes.get(i) != Some(&b':') {
                    continue;
                }
                i = skip_spaces(bytes, i + 1);
                let rest = &line[i..];
                let keyed = ["HashMap<", "BTreeMap<", "HashSet<", "BTreeSet<"]
                    .iter()
                    .find_map(|c| rest.strip_prefix(*c))
                    .map(|after| {
                        let a = after.trim_start();
                        a.starts_with("String,")
                            || a.starts_with("String>")
                            || a.starts_with("String ")
                            || a.starts_with("(String, String)")
                    })
                    .unwrap_or(false);
                if keyed {
                    out.push(hit(n, format!("{ident}: String-keyed")));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Budgets
// ---------------------------------------------------------------------------

/// `(class, directory, ceiling)`.
///
/// Seeded at plan-111's start with the counts this scanner measured at
/// `f79f6212a` (2026-08-29). The `directory` is the first path component under
/// `src/`, or the file stem for a file that sits directly in `src/`.
///
/// **How to lower a budget**: do the conversion, run
/// `cargo test --test no_type_strings`, and the failure message prints the new
/// number and the exact table row to paste. Lower it *in the same commit as
/// the conversion* — plan-111-A §3.
///
/// **The end state is every row at 0** and this table deleted, leaving bare
/// `assert_eq!(count, 0)` (letter G). A row missing from this table is a
/// ceiling of 0, so a new violation in a clean directory fails immediately.
const BUDGETS: &[(&str, &str, usize)] = &[
    // Re-measured 2026-08-30 after plan-111-D Correction D1 strengthened three
    // scanners (tuple match arms, `== Some("X")` compares, ten missing
    // `*type*: &str` parameter names). That surfaced 59 sites letters A-C could
    // not see; every row below is the live count, tight in both directions.
    //
    // --- 1. `ParameterType::parse` below a boundary. Letters D, E, F.
    // --- 2. a type taken as `&str`. Letters D, E, F, G.
    // --- 3. a `match` arm on a spelling, INCLUDING a tuple arm. Letters D, E, F, G.
    // --- 4. `==` / `!=` against a spelling, incl. through `Some(...)`. D, E, F, G.
    //     `optimizer` is new and belongs to G: its three sites read the NIR
    //     `mov_imm` "type" operand-class attribute, whose producer is
    //     `target/shared/abi.rs`'s `move_immediate(type_: &str)` — already a
    //     counted `target` row. Producer and consumers convert together.
    // --- 5. a hand-rolled second grammar. Letter A, then B, E, G. `types` 25
    //     is `ParameterType::contains_state`'s one `contains(" STATE ")` for the
    //     composite-base spelling `parse` declines to split, plus the parser's
    //     own recursion; letter G retires the rest.
    // --- 6. a spelling built with `format!`. Letters E, F, G. `codegen` reached
    //     0 in letter C and its row is gone.
    // --- 7. a type-keyed map keyed by `String`. Reached 0 tree-wide in letter C;
    //     the class has no row at all, which is the shape every class ends in.
    ("parse_sites", "codegen", 92),
    ("str_type_params", "binary_repr", 5),
    ("str_type_params", "codegen", 166),
    ("str_type_params", "hir", 1),
    ("str_type_params", "numeric", 1),
    ("str_type_params", "target", 4),
    ("str_type_params", "types", 2),
    ("spelling_match_arms", "binary_repr", 19),
    ("spelling_match_arms", "codegen", 172),
    ("spelling_match_arms", "types", 9),
    ("spelling_compares", "codegen", 60),
    ("spelling_compares", "optimizer", 3),
    ("spelling_compares", "target", 2),
    ("spelling_compares", "types", 2),
    ("hand_rolled_grammar", "binary_repr", 3),
    ("hand_rolled_grammar", "codegen", 7),
    ("hand_rolled_grammar", "types", 25),
    ("format_type_construction", "binary_repr", 5),
    ("format_type_construction", "types", 6),
];

/// First path component under `src/`, or the stem of a file directly in `src/`.
fn bucket(rel: &str) -> String {
    let tail = rel.strip_prefix("src/").unwrap_or(rel);
    match tail.split_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => tail.trim_end_matches(".rs").to_string(),
    }
}

const CLASSES: &[&str] = &[
    "parse_sites",
    "str_type_params",
    "spelling_match_arms",
    "spelling_compares",
    "hand_rolled_grammar",
    "format_type_construction",
    "string_keyed_type_maps",
];

/// Live counts and offenders, keyed by `(class, bucket)`.
fn scan_tree() -> BTreeMap<(String, String), Vec<String>> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();

    for path in rs_files(&[manifest.join("src")]) {
        let rel = path
            .strip_prefix(manifest)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if is_excluded_from_scan(&rel) || is_test_file(&rel) {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        let b = bucket(&rel);

        let mut record = |class: &str, hits: Vec<Hit>| {
            if hits.is_empty() {
                return;
            }
            let entry = out.entry((class.to_string(), b.clone())).or_default();
            for h in hits {
                entry.push(format!("{rel}:{} — {}", h.line, h.what));
            }
        };

        if !is_boundary_file(&rel) {
            record("parse_sites", parse_sites(&src));
        }
        record("str_type_params", str_type_params(&src));
        record("spelling_match_arms", spelling_match_arms(&src));
        record("spelling_compares", spelling_compares(&src));
        record("hand_rolled_grammar", hand_rolled_grammar(&src));
        record("format_type_construction", format_type_construction(&src));
        record("string_keyed_type_maps", string_keyed_type_maps(&rel, &src));
    }
    out
}

/// The live scan rendered as a paste-ready `BUDGETS` table.
fn print_budget_table(live: &BTreeMap<(String, String), Vec<String>>) -> String {
    let mut s = String::new();
    for class in CLASSES {
        for ((c, b), hits) in live {
            if c == class {
                s.push_str(&format!("    (\"{c}\", \"{b}\", {}),\n", hits.len()));
            }
        }
    }
    s
}

#[test]
fn no_type_strings() {
    let live = scan_tree();

    let mut over: Vec<String> = Vec::new();
    let mut slack: Vec<String> = Vec::new();

    for ((class, b), hits) in &live {
        let budget = BUDGETS
            .iter()
            .find(|(c, d, _)| c == class && d == b)
            .map(|(_, _, n)| *n)
            .unwrap_or(0);
        if hits.len() > budget {
            over.push(format!(
                "\n  {class} / {b}: {} > budget {budget}\n    {}",
                hits.len(),
                hits.join("\n    ")
            ));
        }
    }
    for (class, b, budget) in BUDGETS {
        let count = live
            .get(&(class.to_string(), b.to_string()))
            .map(|h| h.len())
            .unwrap_or(0);
        if *budget > count {
            slack.push(format!(
                "  (\"{class}\", \"{b}\", {budget}) — lower this budget to {count}"
            ));
        }
    }

    assert!(
        over.is_empty(),
        "plan-111: a type spelling reached a decision below the AST. Convert the \
         site to `ParameterType` (see planning/plan-111-A-vocabulary-and-ratchet-gate.md \
         §2 for the seven classes and the five sanctioned boundary files). \
         Offenders:{}\n\nCurrent live table:\n{}",
        over.join(""),
        print_budget_table(&live)
    );
    assert!(
        slack.is_empty(),
        "plan-111: the budget table has slack — a ceiling above reality is an \
         allowance a future regression can hide inside. Lower these rows (the \
         work that cleared them should have lowered them in its own commit):\n{}\n\n\
         Current live table:\n{}",
        slack.join("\n"),
        print_budget_table(&live)
    );
}

/// The gate is only worth its budgets if the scanner actually fires. Each class
/// gets one positive fixture and, where it is the difference between a real
/// hit and a lookalike, one negative.
#[test]
fn scanners_fire_on_their_own_needles() {
    assert_eq!(
        parse_sites("let t = ParameterType::parse(name);").len(),
        1,
        "parse_sites must count a ParameterType::parse( call"
    );
    assert_eq!(
        parse_sites("// let t = ParameterType::parse(name);").len(),
        0,
        "a commented-out call is not a site"
    );
    assert_eq!(
        parse_sites("ParameterType::map_of(ParameterType::parse(k), ParameterType::parse(v))")
            .len(),
        2,
        "two calls on one line are two sites, not one line"
    );

    assert_eq!(
        str_type_params("fn f(element_type: &str) {}").len(),
        1,
        "str_type_params must count a type-named &str parameter"
    );
    assert_eq!(
        str_type_params("fn f(type_: &'a str) {}").len(),
        1,
        "a lifetime does not hide the parameter"
    );
    assert_eq!(
        str_type_params("fn f(type_name: &ParameterType) {}").len(),
        0,
        "an already-typed parameter is not a violation"
    );
    assert_eq!(
        str_type_params("fn f(sub_type_: &str) {}").len(),
        0,
        "`type_` must match at a word boundary, not inside another identifier"
    );

    assert_eq!(
        spelling_match_arms("        \"Integer\" | \"Float\" => 8,").len(),
        1,
        "spelling_match_arms must count a multi-spelling arm"
    );
    assert_eq!(
        spelling_match_arms("        (\"sin\", \"Float\") => Kernel::Sin,").len(),
        1,
        "plan-111-D Correction D1: a TUPLE arm dispatches on a spelling too"
    );
    assert_eq!(
        spelling_match_arms("        (\"min\", \"Integer\" | \"Fixed\") => Kernel::MinSigned,").len(),
        1,
        "a tuple arm with an or-pattern is still one arm"
    );
    assert_eq!(
        spelling_match_arms("            other => self.render(other, \"Integer\"),").len(),
        0,
        "a spelling to the RIGHT of the arrow is an argument, not a pattern"
    );
    assert_eq!(
        spelling_match_arms("        _ if t == \"Integer\" => 8,").len(),
        0,
        "a compare inside a guard belongs to spelling_compares, not here"
    );
    assert_eq!(
        spelling_compares("        _ if t == \"Integer\" => 8,").len(),
        1,
        "…and spelling_compares must be the one that counts it"
    );
    assert_eq!(
        spelling_compares("            if x.get(\"type\").as_deref() == Some(\"Integer\") {").len(),
        1,
        "plan-111-D Correction D1: a compare wrapped in Some() is still a compare"
    );
    assert_eq!(
        spelling_compares("            let v = Some(\"Integer\");").len(),
        0,
        "…but constructing a Some() is not a comparison"
    );
    assert_eq!(
        spelling_match_arms("        \"someKey\" => 8,").len(),
        0,
        "a non-type string arm is not a violation"
    );

    assert_eq!(
        spelling_compares("if t == \"Money\" { }").len(),
        1,
        "spelling_compares must count an == against a spelling"
    );
    assert_eq!(
        spelling_compares("if t != \"Result\" { }").len(),
        1,
        "and a != too"
    );
    assert_eq!(
        spelling_compares("let x = \"Integer\";").len(),
        0,
        "a plain assignment is not a comparison"
    );

    assert_eq!(
        hand_rolled_grammar("name.split_once(\" STATE \")").len(),
        1,
        "hand_rolled_grammar must count a grammar split"
    );
    assert_eq!(
        hand_rolled_grammar("name.strip_prefix(\"List OF \")").len(),
        1,
        "and a container strip"
    );
    assert_eq!(
        hand_rolled_grammar("path.starts_with(\"src/\")").len(),
        0,
        "a non-grammar token is not a violation"
    );

    assert_eq!(
        format_type_construction("format!(\"List OF {inner}\")").len(),
        1,
        "format_type_construction must count a constructed spelling"
    );
    assert_eq!(
        format_type_construction("format!(\"unknown type {t}\")").len(),
        0,
        "a message mentioning a type is not construction"
    );

    assert_eq!(
        string_keyed_type_maps(
            "src/codegen/engine/builder/mod.rs",
            "    pub(crate) union_names: HashSet<String>,"
        )
        .len(),
        1,
        "string_keyed_type_maps must count a listed table keyed by String"
    );
    assert_eq!(
        string_keyed_type_maps(
            "src/codegen/engine/builder/mod.rs",
            "    pub(crate) union_names: HashSet<ParameterType>,"
        )
        .len(),
        0,
        "re-keying by ParameterType is what lowers the count"
    );
    assert_eq!(
        string_keyed_type_maps("src/ir/lower.rs", "    binding_types: HashMap<String, X>,").len(),
        0,
        "a table not on the curated list is not scanned"
    );
}

/// A per-FILE census of the live population, for scoping a letter at kickoff.
///
/// The budget table above is per-`(class, directory)`, which is the right
/// granularity for a ratchet but the wrong one for planning: letters D, E and F
/// each own a *file list*, and each of their §2 tables was built with `rg`,
/// which counts inline `#[cfg(test)]` modules and therefore over-counts (see
/// plan-111-A Correction 3, plan-111-C Correction C3 — three times now). This
/// dump uses the gate's own `test_free_lines` stripper, so re-scoping a letter
/// against it cannot repeat that mistake.
///
/// `#[ignore]`d because it asserts nothing; it is a measuring instrument.
///
/// ```text
/// cargo test --test no_type_strings census_by_file -- --ignored --nocapture
/// ```
#[test]
#[ignore = "measuring instrument, not an assertion — run with --ignored --nocapture"]
fn census_by_file() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    // (file, class) -> count, plus a per-file total for ordering.
    let mut per_file: BTreeMap<String, BTreeMap<&'static str, usize>> = BTreeMap::new();

    for path in rs_files(&[manifest.join("src")]) {
        let rel = path
            .strip_prefix(manifest)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if is_excluded_from_scan(&rel) || is_test_file(&rel) {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        let mut row: BTreeMap<&'static str, usize> = BTreeMap::new();
        if !is_boundary_file(&rel) {
            row.insert("parse_sites", parse_sites(&src).len());
        }
        row.insert("str_type_params", str_type_params(&src).len());
        row.insert("spelling_match_arms", spelling_match_arms(&src).len());
        row.insert("spelling_compares", spelling_compares(&src).len());
        row.insert("hand_rolled_grammar", hand_rolled_grammar(&src).len());
        row.insert(
            "format_type_construction",
            format_type_construction(&src).len(),
        );
        row.insert(
            "string_keyed_type_maps",
            string_keyed_type_maps(&rel, &src).len(),
        );
        row.retain(|_, n| *n > 0);

        // `MFB_CENSUS_DETAIL=<substring>` additionally prints every offending
        // line for the matching files, which is how a phase finds its work.
        if let Ok(filter) = std::env::var("MFB_CENSUS_DETAIL") {
            if !filter.is_empty() && rel.contains(&filter) {
                println!("\n--- {rel}");
                let mut all: Vec<(&str, Hit)> = Vec::new();
                if !is_boundary_file(&rel) {
                    all.extend(parse_sites(&src).into_iter().map(|h| ("parse", h)));
                }
                all.extend(str_type_params(&src).into_iter().map(|h| ("param", h)));
                all.extend(spelling_match_arms(&src).into_iter().map(|h| ("arm", h)));
                all.extend(spelling_compares(&src).into_iter().map(|h| ("cmp", h)));
                all.extend(hand_rolled_grammar(&src).into_iter().map(|h| ("gram", h)));
                all.extend(
                    format_type_construction(&src)
                        .into_iter()
                        .map(|h| ("fmt", h)),
                );
                all.extend(
                    string_keyed_type_maps(&rel, &src)
                        .into_iter()
                        .map(|h| ("map", h)),
                );
                all.sort_by_key(|(_, h)| h.line);
                for (class, h) in all {
                    println!("  {:<6} {}:{} — {}", class, rel, h.line, h.what);
                }
            }
        }

        if !row.is_empty() {
            per_file.insert(rel, row);
        }
    }

    let mut rows: Vec<(String, usize, BTreeMap<&'static str, usize>)> = per_file
        .into_iter()
        .map(|(f, r)| {
            let total = r.values().sum();
            (f, total, r)
        })
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    println!("\nplan-111 live census by file ({} files with any hit)", rows.len());
    println!(
        "{:<62} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>6}",
        "file", "parse", "param", "arm", "cmp", "gram", "fmt", "map", "total"
    );
    let mut grand = 0usize;
    for (file, total, row) in &rows {
        grand += total;
        let g = |c: &str| row.get(c).copied().unwrap_or(0);
        println!(
            "{:<62} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>6}",
            file,
            g("parse_sites"),
            g("str_type_params"),
            g("spelling_match_arms"),
            g("spelling_compares"),
            g("hand_rolled_grammar"),
            g("format_type_construction"),
            g("string_keyed_type_maps"),
            total
        );
    }
    println!("{:<62} {:>50}", "TOTAL", grand);
}

/// The curated `TYPE_KEYED_TABLES` list is only trustworthy if every entry
/// still names something real. A renamed or deleted table would otherwise
/// silently drop out of the population and look like progress.
#[test]
fn curated_type_keyed_tables_all_exist() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut missing: Vec<String> = Vec::new();
    for (file, ident) in TYPE_KEYED_TABLES {
        let src = std::fs::read_to_string(manifest.join(file))
            .unwrap_or_else(|_| panic!("TYPE_KEYED_TABLES names a missing file: {file}"));
        if !test_free_lines(&src).iter().any(|(_, l)| l.contains(ident)) {
            missing.push(format!("{file}: {ident}"));
        }
    }
    assert!(
        missing.is_empty(),
        "TYPE_KEYED_TABLES entries no longer present — if a table was renamed, \
         update the entry; if it was genuinely removed, delete the entry AND \
         lower the string_keyed_type_maps budget:\n{}",
        missing.join("\n")
    );
}

/// Every `BOUNDARY_FILES` entry carries a justification and still exists. The
/// list is the plan's one sanctioned allowance, so it must not rot into a
/// junk drawer.
#[test]
fn boundary_files_exist_and_are_justified() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (file, why) in BOUNDARY_FILES {
        assert!(
            manifest.join(file).is_file(),
            "BOUNDARY_FILES names a missing file: {file}"
        );
        assert!(
            !why.is_empty(),
            "BOUNDARY_FILES entry {file} has no justification"
        );
    }
}
