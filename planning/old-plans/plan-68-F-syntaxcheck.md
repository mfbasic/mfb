# plan-68-F: syntaxcheck front-end

Last updated: 2026-07-27
Overall Effort (AI): large (3h–1d)   (whole plan-68 feature)
Effort (Human): medium (2h–4h)
Effort (AI): small (<1h)
Depends on: plan-68-A
Produces: nothing — test-only. Brings the four `src/syntaxcheck/*.rs`
files below the floor back to ≥95%; no source-behavior change, no new
exception. (If a fixture uncovers a real defect it is fixed on its own
RED-first commit per AGENTS.md, not worked around.)

Part **F** of plan-68. Shared goal, prerequisites, dependency graph, the
worklist, and the standing requirements (tests live in `#[cfg(test)]`
beside the code; run the whole `cargo test`; never except a coverable
body) live in the overview: [plan-68-coverage-gate.md](plan-68-coverage-gate.md).
Re-run the Prerequisites there before starting. F consumes A's fresh
`target/coverage/coverage.json` and worklist; do not re-litigate A's
except-vs-backfill calls.

## Scope (from `sh scripts/coverage-check.sh`, overview §2 table)

| File | covered/total | pct | uncov |
|---|---|---|---|
| src/syntaxcheck/link.rs | 350/686 | 51.02 | 336 |
| src/syntaxcheck/inference.rs | 1709/1824 | 93.70 | 115 |
| src/syntaxcheck/helpers.rs | 624/664 | 93.98 | 40 |
| src/syntaxcheck/mod.rs | 1558/1654 | 94.20 | 96 |

syntaxcheck is pure parse→check analysis: every uncovered line is an
un-exercised type-inference branch, LINK/import resolution path, or error
diagnostic reachable from a source-string fixture. No syscall/subprocess/GUI
boundary exists here, so **none of these four files is exception-eligible** —
all four are pure backfill.

**link.rs (51%, 336 uncov) is the priority and the bulk.** It is the only
one of the four with **no `#[cfg(test)]` module at all** (`grep -n
"cfg(test)" src/syntaxcheck/link.rs` → nothing). Almost every `self.report`
branch in it is unhit; the only lines currently covered are those reached
incidentally by other files' tests (e.g. `NATIVE_CPTR_ESCAPE` is hit 3× from
`src/syntaxcheck/helpers.rs`). So F1 is "author link.rs's test module from
scratch," one fixture per diagnostic. The other three are high already; their
residual is a handful of named branches to be read out of A's fresh report.

### Fixture harness (precedent — reuse verbatim, do not add scaffolding)

`crate::testutil` already provides everything (`src/testutil.rs:75-90`):

- `check_src(src) -> Vec<String>` — parse `src` as `main.mfb`, run the full
  checker, return the emitted rule codes.
- `accepts(src) -> bool` — no rejections.
- `rejects_with(src, "RULE") -> bool` — `RULE` is among the codes.

Every fixture is a whole-program `&str` ending in a valid `FUNC main AS
Integer … END FUNC`. The LINK/RESOURCE/CSTRUCT/ABI shapes come straight from
the two existing LINK tests in `src/syntaxcheck/helpers.rs:670-720` (the
`demoLink` pattern) and the on-disk fixtures under
`tests/rt-behavior/native/*/src/main.mfb`. Assertions use the exact rule
string the branch passes to `self.report(...)`. Inference/helpers/mod branches
that have no `report` are driven the way the existing helpers tests do it —
run a program whose types force the branch and assert `accepts(...)` (or a
specific downstream diagnostic).

### Unreachable-arm candidates (flag to A, do NOT chase in F)

Read against A's fresh report before writing a fixture for these — if the
line is a required-for-exhaustiveness arm no source can name, it is an
**exception candidate for A**, not an F target:

- `src/syntaxcheck/link.rs:844` — `native_function_sig` `param.type_name …
  unwrap_or(Type::Unknown)`: a parsed LINK-func parameter always carries a
  type, so the `Unknown` fallback may be un-source-nameable. Confirm from the
  fresh report; if it stays red after F1's fixtures, flag to A.
- `src/syntaxcheck/inference.rs` / `mod.rs` — any `Type::Result` /
  internal-only match arm that no user source can spell (the class the
  overview and AGENTS.md call out). F names only the arms A's fresh line data
  shows are *reachable*; genuinely-unreachable arms go to A's exception pass.

Everything enumerated in F1 below is a **reachable** `self.report` diagnostic
triggered by a malformed but parseable declaration — backfill, not exception.

## Phases

### Phase F1 — link.rs: author the `#[cfg(test)] mod tests` module (the bulk)

link.rs has no test module. Add one at the end of the file (after
`native_function_sig`), `use crate::testutil::*;`, mirroring the LINK-fixture
style in `src/syntaxcheck/helpers.rs:670`. One `#[test]` per diagnostic below;
each names the `link.rs` line, the rule, and a fixture shape. Source lines are
from the current HEAD read; confirm the still-red set against A's fresh report
first (`sh scripts/coverage.sh`).

RESOURCE / built-in shadow:

- [x] `RESOURCE_SHADOWS_BUILTIN` (link.rs:68) — `RESOURCE File CLOSE BY
      demoLink::close` (reuses a built-in resource name; `is_resource_type`
      true). `assert!(rejects_with(src, "RESOURCE_SHADOWS_BUILTIN"))`.
- [x] `collect_native_resources` register path (link.rs:26-45) + a *clean*
      user `RESOURCE Db CLOSE BY demoLink::close` with a matching LINK close
      that has `SUCCESS_ON` (→ `close_may_fail=true`) and one without
      (→ false). `assert!(accepts(src))` — drives both `unwrap_or(false)` and
      the `success_on.is_some()` map.

CSTRUCT declarations / escape (`check_link_cstructs`, `check_cstruct_escape`):

- [x] `NATIVE_CSTRUCT_INVALID` (link.rs:406) — a LINK block declaring the same
      `CSTRUCT` name twice. `rejects_with(…, "NATIVE_CSTRUCT_INVALID")`.
- [x] `NATIVE_CSTRUCT_ESCAPE` param arm (link.rs:370) — a wrapper `FUNC`
      parameter typed as a declared CSTRUCT name.
- [x] `NATIVE_CSTRUCT_ESCAPE` return arm (link.rs:384) — a wrapper returning a
      declared CSTRUCT name.
- [x] `crate::ir::check_cstruct` fault forwarding (link.rs:421-430) — a CSTRUCT
      whose field uses a bad ctype, asserting the forwarded rule fires and is
      pointed at the field line (any `check_cstruct` fault rule).

Struct slots / BIND IN (`check_struct_slots`, `record_fields_of`):

- [x] `NATIVE_ABI_UNKNOWN_CTYPE` INOUT-non-struct arm (link.rs:196) — a scalar
      slot marked `INOUT` whose ctype is not a CSTRUCT.
- [x] `NATIVE_STRUCT_FIELD_MISMATCH` maps-to-non-record (link.rs:210) — a
      `CSTRUCT` whose `MAPS` target is a union/enum `TYPE`, not a record
      (drives `record_fields_of` → `None`).
- [x] `NATIVE_ABI_RESULT_MARKER` returns-IN-slot (link.rs:240) — a wrapper that
      `RETURN`s a struct slot declared `IN`.
- [x] `NATIVE_STRUCT_FIELD_MISMATCH` return-type-mismatch (link.rs:251) —
      returns a struct slot but the wrapper `return_type` ≠ the CSTRUCT's
      mapped record.
- [x] `crate::ir::check_struct_slot` fault forwarding (link.rs:232) — a CSTRUCT
      whose field/record scalar layout disagrees (any forwarded rule).
- [x] `NATIVE_BIND_IN_INVALID` unknown-slot (link.rs:268) — `BIND IN` names an
      ABI slot that does not exist.
- [x] `NATIVE_BIND_IN_INVALID` non-struct-slot (link.rs:280) — `BIND IN` names
      a slot whose ctype is not a CSTRUCT.
- [x] `NATIVE_BIND_IN_INVALID` OUT-slot (link.rs:292) — `BIND IN` writes an
      `OUT` slot.
- [x] `NATIVE_BIND_IN_INVALID` unknown-field (link.rs:305) — `BIND IN` sets a
      field the CSTRUCT does not declare.
- [x] `NATIVE_BIND_IN_INVALID` duplicate-field (link.rs:316) — `BIND IN` sets
      the same field twice.
- [x] `NATIVE_BIND_IN_INVALID` bad-value (link.rs:342) — `BIND IN` sets a field
      from an expression that is neither a wrapper param nor an int/bool/`-int`
      literal (e.g. a string literal). One clean `BIND IN` fixture that
      `accepts` also covers the `Identifier`/`Number`/`Unary "-"` `ok=true`
      arms (link.rs:327-339).

C ABI escape / ctype validity (`check_link_function_in`):

- [x] `NATIVE_CPTR_ESCAPE` param + return (link.rs:450/464) — ALREADY covered
      incidentally by `src/syntaxcheck/helpers.rs` (3 hits). Confirm green in
      A's fresh report; add a fixture only if the fresh report still shows
      those lines red.
- [x] `NATIVE_ABI_UNKNOWN_CTYPE` bad-return-ctype (link.rs:494) — `AS status
      CBogus` (an ABI return ctype not in the closed table).
- [x] `NATIVE_ABI_UNKNOWN_CTYPE` bad-slot-ctype (link.rs:518) — an ABI slot
      with an unknown ctype in argument position; a second fixture with an
      unknown ctype on an `OUT` slot drives the return-position arm
      (link.rs:511-515).

CONST pins:

- [x] `NATIVE_CONST_OUT` (link.rs:537) — `CONST` pinning a slot that is also
      `OUT`.
- [x] `NATIVE_CONST_UNKNOWN_SLOT` not-foldable (link.rs:690) — a `CONST` whose
      value is not foldable (e.g. an arbitrary identifier). A clean fixture with
      `CONST … = SIZEOF <CStruct>`, a bool literal, `NOTHING`, and a `-int`
      exercises the `foldable` `true` arms (link.rs:672-687) and `accepts`.
- [x] `NATIVE_CONST_UNKNOWN_SLOT` unknown-slot (link.rs:711) — a `CONST`
      pinning a slot name not in the ABI.

Unbound slots / params, result markers:

- [x] `NATIVE_ABI_UNBOUND_SLOT` no-binding (link.rs:561) — an input ABI slot
      with no matching parameter, CONST, OUT, or `BIND IN`.
- [x] `NATIVE_ABI_UNBOUND_SLOT` bad-expr-name (link.rs:594) — a `SUCCESS_ON`
      (or `RETURN`) expression reading an identifier that names no ABI slot and
      is not the ABI return (`SUCCESS_ON typo = 0`).
- [x] `NATIVE_ABI_NO_RESULT` (link.rs:614) — a wrapper with a non-`Nothing`
      return (or `AS RES`) and no `RETURN <expr>`.
- [x] `NATIVE_ABI_RESULT_MARKER` Nothing-with-RETURN (link.rs:626) — a
      `Nothing` wrapper that declares a `RETURN`.
- [x] `NATIVE_ABI_UNBOUND_PARAM` (link.rs:655) — a wrapper parameter with no
      matching ABI slot, no `BIND IN` field, and no `BUFFER … SIZE` use. A
      clean `BUFFER buf SIZE n` fixture that `accepts` also covers the
      `by_buffer_size` arm (link.rs:648-652) and `check_buffer_slots`
      (link.rs:99-159, forwarding `crate::ir::check_buffer_slots`).

FREE blocks:

- [x] `NATIVE_FREE_INVALID` on-resource-producer (link.rs:735) — a `FREE` block
      on an `AS RES` producer (drives the early `return` at :743).
- [x] `NATIVE_FREE_INVALID` malformed (link.rs:770) — a `FREE` whose freed slot
      is not the CPtr return that `RETURN` surfaces / whose deallocator is not
      `(CPtr) AS CVoid`. A well-formed `FREE` fixture that `accepts` covers the
      `ok=true` path (link.rs:745-767).

Re-exports / signatures (`collect_native_functions`, `native_function_sig`):

- [x] FuncAlias re-export (link.rs:808-823) — a `FUNC open AS demoLink::open`
      top-level alias adopting a LINK signature, then a call `open(...)` that
      `accepts` (drives the `link_sigs.get` adopt path). Also a bare LINK func
      called via `demoLink::open(...)` to cover `native_function_sig`'s param /
      return `parse_type` mapping (link.rs:826-860).

Acceptance: `sh scripts/coverage.sh` (fresh) then `sh scripts/coverage-check.sh
src/syntaxcheck/link.rs` shows link.rs ≥95%. Any line still red after these
fixtures is read from A's fresh report and either given a fixture or, if it is
an unreachable exhaustiveness arm (e.g. link.rs:844), flagged to A per the
Unreachable-arm candidates note above. `cargo test` → `0 failed`.
Commit: 779924d0e (36 `#[test]`s, one per diagnostic + the accepts arms; full
`cargo test` 0-failed, main-bin 3275→3311).

### Phase F2 — inference.rs: cover the 115 residual inference branches

inference.rs is at 93.7% with a mature test module
(`src/syntaxcheck/inference.rs:1600`, `wrap(body)` helper). The residual is
scattered un-exercised branches across its 23 `self.report` sites
(`grep -n "self.report(" src/syntaxcheck/inference.rs`) and their
type-promotion arms. **Name the exact still-red lines from A's fresh
`coverage.json`** (the on-disk report is stale — overview §2), then add one
fixture per reachable branch to the existing module.

- [x] From A's fresh report, list every red line in
      `src/syntaxcheck/inference.rs`. For each, read the enclosing branch and
      classify reachable-diagnostic vs unreachable-exhaustiveness arm.
- [x] For each reachable diagnostic branch (a `self.report(...)` at :156,
      :162, :207, :632, :670, :751, :890, :1048, :1080, :1104, :1117, :1138,
      :1177, :1187, :1269, :1309, :1320, :1345, :1421, :1433, :1447, :1457,
      :1477 or the residual subset A's report shows red), add a `wrap(...)` /
      whole-program fixture whose types force that branch and assert the rule
      code (or `accepts` for a no-report type-promotion arm).
- [x] Flag any residual red line that is a `Type::Result`/internal-only match
      arm no source can name to A as an exception candidate (do not chase it).

Acceptance: `sh scripts/coverage.sh` (fresh) then `sh scripts/coverage-check.sh
src/syntaxcheck/inference.rs` shows inference.rs ≥95%. `cargo test` →
`0 failed`.
Commit: 2ba44412b (builtin-predicate value position, Money-scaling warn +
is_bare_decimal_float, expect_arity range message, package-constant trap-guard,
nested-map-key split_top_level_to, negated-Money literal; 7 new `#[test]`s).

### Phase F3 — helpers.rs + mod.rs: cover the residual 40 + 96 lines

Both are high (94.0% / 94.2%) with existing modules
(`src/syntaxcheck/helpers.rs:308`, `src/syntaxcheck/mod.rs:1699`
`testutil` + `:1752` `checker_tests`). helpers.rs holds no-`report` pure
helpers driven indirectly by typed programs (its module already documents this
style); mod.rs holds checker orchestration + diagnostics. Same method: name the
still-red lines from A's fresh report, add one fixture per reachable branch.

- [x] From A's fresh report, list every red line in
      `src/syntaxcheck/helpers.rs`. For each pure helper still red
      (`statement_line` arms, `numeric_binary_result_type`,
      `promote_loop_numeric_type`, `type_from_numeric_name`,
      `read_only_record_type`, `effective_field_visibility`,
      `collect_captured_locals` shapes — `grep -n "fn " src/syntaxcheck/helpers.rs`),
      add a typed-program fixture that forces the branch (`accepts` or a
      downstream rule), matching the existing "exercised indirectly" tests.
- [x] From A's fresh report, list every red line in
      `src/syntaxcheck/mod.rs`. For each reachable diagnostic / orchestration
      branch, add a fixture to the `checker_tests` module (using `check_src` /
      `rejects_with` / `check_project_dir` — the last for `.mfp`/manifest
      paths, which `mod.rs` already exercises via `fixture(...)`).
- [x] Flag any unreachable exhaustiveness arm in either file to A rather than
      writing an unreachable-target fixture.

Acceptance: `sh scripts/coverage.sh` (fresh) then `sh scripts/coverage-check.sh
src/syntaxcheck/helpers.rs src/syntaxcheck/mod.rs` shows both ≥95%.
`cargo test` → `0 failed`.
Commit: a28e8ab19 (helpers: Set-literal capture arm + Money numeric_literal_type /
binary-result / type_from_numeric_name arms; mod: private-type visible_from,
Set-of-resource collection-ownership, union-INCLUDES expansion + self-include
guard, named-arg shape-check; 6 new `#[test]`s).

## Validation Plan

- After all three phases: `sh scripts/coverage.sh` (fresh profile) then
  `sh scripts/coverage-check.sh src/syntaxcheck/` reports every
  `src/syntaxcheck/*.rs` file ≥95% (none of F's four appears as a GATE
  FAILURE). The checker is filter-aware and reuses cached profdata.
- `cargo test` → `0 failed` (run the whole suite, never a single module —
  AGENTS.md). New fixtures must not regress any existing test.
- No `src/**` runtime behavior changed: `git diff --stat` on F's commits shows
  only added `#[cfg(test)]` code (and any RED-first bug-fix commit, if a fixture
  surfaced a real defect — fixed, not worked around).
- No line was added to `scripts/coverage-exceptions.txt` for these four files:
  syntaxcheck is pure analysis with no I/O boundary, so every gap is backfill.
- Any line flagged unreachable (link.rs:844, a `Type::Result` arm, …) is
  recorded in Corrections and handed to A's exception pass, not left red.

## Corrections

- **Fresh-report deltas (from the sibling P-68 `target/coverage/lcov.info`,
  `awk '/^SF:.*<file>$/{f=1} f&&/^DA:[0-9]+,0$/{print} /^end_of_record/{if(f)exit;f=0}'`).**
  - **link.rs:844** (the flagged `native_function_sig` `Type::Unknown` fallback)
    is **already covered** in the fresh report — it is NOT an exception
    candidate. So is the whole `collect_native_functions`/`native_function_sig`
    tail (lines >752). link.rs's still-red set was entirely lines 67–752 (every
    `self.report` diagnostic), which F1's 36 fixtures target.
  - **`NATIVE_CPTR_ESCAPE` (link.rs:459/473)** showed red in the fresh report
    despite the two helpers.rs tests, so F1 adds `wrapper_cptr_param_and_return_escape`.

- **Real bug found: none.** No fixture surfaced a production defect; all four
  files are pure backfill as scoped.

- **Native-rule parity guard interaction (no production change).**
  `ir::verify::tests::native_rule_sets_agree_between_syntaxcheck_and_verify`
  scans `syntaxcheck/{mod,link}.rs` *source text* for `"NATIVE_*"` literals and
  requires the set to match `ir::verify`. F1's original CBuffer test asserted the
  forwarded-only `"NATIVE_BUFFER_INVALID"` literal (emitted from `ir::link`, not
  syntaxcheck), which unbalanced the guard. Fixed by asserting the
  directly-emitted `NATIVE_ABI_NO_RESULT` instead (the fixture still exercises the
  target line 150). Lesson: in a syntaxcheck test, only assert rule literals the
  file's *production* code emits, or the parity scan breaks.

- **Unreachable-arm handoff to A (exception candidates, confirmed against the
  fresh report + source):**
  - **`src/syntaxcheck/mod.rs:1284-1289` — `TYPE_SUB_CANNOT_RETURN_VALUE`.**
    Documented unreachable at `mod.rs` checker_tests (a source `SUB … AS T` never
    parses — the parser reads a return type only for a `FUNC`); the arm is
    defensive for IR/package-decoded functions. NOT chased; flag to A's exception
    pass. Measure: no source fixture can produce this rule (parser rejects the
    declaration before the checker runs).
  - Residual mod.rs red lines are dominated by the **imported-package metadata
    path** (`collect_package_{types,resources,functions}` /
    `validate_package_metadata_type`, e.g. mod.rs:480-489, 558-623, 647-654, 741,
    743, 846-847, 874-875) which is driven only via `check_project_dir` over
    on-disk `.mfp` fixtures, and by per-arm `type_name` (mod.rs:1563-1569) and
    thread-type-name (1649) formatting arms. These are reachable but need shaped
    package fixtures; F3 covered the source-reachable subset (visible_from Private,
    Set collection-ownership, union expansion, shape-check). If mod.rs stays
    <95% after F3, the residual is this package-fixture backfill, not an exception.

- **Coverage measurement is deferred to the parent** (per the task: do not run
  `scripts/coverage.sh`). Line targets above are from the sibling P-68 lcov;
  F's four files received fixtures for every source-reachable still-red region.
