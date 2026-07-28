# plan-18-A: Test-framework surface + build exclusion

Last updated: 2026-07-02
Effort: large

Adds the `TESTING` / `TGROUP` / `TCASE` source surface (lexer, parser, AST, resolver validation) and
the monomorph seam that **drops** `TESTING` blocks for `mfb build` while **retaining** them for a new
`mfb test` subcommand. This sub-plan delivers no runtime behavior of its own — it ends with a proof
that `mfb build` output is byte-identical whether or not `TESTING` blocks are present, and that under
a stub `mfb test` the blocks survive to codegen. It is the safe, separately-valuable foundation the
runner (plan-18-B) and coverage (plan-18-C) build on.

Read first: `mfb spec language` (block forms), `mfb spec diagnostics` (diagnostic style), and the
overview [plan-18-testing.md](plan-18-testing.md) §2 (Current State) and §4 (Grammar).

## 1. Goal

- Parse `TESTING … END TESTING` (top-level), `TGROUP <string> … END TGROUP`, `TCASE <string> … END
  TCASE` (statement-body) into new AST nodes, everywhere a top-level `Item` is allowed, multiple
  blocks per unit.
- Resolver validates: `TGROUP` only inside `TESTING`; `TCASE` only inside `TGROUP`; descriptions are
  string literals; `expect*` calls only inside a `TCASE` body (the last one shares surface with
  plan-18-B but the *placement* diagnostic lands here).
- `mfb build` drops all `TESTING` blocks before codegen — native output byte-identical to the same
  program with the blocks physically removed.
- A new `mfb test` subcommand exists and compiles *with* `TESTING` retained (stub entry for now —
  real driver is plan-18-B).

### Non-goals

- No assertion semantics, no runner, no codegen for `expect*` (plan-18-B).
- No new global reserved words beyond `TESTING` (D1: `TGROUP`/`TCASE` contextual).
- No change to value/copy/move semantics or to any existing `Item`'s handling.

## 2. Current State

- `Item` enum: `src/ast/types.rs:56` (`Binding | Function | Type | Resource | FuncAlias | Link |
  Doc`). `Doc` is the closest precedent for a free-standing, mode-sensitive block.
- Statement block + `Statement { line }`: `src/ast/types.rs:449`.
- Keyword lexing: `Keyword` enum `src/lexer.rs:63`; `keyword()`/`lookup_keyword()`
  `src/lexer.rs:597-601`; identifier/keyword lexer `lex_identifier_or_keyword` `src/lexer.rs:372`.
- Top-level parsing and statement-block parsing live in `src/ast.rs` / `src/ast/stmt.rs` (e.g.
  `EXIT PROGRAM` at `src/ast/stmt.rs:118`). Follow the `DOC`-block parser as the structural model.
- Subcommand dispatch: `src/main.rs:45`; build options `parse_build_options` at `src/main.rs:82`.
- Monomorph is where callees are mangled/selected before typecheck (memory:
  overridable-builtins-returntype-overloads) — the natural place to drop or retain `TESTING`.

## 3. Design

- **AST**: add `pub struct TestingBlock { groups: Vec<TestGroup>, line }`,
  `TestGroup { description: String, cases: Vec<TestCase>, line }`,
  `TestCase { description: String, body: Vec<Statement>, line }`, and `Item::Testing(TestingBlock)`.
  Serialize in `src/ast/serialize.rs` (new `-ast` node kinds) and, if `TESTING` should appear in
  `-ast`/`mfb doc`, thread through those emitters.
- **Lexer**: add `Keyword::Testing` only. `TGROUP`/`TCASE`/`END TGROUP`/`END TCASE` are recognized by
  the block parser via lexeme comparison on identifier tokens (D1).
- **Parser**: `parse_testing_block` invoked from the top-level item loop when the current token is
  `Keyword::Testing`. It requires `TGROUP` children (error on stray statements/`TCASE` directly under
  `TESTING`); `parse_test_group` requires `TCASE` children; `parse_test_case` parses a statement block
  via the existing statement-block parser until `END TCASE`.
- **Resolver**: validate nesting and description-is-string-literal; emit the placement diagnostic for
  any `expect*` call outside a `TCASE` (the resolver already walks call sites). Register new
  diagnostic codes in `src/docs/spec/diagnostics/**`.
- **Mode plumbing**: thread a `TestMode` flag from the CLI (`build` → off, `test` → on) through to
  monomorph. In `build` mode, monomorph discards `Item::Testing` (and any symbol reachable *only* from
  test bodies) before codegen. In `test` mode, `Item::Testing` is retained; a stub entry (real driver
  in B) is emitted so the pipeline runs end-to-end.

## Phases

### Phase 1 — AST + lexer + parser

Parse the three block forms into new AST nodes; no semantics.

- [ ] Add `Keyword::Testing` to `src/lexer.rs:63` + `keyword()` mapping (`src/lexer.rs:601`) + the
      display arm near `src/lexer.rs:754`.
- [ ] Add `Item::Testing(TestingBlock)` and the `TestingBlock`/`TestGroup`/`TestCase` structs to
      `src/ast/types.rs` (beside `Item` at :56).
- [ ] `parse_testing_block` / `parse_test_group` / `parse_test_case` in `src/ast.rs`
      (or `src/ast/stmt.rs`), wired into the top-level item loop; `TGROUP`/`TCASE`/`END …` matched by
      lexeme per D1. Reuse the existing statement-block parser for the `TCASE` body.
- [ ] Serialize the new nodes in `src/ast/serialize.rs` so `-ast` round-trips.
- [ ] Tests: parser fixtures for well-formed nesting and for each structural parse error (stray
      statement under `TESTING`, `TCASE` under `TESTING`, missing `END TGROUP`, non-string
      description).

Acceptance: `mfb … -ast` on a fixture with nested `TESTING`/`TGROUP`/`TCASE` prints the expected AST;
malformed nesting produces a parse diagnostic. Commit: —

### Phase 2 — Resolver validation + diagnostics

Enforce placement rules and register the diagnostics.

- [ ] Nesting + description-literal validation in the resolver (`src/resolver.rs`).
- [ ] `expect*`-only-in-`TCASE` placement diagnostic (call-site walk in `src/resolver.rs`).
- [ ] Register the new diagnostic codes in `src/docs/spec/diagnostics/**` and regenerate whatever
      consumes the `error-codes` table.
- [ ] Tests: `tests/…_invalid/**` for each diagnostic.

Acceptance: each misuse yields its specific diagnostic; `mfb spec diagnostics` lists the new codes.
Commit: —

### Phase 3 — Monomorph drop (build) + retain (test) + `mfb test` stub

The byte-identical gate and the new subcommand skeleton.

- [ ] Thread a `TestMode` flag from CLI through to monomorph; add the `Some("test")` arm at
      `src/main.rs:45` reusing `parse_build_options` (+ a `--coverage` flag parsed but inert until
      plan-18-C).
- [ ] In monomorph, drop `Item::Testing` and test-only-reachable symbols when `TestMode` is off;
      retain when on (`src/monomorph.rs`).
- [ ] `mfb test` compiles with `TESTING` retained and emits a **stub** top-level entry (e.g. prints
      nothing / exits 0) so the full pipeline runs; the real driver is plan-18-B.
- [ ] Tests: build-exclusion golden — a fixture with `TESTING` blocks whose `mfb build` native binary
      is byte-identical to the same program with the blocks deleted.

Acceptance: `diff <(mfb build with-tests.mfb) <(mfb build without-tests.mfb)` is byte-identical for
the emitted binary; `mfb test with-tests.mfb` compiles and links (stub run). Commit: —

## Validation Plan

- Parser/resolver fixtures under `tests/` for every well-formed and malformed shape.
- Byte-identical build-exclusion proof (Phase 3 acceptance).
- Doc sync: `src/docs/spec/language/**` gains the block form; `src/docs/spec/diagnostics/**` gains the
  new codes.
- Acceptance: `scripts/test-accept.sh` green (existing suite unaffected — nothing changes for programs
  without `TESTING`).

## Open Decisions

- D1 (contextual `TGROUP`/`TCASE`) — resolved here per the overview recommendation; revisit only if
  contextual lexeme-matching proves awkward against `END` handling.

## Summary

Pure front-end + a monomorph gate. The only thing that can go wrong is the drop pass leaking a
test-only symbol into a normal build (caught by the byte-identical gate) or the contextual keyword
matching mis-parsing `END`. No runtime, no codegen, no ABI change.
