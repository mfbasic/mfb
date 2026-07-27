# plan-68-H: leaf/misc modules + repository crate

Last updated: 2026-07-27
Overall Effort (AI): large (3h–1d)   (whole plan-68 feature)
Effort (Human): large (4h–8h)
Effort (AI): medium (1h–3h)
Depends on: plan-68-A
Produces: nothing — the twelve files below each reach ≥95% and drop off the
gate's failing list; no artifact other sub-plans consume.

Part **H** of plan-68. Shared goal, prerequisites, dependency graph, the measured
55-file population, the design, and the standing requirements (tests live beside
code in `#[cfg(test)]`; run the full `cargo test`; a found bug is fixed not
worked around; never edit a golden to pass) live in the overview:
[plan-68-coverage-gate.md](plan-68-coverage-gate.md). Re-run its Prerequisites
before starting; they gate this sub-plan too. Line targets are read from A's
regenerated `target/coverage/coverage.json` — where a task below names a
*function area* rather than a line, resolve the exact arm from that fresh report
(the on-disk profile predates current `src/**`).

## Scope

H owns the leaf/misc modules and the `repository` crate — the files A's worklist
marked `backfill:H`. Twelve files, from the overview's population table:

| File | covered/total | uncov |
|---|---|---|
| src/monomorph/lower.rs | 2230/2358 | 128 |
| repository/src/main.rs | 648/815 | 167 |
| src/unicode/runtime_tables.rs | 309/385 | 76 |
| src/testing/desugar/coverage.rs | 177/214 | 37 |
| src/builtins/strings.rs | 460/494 | 34 |
| src/audit/collect/project.rs | 310/342 | 32 |
| src/docs/man/mod.rs | 216/247 | 31 |
| src/manifest/json_edit.rs | 381/406 | 25 |
| repository/src/backfill.rs | 340/358 | 18 |
| src/builtins/money.rs | 79/89 | 10 |
| src/docs/spec/mod.rs | 129/139 | 10 |
| src/json.rs | 13/14 | 1 |

All source counts above are the overview's snapshot; the authoritative current
numbers come from `sh scripts/coverage-check.sh <path>` after a fresh
`sh scripts/coverage.sh`.

### Exception candidates surfaced during authoring (hand to sub-plan A — NOT test targets here)

Reading these files exposed uncovered lines that no unit test can honestly reach.
H does **not** plan tests for them; A decides except-vs-leave with a named
boundary. H only records the finding:

- **`src/json.rs` — the single uncovered line is the `.unwrap_or_else(|_|
  "\"mfb_project\"".to_string())` fallback in `json_string` (`src/json.rs:15`).**
  It is the `Err` arm of `JsonValue::String(value.to_string()).stringify()`.
  `stringify()` of a `JsonValue::String` never returns `Err` for any `&str`
  input, so the arm is defensively unreachable from a unit test. The overview's
  Open Decision recommended covering it in H; on reading it, it is **not**
  coverable — flag to A as a one-line defensive-arm exception (boundary:
  "tinyjson `stringify` is infallible for `String`; fallback is dead"). If A
  disagrees and finds a reachable input, it moves back to phase H4.
- **`src/docs/spec/mod.rs` — the `sort_by_key` `unwrap_or(usize::MAX)` fallback
  (`src/docs/spec/mod.rs:80`)** fires only for a discovered `spec.md` package
  absent from `PACKAGE_ORDER`; all 12 shipped packages are present, so the arm is
  unreachable without adding a fake corpus directory. Flag to A (boundary:
  "generated `SPEC_PACKAGES` is a superset-free of `PACKAGE_ORDER`; sort fallback
  is dead"). The rest of spec/mod.rs IS coverable — see H4.
- **`src/manifest/json_edit.rs` — the "malformed value" `.ok_or_else` arms**
  (`json_edit.rs:150-155`, `:300-305`, and `rewrite_pin_field:331-341`) require
  an entry that parses as valid JSON yet whose field-scanning fails —
  self-contradictory. Near-unreachable; if H5's coverable gaps don't clear 95%,
  flag these to A rather than fabricate an input.
- **`repository/src/main.rs::main()` (`main.rs:48-284`, already wrapped in
  `// coverage:off`/`on`)** is the async entrypoint: reads `env::args()`, opens
  the on-disk SQLite store, constructs the S3/local blob store, binds the socket
  and calls `server::serve(...).await`, and `process::exit`s. Integration-only;
  boundary = socket bind/listen + live-HTTP dispatch (`server::serve`) + process
  argv/exit. Flag to A to confirm the existing `coverage:off` span is the right
  exception; H8 carves out the one pure helper trapped inside it.
- **`repository/src/backfill.rs` — the `BlobFetch::Redirect(_)` arm
  (`backfill.rs:71-77`)** is produced only by `s3_impl::S3BlobStore::get`
  (feature `s3`, presigned redirect); a `Local` store never yields it, so no
  in-process double reaches it. Flag to A (boundary: `feature = "s3"` presigned
  redirect). H8 covers the other three `run()` branches.

## Phases

Group by test-module cohesion. The two large files (`monomorph/lower.rs` 128,
`repository/src/main.rs`+`backfill.rs` 185) own their own phases.

### Phase H1 — builtins: money + strings

`src/builtins/money.rs` (`#[cfg(test)]` at :107) and `src/builtins/strings.rs`
(`#[cfg(test)]` at :464). Existing tests cover the metadata/`resolve_call`
tables; the gaps are the recognizer predicates and the seam-detection AST walk.

- [x] `money.rs::is_builtin_type` (:19) + `is_money_call` (:28): add a test
      asserting `is_builtin_type("Money")` is true and an unknown type false, and
      `is_money_call` true for each money callable name and false for `"nope"`.
- [x] `money.rs` `None` arms of `call_param_names` (:32), `call_return_type_name`
      (:42), `expected_arguments` (:64), `arity` (:88) for an unknown name: one
      test asserting each returns `None` for `"not_a_money_fn"`.
- [x] `strings.rs::uses_package` (:319) + the `*_references_seam` walk
      (`item_references_seam` :332, `group_references_seam` :347,
      `stmt_references_seam` :355, `expr_references_seam` :407, `callee_is_seam`
      :299): build an `AstProject` whose test body calls a strings-seam function
      through nested expression shapes (call, binary, interpolation, member
      access) and assert `uses_package(&ast)` is true; build one with no seam
      reference and assert false. This exercises the recursion arms that carry
      most of the 34 uncovered lines.
- [x] `strings.rs::augmented_project` (:448): call it on a project that uses the
      package and assert the embedded `source_file()` AST is appended (one extra
      file / the seam function is present). `source_file()`'s `Err(())` arm
      (:260) is the embedded-source-fails-to-parse guard — unreachable for the
      shipped constant; if it blocks 95%, note it to A rather than test it.

Acceptance: `sh scripts/coverage-check.sh src/builtins/money.rs
src/builtins/strings.rs` shows both ≥95% (run `sh scripts/coverage.sh` first).
Commit: 4c54aefff

### Phase H2 — audit collection: project.rs

`src/audit/collect/project.rs` (in-file `#[cfg(test)]` at :166). Existing tests
cover `collect_native_resources` derivation and `project_summary`; the untested
mass is `collect_libraries` (never referenced by any test) plus three specific
skip/populate branches. All are pure in-memory AST/manifest transforms.

- [x] `collect_libraries` (:118): manifest with a `libraries` object (one dynamic
      locator) and a `resources` array holding one valid `{ "src", "dst" }`, one
      entry missing `src` (skipped, :154), and one non-object element (skipped,
      :143). Assert one `LibraryEntry` (logical/source per the locator), one
      `ResourceFileEntry { src, dst }`; add a second out-of-order lib to assert
      the `sort_by` on `src`/`dst` (:161). Verify field names against
      `src/audit/report.rs:71,82`.
- [x] `collect_native_resources` internal-file skip (:29-31): two files — an
      `internal: true` file holding the `LINK` block, plus a `RESOURCE`; assert
      the internal file's resource is excluded while `close_may_fail` still
      resolves. Build `AstFile` directly (the `file()` helper hardcodes
      `internal: false`).
- [x] `collect_native_links` internal-file skip (:67-69): an `internal: true`
      file with a `LINK` block; assert its symbols are absent from the result.
- [x] `collect_native_links` `free: Some` branch (:79-83): a `LinkFunction` with
      `free: Some(FreeSpec { symbol: "sqlite3_free", … })` (type at
      `src/ast/types.rs:367`); assert the entry's `close_function ==
      "sqlite3_free"` (existing test only asserts the empty `free: None` case).

Acceptance: `sh scripts/coverage-check.sh src/audit/collect/project.rs` ≥95%
(fresh `sh scripts/coverage.sh` first).
Commit: 2d93e1ded

### Phase H3 — manifest: json_edit.rs

`src/manifest/json_edit.rs` has no `#[cfg(test)]` of its own; its tests live in
`src/manifest/package.rs` (`mod tests`) and two cases in `src/manifest/mod.rs`.
Add a `#[cfg(test)]` block to `json_edit.rs` (or extend `package.rs`, matching
precedent) for the uncovered surgical-edit branches.

- [ ] `insert_packages_array` `needs_comma == false` branch (:91-96): call with a
      root object that has no preceding fields, e.g.
      `insert_packages_array("{}", "ENTRY")`; assert the result inserts
      `"packages"` with **no** leading comma.
- [ ] `project_json_with_updated_version` "has no `version` field" arm (:298-299):
      a manifest whose matched entry lacks `version`; assert `Err` containing
      "has no `version` field" (existing test misses on a non-matching ident, so
      never reaches a matched-but-fieldless entry).
- [ ] `project_json_without_packages` bare-`name` fallback (:211-214): an entry
      with only `"name"` (no `"ident"`); call with that name; assert removal
      succeeds (array empties, `Ok`).
- [x] `project_json_without_packages` malformed-entry arm (:201-202): a manifest
      with an unterminated `packages` entry; assert
      `Err("malformed project.json `packages` entry")`.
- [ ] The three "malformed value" `.ok_or_else` arms (:150-155, :300-305,
      `rewrite_pin_field` :331-341) are near-unreachable (see exception
      candidates above). Attempt only if 95% is not otherwise reached; else flag
      to A.

Acceptance: `sh scripts/coverage-check.sh src/manifest/json_edit.rs` ≥95%
(fresh `sh scripts/coverage.sh` first).
Commit: 5949bcc4b

### Phase H4 — docs: man + spec

`src/docs/man/mod.rs` (`#[cfg(test)]` at :255) and `src/docs/spec/mod.rs`
(`#[cfg(test)]` at :112). NOTE: `function()`, `function_page()`, and
`is_markdown_page()` are already exercised by tests in `src/cli/man.rs`, so they
are not the gap. The man gaps cluster on the **plain-text page path** (the
shipped corpus is 100% Markdown) plus the qualified-name lookup; the spec gaps
are the pure `cross_links`/`citations` break arms plus one dead sort fallback
(flagged to A above).

man/mod.rs:
- [x] `first_synopsis_line` (:245): `first_synopsis_line("NAME\n  demo - x\n\n
      SYNOPSIS\n  demo::x(a AS Int) AS Int\n")` → `Some("demo::x(a AS Int) AS
      Int")`; a no-`SYNOPSIS` input → `None`.
- [x] `parse_rendered_function_page` plain-text `else` (:172-180): a plain-text
      page string; assert `name`/`summary`/`signature` parsed and `example` empty.
- [x] `build_package` plain-text `else` (:116-125): `build_package("demo", "mfb
      man demo", "NAME\n  demo - numeric constants\n")` → `summary == "numeric
      constants"`, `functions` empty.
- [ ] `function`/`function_page` qualified-name `strip_prefix` Some branch
      (:91-93, :102-104): `let p = package("io").unwrap();` assert
      `function(p, "io.print").is_some()` and `function_page(p,
      "io.print").is_some()` (cli tests pass only unqualified names).
- [ ] `markdown_synopsis` heading-absent `?`→`None` (:212): a Markdown page with a
      `## Description` but no `## Synopsis`; assert `None` (existing test covers
      only the empty-section early return).

spec/mod.rs:
- [ ] `cross_links` and `citations` (the pure `#[cfg(test)]` scan helpers, :206,
      :228) break/fallthrough arms: unit-test them directly with an unterminated
      `[[…` and a no-terminator input to cover the `break` / `unwrap_or(rest.len())`
      arms that the clean corpus never triggers. `summary_line` (:99) heading-skip
      and empty-fallback are already covered.
- [x] If, after the above, spec/mod.rs still sits below 95% because the remaining
      uncovered lines are only the dead sort fallback (:80) and the broken-corpus
      `broken.push` drift-guard arms, hand those to A as unreachable — do not
      fabricate a broken corpus to hit them.

Acceptance: `sh scripts/coverage-check.sh src/docs/man/mod.rs
src/docs/spec/mod.rs` ≥95% (fresh `sh scripts/coverage.sh` first). If spec
cannot reach 95% without covering a dead arm, its exception is added by A and the
file leaves the failing list that way.
Commit: 0d6f09240

### Phase H5 — unicode: runtime_tables.rs

`src/unicode/runtime_tables.rs` (`#[cfg(test)]` at :443) — hand-written OnceLock
accessors over the utf8proc corpus (NOT a generated table, NOT excluded by
`IGNORE`; confirmed by reading the head). The value-mapping arms
(`decomp_type_value`, `boundclass_value`, `indic_conjunct_break_value`,
`parse_value`, `parse_bool`) already fire when `parse_tables()` walks the full
corpus at first `tables()` call; the untested surface is the **`*_hex`
serializers** (used only by codegen, never by the runtime tests) plus the
mapping-table builders and one edge accessor.

- [x] Call every `*_hex` serializer once and assert each returns a non-empty,
      even-length hex string: `stage1_hex` (:78), `stage2_hex` (:82),
      `sequences_hex` (:86), `properties_hex` (:90), `combinations_second_hex`
      (:115), `combinations_combined_hex` (:119), `nfd_entries_hex` (:123),
      `nfd_sequences_hex` (:134), `uppercase_entries_hex` (:138),
      `uppercase_sequences_hex` (:142), `lowercase_entries_hex` (:146),
      `lowercase_sequences_hex` (:150), `casefold_entries_hex` (:154),
      `casefold_sequences_hex` (:158). This drives `mapping_entries_hex` (:233),
      `u16_hex`/`u32_hex` (:360/:368), and `build_mapping_tables` (:207) for
      uppercase/lowercase/casefold, plus `build_nfd_tables` (:203).
- [x] `property_for_codepoint` (:108): assert a known combining mark (U+0301
      COMBINING ACUTE ACCENT → `combining_class == 230`) and an out-of-range
      codepoint (e.g. `0x11_0000`) returns the default `PackedProperty` — covering
      the range-guard arm.

Acceptance: `sh scripts/coverage-check.sh src/unicode/runtime_tables.rs` ≥95%
(fresh `sh scripts/coverage.sh` first).
Commit: 4de16d8c1

### Phase H6 — testing/desugar: coverage.rs

`src/testing/desugar/coverage.rs` — the file has no `#[cfg(test)]` of its own;
its tests live in `src/testing/desugar/mod.rs` (`#[cfg(test)]` at :55, one direct
`instrument_block` case) and `src/testing.rs` (full-pipeline
`coverage_mode_instruments_statements_and_adds_runtime_helpers`). The existing
fixtures only feed `Let`/`If`/`Expression` statements, so `is_generated`,
`statement_line`, `coverage_helpers` (both globals + SUBs), `dump_list_to_file`
(both `numeric` values), the top-level `internal`/`<…>`/generated skips, and the
`instrument_trapped_handler` true-branch on a bare `Expression` are **already
covered** — do NOT re-target them. The 37 uncovered lines concentrate in the
`instrument_nested` (:77) statement-kind arms and the `Assign`/`Return`
trapped-value handler arms that those fixtures never build. Add a `#[cfg(test)]`
block (or extend the mod.rs one) that calls the already-exported
`instrument_block` directly with hand-built statements:

- [x] Loop arm `For | ForEach | While | DoUntil` (:87-90): a block with a
      `Statement::For { body: [expr-stmt at line 10], line: 5, … }`; call
      `instrument_block`; assert `slots` carry both `line == 10` (body) and
      `line == 5` (the loop statement). One fixture covers the whole OR-group;
      optionally add a `ForEach`/`While`/`DoUntil` twin.
- [x] `Match` arm (:91-95): `Statement::Match { cases: [MatchCase { body:
      [expr-stmt at line 20], … }], line: 8 }`; assert slots with `line == 20`
      and `line == 8`.
- [x] `Assign`/`StateAssign` inline-TRAP value arm (:109-111 →
      `instrument_trapped_handler` :121 through the Assign path): `Statement::
      Assign { value: Expression::Trapped { handler: [expr-stmt at line 30], … },
      line: 12 }`; assert slots with `line == 30` and `line == 12`. Add a
      `StateAssign` twin for the second OR-alternative.
- [x] `Return { value: Some(Trapped …) }` arm (:105-108): `Statement::Return {
      value: Some(Expression::Trapped { handler: [expr-stmt at line 40], … }),
      line: 15 }`; assert slots with `line == 40` and `line == 15`.

Acceptance: `sh scripts/coverage-check.sh src/testing/desugar/coverage.rs` ≥95%
(fresh `sh scripts/coverage.sh` first).
Commit: 5e5a2d0f8

### Phase H7 — monomorph: lower.rs (128 uncovered — big)

`src/monomorph/lower.rs` (`#[cfg(test)]` at :1915) already carries a large,
fixture-rich suite driven by `monomorphize(src)` / `monomorphize_files(files)`,
which run the full `monomorphize_project` pass — so every private lowering arm is
reachable from an MFB source snippet (that is what "unit-coverable" means here).
The free helpers (`unify_type`, `mangle_name`, `overload_key`, `params_match`, …)
are already covered by `src/monomorph/helpers.rs`'s own test module — do NOT
re-target them. The 128 uncovered lines are less-common statement/expression/
type-shape arms. Tasks are ordered highest-value-first; pin exact lines from A's
fresh report:

- [ ] `lower_statement` (:812) untested statement arms — one fixture, six arms:
      a function body using `EXIT` (:876), `CONTINUE` (:883), `FAIL` (:887),
      error propagation/`PROPAGATE` (:891), `RECOVER` (:892), and a resource
      `STATE` assignment `StateAssign` (:916), each wrapping a generic call so
      lowering is observable; assert the bodies lower and the inner generic
      instantiates.
- [ ] `expression_type` (:1766) inference arms: put each expression where its
      inferred type drives a generic/overload choice — `Fixed`/`Money` numeric
      literals (:1777-1778), `NOTHING`→`Nothing` (:1784), `SetLiteral`→`Set OF …`
      (:1808), `MapLiteral`→`Map OF … TO …` (:1811), assign-bodied lambda→`Nothing`
      (:1851), `NOT`→Boolean (:1880), and a `Trapped` expression in a typed
      position (:1886).
- [ ] `concrete_type_name` (:1523) + `template_view_type` (:1606) shape arms:
      declare a generic type/param of each shape and instantiate — `Set OF`
      (:1541), `MapEntry OF … TO …` (:1559), THREAD types via `thread_parts_full`
      (:1568), `ISOLATED FUNC(…) AS …` (:1580-1595), and a grouped `(T)` via
      `strip_type_group` (:1531); assert the lowered field/concrete type name.
- [x] `lower_expression` (:1114) literal arms: a `SetLiteral` (:1363) and a
      `MapLiteral` (:1377) each holding a generic call as element/value; plus the
      constructor-inference `TypeDeclKind::Union`/`Enum` arms (:1306) via a
      generic UNION/ENUM value with no expected type. (The adjacent
      `unreachable!()` at :1301 is a defensive guard — leave uncovered / flag to A.)
- [ ] `into_project` (:299) item pass-through: source containing a `RESOURCE`, a
      native `LINK`, a `DOC` block, and a func alias; assert they survive verbatim
      (arms :299-312). Plus the import-union `seen.insert` push (:339-348) via
      `monomorphize_files` with two files importing *different* bindings; assert
      the first file's imports hold the union. Plus the generated-*type* sort
      (:350-358) by instantiating one generic TYPE at two arg types.
- [x] `instantiate_function` (:561) return-type-only-unresolvable branch
      (:607-636): `FUNC make OF T() AS T` called bare with no expected type;
      assert the "appears only in the return type" error and the call left
      unresolved. And both recursion-depth guards + `report_instantiation_too_deep`
      (:1904): a self-instantiating generic function (`f(x)` → `f([x])`) past the
      256-deep `MAX_TEMPLATE_INSTANTIATION_DEPTH` (:648) and a self-referential
      generic TYPE hitting `instantiate_type`'s guard (:775); assert
      `TYPE_INSTANTIATION_TOO_DEEP` fires.
- [ ] `unique_concrete_symbol` (:537) mangling-collision suffix loop (:542-554,
      bug-226): instantiate one generic at two argument tuples that `mangle_name`
      sanitizes to the same symbol; assert two distinct concrete names (one
      suffixed `$2`). Highest-effort arm — attempt last.

Because the file is already ~94.6%, each fixture buys only a few lines. Genuinely
dead arms A's report may surface — the `strip_qualifier_prefixes` empty-qualifier
early return (:8-9), the `report` zero-source `"src/main.mfb"` fallback (:1900) —
are flagged to A, not tested.

Acceptance: `sh scripts/coverage-check.sh src/monomorph/lower.rs` ≥95% (fresh
`sh scripts/coverage.sh` first).
Commit: b7b8639dd

### Phase H8 — repository crate: main.rs + backfill.rs

`repository/src/main.rs` (`#[cfg(test)]` at :547) and `repository/src/backfill.rs`
(`#[cfg(test)]` at :165). NOTE: the five `parse_*` arg helpers and the two
store-operation helpers in main.rs are already fully unit-tested; the ~167
uncovered main.rs lines are the body of `main()` (`:48-284`), which is
integration-only and already `// coverage:off`-wrapped — flagged to A above. Only
one pure helper is trapped inside it, and three of backfill's four dark branches
are unit-coverable with the test builders already in the file.

main.rs:
- [x] Extract the `--expires-days` overflow/positivity guard currently inline in
      `main()` (`main.rs:111-121`, the `bug-276 R10` check
      `checked_mul(24*3600).and_then(checked_add).filter(|_| days > 0)`) into a
      free helper, e.g. `fn init_root_expires_at(expires_days: i64, now: i64) ->
      Result<i64, String>`, and call it from `main`. Then unit-test it:
      `init_root_expires_at(365, 1000)` → `Ok(1000 + 365*24*3600)`;
      `(0, now)` → `Err` containing "must be a positive"; `(-5, now)` → `Err`;
      `(i64::MAX, now)` → `Err` (checked_mul overflow). This is a pure refactor
      of coverable logic out of the integration entrypoint — no behavior change.
- [x] Confirm with A that the remainder of `main()` stays under `// coverage:off`
      with boundary = socket bind/listen + live-HTTP `server::serve` + process
      argv/exit (see exception candidates above). No test is written for it.

backfill.rs (extend the in-file test module using its existing
`container`/`serialize`/`payload_for` builders):
- [x] `abi::parse_manifest_metadata` `Err` arm (:98-103): a `.mfp` that parses as
      a package but whose section-1 manifest payload is malformed/short; publish
      and run; assert `report.unparseable == 1`, `report.updated == 0`, and the
      skip label is recorded.
- [x] `abi::parse_vendor_blobs` `Err` arm (:117-124): a payload with a valid
      manifest but a truncated/malformed section-10 vendor table; assert
      `report.unparseable == 1` and no target rows written.
- [x] `signed == None` metadata arm (:140-143): a payload where
      `parse_manifest_metadata` returns `Ok(None)` (no MANIFEST author/url);
      assert `report.updated == 1`, author/url stay `None`, description still
      populated from section 18. First verify against `repository/src/abi.rs`
      that the crafted shape yields `Ok(None)` rather than `Err`; if it errors,
      this collapses into the unparseable case above.
- [x] The `BlobFetch::Redirect(_)` arm (:71-77) is S3-only — flagged to A above,
      not tested here.

Acceptance: `sh scripts/coverage-check.sh repository/src/main.rs
repository/src/backfill.rs` ≥95% (fresh `sh scripts/coverage.sh` first). main.rs
reaches 95% via the extracted helper plus the confirmed `coverage:off` span;
backfill.rs via the three unit-coverable branches (its `Redirect` exception,
added by A, removes the last dark line from the gate).
Commit: 7868f0ad0

## Validation Plan

- **Per file:** after `sh scripts/coverage.sh`, `sh scripts/coverage-check.sh
  <path>` shows the file ≥95% (or, for the flagged dead arms, the file left the
  failing list because A excepted it).
- **Whole H set:** `sh scripts/coverage-check.sh src/monomorph/lower.rs
  src/builtins/money.rs src/builtins/strings.rs src/docs/man/mod.rs
  src/docs/spec/mod.rs src/unicode/runtime_tables.rs
  src/testing/desugar/coverage.rs src/manifest/json_edit.rs src/json.rs
  src/audit/collect/project.rs repository/src/main.rs repository/src/backfill.rs`
  lists no GATE FAILURE.
- **Suite:** `cargo test` → `0 failed` (the full suite, never a single module —
  new tests must not regress it).
- **Exception hand-off:** the five findings under "Exception candidates" reach A
  with their named boundaries; H does not add any line to
  `scripts/coverage-exceptions.txt` itself.
- **No production behavior change** except the pure `init_root_expires_at`
  extraction in H8 (a coverable-logic refactor, no semantic change) and any real
  bug a coverage test uncovers (fixed on its own RED-first commit per `AGENTS.md`,
  not worked around).

## Corrections

Executed 2026-07-27 in worktree P-68-H (branch `worktree-P-68-H`). Full `cargo
test` green after each commit (final: 3771 passed, 0 failed). Coverage verified
centrally by the parent; per-file line targets were taken from A's fresh
`lcov.info`, which differs materially from the stale line numbers this sub-plan
was authored against — several enumerated sub-tasks pointed at lines that are
**already covered** in the current profile, so they were not re-tested (boxes
left unchecked). Tasks were re-aimed at the fresh uncovered set.

Commits: H1 4c54aefff · H2 2d93e1ded · H3 5949bcc4b · H4 0d6f09240 ·
H5 4de16d8c1 · H6 5e5a2d0f8 · H7 b7b8639dd · H8 7868f0ad0.

- **H1 (money/strings):** added the `call_return_type_name`/`expected_arguments`/
  `arity`-None arms for money, and for strings the scalar-seam `resolve_call`
  family, the seam-walk recursion arms (For/DoUntil/Lambda via a no-seam body +
  nested TESTING group), and `augmented_project`/`source_file`. `strings.rs`
  `source_file()` `Err(())` arm is unreachable for the shipped constant (already
  100%-parsing) — not tested; not blocking.
- **H2 (audit/project.rs):** added `collect_libraries`, the internal-file skips
  in both collectors, and the `free: Some` link branch.
- **H3 (json_edit.rs):** fresh uncovered = 191, 202, 268, 276, 398–406, 434, 436
  (NOT the plan's 91–96/211–214/298–299, which are covered). New `#[cfg(test)]`
  module tests the missing-array + malformed-entry error paths and drives
  `find_json_punct`/`matching_json_delimiter` directly over quoted/escaped
  strings. The three "malformed value" `.ok_or_else` arms (150-155/300-305/
  331-341) were NOT in the fresh uncovered set — no exception needed.
- **H4 (man/mod.rs, spec/mod.rs):** man — added `build_package`/
  `parse_rendered_function_page` plain-text branches, `first_synopsis_line`, and
  a `markdown_synopsis` fall-off-EOF `None`. Man's residual uncovered are the
  `man_citations_resolve` broken-corpus drift-guard arms (294/298/305/308-311/
  323) — test code that only fires on a corrupt corpus; file still clears 95%.
  **spec/mod.rs: no coverable lines remain.** All 10 fresh-uncovered lines
  (148/151/155/159-162/171/193/201) are the `broken.push`/`join` arms inside the
  `spec_links_resolve`/`spec_citations_resolve` drift-guard tests; `cross_links`,
  `citations`, and `summary_line` are already fully covered, and the :80
  `sort_by_key` fallback is ALSO already covered in the fresh profile. → **flag
  to A as an exception** (boundary: "drift-guard test arms fire only on a corrupt
  embedded spec corpus; the shipped corpus resolves 100%"); do not fabricate a
  broken corpus.
- **H5 (unicode/runtime_tables.rs):** one test drives every `*_hex` serializer;
  `u16_hex`/`u32_hex` LE checks; direct calls to `boundclass_value` (E_ZWG arm),
  `decomp_type_value`, `indic_conjunct_break_value`, `parse_bool`; and
  `#[should_panic]` tests for the four defensive panics. `property_for_codepoint`
  was already covered.
- **H6 (testing/desugar/coverage.rs):** new `#[cfg(test)]` module; one
  `instrument_block` fixture carrying every statement variant covers all
  `instrument_nested` compound arms, both value-bearing `instrument_trapped_handler`
  paths, the `_ => {}` leaf arm, and every `statement_line` arm; a second test
  drives `instrument_coverage` on a function with a FUNC-level TRAP (lines 27/29).
- **H7 (monomorph/lower.rs):** file was 94.57% (2230/2358) — only ~11 covered
  lines needed. Two fixtures added: `FUNC make OF T() AS T` called bare (the
  return-type-only-unresolvable error, lines 619-635) and a generic `Set OF T`
  body (SetLiteral lowering 1363-1375 + `concrete_type_name` Set arm). The other
  enumerated H7 tasks (lower_statement arms, expression_type, full
  concrete_type_name, into_project, recursion-depth guard, unique_concrete_symbol
  bug-226 collision) were NOT needed to clear 95% and were not written. Residual
  uncovered includes ~7 test-module lines (2175-2780) and the flagged dead arms
  (:9 empty-qualifier early return, :1900 report fallback).
- **H8 (repository main.rs + backfill.rs):** extracted `init_root_expires_at(
  expires_days, now) -> Result<i64,String>` out of `main()` (pure refactor, no
  behavior change) and unit-tested its positivity/overflow guards. backfill:
  added tests for the `parse_manifest_metadata` Err (short section-1),
  `parse_vendor_blobs` Err (truncated section-10), and the `signed == None`
  metadata arm (no section-1, description still captured). `main()` body stays
  under the existing `// coverage:off` (socket/HTTP/argv boundary — confirmed
  correct); `backfill.rs` `BlobFetch::Redirect` (:71-77) is S3-only and left
  uncovered, but backfill still clears 95% without it.

### Exception candidates handed to A

1. **src/docs/spec/mod.rs — WHOLE FILE (10 lines: 148, 151, 155, 159-162, 171,
   193, 201).** All are `broken.push`/`broken.join` arms in the
   `spec_links_resolve`/`spec_citations_resolve` drift-guard tests; unreachable
   without a corrupt embedded corpus. The file cannot reach 95% by any honest
   unit test. Boundary: "drift-guard arms fire only on a corrupt spec corpus."
2. **src/builtins/strings.rs:260 `source_file()` `Err(())`** — the embedded
   constant always parses; defensively unreachable. (File clears 95% without it.)
3. **repository/src/backfill.rs:71-77 `BlobFetch::Redirect`** — S3-presigned-only;
   a `Local` store never yields it. (File clears 95% without it.)
4. **src/monomorph/lower.rs** residual: :9 empty-qualifier early return and :1900
   zero-source `"src/main.mfb"` report fallback are dead; ~7 test-module lines
   (2175-2780) are unexercised assertion/panic branches. (File clears 95%.)
5. **repository/src/main.rs:48-284 `main()`** — confirmed the existing
   `// coverage:off` span is correct (async socket bind/listen + live-HTTP
   `server::serve` + process argv/exit). The one coverable helper was extracted
   (H8).

No real bug was surfaced by any coverage test. No golden/existing test was
edited. The only production change is the H8 `init_root_expires_at` extraction
(pure refactor).
