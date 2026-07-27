# plan-68-G: AST front-end

Last updated: 2026-07-27
Overall Effort (AI): large (3h–1d)   (whole plan-68 feature)
Effort (Human): large (2h–4h)
Effort (AI): medium (1h–3h)
Depends on: plan-68-A
Produces: nothing — these five files reach ≥95% and drop off the gate; no
source-behavior change (test-only), unless a fixture uncovers a real defect
(then fixed on its own commit per `AGENTS.md`).

Part **G** of plan-68. Shared goal, prerequisites, dependency graph, standing
requirements (tests-beside-code, full `cargo test`, never-except-a-coverable
file, git discipline), and the measured 55-file population live in the overview:
[plan-68-coverage-gate.md](plan-68-coverage-gate.md). The worklist decisions and
the fresh `target/coverage/coverage.json` this letter reads come from
[plan-68-A-triage-exceptions.md](plan-68-A-triage-exceptions.md). Re-run the
overview's prerequisites before starting; do not restate them here.

## Scope

Five pure files (parse + AST serialize, no I/O — fully unit-coverable). Uncovered
counts, from the overview table (measured by `sh scripts/coverage-check.sh`):

| File | covered/total | uncov |
|---|---|---|
| src/ast/link_items.rs | 487/568 | 81 |
| src/ast/pipeline.rs | 146/166 | 20 |
| src/ast/scope_privates.rs | 434/480 | 46 |
| src/ast/expr.rs | 653/706 | 53 |
| src/ast/serialize.rs | 764/813 | 49 |

Total to cover (or reclassify): `python3 -c "print(81+20+46+53+49)"` → **249**.

These are un-exercised **grammar productions** (`expr.rs`, `link_items.rs`),
**pipeline-desugar walk arms** (`pipeline.rs`), **scope-privacy rewrite arms**
(`scope_privates.rs`), and **un-serialized AST-node variants** (`serialize.rs`).
Every one is reachable from a hand-written source string.

### Fixture style (precedent — follow it, do not re-invent)

- **Existing test homes:** `src/ast/tests.rs` (2682 lines, wired as
  `#[cfg(test)] mod tests` off `mod.rs`) is the main out-of-line harness;
  `scope_privates.rs`, `overloads.rs`, `manifest.rs`, `mod.rs` each carry an
  inline `#[cfg(test)] mod tests`. Add new tests to the **inline module of the
  file being covered** (create one where absent: `expr.rs`, `link_items.rs`,
  `pipeline.rs`, `serialize.rs`) so the coverage is attributed locally and the
  test sits beside the code — matching `scope_privates.rs`'s existing module.
- **Parse an OK case:** `crate::testutil::parse_file(src)` (panics on error) or
  `crate::ast::parse_source(Path::new("main.mfb"), "main.mfb", src)` returning
  `Result<AstFile, ()>` (precedent: `src/ast/tests.rs:19` `parse_import_aliases`,
  `:39` `string_concat_has_lower_precedence_than_addition`), then `match` on the
  returned AST.
- **Parse an ERROR arm:** `testutil::parse_file` panics, so error-arm tests call
  `parse_source(…)` directly and assert `.is_err()` (`testutil.rs:46` documents
  this split). A **recoverable** arm (`report` + `synchronize` then continue)
  may still return `Ok` with the malformed item dropped — assert the observable
  consequence (missing item / degraded AST), not `.is_err()`, for those.
- **Serialize:** `parse_file(src).to_json(0)` or
  `crate::testutil::project_from_src(src).to_json()` and assert the JSON
  substring (precedent: `scope_privates.rs` tests assert on
  `project.to_json()`).
- **Direct helper call** (for `pipeline.rs`'s `pub(super)` walk fns and
  `serialize.rs` free fns): build small `Expression`/AST values and call the fn
  (precedent: `overloads.rs` tests call `normalize_ws` directly).

### Getting the exact uncovered lines (do this per file, first)

`target/coverage/coverage.json` on disk is stale (overview §2); plan-68-A
regenerates it. After A lands, run `sh scripts/coverage.sh` once, then read the
per-file uncovered **line numbers** from `lcov.info`:

```
awk '/^SF:.*ast\/expr.rs$/{f=1} f&&/^DA:[0-9]+,0$/{print} /^end_of_record/{f=0}' \
  target/coverage/lcov.info
```

(substitute each target file). The candidate branches enumerated per phase below
were read from the source at authoring time; confirm each against that
`DA:LINE,0` list before writing the fixture, and cover any line the list shows
that the enumeration missed.

## Phases

### Phase G1 — `src/ast/pipeline.rs` (20 uncov)

Pure `|>`-desugar walk: `contains_placeholder` / `substitute_placeholder` plus
their `_arg` helpers. An arm is covered when the corresponding `Expression`
variant carries a `_` placeholder that gets rewritten. Cover each match arm —
easiest by **direct calls** to the two `pub(super)` fns with hand-built
`Expression` trees, or parse-driven with `x |> <rhs-with-_>`.

- [x] `Binary` — `a |> _ + 1` (both `contains_placeholder` left/right recursion
      and the `substitute_placeholder` `Binary` arm). Assert the substituted tree
      no longer contains the `_` identifier.
- [x] `Unary` — `a |> -_` and `a |> NOT _`.
- [x] `Call` named + positional args — `a |> f(k := _)` and `a |> f(_)` (covers
      `call_arg_contains_placeholder` + `substitute_placeholder_call_arg` both
      variants).
- [x] `Constructor` positional + named — `a |> T[_]` and `a |> T[fld := _]`
      (covers `constructor_arg_contains_placeholder` +
      `substitute_placeholder_constructor_arg` both variants).
- [x] `Lambda` body — placeholder inside a lambda body.
- [x] `ListLiteral` — `a |> [_]`; `SetLiteral` — `a |> Set OF Integer { _ }`.
- [x] `MapLiteral` key **and** value — `a |> Map OF Integer TO Integer { _ := 1 }`
      and `{ 1 := _ }` (the closure walks/rewrites both).
- [x] `MemberAccess` — `a |> _.field`.
- [x] `Trapped` — `_` inside a trapped subexpression (the bug-171-C arm at
      `pipeline.rs:161`).
- [x] `WithUpdate` target **and** update value — `a |> WITH _ { f := 1 }` and
      `a |> WITH r { f := _ }`.
- [x] Leaf/`other` arms — confirm the `String|Number|Scalar|Boolean => false`
      (contains) and `other => other` (substitute) arms are hit by an rhs whose
      placeholder sits beside a literal (e.g. `a |> _ + 1` already exercises the
      literal-`false` arm).

Acceptance: `sh scripts/coverage.sh` (fresh) then
`sh scripts/coverage-check.sh src/ast/pipeline.rs` shows ≥95%.

NOTE: fresh lcov shows only TWO arms actually uncovered (SetLiteral contains:27 +
substitute:127-134, Trapped contains:32 + substitute:162-171); the rest of the
enumerated arms are already covered by the existing
`pipeline_placeholder_each_expression_kind_as_rhs` fixture in `tests.rs`. Added an
inline `mod tests` in `pipeline.rs` with two direct-call fixtures for those arms.
Commit: 10bd87248

### Phase G2 — `src/ast/expr.rs` (53 uncov)

Expression + type-name grammar. Covered productions are dense (acceptance drives
them); uncovered lines are error arms, rare productions, and the depth guards.
Confirm against the `DA:LINE,0` list, then cover:

- [x] **Depth guards** (`enter_expr`/`enter_type` false paths, ~lines 25–36,
      113, 235, 255, 566–586): a source nesting past `MAX_EXPR_DEPTH`/
      `MAX_TYPE_DEPTH` (256) — e.g. 300 nested `(` for the expr guard, `List OF
      List OF …`×300 for the type guard. Assert `parse_source(…).is_err()` and
      the `MFB_PARSE_…too deep` diagnostic. (bug-171 / bug-191 may already cover
      some — check the list before adding.)
- [x] **`parse_pipeline` missing-placeholder** (58–63): `a |> f(1)` (no `_`) →
      `MFB_PARSE_PIPELINE_PLACEHOLDER_MISSING`.
- [x] **`parse_or` XOR** (Keyword::Xor, ~72–80): `a XOR b`.
- [x] **`parse_multiplication` MOD / DIV** (206–217): `a MOD b`, `a DIV b`.
- [x] **`parse_with_update` errors** (276, 289–293, 302): missing `{`, missing
      `:=`, non-identifier field, missing `}`.
- [x] **`parse_call_or_constructor` non-identifier callee / constructor**
      (330–338, 355–363): `(a + b)(1)` and `(a + b)[1]`.
- [x] **`parse_primary`**: Eof guard (459–467) — a bare trailing operator (e.g.
      `RETURN a +` at EOF); Map-literal missing `TO` (483–491) —
      `Map OF Integer Integer { }`; Set-literal `RES` forbidden (507–515) —
      `Set OF RES File { }`; the `NOTHING` / `TRUE` / `FALSE` / `Scalar`
      primaries if the list shows them uncovered.
- [x] **`finish_qualified_name` three-part error** (550–558): `a::b::c`.
- [x] **`parse_type_name_inner`**: ISOLATED FUNC (608–612) —
      `AS ISOLATED FUNC(Integer) AS Integer`; grouped type `(T)` (614–617);
      Map-type missing `TO` (628–636); Set-type `RES` forbidden (673–681);
      multi-arg template `Foo OF A, B` (688–706).
- [x] **`parse_thread_type_name`** (714–753): the three shapes —
      `Thread OF RES Res TO Out` (RES-first, message defaults to Nothing),
      `Thread OF Msg RES Res TO Out`, `Thread OF Msg TO Out`; and the missing-`TO`
      error. `ThreadWorker` variant for the `canonical` branch.
- [x] **`parse_resource_plane_type` STATE** (760–765): `RES File STATE Cursor`
      inside a thread plane.
- [x] **`parse_function_type_name` errors** (769–787): missing `(`, missing `)`,
      missing `AS`.
- [x] **`parse_lambda`**: assign-target body `LAMBDA(x AS Integer) -> c = c + x`
      (815–829) and the error arms — missing `(`, missing `)`, missing `->`.
- [x] **`parse_type_base_name`**: `Nothing` keyword arm (844–847) — a param typed
      `AS Nothing`; invalid-identifier error (848–852) — a type position holding a
      non-name token.
- [x] **`parse_map_literal` / `parse_set_literal` error arms** (missing `{`,
      missing `:=` in map) if the list flags them.

Note: `expr.rs` already carries `coverage:off/on` around the `unreachable!()`
match-guard arms (`parse_or`, `parse_comparison`, `parse_addition`,
`parse_multiplication`) and the defensive empty-`args` guard (692–703) — those
are excluded from the denominator, so do **not** chase them.

Acceptance: `sh scripts/coverage.sh` (fresh) then
`sh scripts/coverage-check.sh src/ast/expr.rs` shows ≥95%.

NOTE (fresh lcov): most enumerated branches (XOR, MOD/DIV, with_update errors,
non-identifier callee/constructor, `finish_qualified_name` 3-part, ISOLATED FUNC,
grouped type, map/set/thread type shapes, resource-plane STATE, function-type
errors, lambda, `type_base_name` Nothing/invalid) are ALREADY covered — their
lines are absent from the `DA:LINE,0` list. The genuinely-red set was: expr-depth
guard (`enter_expr` false body + the four `return None` bails: parse_expression 45,
parse_not 114, parse_power 236, parse_unary 256), type-depth guard (`enter_type`
false body 569-582 + `parse_type_name` bail 597), parse_primary Eof guard 460-466,
Scalar primary 472, Set-`RES`-literal 508-514, `parse_map_literal` missing-`{` 882,
`parse_set_literal` missing-`{` 913 + element loop 918-921, and the DEFENSIVE
`detail`-selection arms 405/406 (`parse_argument_list`) + 445
(`parse_constructor_argument_list`) — those two fns are each only ever called with
one closing token, so those arms are unreachable through the grammar; covered by a
direct `pub(super)` call whose leading token is the requested closing delimiter
(not a fabricated grammar path). Added an inline `mod tests` (13 tests). The
`coverage:off` `unreachable!()` arms (78/148/187/215) and empty-args guard
(695-701) were left alone per the note above.
Commit: 401dbfd91

### Phase G3 — `src/ast/link_items.rs` (81 uncov)

`LINK`/`CSTRUCT`/native-`FUNC`/`ABI`/`BIND`/`FREE`/`BUFFER`/`CONST` grammar. The
happy paths are exercised by the link fixtures; uncovered lines are the error /
`synchronize` / unterminated-block arms and a few rare clauses. Each is reached
by a crafted `LINK … END LINK` source. Confirm against the `DA:LINE,0` list, then
cover:

- [x] **`parse_link_block`** (11–20, 22–24, 26–28, 66–74, 75–79): non-string
      library name; missing `AS`; missing alias; an unexpected statement inside
      the block; EOF before `END LINK` (unterminated).
- [x] **`parse_cstruct`** (108–117, 127–131, 132–136, 145–151): `END` not naming
      `CSTRUCT`; missing field name; missing/invalid C type; unterminated.
- [x] **`parse_link_function`** (158–162, 166–172, 247–262, 273–275, 286–291,
      319–326, 329–336, 337–344): bad name; unclosed param list; `ERROR_ON`
      (the De-Morgan `NOT` wrap); `RETURN <e> LENGTH <e>`; duplicate `BIND STATE`
      (the "at most one" diagnostic); an unexpected clause; missing `SYMBOL`;
      missing `ABI`. Also the `RES` return with a `STATE T` clause (177–190).
- [x] **`parse_buffer_spec`** (378–386): `BUFFER slot` without `SIZE`.
- [x] **`parse_bind_state`** (398–404): `BIND STATE r struct` (missing `=`).
- [x] **`parse_bind_in`** (423–432, 441–450, 460–463, 478–484): missing `IN`;
      `END` not naming `BIND`; field missing `=`; unterminated `BIND` block. Plus
      one well-formed `BIND IN … END BIND` with a named field if the list shows
      the happy body uncovered.
- [x] **`parse_free_block`** (502–511, 522–528, 531–534, 535–541, 548–555,
      564–572): `END` not naming `FREE`; `ABI` missing `(`; missing `)`; missing
      `AS`; an unexpected statement; a `FREE` block reaching `END FREE` without a
      complete `SYMBOL`+`ABI` (`NATIVE_FREE_INVALID`). Plus one complete `FREE`
      block if its happy path is uncovered.
- [x] **`parse_abi_spec`** (588–593, 604–612, 625–628, 629–635): missing `(`; the
      `INOUT` / `OUT` / bare-`IN` direction arms; missing `)`; missing `AS`.
- [x] **`parse_const_pin`** (663–666, 667–670, 676–689): bad slot; missing `=`;
      the `SIZEOF <CStruct>` pin.
- [x] **`parse_string_literal`** (700–705): a `SYMBOL` clause whose argument is
      not a string.
- [x] **`parse_optional_state`** (710–716): the present branch (`RES … STATE T`)
      and the absent branch (bare `RES`).

Acceptance: `sh scripts/coverage.sh` (fresh) then
`sh scripts/coverage-check.sh src/ast/link_items.rs` shows ≥95%.

NOTE (fresh lcov): the actual red set was narrower than the enumeration.
`parse_free_block` (502-572), `parse_abi_spec` (588-635), `parse_string_literal`
(700-705), `parse_optional_state` (710-716), and most of `parse_link_block` /
`parse_link_function` (bad name, unclosed params, ERROR_ON, RETURN…LENGTH,
missing SYMBOL/ABI, RES+STATE return) are ALREADY covered — absent from the
`DA:LINE,0` list. The genuinely-red arms, all now covered by an inline `mod tests`
(15 tests): LINK-block CSTRUCT else-synchronize (61-62); `parse_cstruct` END-not-
CSTRUCT (109-116), missing field name (128-130), missing ctype (133-135),
unterminated (145-151); native-FUNC duplicate BIND STATE (287-291);
`parse_buffer_spec` missing slot (375-376) + missing SIZE (379-386);
`parse_bind_state` missing `=` (402-403); `parse_bind_in` missing IN (424-431),
END-not-BIND (442-449), field missing name (456-458)/`=`(461-463)/value(466-468),
unterminated (478-484); `parse_const_pin` SIZEOF pin (677-688). No production
change; no bug surfaced.
Commit: bb6ecb537

### Phase G4 — `src/ast/scope_privates.rs` (46 uncov)

Already carries an inline `#[cfg(test)] mod tests` with the broad `BROAD`
program driving most rewrite arms. Extend it. Read the `DA:LINE,0` list — the
uncovered set is the arms `BROAD` misses:

- [x] **Duplicate-path arm `Some(_) => {}`** (`scope_privates.rs:50`): build an
      `AstProject` whose `files` hold two `AstFile`s with the **same `path`** and
      run `scope_privates`; the second file's identical hash hits this arm. (Use
      `parse_file` twice into a hand-built `AstProject`, not `project_from_src`.)
- [x] **Any rewrite arm the fresh list flags** in `rewrite_stmt` / `rewrite_expr`
      / `rewrite_test_group` (e.g. `Statement::StateAssign` at :340,
      `MatchPattern::OneOf` / `Union` / `Else`, specific `Expression` arms):
      add the missing construct to `BROAD` (or a focused second fixture) so the
      private name flows through that arm, and assert the mangled name appears in
      `to_json()`. Cross-check each added arm against the `DA:LINE,0` list.
- [x] **Toolchain-file skip** (75–77, `file.internal || path.starts_with('<')`):
      confirm covered by the prelude that `project_from_src` appends; if the list
      shows it uncovered, assert a project whose only private-bearing file is
      `internal` is left untouched.

**Flag for plan-68-A (candidate unreachable arm):** the
`PRIVATE_PATH_HASH_COLLISION` arm (`scope_privates.rs:41–49`,
`Some(prev) if prev != &file.path`) fires only on a genuine `file_scope_hash`
collision between two **distinct** paths — not constructible from a unit test
without a found hash collision. If the fresh list still shows lines 41–49
uncovered after G4, it is a defensive/unreachable arm: it should get a
`coverage:off/on` fence (matching the `expr.rs` precedent) OR a documented
exception, decided by A — **not** chased with a fabricated fixture. Record the
disposition in Corrections.

Acceptance: `sh scripts/coverage.sh` (fresh) then
`sh scripts/coverage-check.sh src/ast/scope_privates.rs` shows ≥95% (or ≥95% of
the reachable denominator once A fences the collision arm).

NOTE (fresh lcov, 46 red). Covered by 8 new tests (32 lines): duplicate-path
`Some(_) => {}` (50); `item_line` Function/Type/Resource/FuncAlias arms (132-135)
via PUBLIC+PRIVATE shadow pairs; `item_line` Link/Doc/Testing `=> 0` arm (136) via
a DIRECT call (its only production caller, the shadow path, never passes those
variants); function-param STATE rewrite (175); resource `close_fn`→private (236);
LINK-block param/return type+STATE + cstruct `maps_to` rewrite (245-264); LET/RES
binding STATE rewrite (327); SetLiteral element_type+elements rewrite (505-511).
RESIDUAL uncovered, all documented — file still reaches ~97% (466/480):
- 41-49 `PRIVATE_PATH_HASH_COLLISION`: EXCEPTION-CANDIDATE for A — a distinct-path
  `file_scope_hash` collision, not constructible from a unit test. Not fabricated.
  A decides fence-vs-exception (cargo-llvm-cov 0.8.7 ignores inline markers → A
  will likely use the exception FILE).
- 451, 474: llvm-cov CLOSING-BRACE ARTIFACTS — the enclosing `if let Some(mangled)`
  blocks execute 14× (DA:447-449 = 14, DA:472 = 2) yet the trailing `}` line
  reports 0. NOT coverable by any test; flag for A alongside the collision arm.
- 720, 731, 776: pre-existing TEST-MODULE panic-message argument expressions
  (`assert!(…, "…", <expr>)`) that only evaluate on assertion failure; uncoverable
  on a green run. Left untouched (not our target; would require failing an
  existing test).
Commit: 3b0e5bf73

### Phase G5 — `src/ast/serialize.rs` (49 uncov)

**One-direction dumper only** — `serialize.rs` holds AST→JSON `to_json`
exclusively; there is **no** `from_json`/deserialize anywhere in `src/ast`
(`grep -rn "from_json\|deserialize" src/ast` → none), so this is *not* a
round-trip file. Every uncovered line is an **AST-node variant / arm never
serialized** by an existing test. Cover each by parsing a source that yields the
node, then asserting on `to_json`. Confirm against the `DA:LINE,0` list:

- [x] **`visibility_prefix` / `visibility_name`** (Export + Private arms): an
      `EXPORT` and a `PRIVATE` declaration.
- [x] **`exit_target_name`** (for/do/while/sub/func/program): `EXIT FOR`,
      `EXIT DO`, `EXIT WHILE`, `EXIT SUB`, `EXIT FUNC`, `EXIT PROGRAM`.
- [x] **`DocBlock::to_json`** (167–265, a ~100-line block): a `DOC` block with
      non-empty `attrs`, `desc` prose, `deprecated`, `group`, `args`/`props`
      (named lists), `ret`, `errors`, `example`, and both `header_params` =
      `Some` (a signature) and `None`. Assert the emitted `"kind": "doc"` object
      carries each field.
- [x] **`LinkFunction::to_json`** (453–…): a native `FUNC` exercising
      `bind_in`, `bind_state`, `buffers`, `free`, `result_length`, and
      `return_state_type` present (reuse a G3 fixture). Plus `CStructDecl` /
      `CStructField` / `AbiSlot` (each direction) / `ConstPin` / `FreeSpec` /
      `BindState` serializers if the list flags them.
- [x] **`signature_line` pub fns** (Function / TypeDecl / ResourceDecl): these
      have out-of-crate callers (`src/ir/docs.rs`, `src/doc/mod.rs`,
      `src/ast/tests.rs`) so may already be covered — only add a direct
      `assert_eq!` on the returned string for the specific `kind`/`FunctionKind`/
      `TypeDeclKind` arm the list shows uncovered.
- [x] **`Statement::to_json` arms** the list flags (e.g. the loop / `MATCH` /
      `FAIL` / `EXIT` variants) and **`Expression::to_json` arms**: `WithUpdate`,
      `SetLiteral`, `MapLiteral`, `Trapped`, `Lambda` with **and** without
      `assign_target`, `MemberAccess`, `Scalar`.
- [x] **`MatchPattern::to_json`** (`Literal` / `OneOf` / `Union` / `Else`) and a
      guarded `MatchCase`; **`resource_json_suffix`** with and without a
      `STATE T`.

Acceptance: `sh scripts/coverage.sh` (fresh) then
`sh scripts/coverage-check.sh src/ast/serialize.rs` shows ≥95%.

NOTE (fresh lcov, 49 red — much narrower than the enumeration). `DocBlock`,
`signature_line`, `MatchPattern`, `resource_json_suffix`, `visibility_*`,
`exit_target_name`, and most `Statement`/`Expression` arms are ALREADY covered
(absent from `DA:LINE,0`). The genuinely-red arms, covered by 4 new tests in an
inline `mod tests`: `impl ToJson for AstFile` trait delegate (46-48) via an
explicit `ToJson::to_json` call (no production caller — AstProject uses the
inherent method); the LINK sub-serializers `CStructDecl` (328-353),
`CStructField` (357-376), `BindIn` (382-404), `BindInField` (408-429),
`BindState` (433-450) via one LINK+CSTRUCT+FUNC(BIND IN + BIND STATE) fixture;
`Expression::Scalar` (1289-1290); `Expression::SetLiteral` element loop
(1391-1401). No production change; no bug surfaced.
Commit: f97061251

## Validation Plan

- **Per-file:** after each phase, `sh scripts/coverage.sh` (rebuilds the profile
  the checker reads) then `sh scripts/coverage-check.sh <path>` shows the file
  ≥95%. The checker is filter-aware; a single trailing path substring scopes it.
- **Letter-wide:** `sh scripts/coverage-check.sh src/ast/` lists none of the five
  files as a GATE FAILURE (each ≥95% or, for the collision arm only, fenced by A).
- **Suite:** `cargo test` → `0 failed` (run the whole suite, never one module —
  overview §4). New fixtures must not regress any existing test.
- **No source-behavior change:** `git diff --stat` shows edits confined to the
  five files' `#[cfg(test)]` modules (plus `src/ast/tests.rs` if used) — unless a
  fixture uncovered a real defect, which is fixed on its own RED-first commit per
  `AGENTS.md`, recorded in Corrections.
- **Exceptions:** this letter adds **no** whole-file exception. The only
  exception candidate is the `PRIVATE_PATH_HASH_COLLISION` arm (G4), which A —
  not G — dispositions (fence or documented exception with the collision boundary
  named).

## Corrections

<Filled during execution — especially: any candidate branch the fresh
`DA:LINE,0` list showed uncovered that this doc's enumeration missed; the
disposition of the G4 collision arm; and any real defect a fixture surfaced.>
