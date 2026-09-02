//! plan-112's ratchet gate: **no operator strings after the parser**.
//!
//! `BinaryOp`/`UnaryOp` (`src/operators.rs`) are the compiler's only operator
//! currency, from the token the parser consumes to the byte codegen emits. An
//! operator *spelling* — `"MOD"`, `"<>"`, `"^"` — is a rendering, and rendering
//! is legitimate at exactly three sinks: the `.ast` JSON, the `.ir` JSON, and
//! the length-prefixed operator string in the `.mfp` wire format. All three go
//! through `name()`. What is never legitimate is a spelling flowing *into* a
//! decision: matched on, compared against, or carried by a tree node.
//!
//! The vocabulary is **closed** — MFB has no operator overloading and no
//! user-declared operator — so unlike `tests/no_type_strings.rs` this gate needs
//! no budget table. Every class is a **hard zero**.
//!
//! | class | |
//! |---|---|
//! | 1 `spelling_match_arms` | no `match`/`matches!` arm on an operator spelling |
//! | 2 `spelling_compares`   | no `==`/`!=` against an operator spelling |
//! | 3 `node_string_operators` | no `Binary`/`Unary`/`Compare` node carrying a `String` operator |
//! | 4 `str_operator_params` | no fn taking an operator as `&str` |
//!
//! Two allowances carry the rest, and each is pinned by its own test:
//!
//! * `is_vocabulary_file` — `src/operators.rs` DEFINES `name` and `parse`.
//!   Asking the vocabulary not to mention its own spellings is asking it not to
//!   be the vocabulary. Total exemption, exactly one file
//!   (`the_vocabulary_file_is_exactly_one`).
//! * `OPERATOR_SHAPED_NON_OPERATORS` — a closed, justified list for classes 3
//!   and 4 only. Two kinds of entry: the one genuine wire boundary
//!   (`src/ir/link.rs`), and the places where an identifier named `op` denotes
//!   something that is not a language operator at all — a `CodeOp` mnemonic, a
//!   `vector::` member name, an AudioUnit property label. The list is
//!   asserted exactly (`operator_shaped_exemptions_are_closed`), so it cannot
//!   grow quietly.
//!
//! Note what is **not** exempt: classes 1 and 2 apply to every file outside
//! `src/operators.rs`, including `src/ast/` and the wire boundary. That is the
//! load-bearing half — the machine-op layers can name their mnemonics `op` all
//! they like, but nothing anywhere may decide by comparing a *language*
//! operator spelling, because after `BinaryOp::from_token` no such spelling
//! exists to compare.
//!
//! Like `tests/no_type_strings.rs` this lives in `tests/` — an integration
//! crate — so neither the scan roots nor the self-exemption have to reason
//! about this file's own needles.
//!
//! See `planning/completed/plan-112-operator-enum.md` for the design and the
//! census commands that seeded these classes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// The vocabulary
// ---------------------------------------------------------------------------

/// Every spelling `BinaryOp::name`/`UnaryOp::name` can return. Kept here rather
/// than imported because `tests/` is a separate crate and `operators` is
/// `pub(crate)` — and because a gate that reads its needles from the thing it
/// guards cannot catch that thing changing. `vocabulary_matches_the_enum` pins
/// the two lists together against `src/operators.rs`'s source.
const SPELLINGS: &[&str] = &[
    "OR", "XOR", "AND", "=", "<>", "<", "<=", ">", ">=", "&", "+", "-", "*", "/", "MOD", "DIV",
    "^", "NOT", "SIZEOF",
];

// ---------------------------------------------------------------------------
// Scan scope
// ---------------------------------------------------------------------------

/// The embedded `mfb spec` / `mfb man` corpus: its operator spellings are prose
/// and examples shown to the user, not decisions.
fn is_excluded_from_scan(rel: &str) -> bool {
    rel.starts_with("src/docs/")
}

/// Test-only files: a test's *input* may legitimately be a spelling — the
/// `legacy_*` promotion oracles in `src/numeric.rs` are deliberately frozen
/// name-keyed copies, and `src/ir/binary.rs`'s decode tests hand-assemble bytes
/// carrying operator strings a hostile `.mfp` could contain. Mirrors
/// `tests/no_type_strings.rs::is_test_file`.
fn is_test_file(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or("");
    rel.contains("/tests/")
        || name == "tests.rs"
        || name == "testutil.rs"
        || name == "test_support.rs"
        || name.ends_with("_tests.rs")
        || name.starts_with("tests")
}

/// `src/operators.rs` defines the vocabulary: `name` renders every spelling and
/// `parse` matches every spelling. Total exemption, and the only one.
fn is_vocabulary_file(rel: &str) -> bool {
    rel == "src/operators.rs"
}

/// Files where an `op`/`operator` identifier typed `String`/`&str` is **not** a
/// language operator, plus the one place where it is and has to stay.
///
/// Honoured by classes 3 and 4 only. A spelling *decision* (classes 1 and 2) is
/// exempt nowhere — that split is what stops this list becoming a junk drawer.
const OPERATOR_SHAPED_NON_OPERATORS: &[(&str, &str)] = &[
    // The one genuine boundary. `IrLinkExpr::Compare` carries the operator as
    // the `.mfp` wire spelling, and `link_compare_op_valid` is the decoder's
    // validator for it (bug-403: a decoded operator outside the six comparisons
    // must be a hard error, never a silent `=`). Both now route through
    // `BinaryOp::parse`/`is_comparison`, so the file makes no spelling decision
    // of its own; the `String` is the wire format, which §Compatibility freezes.
    (
        "src/ir/link.rs",
        "the `.mfp` wire spelling of `IrLinkExpr::Compare` (bug-403's decode guarantee)",
    ),
    // `CodeOp` mnemonics — a different, already-enumerated vocabulary
    // (`"adrp"`, `"fadd_d"`, `"add_imm"`), explicitly out of plan-112's scope.
    (
        "src/arch/ops.rs",
        "`CodeOp::from_mnemonic` — machine mnemonic",
    ),
    (
        "src/arch/riscv64/v128.rs",
        "RVV mnemonic suffix, not a language operator",
    ),
    (
        "src/target/shared/abi.rs",
        "`CodeInstruction` mnemonic builders",
    ),
    (
        "src/codegen/engine/builder/code_impl.rs",
        "`CodeInstruction::new` takes a machine mnemonic",
    ),
    // Builtin-member names that happen to be called `op`.
    (
        "src/codegen/builtins/audio/gen_macos_shared.rs",
        "an AudioUnit property label",
    ),
    (
        "src/codegen/builtins/collections/gen_mutate.rs",
        "a `collections::` member name (`push`/`set`/…)",
    ),
    (
        "src/codegen/builtins/vector/builder_vector_inline.rs",
        "a `vector::` member name (`add`/`dot`/…)",
    ),
];

fn is_operator_shaped_non_operator(rel: &str) -> bool {
    OPERATOR_SHAPED_NON_OPERATORS
        .iter()
        .any(|(file, _)| *file == rel)
}

// ---------------------------------------------------------------------------
// Line filtering
// ---------------------------------------------------------------------------

/// The non-test lines of a source file, as `(one-based line number, line)`.
///
/// Strips each `#[cfg(test)]`-attributed item by brace depth rather than
/// truncating at the first one — this tree puts `#[cfg(test)]` on individual
/// mid-file items, and truncating there discards the rest of the file. Same
/// reasoning, and same shape, as `tests/no_type_strings.rs::test_free_lines`;
/// `the_line_filter_strips_cfg_test_items_not_the_file_tail` pins it.
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
        out.push((n + 1, line));
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
// The four scanners
// ---------------------------------------------------------------------------

/// A string literal starting at `bytes[at]` (which must be the opening quote),
/// or `None` if it is unterminated. Escapes are not interpreted — an operator
/// spelling contains none.
fn literal_at(bytes: &[u8], at: usize) -> Option<&str> {
    let rest = &bytes[at + 1..];
    let end = rest.iter().position(|b| *b == b'"')?;
    std::str::from_utf8(&rest[..end]).ok()
}

fn is_spelling(text: &str) -> bool {
    SPELLINGS.contains(&text)
}

/// A comment line carries no code. Crude (it does not track block comments or
/// strings containing `//`), and deliberately so: over-skipping a line can only
/// hide a needle inside a comment, and a needle inside a comment is prose.
fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") || t.starts_with("*") || t.starts_with("/*")
}

/// Class 1 — a `match` arm or `matches!` pattern whose literal is an operator
/// spelling. The pattern position is what makes it a decision: `"MOD" => …`
/// dispatches on a rendering instead of on the operator.
fn spelling_match_arms(line: &str) -> bool {
    if is_comment(line) {
        return false;
    }
    let t = line.trim_start();
    // A match arm: one or more `|`-separated spellings, then `=>` (with an
    // optional guard between).
    let bytes = t.as_bytes();
    if bytes.first() == Some(&b'"') {
        let mut at = 0usize;
        let mut saw_spelling = false;
        while at < bytes.len() && bytes[at] == b'"' {
            let Some(text) = literal_at(bytes, at) else {
                break;
            };
            if !is_spelling(text) {
                return false;
            }
            saw_spelling = true;
            at += text.len() + 2;
            while at < bytes.len() && (bytes[at] == b' ' || bytes[at] == b'|') {
                at += 1;
            }
        }
        if saw_spelling && t[at.min(t.len())..].contains("=>") {
            return true;
        }
    }
    // `matches!(scrutinee, "MOD" | "DIV")` — the same decision, written as a macro.
    if let Some(open) = t.find("matches!(") {
        let tail = &t[open..];
        if let Some(comma) = tail.find(',') {
            let after = tail[comma + 1..].trim_start();
            if after.starts_with('"') {
                if let Some(text) = literal_at(after.as_bytes(), 0) {
                    return is_spelling(text);
                }
            }
        }
    }
    false
}

/// Class 2 — `== "MOD"` / `!= "<>"`. The `matches!`-free form of class 1.
fn spelling_compares(line: &str) -> bool {
    if is_comment(line) {
        return false;
    }
    let bytes = line.as_bytes();
    for at in 0..bytes.len() {
        if bytes[at] != b'"' {
            continue;
        }
        let Some(text) = literal_at(bytes, at) else {
            continue;
        };
        if !is_spelling(text) {
            continue;
        }
        let before = line[..at].trim_end();
        if before.ends_with("==") || before.ends_with("!=") {
            return true;
        }
    }
    false
}

/// Class 3 — a `Binary`/`Unary`/`Compare` tree node carrying a `String`
/// operator. This is the shape the plan deleted: four carriers each cloning a
/// spelling per node. Scoped to the node variants rather than to any `String`
/// field, because that is exactly the regression worth catching.
fn node_string_operator(lines: &[(usize, &str)], index: usize) -> bool {
    let (_, line) = lines[index];
    let t = line.trim();
    if !(t.starts_with("op: String") || t.starts_with("operator: String")) {
        return false;
    }
    // Walk back to the enclosing variant/struct header.
    for (_, prior) in lines[..index].iter().rev().take(12) {
        let p = prior.trim();
        if p.starts_with("Binary {") || p.starts_with("Unary {") || p.starts_with("Compare {") {
            return true;
        }
        if p.ends_with('}') {
            break;
        }
    }
    false
}

/// Class 4 — a function taking an operator as `&str`/`&String`. A signature is
/// the contract: once one exists, every caller has a spelling to hand it.
fn str_operator_param(line: &str) -> bool {
    if is_comment(line) {
        return false;
    }
    // The parameter may be on its own line (a wrapped signature) or inline in a
    // one-line `fn f(op: &str) -> …`, so search the whole line rather than only
    // its start — `link_compare_op_valid(op: &str)` is the inline shape, and a
    // scanner that only reads line starts would never have seen it.
    for name in ["op", "operator"] {
        let mut from = 0usize;
        while let Some(at) = line[from..].find(name) {
            let at = from + at;
            from = at + name.len();
            // A whole identifier, not the tail of `binop` or the head of `opts`.
            let before_ok = line[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_');
            let rest = &line[at + name.len()..];
            if !before_ok {
                continue;
            }
            let Some(rest) = rest.strip_prefix(':') else {
                continue;
            };
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('&') else {
                continue;
            };
            // An optional lifetime: `&'a str`.
            let rest = rest.trim_start();
            let rest = if let Some(lt) = rest.strip_prefix('\'') {
                lt.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_')
                    .trim_start()
            } else {
                rest
            };
            if rest.starts_with("str") || rest.starts_with("String") {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

fn scan_tree() -> BTreeMap<&'static str, Vec<String>> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();

    for path in rs_files(&[manifest.join("src")]) {
        let rel = path
            .strip_prefix(manifest)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if is_excluded_from_scan(&rel) || is_test_file(&rel) || is_vocabulary_file(&rel) {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        let lines = test_free_lines(&src);
        let shaped = is_operator_shaped_non_operator(&rel);

        for (i, (n, line)) in lines.iter().enumerate() {
            if spelling_match_arms(line) {
                out.entry("spelling_match_arms")
                    .or_default()
                    .push(format!("{rel}:{n} — {}", line.trim()));
            }
            if spelling_compares(line) {
                out.entry("spelling_compares")
                    .or_default()
                    .push(format!("{rel}:{n} — {}", line.trim()));
            }
            if shaped {
                continue;
            }
            if node_string_operator(&lines, i) {
                out.entry("node_string_operators")
                    .or_default()
                    .push(format!("{rel}:{n} — {}", line.trim()));
            }
            if str_operator_param(line) {
                out.entry("str_operator_params")
                    .or_default()
                    .push(format!("{rel}:{n} — {}", line.trim()));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// The hard zero. No budget table: the operator vocabulary is closed, so there
/// is no legitimate remainder to enumerate — every site converts.
#[test]
fn no_operator_strings() {
    let live = scan_tree();
    assert!(
        live.is_empty(),
        "plan-112: an operator spelling reached a decision, or a tree node went \
         back to carrying one. `BinaryOp`/`UnaryOp` (src/operators.rs) are the \
         compiler's only operator representation after `BinaryOp::from_token`; \
         `name()` renders at the three sinks and `parse()` is for the `.mfp` \
         decode boundary alone. Offenders:\n{}",
        live.iter()
            .map(|(class, hits)| format!(
                "\n  {class} ({}):\n    {}",
                hits.len(),
                hits.join("\n    ")
            ))
            .collect::<Vec<_>>()
            .join("")
    );
}

/// A scanner that never fires is indistinguishable from a clean tree. Each one
/// is fed a line it must catch and a near-miss it must not.
#[test]
fn scanners_fire_on_their_own_needles() {
    assert!(spelling_match_arms(
        r#"            "MOD" => self.lower_mod(),"#
    ));
    assert!(spelling_match_arms(r#"        "+" | "-" => Some(a),"#));
    assert!(spelling_match_arms(
        r#"    if matches!(op, "AND" | "OR") {"#
    ));
    // Not an operator spelling: a type name, a mnemonic, a member name.
    assert!(!spelling_match_arms(
        r#"            "Money" => lower_money(),"#
    ));
    assert!(!spelling_match_arms(
        r#"            "fadd_d" => emit_fadd(),"#
    ));
    assert!(!spelling_match_arms(
        r#"            // "MOD" => used to be here"#
    ));
    // A spelling that is not in pattern position is not a decision.
    assert!(!spelling_match_arms(r#"        let text = "MOD";"#));

    assert!(spelling_compares(r#"        if op == "&" {"#));
    assert!(spelling_compares(r#"        if operator != "<>" {"#));
    assert!(!spelling_compares(r#"        if name == "Money" {"#));
    assert!(!spelling_compares(r#"        // if op == "&" { ... }"#));
    assert!(!spelling_compares(r#"        let rendered = op.name();"#));

    let carrier = vec![
        (1usize, "    Binary {"),
        (2, "        op: String,"),
        (3, "        left: Box<IrValue>,"),
    ];
    assert!(node_string_operator(&carrier, 1));
    let unrelated = vec![(1usize, "    Call {"), (2, "        op: String,")];
    assert!(!node_string_operator(&unrelated, 1));

    assert!(str_operator_param("        op: &str,"));
    assert!(str_operator_param("    fn f(operator: &str) -> bool {"));
    assert!(str_operator_param("        op: &'a str,"));
    assert!(!str_operator_param("        op: BinaryOp,"));
    assert!(!str_operator_param("        target: &str,"));
    assert!(!str_operator_param("        // op: &str, — was"));
}

/// The line filter must strip a mid-file `#[cfg(test)]` item and keep what
/// follows it — the trap `tests/no_type_strings.rs` documents.
#[test]
fn the_line_filter_strips_cfg_test_items_not_the_file_tail() {
    let src = "fn a() {}\n#[cfg(test)]\nfn t() {\n    let x = 1;\n}\nfn b() {}\n";
    let kept: Vec<&str> = test_free_lines(src).into_iter().map(|(_, l)| l).collect();
    // The whole item goes, closing brace included, and `fn b` — the file tail —
    // survives. That tail is the point: truncating at the first `#[cfg(test)]`
    // would have dropped it.
    assert_eq!(kept, vec!["fn a() {}", "fn b() {}"]);
    // A single-line `#[cfg(test)]` item suppresses only its own line.
    let field = "struct S {\n    a: u8,\n    #[cfg(test)]\n    probe: u8,\n    b: u8,\n}\n";
    let kept: Vec<&str> = test_free_lines(field).into_iter().map(|(_, l)| l).collect();
    assert_eq!(kept, vec!["struct S {", "    a: u8,", "    b: u8,", "}"]);
}

/// The vocabulary exemption is total — every needle class — so it is the widest
/// allowance in this gate, and the only thing keeping it honest is that it names
/// one file for a reason that is true of no other. This asserts both halves.
#[test]
fn the_vocabulary_file_is_exactly_one() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exempt: Vec<String> = rs_files(&[manifest.join("src")])
        .into_iter()
        .map(|p| {
            p.strip_prefix(manifest)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .filter(|rel| is_vocabulary_file(rel))
        .collect();
    assert_eq!(
        exempt,
        vec!["src/operators.rs".to_string()],
        "the vocabulary-file exemption must cover exactly one file"
    );
    let src =
        std::fs::read_to_string(manifest.join("src/operators.rs")).expect("read operators.rs");
    assert!(
        src.contains("pub(crate) fn name(self) -> &'static str"),
        "src/operators.rs must be the file that DEFINES `name` — that is the \
         whole justification for exempting it"
    );
    assert!(
        src.contains("pub(crate) fn parse(text: &str) -> Option<Self>"),
        "…and `parse`"
    );
}

/// `SPELLINGS` is hand-written, so something has to prove it still matches the
/// enum it guards. A spelling that drifts out of this list silently stops being
/// scanned for.
#[test]
fn vocabulary_matches_the_enum() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src =
        std::fs::read_to_string(manifest.join("src/operators.rs")).expect("read operators.rs");
    // Every arm of `name()` is `Variant => "spelling",`; collect the spellings
    // rendered next to a `BinaryOp::`/`UnaryOp::` variant.
    let mut rendered: Vec<String> = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if !(t.starts_with("BinaryOp::") || t.starts_with("UnaryOp::")) {
            continue;
        }
        let Some(arrow) = t.find("=> \"") else {
            continue;
        };
        let rest = &t[arrow + 4..];
        let Some(end) = rest.find('"') else { continue };
        rendered.push(rest[..end].to_string());
    }
    rendered.sort();
    rendered.dedup();
    let mut expected: Vec<String> = SPELLINGS.iter().map(|s| (*s).to_string()).collect();
    expected.sort();
    expected.dedup();
    assert_eq!(
        rendered, expected,
        "SPELLINGS has drifted from `name()` in src/operators.rs — update this \
         list, or the scanners stop seeing the spelling that moved"
    );
    assert_eq!(
        SPELLINGS.len(),
        19,
        "17 BinaryOp + 3 UnaryOp variants render 19 DISTINCT spellings — `-` is \
         shared by `Subtract` and `Negate`, which is why the scanners key on the \
         spelling and the two enums keep the arities apart"
    );
}

/// The operator-shaped exemption list is CLOSED, and adding to it requires
/// editing this test.
///
/// It is the gate's one allowance for classes 3 and 4, and the way an allowance
/// rots is by growing quietly — so the exact membership is asserted here rather
/// than merely justified in a string. A tenth entry is not forbidden. It is
/// required to be *deliberate*: whoever adds one has to come here, read why the
/// list is closed, and say what makes their `op` something other than a
/// language operator.
#[test]
fn operator_shaped_exemptions_are_closed() {
    let listed: Vec<&str> = OPERATOR_SHAPED_NON_OPERATORS
        .iter()
        .map(|(f, _)| *f)
        .collect();
    assert_eq!(
        listed,
        vec![
            // the one genuine wire boundary
            "src/ir/link.rs",
            // `CodeOp` mnemonics — a different, already-enumerated vocabulary
            "src/arch/ops.rs",
            "src/arch/riscv64/v128.rs",
            "src/target/shared/abi.rs",
            "src/codegen/engine/builder/code_impl.rs",
            // builtin-member / operation names that happen to be called `op`
            "src/codegen/builtins/audio/gen_macos_shared.rs",
            "src/codegen/builtins/collections/gen_mutate.rs",
            "src/codegen/builtins/vector/builder_vector_inline.rs",
            // `src/codegen/builtins/process/gen_windows.rs` was here for
            // `unimplemented_on_windows(op)`. plan-119 implemented the last two
            // Windows `process` members, that function has no callers left, and
            // deleting it took the file's only `op` site with it.
        ],
        "the operator-shaped exemption list is closed. Adding an entry means \
         claiming the identifier is not a language operator — not that the site \
         has not been converted yet."
    );
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (file, why) in OPERATOR_SHAPED_NON_OPERATORS {
        assert!(
            manifest.join(file).is_file(),
            "OPERATOR_SHAPED_NON_OPERATORS names a missing file: {file}"
        );
        assert!(
            !why.is_empty(),
            "OPERATOR_SHAPED_NON_OPERATORS entry {file} has no justification"
        );
    }
}

/// An exemption above reality is an allowance a regression can hide inside —
/// the same second direction `tests/no_type_strings.rs` asserts on its budget
/// table. Every listed file must still *need* its exemption: if the site that
/// justified it is gone, the entry must go with it in the same commit.
#[test]
fn every_exemption_is_still_load_bearing() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut stale: Vec<&str> = Vec::new();
    for (file, _) in OPERATOR_SHAPED_NON_OPERATORS {
        let src = std::fs::read_to_string(manifest.join(file)).expect("read exempt file");
        let lines = test_free_lines(&src);
        let needed = lines
            .iter()
            .enumerate()
            .any(|(i, (_, line))| str_operator_param(line) || node_string_operator(&lines, i));
        if !needed {
            stale.push(file);
        }
    }
    assert!(
        stale.is_empty(),
        "these files no longer contain an `op`/`operator` `String`/`&str` site, \
         so their exemption is stale — delete the entry: {stale:?}"
    );
}

/// The exemption list buys classes 3 and 4 only. If a listed file ever starts
/// *deciding* on a spelling, the gate must still catch it — this asserts the
/// scan does not skip those files wholesale.
#[test]
fn exempt_files_are_still_scanned_for_decisions() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    // The walker must actually reach each one — a path predicate that says
    // "not excluded" proves nothing if the file is never enumerated.
    let walked: Vec<String> = rs_files(&[manifest.join("src")])
        .into_iter()
        .map(|p| {
            p.strip_prefix(manifest)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    for (file, _) in OPERATOR_SHAPED_NON_OPERATORS {
        let rel = (*file).to_string();
        assert!(
            walked.contains(&rel),
            "{file} is never enumerated by the scan walker"
        );
        assert!(
            !is_excluded_from_scan(&rel) && !is_test_file(&rel) && !is_vocabulary_file(&rel),
            "{file} is exempt from classes 3-4 only; it must still be scanned \
             for spelling decisions"
        );
    }
}
