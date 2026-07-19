# bug-343: AST / lexer / fmt cleanup cluster — pipeline desugaring lives in the JSON dumper, the shared JSON escaper lives in the binary entrypoint, and `fmt.rs` re-implements a lexer closure verbatim

Last updated: 2026-07-18
Effort: medium (1h–2h per item; large for the cluster)
Severity: LOW
Class: Other (cleanup)

Status: Open
Regression Test: existing acceptance goldens (no new expected output); new unit
tests only where an item collapses two implementations into one (B1, B3).

A cluster of misplacement, duplication, and stale-documentation residue across
`src/ast/`, `src/lexer.rs`, `src/fmt.rs`, and the loose top-level modules beside
them. Every item below was re-verified against the current worktree; several
leads from the original review **did not hold** and have been corrected or
dropped in place (see the per-item notes and the "Leads that did not verify"
section).

The theme is that the AST layer's *responsibilities* have drifted away from its
*file names*: a 188-line semantic rewrite (pipeline `|>` placeholder
substitution) is implemented inside the `-ast` JSON dumper and re-imported back
out of it to reach the parser; the JSON string escaper that ~15 modules depend
on is defined in `src/main.rs`; a 733-line AST pass with one caller sits at the
crate root; and `src/escape.rs` is resource-escape analysis sitting next to the
lexer, which is where escape *sequences* actually live.

The single correct outcome of a fix is that each module's contents match its
name and doc, each duplicated helper has one definition, and the generated
artifacts are **byte-identical** to today's committed goldens.

References:

- Found during the tree-wide cleanup review (Agent 14 — AST/lexer/fmt), base
  `25c38ba1`.
- `src/docs/spec/tooling/05_fmt.md` — the `mfb fmt` contract (its own drift is
  bug-338, not this bug).
- `src/docs/spec/language/19_grammar.md` — the grammar (drift is bug-338).
- `planning/old-plans/plan-link-update.md`, `planning/old-moved-to-src-spec/mfbasic.md`
  — the two retired documents that D2/D3's stale citations point at.

### Covered by sibling bugs — NOT in scope here

- `BindState.resource_slot` parsed but silently discarded, and `AbiSpec.line`
  carrying an `#[allow(dead_code)]` whose comment claims it *is* used →
  **bug-326** (repo-wide dead-code sweep).
- `src/ast/items.rs` (1,621 lines) splitting into three unrelated parsers
  (items / LINK / DOC) → **bug-327-T1-8**.
- `19_grammar.md` omitting constructs the parser accepts; the deleted `"return"`
  ABI slot name; the grammar being over-strict where the parser is permissive;
  `05_fmt.md` contradicting itself and `fmt.rs` on whitespace; `mfb fmt`
  flattening `TESTING` blocks; the `05_fmt.md` `--indent`/check-mode gaps; and
  the duplicated section number in `02_lexical-structure.md` → **bug-338**
  (grammar / fmt spec-drift cluster). Note: `bugs/bug-338-*.md` was not yet
  present in `bugs/` when this document was written — if it is renumbered, fix
  these cross-references.

## Current State

Measured line counts (worktree `cleanup-review`, base `25c38ba1`):

| File | Lines |
| --- | --- |
| `src/ast/tests.rs` | 2,647 |
| `src/lexer.rs` | 1,754 |
| `src/ast/serialize.rs` | 1,725 |
| `src/ast/items.rs` | 1,621 |
| `src/fmt.rs` | 959 |
| `src/main.rs` | 880 |
| `src/ast/expr.rs` | 873 |
| `src/ast/stmt.rs` | 786 |
| `src/ast/types.rs` | 749 |
| `src/scope_privates.rs` | 733 |
| `src/escape.rs` | 560 |
| `src/unicode_runtime_tables.rs` | 523 |
| `src/unicode_backend.rs` | 66 |
| `src/ast/mod.rs` | 36 |

`src/lib.rs` is **0 bytes** — `src/main.rs` really is the sole crate root, which
is why A6 below matters more than it looks.

Nothing here is a runtime failure. Each item's measured evidence is recorded
with it.

## Items

### Theme A — code living in the wrong file

#### A1 — pipeline `|>` placeholder desugaring (188 lines of expression-tree rewriting) lives in the `-ast` JSON serializer, and is re-imported out of it to reach the parser

- `src/ast/serialize.rs` is the `-ast` JSON dumper: `AstProject::to_json` at
  `:4`, reached via `BuildOutput::Ast` / the `-ast` flag at
  `src/cli/build.rs:134`.
- The desugaring occupies `src/ast/serialize.rs:1538` through `:1725` (EOF) —
  **188 lines**, six functions: `contains_placeholder` `:1538`,
  `constructor_arg_contains_placeholder` `:1569`,
  `call_arg_contains_placeholder` `:1576`, `substitute_placeholder` `:1583`,
  `substitute_placeholder_constructor_arg` `:1700`,
  `substitute_placeholder_call_arg` `:1716`. (The review note said ~193 lines at
  `:1538-1730`; the file ends at 1725.)
- It is consumed by the **parser**: `src/ast/expr.rs:56` and `:64`, inside
  `parse_pipeline`.
- The route between them is the tell: `src/ast/mod.rs:31` is
  `use serialize::{contains_placeholder, substitute_placeholder};` — a private
  `use` in the module root, which `expr.rs` (which opens with `use super::*;`)
  then picks back up through the parent. The parser reaches into the JSON dumper
  via a re-import in a third file.
- Fix: new `src/ast/pipeline.rs` owning the six functions; `expr.rs` imports it
  directly; delete the `mod.rs:31` laundering.

#### A2 — `normalize_ws`, a generic whitespace helper, is exported from the DOC-header parser and used for type-name normalization

- `pub fn normalize_ws(text: &str) -> String` at `src/ast/items.rs:1576`, body
  `text.split_whitespace().collect::<Vec<_>>().join(" ")`; re-exported
  `pub use items::normalize_ws;` at `src/ast/mod.rs:22`.
- Complete consumer list (searched, not recalled): in-module DOC-param splitting
  at `src/ast/items.rs:1564` and `:1570`; and three **external** consumers that
  all use it for *parameter-type* normalization, nothing to do with DOC headers —
  `src/doc.rs:362`/`:367`, `src/ir/lower.rs:155`/`:160`,
  `src/resolver/mod.rs:120`/`:121`.
- Fix: move it to a neutral home (it is one line; it belongs wherever B1's
  merged normalizer lands) and re-point the five external call sites.

#### A3 — `src/escape.rs` is resource-escape analysis, not escape sequences, and sits beside the file that does own escape sequences

- `src/escape.rs:1`: `//! Resource escape analysis (mfbasic.md §15.6).` The
  module doc goes on to describe `RES` binding ownership floating up scopes and
  `ResOwner::Local`. There is zero escape-*sequence* handling in the file.
- The lexer owns escape sequences: `src/lexer.rs:342-370` (string escapes),
  `:455-497` (scalar-literal escapes), `:567` `fn lex_unicode_escape` for
  `\u{HEX}`.
- The file's consumers all live under `src/ir/` and `src/target/`.
- Fix: rename to `resource_escape` and move it under `src/ir/`, next to its
  consumers. (Its `mfbasic.md` citation is D2.)

#### A4 — `unicode_backend.rs` + `unicode_runtime_tables.rs` are the only prefix-paired loose top-level files

- Full inventory of loose top-level `.rs` files in `src/`: `coverage.rs`,
  `doc.rs`, `escape.rs`, `fmt.rs`, `internal_name.rs`, `lexer.rs`, `lib.rs`,
  `main.rs`, `numeric.rs`, `scope_privates.rs`, `target.rs`, `terminal_safe.rs`,
  `testing.rs`, `testutil.rs`, `unicode_backend.rs`, `unicode_runtime_tables.rs`.
- `unicode_*` is the only `_`-delimited prefix pair — the claim holds. Two
  weaker neighbours worth noting while the file is open: `testing.rs` /
  `testutil.rs` share a `test` prefix, and `target.rs` sits beside a `src/target/`
  directory.
- `src/unicode_runtime_tables.rs:1` carries a blanket `#![allow(dead_code)]`.
  What it hides, measured:
  - `property_for_codepoint` (`:100-105`) — **correction**: not truly dead. It
    has five callers, all in the module's own unit tests at `:495-499`. Dead in
    production, live in tests; outside tests its only reference is spec prose at
    `src/docs/spec/unicode/01_tables-and-algorithms.md:57`.
  - `category_value` (`:372-407`, 36 lines) — **statically reachable but can
    never fire**, confirmed. Its only caller is `parse_value` at `:315`
    (`_ if value.starts_with("UTF8PROC_CATEGORY_")`), and `parse_properties`
    (`:245-287`) reads fields 1,3,4,5,6,7,9,10,11,13,14,15,19,20 — **never field
    0**, the category. Nothing else feeds a `UTF8PROC_CATEGORY_*` string in.
- Fix: `src/unicode/{mod,backend,runtime_tables}.rs`; drop the blanket allow in
  favour of a targeted `#[cfg(test)]`-aware attribute on
  `property_for_codepoint`; delete `category_value` or wire field 0 through
  `parse_properties` — decide which (see Open Decisions). The blanket-allow half
  overlaps **bug-326**; coordinate.

#### A5 — `src/scope_privates.rs`: a 733-line AST pass with exactly one caller, at the crate root

- 733 lines. Sole consumer: `src/cli/build.rs`.
- It is the third hand-written AST walker in the tree, alongside `src/escape.rs`
  (A3) and `serialize.rs`'s placeholder substitution (A1) — the same traversal
  written three times under three file names, none of which says "AST pass".
- Fix: move under `src/ast/` (or `src/resolver/`) as a named pass. Do **not**
  attempt to unify the three walkers in this bug — that is bug-328's shape of
  problem and needs its own design.

#### A6 — the JSON string escaper that ~15 modules depend on is defined in `src/main.rs`

- `pub(crate) fn json_string` at `src/main.rs:526`, wrapping
  `tinyjson::JsonValue::stringify`.
- **Correction to the review note**, and it makes the item worse rather than
  better: this is not a two-consumer helper shared by the AST and IR emitters.
  Callers span ~15 modules — `src/coverage.rs:32`, `src/target/shared/plan/`,
  `src/target/shared/code/`, `src/target/shared/nir/json.rs` (100+ uses),
  `src/os/macos/object.rs`, `src/os/linux/object.rs`,
  `src/manifest/package.rs`, `src/cli/{resolve,init,build}.rs`,
  `src/ast/mod.rs:1`, `src/ir/mod.rs:7`.
- With `src/lib.rs` at 0 bytes, every one of those modules reaches into the
  binary entrypoint for a string utility.
- Fix: new `src/json.rs` owning `json_string`. This converges with the
  cross-cutting finding that six byte-identical `join_json`/`join_indented`
  helpers sit behind six near-identical `To*Json` traits (B2) — land both into
  the same new module.

### Theme B — duplication

#### B1 — DOC-overload param-type normalization: two operations, five names, plus a sixth that inlines both

- `src/ir/lower.rs:152` `fn function_param_types(function: &crate::ast::Function)
  -> Vec<String>` and `src/doc.rs:358` `fn param_types(function: &Function) ->
  Vec<String>` — identical bodies modulo the closure binder (`param` vs `p`) and
  the type path.
- `src/ir/lower.rs:159` `fn normalize_types(types: &[String]) -> Vec<String>` and
  `src/doc.rs:366` `fn normalize(types: &[String]) -> Vec<String>` — bodies
  **character-identical**:
  `types.iter().map(|t| crate::ast::normalize_ws(t)).collect()`.
- **Correction**: `src/resolver/mod.rs:115` is *not* a third copy. It is
  `fn overload_types_match(function: &Function, wanted: &[String]) -> bool`,
  which inlines both operations into a zip/compare at `:119-122` — different
  signature, different return type. So the honest count is 4 copies of 2
  operations under 5 names, plus a 6th name doing both inline.
- Fix: one `param_types` + one `normalize_types` (next to A2's `normalize_ws`);
  `overload_types_match` becomes a two-line composition of them.

#### B2 — `ToAstJson` and `ToIrJson` are the same hand-rolled emitter written twice

- `trait ToAstJson` at `src/ast/serialize.rs:44-46` with join helper
  `fn join_indented<T: ToAstJson>` at `:1514-1520`.
- `trait ToIrJson` at `src/ir/json.rs:56-58` with `fn join_json<T: ToIrJson>` at
  `:893-899`.
- The two join helpers are **character-identical** apart from the name and the
  trait bound.
- The wider cluster (six such traits / six byte-identical join helpers across
  `nir/json.rs`, `plan/json.rs`, `code/serialization_utils.rs`, `ir/json.rs`,
  `os/linux/object.rs`, `os/macos/object.rs`) is a cross-cutting finding; this
  bug owns only the `ast`↔`ir` pair and the `src/json.rs` home that A6 creates.
- Fix: one `ToJson` trait + one generic `join_json` in the new `src/json.rs`.

#### B3 — `fmt.rs` re-implements a lexer closure verbatim, in a file the lexer already exports a helper *for*

- The verbatim clone is the `is_end` closure — `src/fmt.rs:526-530` vs
  `src/lexer.rs:980-984`, char-for-char identical:
  ```rust
  let is_end = |kw: &str| {
      words.len() == 2
          && words[0].eq_ignore_ascii_case("END")
          && words[1].eq_ignore_ascii_case(kw)
  };
  ```
- The surrounding `in_example` / `EXAMPLE` / `END DOC` state machine is
  parallel-but-not-identical (`src/fmt.rs:524-546` vs `src/lexer.rs:979-992`),
  and `src/fmt.rs:490` `fn is_doc_start` is a *reimplementation* of the lexer's
  DOC-line attribute check rather than a copy.
- **Correction**: the review note claimed "+4 more pairs" of verbatim
  duplication. That could not be substantiated — there is one verbatim pair and
  a broader parallel structure. Sized honestly, this is ~5 duplicated lines plus
  a duplicated ~20-line state machine, not ~25 verbatim lines.
- The precedent for sharing already exists and is explicit:
  `src/lexer.rs:1096-1098` documents `lookup_keyword` as
  `/// Exposed for source tools (such as \`mfb fmt\`) that re-tokenize raw text
  without building a full lexer.`, and `src/fmt.rs:232` uses it. The DOC
  recognizers were simply never given the same treatment.
- The consequence of the fork is already visible: `DOC_UNTERMINATED` exists only
  at `src/lexer.rs:1002` (plus `src/rules/table.rs:1205` and two spec files);
  `src/fmt.rs:554-557` silently flushes instead of diagnosing.
- Fix: export the DOC recognizers from `lexer.rs` beside `lookup_keyword`, with
  the same "for source tools" doc; `fmt.rs` consumes them. Whether `fmt` should
  *emit* `DOC_UNTERMINATED` is a behavior change and is **out of scope** here
  (see Non-goals).

### Theme C — file and test organization

#### C1 — `src/ast/tests.rs`: four modules each get three or four separate, non-adjacent section banners

- 2,647 lines, exactly **150** `#[test]` functions, 19 banners.
- Non-adjacent banner groups (banner rule at line N, title at N+1; the review
  note's numbers were off by one and undercounted):
  - expr — `:187`, `:1921`, `:2171` (3)
  - manifest — `:697`, `:2029`, `:2110`, `:2459` (**4**)
  - serialize — `:1110`, `:1341`, `:1585` (3), plus `:1075` "plan-12 coverage:
    source-driven parse + serialize tests."
  - items — `:1729`, `:1810`, `:2234` (3)
- This is the append-only-growth signature: each new batch of tests got a fresh
  banner at EOF instead of joining the existing section.
- **Correction — one lead FAILED**: the claim that `stmt.rs` (786 lines) gets no
  banner is **wrong**. `src/ast/tests.rs:2327` is
  "stmt.rs — inline-trap propagation, match/for/do edge and error paths."
- Fix: one banner per subject, sections contiguous. Pure reordering of test
  functions; no assertion changes.

#### C2 — `fmt.rs` files its LINK helpers under the DOC banner and splits the DOC group in half

- `src/fmt.rs` has **3** section banners (not the 6 the review note listed):
  `:118` `// --- Per-line scanning ---`, `:303` `// --- Block structure ---`,
  `:486` `// --- DOC blocks ---`.
- The substance holds and is worse than stated: `format_link_block` (`:585`) and
  `is_link_start` (`:633`) sit under the **DOC blocks** banner with no banner of
  their own, and they *split* the DOC group — `doc_header` (`:644`) lands after
  them, orphaned from `is_doc_start` (`:490`), `format_doc_block` (`:506`), and
  `flush_example` (`:563`).
- Fix: add a `// --- LINK blocks ---` banner; move `doc_header` back up with the
  rest of the DOC group.

### Theme D — stale documentation and dead code

#### D1 — `LinkFunction.result` is documented as the `RESULT` clause that plan-50-H deleted

- `src/ast/types.rs:337-338`:
  ```rust
  /// `RESULT <expr>` value mapping, if any (plan-link-update.md §5b).
  pub result: Option<Expression>,
  ```
- It is parsed at `src/ast/items.rs:860` under `Keyword::Return`, and the
  adjacent comment at `:856-859` says outright: "plan-50-H: `RETURN <expr>` is
  the ONE result clause… the computed case that used to spell RESULT."
- So the field name *and* its doc comment both describe syntax that no longer
  exists, while the code beside it documents the correction.
- Fix: rename the field to `return_value` (or document the historical name
  explicitly) and rewrite the doc to cite the spec, not the retired plan.

#### D2 — nine comments cite `mfbasic.md`, a spec retired to `planning/old-moved-to-src-spec/`

- Exact count: **9** references in `src/`. Retired location confirmed to exist:
  `planning/old-moved-to-src-spec/mfbasic.md`.
- **Correction to the site list**: `src/fmt.rs:11` does *not* cite `mfbasic.md`
  — it says "§2 of the language spec" and is fine. The nine real sites are
  `src/escape.rs:1`, `src/ir/link.rs:439`,
  `src/target/shared/code/mod.rs:686`,
  `src/target/shared/code/link_thunk.rs:977` and `:1140`,
  `src/cli/fmt.rs:60`, `src/ast/types.rs:340` and `:347`,
  `src/syntaxcheck/mod.rs:991`.
- Fix: re-point each at the corresponding `src/docs/spec/` section by topic, not
  by line number.

#### D3 — `plan-link-update.md` citations point at a design superseded by plan-50; the tree has 62 of them

- The 14 cited sites are exactly right: `src/ast/types.rs:61, 64, 66, 242, 252,
  258, 269, 310, 327, 337, 363, 419` (12) and `src/ast/items.rs:492, 1194` (2).
- **Correction on scope**: crate-wide there are **62** `plan-link-update.md`
  citations across ~25 files (`src/ir/`, `src/syntaxcheck/`, `src/resolver/`,
  `src/binary_repr/`, `src/target/`, `src/audit/`, `src/monomorph/`). The doc was
  moved, not deleted: `planning/old-plans/plan-link-update.md`.
- Two generations of design docs are interleaved inside one struct, and at least
  `src/ast/types.rs:337` is actively wrong (that is D1).
- Fix: cite the **spec topic**, not the plan. Scope decision required — this bug
  can own the 14 AST sites, or all 62 (see Open Decisions).

#### D4 — the native-FUNC clause error message omits `BIND STATE`, which is accepted

- `src/ast/items.rs:894-896`, verbatim:
  > `"A native FUNC body may only contain SYMBOL, ABI, CONST, SUCCESS_ON,
  > ERROR_ON, RETURN, BIND IN, or FREE clauses."`
- Actually accepted by the clause loop: SYMBOL, ABI, CONST, SUCCESS_ON, ERROR_ON,
  RETURN (`:860`), **BIND STATE** (`:868-882`), BIND IN (`:881`), FREE (`:887`).
- `BIND STATE` is parsed as a distinct single-line clause with its own
  duplicate-detection diagnostic at `:873-879` — it is unambiguously supported,
  and the error text tells a user it does not exist.
- This is the one user-facing item in this bug. Fix: add `BIND STATE` to the
  message. The message text is a diagnostic string, so check for goldens
  asserting it before editing.

#### D5 — `fmt.rs` dead `PartialEq` derive on `Sig`

- `#[derive(Clone, Copy, PartialEq)] enum Sig` at `src/fmt.rs:122-123`. No `==`
  or `!=` on `Sig` anywhere; every discrimination goes through `matches!`
  (`:71`, `:346`, `:352`, `:353`, `:437`). The derive is unused.
- The contrast confirms it is not a house style:
  `#[derive(Clone, Copy, PartialEq)] enum Block` at `:305-306` **does** use
  `PartialEq`, at `:460` and `:468` (`stack.last() == Some(&Block::Case)`).
- **Correction — one lead FAILED**: the companion claim that the `'\r'` arm at
  `src/fmt.rs:161` is unreachable after `strip_cr` is **wrong**. `strip_cr`
  (`:110-112`) is `line.strip_suffix('\r').unwrap_or(line)` — it removes only a
  *trailing* CR. An interior CR (an old-Mac line ending, or a stray CR mid-line)
  survives into `scan_line` (`:141`, called at `:70`). Only the trailing-CR case
  is unreachable; the arm stays.
- Fix: drop `PartialEq` from `Sig`. Do not touch `:161`.

#### D6 — `mfb fmt` has no man page

- `src/docs/man/` contains `builtins/`, `errors/`, `flow/`, `lambda/`, `link/`,
  `mod.rs`, `tour/`, `types/`, `unicode/`. There is no `tooling/` directory and
  no fmt page.
- `mfb fmt` is documented only as spec prose at
  `src/docs/spec/tooling/05_fmt.md`.
- Fix: add `src/docs/man/tooling/fmt.md`. Note the man-corpus guard tests are
  extension-sensitive (a separate cross-cutting finding) — confirm a new page is
  actually picked up rather than silently skipped.

#### D7 — three names for two indent concepts, and two names for one stack op, in `fmt.rs`

- Indent **level**: `level` (param of `indent_str` `:114`, param of
  `flush_example` `:563`), `base` (`:82`, `:449`, `:510`, `:589`), plus local
  aliases `body` (`:520`) and `depth` (`:599`).
- Indent **width**: `indent_width` (public param `:22`; passed at `:50`, `:60`,
  `:85`) vs `width` (`:114`, `:511`, `:563`, `:590`).
- Block-close ops: `Op::End(kw)` (`:406`, handled `:467`) vs `Op::Pop` (`:407`
  for `NEXT`/`WEND`/`LOOP`, handled `:474`) — two names for popping the block
  stack.
- Fix: settle on `level` and `indent_width` throughout; either rename `Op::Pop`
  to `Op::EndImplicit` or document why the two are distinct.

## Goal

- Pipeline placeholder desugaring lives in its own AST module, imported directly
  by the parser, with no re-import through `src/ast/mod.rs`.
- `json_string` and one shared `ToJson`/`join_json` live in `src/json.rs`, not in
  `src/main.rs` and not duplicated per emitter.
- `param_types` / `normalize_types` / `normalize_ws` each have exactly one
  definition, in a module whose name describes them.
- `src/escape.rs`, `src/scope_privates.rs`, and the `unicode_*` pair sit in
  directories matching their subject.
- `fmt.rs` consumes the lexer's DOC recognizers instead of re-implementing them.
- Every doc comment, error message, and citation in the touched files describes
  syntax and documents that currently exist.
- **All artifact goldens are byte-identical before and after** (with the single
  audited exception of D4's diagnostic text, if any golden asserts it).

### Non-goals (must NOT change)

- Any emitted artifact: `-ast`, `-ir`, `-br`, `-nir`, `-nplan`, `-nobj`,
  `-ncode`, `-mir`, or any linked binary. This bug is output-preserving by
  construction — A1 and B3 move parser- and formatter-adjacent code and must be
  proven not to shift a single byte.
- The set of programs accepted or rejected, and the formatted output of
  `mfb fmt` on any input. B3 shares recognizers; it must not change what `fmt`
  produces or what it accepts.
- Adding `DOC_UNTERMINATED` to `mfb fmt`. That is a real behavior change with a
  spec question attached; it belongs in bug-338's territory, not here. B3 shares
  the recognizer and stops.
- The `'\r'` handling at `src/fmt.rs:161` — verified reachable; deleting it is
  the tempting wrong fix and is forbidden.
- Any diagnostic rule id. D4 changes one message's *text*; it must not renumber,
  add, or remove a rule.
- The grammar and `05_fmt.md` — owned by bug-338.

## Blast Radius

Searched, not recalled. Every site below was confirmed by reading it.

- `src/ast/serialize.rs:1538-1725`, `src/ast/expr.rs:56`/`:64`,
  `src/ast/mod.rs:31` (A1) — fixed by this bug.
- `src/ast/items.rs:1576`, `src/ast/mod.rs:22`, and the five consumers
  `src/ast/items.rs:1564`/`:1570`, `src/doc.rs:362`/`:367`,
  `src/ir/lower.rs:155`/`:160`, `src/resolver/mod.rs:120`/`:121` (A2) — fixed by
  this bug.
- `src/escape.rs` (A3), `src/scope_privates.rs` + `src/cli/build.rs` (A5),
  `src/unicode_backend.rs` + `src/unicode_runtime_tables.rs` (A4) — fixed by this
  bug; A4's blanket-allow half overlaps **bug-326**, coordinate ownership.
- `src/main.rs:526` (A6) — fixed by this bug, but the **~15 consumer modules**
  listed in A6 all need their import re-pointed. This is the widest-touching item
  in the cluster and should be one mechanical commit of its own.
- `src/ir/lower.rs:152`/`:159`, `src/doc.rs:358`/`:366`,
  `src/resolver/mod.rs:115-122` (B1) — fixed by this bug. These sites are also
  touched by **bug-342-A1** (`ir/lower.rs` dispatch chain); different line ranges,
  no conflict, but land them in a known order.
- `src/ast/serialize.rs:44-46`/`:1514-1520`, `src/ir/json.rs:56-58`/`:893-899`
  (B2) — fixed by this bug. The four other `join_json` copies
  (`nir/json.rs`, `plan/json.rs`, `code/serialization_utils.rs`,
  `os/{linux,macos}/object.rs`) are **latent, same duplication, out of scope** —
  they belong to the cross-cutting serialization finding and would triple this
  bug's blast radius.
- `src/fmt.rs:526-530` ↔ `src/lexer.rs:980-984`, `src/lexer.rs:1096-1098`,
  `src/fmt.rs:232`, `src/fmt.rs:490`, `:524-546` (B3) — fixed by this bug.
- `src/lexer.rs:1002`, `src/rules/table.rs:1205`, `src/fmt.rs:554-557`
  (`DOC_UNTERMINATED`) — **unaffected**: recorded as the consequence of B3's
  fork, explicitly not changed.
- `src/ast/tests.rs` (C1), `src/fmt.rs:118`/`:303`/`:486`/`:585`/`:633`/`:644`
  (C2) — fixed by this bug; reordering only.
- `src/ast/types.rs:337-338`, `src/ast/items.rs:856-860` (D1);
  the 9 `mfbasic.md` sites (D2); the 14 AST `plan-link-update.md` sites (D3);
  `src/ast/items.rs:894-896` (D4); `src/fmt.rs:122-123` (D5);
  `src/docs/man/` (D6); `src/fmt.rs` naming (D7) — fixed by this bug.
- The other **48** `plan-link-update.md` citations outside `src/ast/` — latent,
  same staleness, out of scope unless the Open Decision says otherwise.
- `src/ast/items.rs` three-parser split — out of scope, owned by **bug-327-T1-8**;
  A1/A2/D1/D4 all touch that file, so sequence against it (see Fix Design).
- `BindState.resource_slot`, `AbiSpec.line` — out of scope, owned by **bug-326**.
- Grammar / `05_fmt.md` drift — out of scope, owned by **bug-338**.

## Fix Design

As with bug-342, the risk is not the edits — it is a "pure cleanup" silently
shifting an artifact. A1 (parser-adjacent) and B3 (formatter-adjacent) are the
two items where that could actually happen, so the gate runs on every commit.

Every commit in this bug must pass, on an unmodified fixture tree:

```
cargo build --release
scripts/artifact-gate.sh target/release/mfb     # execution-free, ~5 min
```

`scripts/artifact-gate.sh` regenerates the deterministic artifact dumps for
every fixture carrying the matching golden and `cmp`s each against the committed
file (`scripts/artifact-gate.sh:29-38`). Any non-zero `diffs` means the item is
not output-preserving and must be re-examined, **not** re-goldened. No item in
this bug may run `scripts/sync-goldens.sh` — with the single audited exception
of D4, and only if an existing golden asserts the clause-error text, in which
case the diff must be exactly that one string. Before merge, the full harness
(`scripts/test-accept.sh <mfb-exe> <actual-dir>`) must be green, since the
artifact gate does not link or run.

`mfb fmt` needs its own proof beyond the artifact gate, because the gate does
not exercise it: after B3, C2, D5, and D7, run `mfb fmt --check` over every
committed `.mfb` source in `tests/` and confirm the result is unchanged from
before the commit. (Note: `mfb fmt --check` is *already* red on `TESTING`-block
sources — that is bug-338's live defect. Capture the before-state failure list
and require it to be byte-identical after, rather than requiring green.)

Ordering against sibling bugs:

- Land **D5, D7, C2, D1, D2, D4** first — small, local, gate-cheap.
- Land **A6** as one mechanical commit (it touches ~15 modules and nothing else).
- Land **A1** and **B3** with the gate plus the `mfb fmt --check` before/after
  diff; these are the two that can move output.
- Sequence **A1/A2/D1/D4** against **bug-327-T1-8** (the `ast/items.rs` split) —
  whichever lands second rebases onto the other. Recommend this bug first: its
  edits are small and the split is large.
- **A4**'s blanket-allow removal overlaps **bug-326**; pick one owner.

Rejected alternatives, so they are not re-litigated:

- *Leave the pipeline desugaring in `serialize.rs` and just re-export it
  properly.* Rejected: the import laundering is a symptom. A semantic AST rewrite
  in a dump-only module will keep attracting dump-only assumptions.
- *Promote `json_string` by adding a `src/lib.rs`.* Rejected as part of this bug:
  turning the crate into a lib+bin is a structural change with its own test and
  build implications. A6 only moves the function to `src/json.rs`.
- *Have `fmt` adopt `DOC_UNTERMINATED` while sharing the recognizer.* Rejected:
  behavior change, and the spec question is bug-338's.
- *Delete `src/fmt.rs:161`'s `'\r'` arm as dead.* Rejected on evidence — verified
  reachable for interior CRs.
- *Fix D3 by deleting the citations.* Rejected: they carry real provenance;
  re-point them at the spec instead.

## Phases

### Phase 1 — gate + audit (no behavior change)

- [ ] Confirm `scripts/artifact-gate.sh target/release/mfb` reports `diffs=0` on
      a clean tree; record `checked`/`ran` as the baseline.
- [ ] Capture the current `mfb fmt --check` result over all committed `.mfb`
      sources (expected: already-failing on `TESTING` blocks per bug-338) as the
      before-state.
- [ ] Grep for goldens asserting the D4 clause-error text; record the verdict
      here.
- [ ] Land D5 (drop `PartialEq` on `Sig`) as the gate smoke test.

Acceptance: gate at `diffs=0` after D5; the fmt before-state and the D4 golden
audit are recorded in this file.
Commit: —

### Phase 2 — placement and duplication

- [ ] A6: `src/json.rs` owning `json_string`; re-point the ~15 consumers. One
      commit, no other change.
- [ ] B2: one `ToJson` + one `join_json` in `src/json.rs`; `ast`/`ir` adopt it.
- [ ] A1: `src/ast/pipeline.rs`; `expr.rs` imports directly; delete
      `src/ast/mod.rs:31`.
- [ ] A2 + B1: one `param_types`, one `normalize_types`, one `normalize_ws`, in
      one home; re-point all consumers.
- [ ] B3: export the DOC recognizers from `lexer.rs` beside `lookup_keyword`;
      `fmt.rs` consumes them. `DOC_UNTERMINATED` behavior unchanged.
- [ ] A3, A4, A5: module moves/renames.

Acceptance: gate `diffs=0` after each commit; `cargo test` green;
`mfb fmt --check` before/after lists byte-identical.
Commit: —

### Phase 3 — docs, ordering, and full validation

- [ ] C1: one banner per subject in `src/ast/tests.rs`; sections contiguous.
- [ ] C2: add the LINK banner; restore the DOC group in `src/fmt.rs`.
- [ ] D1, D2, D3, D4, D6, D7.
- [ ] Full `scripts/test-accept.sh` run; `cargo fmt` (second pass in
      `repository/`, which is not a workspace member); `cargo clippy`.

Acceptance: full acceptance suite green; artifact gate `diffs=0`; zero modified
files under `tests/**/golden/` except the audited D4 string, if any.
Commit: —

## Validation Plan

- Regression tests: no new fixture. New unit tests for the merged helpers
  (B1's single `normalize_types`, B3's shared DOC recognizers) asserting the
  full behavior set, so a future fork fails a test rather than drifting silently.
- Runtime proof: `scripts/artifact-gate.sh` at `diffs=0` on every commit — this
  *is* the proof for an output-preserving bug — plus one full
  `scripts/test-accept.sh` run and the `mfb fmt --check` before/after diff before
  merge.
- Byte-identity guard: `git status` must show zero modified files under any
  `tests/**/golden/` directory, except the single audited D4 diagnostic string.
- Doc sync: `src/docs/man/tooling/fmt.md` is new (D6); the D2/D3 citations
  re-point into `src/docs/spec/`. No spec *content* changes here — that is
  bug-338.
- Full suite: `cargo test`, `scripts/test-accept.sh`, `cargo clippy`.

## Open Decisions

- **D3 scope** — recommended: fix only the 14 `src/ast/` citations in this bug
  and file the remaining 48 as a mechanical follow-up; alternative is one
  tree-wide sweep of all 62, which is a larger but genuinely one-shot change.
- **A4 `category_value`** — recommended: delete the 36 dead lines; alternative is
  to wire field 0 through `parse_properties` if the category was always meant to
  be parsed. This needs a look at what the table is *for* before deleting.
- **A4 blanket-allow ownership** — this bug or bug-326. Recommended: bug-326
  (it owns the blanket-allow inventory); this bug does the directory move only.
- **A5 destination** — `src/ast/` vs `src/resolver/`. Recommended `src/ast/`
  (it is an AST pass); `src/resolver/` is defensible if its output is really
  resolution state.
- **D1 field rename** — recommended: rename `result` → `return_value`;
  alternative is doc-only, which leaves the misleading name in place.

## Summary

Seventeen verified items across the AST, lexer, and formatter. The engineering
risk sits in exactly two places: **A1**, which relocates a semantic parser
rewrite currently hiding in the `-ast` dumper, and **B3**, which makes `fmt`
share the lexer's DOC recognizers — both are parser/formatter-adjacent and are
gated by `scripts/artifact-gate.sh` staying at `diffs=0` plus an `mfb fmt
--check` before/after diff. **A6** is the widest but the most mechanical: ~15
modules currently import a string escaper from the binary entrypoint. Everything
else is placement, banner ordering, and citations that point at retired
documents.

Notably, three of the original leads did **not** survive verification and have
been corrected in place rather than carried forward: `src/ast/stmt.rs` *does*
have a test banner (`src/ast/tests.rs:2327`); the `'\r'` arm at
`src/fmt.rs:161` is *reachable* for interior carriage returns; and
`src/resolver/mod.rs:115` is not a third copy of the normalizer but a different
function that inlines both operations. Left untouched: the grammar and
`05_fmt.md` drift (bug-338), the `ast/items.rs` split (bug-327), the dead-code
sweep (bug-326), and `mfb fmt`'s diagnostic behavior.
