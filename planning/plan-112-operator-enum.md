# Operator Enum Plan

Last updated: 2026-08-30
Effort: large (3h–1d)

The compiler's binary and unary operators are carried as `String` from the
parser to the emitted byte, and every stage re-decides on the spelling. This
plan replaces that with two `Copy` enums in a new `src/operators.rs`, so that
after `src/ast/expr.rs` mints one, no stage ever compares an operator to a
string again.

The behavioral outcome: **`BinaryOp`/`UnaryOp` are the compiler's only operator
representation from the parser to codegen, every `match` on them is exhaustive
and checked by rustc, and the 18 "operator is not lowered" runtime error paths
that exist today are deleted because the state they report is unrepresentable.**

Unlike `ParameterType`, the operator vocabulary is **closed and cannot be
extended by user code**: there is no operator overloading, no user-declared
operator, and the parser mints an operator only from a fixed `TokenKind`. So
this needs no interner, no `Symbol`, no `UserOf` escape hatch, and no
generic-variable classification — the whole reason it should be much simpler
than `src/types.rs`.

References:

- `src/ast/expr.rs:70-270` — `parse_or`/`parse_and`/`parse_not`/
  `parse_comparison`/`parse_concat`/`parse_addition`/`parse_multiplication`/
  `parse_power`/`parse_unary`, the only place a binary/unary operator is minted.
  Each already `match`es a `TokenKind` and then `.to_string()`s the result — the
  enum is one step *shorter* than what is there now.
- `src/types.rs` — the precedent this mirrors in shape (`parse`/`name`, a
  round-trip test, one grammar file) and deliberately does **not** mirror in
  complexity (no interner, no recursion, no user-extensible leaf).
- `tests/no_type_strings.rs` — the ratchet-gate precedent (scan roots,
  `#[cfg(test)]`-stripping line filter, hard-zero assertion, exemption fn pinned
  by its own test). Phase 5's gate mirrors its structure, not its needle-list
  design.
- `planning/completed/plan-111-A-vocabulary-and-ratchet-gate.md` — the type
  migration this plan is the operator analogue of, and whose gate scope
  (`plan-111 has exactly one task: delete every type string after the AST`) is
  why operators were left behind.
- `bugs/completed/` — bug-403, which established that an IR-binary-decoded
  operator outside the valid set must be a hard error, never a silent pass
  (`src/ir/binary.rs:1728-1762`). This plan makes that guarantee structural.
- `.ai/testing-gates.md` — artifact-gate mechanics and golden regeneration.
- `.ai/codegen-invariants.md`, `AGENTS.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Tree compiles clean at HEAD | `cargo check --all-targets` → 0 warnings | UNMEASURED — run at kickoff |
| The type-string gate is green (this plan must not regress it) | `cargo test --test no_type_strings` → 7 passed, 0 failed | MET (measured 2026-08-30) |
| `artifact-gate all` reads 0 diffs before any work starts | `scripts/artifact-gate.sh all` | UNMEASURED — run at kickoff, **before** the first edit |

The third row is not optional bookkeeping. Per
`abi-function-migration-drifts-ncodesum`, gating to 0 diffs *before* landing is
what makes every subsequent diff attributable to this plan without a
`git archive` baseline build. Establish it first; it is the cheap version of
plan-111-G's Phase 4.

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop.
>
> **If you stop, report the current status of *all* prerequisites**, not only the
> one that blocked you.

## 1. Goal

- A new `src/operators.rs` defines `BinaryOp` (17 variants) and `UnaryOp` (3
  variants), both `Copy + Eq + Hash`, with `name(self) -> &'static str` and
  `parse(&str) -> Option<Self>`.
- `HirExpression::Binary/Unary`, `IrValue::Binary/Unary` and
  `NirValue::Binary/Unary` carry those enums instead of `String`
  (`src/hir/mod.rs:420,426`; `src/ir/value.rs:123,132`;
  `src/target/shared/nir/mod.rs:354,360`).
- **0** comparisons or `match` arms against an operator spelling outside
  `src/operators.rs` and the single IR-binary decode site.
- **0** functions taking an operator as `&str`.
- The 18 `"does not lower … operator '{other}'"` error paths are deleted, their
  fallback arms replaced by exhaustive matches (listing below, §2).
- `tests/no_operator_strings.rs` asserts the above as hard zeros and fails
  `cargo test` if any is reintroduced.

### Non-goals (explicit constraints)

- **No behavior change.** Diagnostic text, codes, ordering and source locations
  unchanged on both corpora. The only user-visible deltas permitted are the
  deletion of unreachable internal error strings that no valid program can
  trigger (§4).
- **No format change.** `.ast` and `.ir` goldens stay **byte-identical**: the
  JSON must keep the exact current spellings (`"op": "<>"`, `"operator": "AND"`,
  …). The IR binary format keeps its length-prefixed operator string; only the
  in-memory type changes. `name()` is the render at both sinks.
- **No new abstraction.** No operator traits, no precedence table refactor, no
  "operator kind" hierarchy, no merging `BinaryOp` and `UnaryOp` into one enum.
  Two plain data enums and two functions.
- **No precedence or associativity change.** The parser's structure
  (`parse_or` → `parse_and` → … → `parse_unary`) is untouched; only the value it
  stores in the node changes.
- **No interner, no `Symbol`.** The vocabulary is closed; a `u8`-sized enum is
  the whole representation.
- **This plan does not touch any other string category.** Not names, not
  `ctype`, not `IrType.kind`/`visibility`, not `Operand::Raw`. Those are
  separate plans. Do not absorb them.
- **Machine-level `op` is a different vocabulary and is out of scope.**
  `src/optimizer/opt2/**` and `src/arch/**` match `CodeOp` mnemonics
  (`"adrp"`, `"fadd_d"`, `"add_imm"`), not language operators. `CodeOp` is
  already an enum. Only `src/optimizer/opt1/**` operates on language operators.

## 2. Current State

The operator is minted as a `String` in the parser, copied verbatim through
three tree layers, and re-decided on by spelling at every consumer.

- **Minted** in `src/ast/expr.rs`: seven parse functions each `match` a
  `TokenKind` to a `&'static str` and then `.to_string()` it
  (`:73-85`, `:101`, `:120`, `:139-155`, `:171`, `:183-194`, `:208-222`,
  `:243`, `:262`). Two more sites in `src/ast/link_items.rs:253,682` mint
  `"NOT"` and `"SIZEOF"` directly. `src/ast/build.rs:23` is a `&str`-taking
  test/desugar constructor.
- **Copied** through `hir::elaborate` — `src/hir/mod.rs:965-982` clones the
  `String` per node — into `HirExpression::Binary/Unary { operator: String }`
  (`:420,426`), then into `IrValue::Binary/Unary { op: String }`
  (`src/ir/value.rs:123,132`), then into `NirValue::Binary/Unary { op: String }`
  (`src/target/shared/nir/mod.rs:354,360`).
- **Decided on** by spelling in 193 places (§Measured populations), including
  the two largest dispatch tables:
  `src/codegen/engine/operators/builder_numeric.rs` (41 sites) and
  `src/optimizer/opt1/constant_folding.rs` (24 sites).
- **Rendered** at three sinks that must not change: `src/ast/serialize.rs:1302,
  1312` (`.ast` JSON), `src/ir/json.rs` (`.ir` JSON), and
  `src/ir/binary.rs:981` `encode_op` (IR binary).
- **Decoded** at exactly one site: `src/ir/binary.rs:1585,1592`
  (`op: r.string()?`). This is the plan's only `parse` boundary.

The 18 fallback arms that exist solely because the spelling is open
(`grep -rn "operator '{\|unsupported operator\|unknown operator" src/codegen src/ir src/target`):

| File | Message |
|---|---|
| `src/codegen/builtins/money/gen_money_math.rs:131` | `cannot lower Money operator '{other}'` |
| `src/codegen/engine/operators/builder_numeric.rs:29` | `does not lower boolean operator '{other}'` |
| `src/codegen/engine/operators/builder_numeric.rs:516` | `does not lower record comparison operator '{op}'` |
| `src/codegen/engine/operators/builder_numeric.rs:610` | `does not lower comparison operator '{other}'` |
| `src/codegen/engine/operators/builder_numeric.rs:738` | `does not lower comparison operator '{other}'` |
| `src/codegen/engine/operators/builder_numeric.rs:942` | `does not lower integer operator '{other}'` |
| `src/codegen/engine/operators/builder_numeric.rs:995` | `does not lower Fixed operator '{other}'` |
| `src/codegen/engine/operators/builder_numeric.rs:1078` | `does not lower Float operator '{other}'` |
| `src/codegen/string/repr/builder_strings.rs:2023` | `does not lower string comparison operator '{other}'` |
| `src/codegen/engine/value/builder_values.rs:1631` | `does not lower unary operator '{op}' for …` |
| (+ 8 more found by the same grep) | |

Each is an internal error a valid program cannot reach; each becomes an
exhaustive match arm or a deleted arm in Phase 4.

### Measured populations

All commands run at HEAD on 2026-08-30, `src/ast/` and `src/lexer.rs` excluded
(they are the string domain by definition), tests excluded via
`grep -v _tests.rs | grep -v '/tests.rs'`.

| What | Count | Command |
|---|---|---|
| Operator decision sites, post-AST | **193** | `grep -rnE --include="*.rs" '(\bop\b\|\boperator\b)[^A-Za-z_"]{0,25}"(OR\|XOR\|AND\|NOT\|MOD\|DIV\|SIZEOF\|\+\|-\|\*\|/\|\^\|&\|=\|<>\|<\|<=\|>\|>=)"\|^\s*"(OR\|XOR\|AND\|NOT\|MOD\|DIV\|SIZEOF\|\+\|-\|\*\|/\|\^\|&\|=\|<>\|<\|<=\|>\|>=)"(\s*\|\s*"[^"]*")*\s*=>' src/ \| grep -v _tests.rs \| grep -v '/tests.rs' \| grep -v '^src/ast/' \| grep -v '^src/lexer.rs' \| wc -l` |
| Distinct files holding them | **30** | same pipeline, `cut -d: -f1 \| sort -u \| wc -l` |
| …in `src/codegen` | **87** | same regex, scoped to `src/codegen` |
| …in `src/ir` | **48** | same regex, scoped to `src/ir` |
| …in `src/optimizer/opt1` | **39** | same regex, scoped to `src/optimizer/opt1` |
| …in `src/target` (incl. `shared/nir`) | **4** | same regex, scoped to `src/target` |
| …in `src/monomorph` | **2** | same regex, scoped to `src/monomorph` |
| …in `src/hir` | **1** | same regex, scoped to `src/hir` |
| Carrier fields to retype | **6** | `grep -rn 'operator: String,\|op: String,' src/hir/mod.rs src/ir/value.rs src/target/shared/nir/mod.rs` |
| Mint sites in `src/ast` | **12** | `grep -rnE 'operator: (operator\.to_string\(\)\|"[^"]*"\.to_string\(\))' src/ast \| grep -v tests.rs \| wc -l` |
| Fallback "does not lower operator" arms | **18** | `grep -rn -B 2 'does not lower\|unsupported operator\|unknown operator\|unsupported binary\|unsupported unary' src/codegen src/ir src/target \| grep -E 'format!\|=>' \| wc -l` |
| `.ast` goldens carrying an operator | **793** | `find tests -name "*.ast" \| wc -l` |
| `.ir` goldens carrying an operator | **793** | `find tests -name "*.ir" \| wc -l` |
| Distinct operator spellings in the golden corpus | **18** | `grep -ohE '"op": "[^"]*"' $(find tests -name "*.ir") \| sort -u` (35 lines, of which 17 are `IrOp` statement tags — `bind`, `if`, `while`, … — not operators) |

Top five files by site count (`cut -d: -f1 | sort | uniq -c | sort -rn`):

| Count | File |
|---|---|
| 41 | `src/codegen/engine/operators/builder_numeric.rs` |
| 24 | `src/optimizer/opt1/constant_folding.rs` |
| 13 | `src/codegen/engine/control/builder_control.rs` |
| 12 | `src/numeric.rs` |
| 12 | `src/ir/verify/values.rs` |

### The vocabulary (measured, closed)

Read out of `src/ast/expr.rs:70-270` and `src/ast/link_items.rs:253,682`, and
cross-checked against the golden corpus (which contains exactly these 18
spellings, `SIZEOF` excepted — see the UNVERIFIED note below).

**`BinaryOp` — 17 variants.** Spelling → variant, grouped by the parse function
that mints it:

| Parser fn | Spellings |
|---|---|
| `parse_or` | `OR`, `XOR` |
| `parse_and` | `AND` |
| `parse_comparison` | `=`, `<>`, `<`, `<=`, `>`, `>=` |
| `parse_concat` | `&` |
| `parse_addition` | `+`, `-` |
| `parse_multiplication` | `*`, `/`, `MOD`, `DIV` |
| `parse_power` | `^` |

**`UnaryOp` — 3 variants.** `NOT` (`parse_not`, and `link_items.rs:253` for
`ERROR_ON`'s De Morgan negation), `-` (`parse_unary`), `SIZEOF`
(`link_items.rs:682`, LINK const-pin only).

### Verified properties

- **The vocabulary is closed and user code cannot extend it.** Verified by
  reading `src/ast/expr.rs:70-270`: every mint site is a `match` on a
  `TokenKind` or `Keyword` with an `unreachable!()` default guarded by the
  preceding `match_any`/`match_keyword`. There is no path from an identifier or
  a user declaration to an operator. This is what makes an interner-free plain
  enum correct here and wrong for `ParameterType`.
- **Unary `+` is never produced, and two arms handling it are dead.**
  `grep -rn 'operator: "+"' src/` returns nothing — no parser mints a unary
  plus. Yet `src/ir/lower_link.rs:294` (`} if operator == "+" =>`) and
  `src/ir/shape.rs:388` (`operator == "-" || operator == "+"`) handle it. The
  enum makes this visible: `UnaryOp` has no `Plus` variant, so both arms fail
  to compile and are deleted in Phase 3. **This is a found defect, not a
  refactor artifact** — it is dead code that has been silently carried.
- **`typed_binary_result_type` takes the operator as `&str`.**
  `src/numeric.rs:442` — the one numeric-promotion algebra is keyed by operator
  spelling, and `src/numeric.rs:478-479` calls it with a literal `"+"`. It
  retypes to `BinaryOp` in Phase 4. Its `&str`-typed *type* twin at
  `src/numeric.rs:410` is already `#[cfg(test)]`, so it is out of scope.
- **The IR binary format has exactly one decode site.**
  `src/ir/binary.rs:1585,1592` (`op: r.string()?`). Read `:1728-1762`: bug-403's
  regression test feeds `op: "GARBAGE"` and requires an error. With
  `BinaryOp::parse` returning `Option`, that error becomes structural rather
  than a hand-written validation, and the existing test keeps passing unchanged.
- **`.ir` and `.ast` goldens pin the spelling.** `grep -ohE '"op": "[^"]*"'`
  over all 793 `.ir` goldens yields the 17 binary spellings plus `NOT`. So
  `name()` must reproduce each byte-exactly; the goldens are the corpus test.
- **UNVERIFIED: whether `SIZEOF` appears in any committed golden.** It is absent
  from the `"op":` extraction above, and its only consumers are
  `src/ir/lower_link.rs:275` and `src/ir/shape.rs:383` (LINK const-pin
  evaluation, which folds before serialization). Phase 1 task 4 greps the
  `link-const-pins` fixture and records the answer; if no golden covers it, that
  is a coverage gap to fill with a fixture, not a reason to drop the variant.

## 3. Design Overview

Three pieces, layered:

1. **`src/operators.rs`** — two `#[derive(Clone, Copy, PartialEq, Eq, Hash,
   Debug)]` enums, `name(self) -> &'static str`, `parse(&str) -> Option<Self>`,
   and a `TokenKind`→variant constructor for the parser. ~120 lines with docs.
   No recursion, no allocation, no interning. This is the piece `src/types.rs`
   would be if types were a closed set.
2. **Retyping the three carriers** (HIR, IR, NIR). This is where the compiler
   does the work for us: changing `op: String` to `op: BinaryOp` turns all 193
   decision sites into compile errors. There is no census risk here and no
   grep blind spot — unlike plan-111, the site list is generated by rustc, not
   by a regex over prose-contaminated source.
3. **The gate** (`tests/no_operator_strings.rs`) — hard zero, mirroring
   `tests/no_type_strings.rs`'s structure.

**Where correctness risk sits.** Not in the conversion — an exhaustive `match`
that compiles is a proof the old chain of `if op == "…"` never was. The risk is
concentrated in exactly two places:

- **The render sinks.** If `name()` disagrees with the parser's old
  `&'static str` by one byte, 1586 goldens churn at once. Phase 1 proves
  `name()` against the golden corpus *before* any carrier is retyped, which is
  why the enum lands with no callers first.
- **Silently changed dispatch order.** A chain like
  `if op == "-" { … } else if op == "+" { … }` converted to a `match` can
  reorder guard evaluation where the arms overlap on a *second* condition
  (a type test, a constant-ness test). Convert arm-for-arm in source order and
  never merge two arms in the same commit that retypes them.

**Byte-identity is this plan's correctness gate, and it is the right one.**
This is provably-neutral work: the same decisions, made on a `u8` instead of a
`String`. No target is expected to diff. `scripts/artifact-gate.sh all` reading
0 diffs is the acceptance check for Phase 4, and `.ast`/`.ir` goldens are the
acceptance check for Phases 2 and 3. Per `AGENTS.md`: if a diff appears, objdump
one fixture, localize it, and fix the bug — a diff is never grounds to conclude
the enum cannot work.

**Rejected alternatives:**

- *One `Operator` enum for both arities.* Rejected: it re-admits the illegal
  state the plan exists to remove (`Binary { op: Operator::Not }`). Two enums
  make arity a type error.
- *`Symbol`-interned operators, mirroring `ParameterType`'s leaves.* Rejected:
  interning exists for an open, user-extensible set. Operators are 20 closed
  values; a `u8` enum is strictly smaller, `Copy`, and needs no lock.
- *Keeping `String` in the AST and converting at `hir::elaborate`.* Rejected:
  the parser already has a `TokenKind`, so going token → enum is one step
  *shorter* than token → `&str` → `String` → enum. The `.ast` goldens are
  preserved by rendering `name()` in `serialize.rs`, which is the same
  guarantee at a lower cost. (See Open Decisions — this is the one genuine fork.)
- *A `precedence()` method on `BinaryOp`, folding the parse ladder into a
  table-driven loop.* Rejected: out of scope, changes parser structure, and the
  non-goals forbid it. Note it for a future plan; do not do it here.

## Compatibility / Format Impact

| Contract | Change |
|---|---|
| `.ast` JSON golden format | **None.** `serialize.rs` renders `op.name()`, byte-identical. |
| `.ir` JSON golden format | **None.** `ir/json.rs` renders `op.name()`, byte-identical. |
| IR binary format | **None on the wire.** `encode_op` writes `op.name()` as the same length-prefixed string; `binary.rs` decodes via `BinaryOp::parse`. |
| `.ncode` / `.ncodesum` | **None expected.** Phase 4's gate is 0 diffs across all targets. |
| `.mfp` package format | **Untouched.** Operators are not in the package type table. |
| `mfb man` / `mfb spec` | **Untouched.** No public surface change. |
| Diagnostics | Unchanged text and codes, except the deletion of 18 unreachable internal error strings (§2) that no valid program can produce. |

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains; mark
> moot tasks `- [x] ~~text~~ — moot: <evidence>`; fill `Commit:` when a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — `src/operators.rs`, no callers

Lands the vocabulary and proves `name()` reproduces every spelling in the
golden corpus *before* anything depends on it. Safe alone: nothing calls it yet.

- [ ] Create `src/operators.rs` with `BinaryOp` (17 variants) and `UnaryOp` (3
      variants), both `#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]`, per
      the §2 vocabulary table. Register the module in `src/main.rs`.
- [ ] Implement `BinaryOp::name(self) -> &'static str` and
      `UnaryOp::name(self) -> &'static str` returning the exact current
      spellings.
- [ ] Implement `BinaryOp::parse(&str) -> Option<Self>` and
      `UnaryOp::parse(&str) -> Option<Self>`. `parse` is for the IR-binary
      decode boundary only; document that in the fn doc comment.
- [ ] Grep the `link-const-pins` fixture for `SIZEOF` and record in Corrections
      whether any committed golden covers it (§2 UNVERIFIED). If none does, add
      a fixture in Phase 5 rather than dropping the variant.
- [ ] Tests: `src/operators.rs` `#[cfg(test)]` module with (a) `round_trip` —
      `parse(name(op)) == Some(op)` for all 20 variants, enumerated explicitly,
      not via a helper that could silently skip one; (b) `parse` returns `None`
      for `"GARBAGE"`, `""`, `"and"` (wrong case), `"=="`; (c) a
      **golden-corpus test** that extracts every `"op"`/`"operator"` string from
      `tests/**/*.ir` and `tests/**/*.ast` and asserts each parses to a variant
      whose `name()` is byte-equal to the input.

Acceptance: `cargo test operators` passes, and the golden-corpus test proves all
18 corpus spellings round-trip byte-exactly. `cargo check --all-targets` → 0
warnings (the enums are `#[allow(dead_code)]`-free because the tests use them).
Commit: —

### Phase 2 — the parser mints the enum

Converts the 12 mint sites and the `.ast` render sink. Lands alone because HIR
still takes a `String`: `elaborate` calls `.name().to_string()` at the seam for
exactly one commit.

- [ ] Add `BinaryOp::from_token(TokenKind) -> Option<Self>` in
      `src/operators.rs`; rewrite `src/ast/expr.rs:73,139,183,208` to use it
      instead of matching a `TokenKind` to a `&str`.
- [ ] Retype `Expression::Binary { operator }` and `Expression::Unary
      { operator }` in `src/ast/types.rs` to `BinaryOp`/`UnaryOp`; fix the 12
      mint sites in `src/ast/expr.rs` and `src/ast/link_items.rs:253,682`.
- [ ] Retype `src/ast/build.rs:23` `binary(left, operator: BinaryOp, right)`.
- [ ] `src/ast/serialize.rs:1303,1313`: render `json_string(operator.name())`.
- [ ] `src/hir/build.rs:26,68` and `src/hir/mod.rs:965-982`: at the elaborate
      seam, emit `operator.name().to_string()` so HIR is unchanged this phase.
      Mark it with a `// plan-112 Phase 3 deletes this seam` comment.
- [ ] Tests: update `src/ast/tests.rs` operator assertions
      (`:67,79,298,299`, …) from string compares to variant compares.

Acceptance: all 793 `.ast` goldens byte-identical
(`scripts/test-accept.sh <target> /tmp/accept-112` → 0 mismatches);
`cargo test --no-fail-fast` green.
Commit: —

### Phase 3 — HIR and IR carriers, and the decode boundary

Retypes 2 of the 3 carriers and deletes the Phase 2 seam. 48 `src/ir` sites plus
1 `src/hir` site become compile errors; fix them arm-for-arm in source order.

- [ ] Retype `src/hir/mod.rs:420,426` to `BinaryOp`/`UnaryOp`; delete the
      `.name().to_string()` seam in `src/hir/build.rs` and `src/hir/mod.rs:971,
      982` (the per-node `String` clone goes with it).
- [ ] Retype `src/ir/value.rs:123,132` to `BinaryOp`/`UnaryOp`.
- [ ] Fix the resulting compile errors across `src/ir/**` (48 sites; heaviest:
      `verify/values.rs` 12, `lower_link.rs` 11, `verify/mod.rs` 7, `shape.rs`
      7, `lower.rs` 5) and `src/monomorph/lower.rs` (2).
- [ ] **Delete the two dead unary-`+` arms** at `src/ir/lower_link.rs:294` and
      `src/ir/shape.rs:388` (§2 Verified properties — no parser mints a unary
      `+`). Record in Corrections that this was a live defect the enum exposed.
- [ ] `src/ir/json.rs`: render `op.name()`.
- [ ] `src/ir/binary.rs:981` `encode_op`: write `op.name()`.
      `src/ir/binary.rs:1585,1592`: decode via
      `BinaryOp::parse(&r.string()?).ok_or_else(|| format!("invalid operator …"))`.
- [ ] Tests: confirm bug-403's `"GARBAGE"` regression test at
      `src/ir/binary.rs:1728-1762` still passes **unmodified**. If it needs an
      edit, stop — that is a behavior change, and the 4-question gate in
      `AGENTS.md` applies.

Acceptance: all 793 `.ir` goldens byte-identical; `cargo test --no-fail-fast`
green including bug-403's unmodified regression test; `cargo check
--all-targets` → 0 warnings.
Commit: —

### Phase 4 — NIR, codegen, opt1, numeric (largest blast radius)

The remaining 130 sites, and the deletion of the 18 fallback error arms. Last
because it is where the codegen output could move if a conversion is wrong.

- [ ] Retype `src/target/shared/nir/mod.rs:354,360` to `BinaryOp`/`UnaryOp`;
      fix `src/target/shared/nir/visit.rs` and `constfold.rs` (4 sites).
- [ ] Fix `src/codegen/**` (87 sites). Take
      `src/codegen/engine/operators/builder_numeric.rs` (41) first — it holds 6
      of the 18 fallback arms — then `engine/control/builder_control.rs` (13),
      `string/repr/builder_strings.rs` (7), `link/thunk/link_thunk.rs` (6),
      `engine/value/builder_values.rs` (6), the rest.
- [ ] Fix `src/optimizer/opt1/**` (39 sites: `constant_folding.rs` 24,
      `algebraic.rs` 6, `strength.rs` 5, `rotate.rs` 2, `dce.rs` 1,
      `branches.rs` 1). Do **not** touch `src/optimizer/opt2/**` — different
      vocabulary (non-goals).
- [ ] Retype `src/numeric.rs:442` `typed_binary_result_type(operator: BinaryOp,
      …)` and its two literal callers at `:478-479`; retype
      `typed_money_result_type` at `:531`.
- [ ] Delete all 18 fallback arms as each match becomes exhaustive. Where an
      operator is genuinely invalid *for that operand type* (e.g. `^` on
      `Money`), keep the error but match the specific variants — do not leave a
      `_ =>` catch-all, which would re-hide the next addition.
- [ ] Verify no `_ =>` arm was left on a `BinaryOp`/`UnaryOp` match:
      `grep -rn -B 20 '_ =>' src/codegen src/ir src/optimizer/opt1` reviewed for
      operator scrutinees.

Acceptance: `scripts/artifact-gate.sh all` → **0 diffs** (the same reading the
Prerequisites row established before the first edit, so every diff seen on the
way is attributable to this plan and gets root-caused, not regenerated);
`scripts/test-accept.sh <target> /tmp/accept-112` → 0 mismatches and the same
`N ran` count as the pre-plan baseline; `cargo test --no-fail-fast` green;
`MFB_OPT=3 scripts/test-accept.sh` green (the `opt1` folding rows are only
exercised at higher `-O`, per `optimizer-rows-need-giant-function-stress`).
Commit: —

### Phase 5 — lock the gate, docs, archive

- [ ] Write `tests/no_operator_strings.rs`, mirroring
      `tests/no_type_strings.rs`'s structure (scan roots, `#[cfg(test)]`-aware
      line stripper, exemption fn pinned by its own test). Assert **hard zero**,
      no budget table, for: (a) an operator spelling in a `match` arm or `==`
      comparison; (b) a fn taking `op`/`operator` as `&str`; (c)
      `op: String`/`operator: String` as a struct field. Exempt exactly two
      files: `src/operators.rs` (defines the vocabulary) and `src/ast/expr.rs`
      + `src/ast/link_items.rs` (mint from tokens — or zero files, if Open
      Decision 1 lands as recommended and the parser never sees a spelling).
- [ ] Add a test that the exemption list is exactly the intended files
      (mirroring `the_grammar_file_is_exactly_one`).
- [ ] Re-run the §Measured populations commands and paste the new counts into
      Corrections. Every line must read 0. Do not annotate a non-zero line.
- [ ] If Phase 1 found no `SIZEOF` golden, add a `link-const-pins` fixture that
      covers it, and sync its goldens.
- [ ] Doc sync: `.ai/codegen-invariants.md` — record that operators are a closed
      enum and the two mint sites are the only construction points. Check
      whether `src/docs/spec/**` states the operator set; if it does, it is
      prose and needs no change, but verify it agrees with the 20 variants.
- [ ] Move `planning/plan-112-operator-enum.md` to `planning/completed/`.

Acceptance: `cargo test --test no_operator_strings` passes with hard zeros;
`cargo test --no-fail-fast` green; `scripts/artifact-gate.sh all` → 0 diffs;
every §Measured populations line re-measures to 0.
Commit: —

## Validation Plan

- **Tests:** `src/operators.rs` unit tests (round-trip, negative parse,
  golden-corpus spelling coverage); updated `src/ast/tests.rs` operator
  assertions; `tests/no_operator_strings.rs` as the standing gate. bug-403's
  regression test at `src/ir/binary.rs:1728` must pass **unmodified** — it is
  the proof the decode boundary did not weaken.
- **Coverage check:** per `coverage-measurement-mechanics`, `mfb` is a binary
  crate — measure with `--bin mfb`. Confirm `src/operators.rs` is in the
  denominator; a `const`-only table needs a `black_box` runtime test to register.
- **Runtime proof:** build and run a program exercising every operator at every
  operand type — `examples/` plus a purpose-built fixture — and diff its output
  against the pre-plan binary. An enum conversion that compiles can still have
  swapped two arms; only execution catches that. Also run `MFB_OPT=3` per the
  Phase 4 acceptance.
- **Doc sync:** `.ai/codegen-invariants.md`; verify `src/docs/spec/**`'s
  operator prose agrees with the 20 variants.
- **Acceptance:** `cargo test --no-fail-fast` (never bare `cargo test` — it
  fail-fasts at `golden.rs` and silently skips every `rt_*` test);
  `scripts/artifact-gate.sh all`; `scripts/test-accept.sh <target>
  /tmp/accept-112` (**never** pass a real directory as the second argument — it
  is `rm -rf`'d); `rustup run 1.96.0 cargo fmt --all && (cd repository &&
  rustup run 1.96.0 cargo fmt)`.

## Open Decisions

1. **Does the AST carry the enum, or keep `String` and convert at
   `hir::elaborate`?** — **Recommended: the AST carries the enum** (Phase 2 as
   written). The parser already holds a `TokenKind`; going token → enum is
   strictly shorter than token → `&str` → `String`, it deletes 12 allocations
   per expression node at parse time, and the `.ast` goldens are preserved by
   rendering `name()` at `serialize.rs`. The alternative — respecting
   plan-111's "the AST *is* the string domain" boundary literally — buys
   nothing here, because an operator is not a *name*: it is minted from a token,
   never from user text, so there is no spelling for the AST to be the domain
   of. If this is rejected, Phase 2 becomes "convert at elaborate" and the
   Phase 5 gate exempts `src/ast/` instead of nothing. (§3, Phase 2)

2. **Should `MOD`/`DIV` keep their keyword spelling in `name()`?** —
   **Recommended: yes, unchanged.** They are pinned by 793 `.ir` goldens. Raised
   only so nobody "tidies" them mid-conversion. (§Compatibility)

3. **Does `UnaryOp::SIZEOF` belong in the same enum as `NOT` and `-`?** —
   **Recommended: yes.** It is syntactically a unary operator over one operand
   and flows through the same `HirExpression::Unary` node. Splitting it into a
   LINK-only enum would need a second node kind for no benefit. Revisit only if
   Phase 1 finds it has no golden coverage at all. (§2)

## Corrections

<!-- Filled in DURING execution. Every place this plan turned out to be wrong:
     the claim, what was actually true, and the evidence. -->

## Summary

The real engineering risk is not the conversion — retyping the carrier makes
rustc enumerate all 193 sites, which is a stronger census than any grep, and an
exhaustive `match` that compiles is a proof the old spelling chain is gone. The
risk is that `name()` must reproduce 18 spellings byte-exactly across 1586
committed goldens, which is why Phase 1 lands the enum with no callers and
proves the render against the corpus before anything depends on it.

Left untouched, deliberately: every other post-AST string category — binding and
function names (`IrValue::Local(String)`, `Call { target: String }`), member and
field names, the C FFI type vocabulary (`ctype: String`), declaration keyword
tags (`IrType.kind`/`visibility`), literal payloads (`Const { value: String }`),
and machine operands (`Operand::Raw`). Each is a separate plan. This one is
operators, and it finishes them.
