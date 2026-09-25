# plan-145-A: The field-site harness and census

Last updated: 2026-09-21
Overall Effort: huge (>3d)
Effort: large (3h–1d)
Depends on: nothing (the Prerequisites below gate the whole of plan-145)

## plan-145 as a whole

**Goal of the whole plan:** a self-update of a **field** mutates the field inside
its owner's existing block. The owner's block is not rebuilt and nothing is copied
beyond what computing the new value costs on its own. The two field forms are:

- a record field, `r = WITH r { f := op(r.f, …) }`;
- a `RES … STATE` payload field, `h.state.f = op(h.state.f, …)`.

This must hold at every site plan-144 audited: local record not-last/last (S3/S4),
global record (S5), nested (S6), inside a `FOR EACH` over the field (S7), a by-ref
lambda capture (S9), a two-field `WITH` (S10), and `STATE` T1–T8. It must hold for
every field kind that has an in-place form:

- fixed-width scalars;
- `List`/`Map`/`Set` under every plan-142 arm;
- fixed-size inlined records (`vector::*`, `color::Color`, the `datetime` types);
- nested records.

A guard makes this permanent, as plan-142's does for plain bindings. After
plan-145, a field site or field kind that has no in-place lowering must have a
row that says why, or the suite fails.

"In place" is measured the way plan-142 measured it. Run the statement `N` times,
then `2N` times, and sum `arena.<k>.alloc_calls` from `mfb build --debug`
(plan-142-A Correction A4). Two bounds apply:

- **`arm`:** `count(2N) − count(N) < N/8`.
- **`arm+value`:** for a field whose new value allocates on its own
  (`color::mix(…)` returns a block), the bound is `≤ 1.125 × (control(2N) −
  control(N))`. The control program runs `LET t = <same expression>` instead of
  the self-update.

A rebuild allocates the new record (or payload) every statement, so it fails both
bounds.

The facts this plan starts from are plan-144's findings,
`planning/plan-144-findings/record-state-self-update-audit.md`. §3.3 has the
counts, computed by `summary.py` (Appendix C.15), with 0 disagreements between
reading and dump across 5,175 cells:

- **Record sites:** 10 of 2,415 cells are in place, all at S4. They are the 10
  overloads that already have a record arm.
- **`STATE` sites:** 438 of 2,760 cells are in place. That is Layer 1 (68 scalar
  rows at T1–T5 and T8) plus the same 10 collection overloads at T2, T4 and T8.

| Letter | What it lands | Findings it closes | Effort |
|---|---|---|---|
| **A** | field sites in the runtime harness and the matrix test, an expectation file per (row, site), a field-kind census | — (guard) | large |
| **B** | one seam for fields: record and `STATE` statements build a `SelfUpdateSite`, and the 17 record/`STATE` arms fold into the seam arms (byte-identical) | §3.4 items 1–3 (their code half) | large |
| **C** | scalar fields of a record stored in place (the Layer 1 twin); a pointer field replaced in its slot; a mixed `WITH` (scalars + one arm) at S10/T5 | findings 1, 6 (`G14`) | large |
| **D** | the plan-142 arms that cannot reallocate, at a field: `filter take drop mid distinct`, `math`, `sort sortBy`, `intersection difference`, fixed-width `replace transform mapValues` | finding 3 (part) | large |
| **E** | the plan-142 arms that can reallocate, at a last-inlined field, through `InlineGrow`: `union symmetricDifference merge`, variable-width `replace transform mapValues` | finding 3 (rest) | large |
| **F** | fixed-size inlined record fields overwritten in place; nested record paths (S6/T6) | findings 4, 6 (`G17` at S6/T6) | large |
| **G** | module-level records (S5) | finding 5 | large |
| **H** | the aliasing sites: `FOR EACH` over the field (S7/T7, closes the §3.5 leaks) and a by-ref captured record (S9) | findings 6 (`G15`/`G16`/`G1`), §3.5 | large |
| **I** | lock the guard, correct the docs (§3.4), run the full gate | §3.4 | medium |

Letter order is implementation order. A is tests only. B is code motion (byte
identity). C–F add in-place lowerings at sites that are already local and have no
second holder. G and H open aliasing surfaces (a global block reachable from any
function; a loop or a caller holding the owner's pointer), so they come last,
behind the harness that A–F build. That is the same order plan-142 used.

References:

- `planning/plan-144-findings/record-state-self-update-audit.md`: site legend, §1
  and §2 cell tables, §3.2 findings, §3.4 contradictions, §3.5 observations,
  Appendix B (the code paths), Appendix C (probes and scripts).
- `planning/completed/plan-142-A-self-update-guard-and-seam.md`: the table, the
  seam, the harness this plan extends, the failure-atomicity rule, Corrections A4
  and A6 (what the harness counts, and why an arm may not allocate per statement).
- `.ai/collections.md` §"A collection inlined in a record" (`:205-258`): the
  reallocation split (sub-block route vs `InlineGrow`). Read it before writing any
  arm here.
- `src/codegen/collection/assign/self_update.rs` (`SELF_UPDATE_ARMS`,
  `SelfUpdateSite`, `try_inplace_self_update` `:234`, `Site` `:1287`,
  `ENABLED_SITES` `:1303`); `src/codegen/collection/assign/inplace_dest.rs`
  (`InPlaceDest`, `resolve_inplace_record_field` `:310`,
  `resolve_inplace_state_field` `:371`, `open_inplace_state_dest` `:459`,
  `close_inplace_dest` `:666`).
- `src/codegen/engine/control/builder_control.rs`: Layer 1
  `try_inplace_state_scalar_assign` `:144`, `record_collection_last_inlined`
  `:295`, the `STATE` Layer 2 dispatcher `:351`, `NirOp::StoreGlobal` `:1076`,
  `NirOp::Assign` `:1207` (the seam call and the 8-link record chain at `:1255`),
  `NirOp::StateAssign` `:1511`.
- `.ai/testing-gates.md:10` (per-phase gate), `:809` (a codegen-inspection test
  must be proven RED by reverting the fix), `.ai/compiler.md:85-86` (acceptance
  plus an execution test).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-142 is complete and archived | `ls planning/completed/plan-142-I-lock-and-docs.md` → exists | MET (2026-09-21, re-checked) |
| plan-144 is complete and its findings exist | `ls planning/completed/plan-144-B-state-self-update-audit.md planning/plan-144-findings/record-state-self-update-audit.md` → both exist | MET (2026-09-21, re-checked) |
| bug-671 fixed: a `STATE` field passed to a source-generic `collections` member type-checks | `ls bugs/completed/bug-671-*.md` → exists, **and** `scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/resources/state-field-source-generic-arg-valid'` → passes (the regression test bug-671 names) | MET (2026-09-21, after fixing it in this session: `bugs/completed/bug-671-state-field-arg-to-source-generic-is-unknown.md` exists, fix `bf85f4900`, merged to `main` at `4e0c50a8b`; `test-accept.sh target/debug/mfb target/accept-actual rt-behavior/resources/state-field-source-generic-arg-valid` → "acceptance tests passed (1 test(s) ran)") |
| The release compiler exists for `--ncode` probes | `ls target/release/mfb` → exists | MET (2026-09-21, re-checked) |

Why bug-671 gates the whole plan: 11 `collections` members (`distinct take drop
sort sortBy union intersection difference symmetricDifference merge mapValues`)
cannot take `h.state.f` at all, so 88 of the `STATE` cells do not compile (findings
§3.2 item 7). Without the fix, this letter's harness cannot build their `STATE`
cases, and D and E cannot land or measure arms for them at T sites. Marking those
cases `n/a` until the fix lands would be a fallback that ties this plan to the
bug. It is a precondition instead.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. **If you stop, report the current status of *all*
> prerequisites.**

## 1. Goal (this letter)

- `rt_inplace_self_update` runs every `cases.tsv` line at the field sites as well as
  the four plain sites. Each (line, field-site) pair has an expectation:
  - `arm`: must meet the bound;
  - `copy:<letter>`: must still rebuild, so the letter that lands it has to flip
    the line;
  - `rebuild:<reason>` or `deferred:<plan>`: must still rebuild; see Open
    Decisions 1 and 3;
  - `na:<reason>`: the program must fail to compile with the diagnostic named.
- A second case file covers field **kinds** that `cases.tsv` does not: scalars,
  fixed-size and variable-size inlined records, a pointer field, `String`. Each has
  one line per field kind with its own expectations per site.
- A unit census fails if a field kind that can appear in a record has no row in a
  new `FIELD_KIND_TABLE`. The kinds come from the registry's package record types
  and the builtin scalar types, classified by `record_field_is_inlined`,
  `record_field_is_pointer` and fixed size.
- The matrix test `every_arm_row_fires_at_every_enabled_site` gains the field sites.
  A `(ArmId, Site)` pair is expected to fire unless it is listed in
  `FIELD_PENDING` with the letter that lands it. Letter I deletes `FIELD_PENDING`.

### Non-goals (explicit constraints)

- No codegen change. The only behavioral change in this letter is to the test
  harness.
- No change to plan-142's four plain sites or their expectations.
- No `mfb man` change.

## 2. Current State

- The harness (`tests/runtime/rt_inplace_self_update.rs`) knows four sites
  (`enum Site`, `:197`; `ENABLED_SITES`, `:218`). It builds programs by wrapping
  the case's `x` statements in a site template (`frame` `:231`, `at_site` `:249`).
  Its statuses are `arm` and `exempt` (plan-142-I Phase 1). Any other status
  panics.
- `cases.tsv` has 79 lines (`wc -l tests/runtime/inplace_self_update/cases.tsv` →
  79, including the comment header). They cover the 63 self-update-shaped
  collection overloads plus `&`. None are field cases.
- The unit matrix (`self_update.rs` `Site` `:1287`, `ENABLED_SITES` `:1303`)
  compiles each `Arm` row's probe at each plain site and checks for the arm's
  marker slot. Record and `STATE` arms are not in `SELF_UPDATE_ARMS`, so no
  matrix test covers them. They are covered only by
  `tests/codegen/codegen_inplace_record_field.rs` and
  `tests/runtime/rt_res_state_inplace_mutation.rs`.
- plan-144's probe generators (`gen_rec.py`, `gen_state.py`, findings Appendix
  C.3, C.10) already express every field site as MFBASIC source. They are the
  templates the harness needs.

### Measured populations

| What | Count | Command |
|---|---|---|
| Rows × record sites; in place today | 345 × 7 = 2,415; 10 `y` (S4 only) | findings §3.3 (`summary.py`, Appendix C.15) |
| Rows × `STATE` sites; in place today | 345 × 8 = 2,760; 438 `y` | same |
| `cases.tsv` lines | 79 (with header) | `wc -l tests/runtime/inplace_self_update/cases.tsv` |
| Record-field arms (non-seam) | 9 | `grep -c 'fn try_inplace_record_field_' src/codegen/collection/assign/builder_inplace_assign.rs` → 9 |
| `STATE` arms (Layer 1 + dispatcher + 8 Layer 2 + splice) | 11 | `grep -c 'fn try_inplace_state_' src/codegen/collection/assign/builder_inplace_assign.rs src/codegen/engine/control/builder_control.rs` → 8 + 3 |
| Field kinds among the rows | scalar 4 (`Integer Float Fixed Money`); inlined record 15 (9 `vector`, `color::Color`, `big::Int`, 3 `datetime`, `http::Response`); pointer 1 (`json::Json`); `String`/`AttributedString` 2; collection 3 | findings Appendix B.4 |
| Which inlined record kinds have a compile-time fixed size | 31 of the 68 exported package record types (the compiler's `field_kind_class`); 35 inlined but variable-size; 2 pointer (`json::JsonArr`, `json::JsonObj`) | Phase 1 |
| Harness cost per (line, site) pair | 2.7 s at plan-142's sites; 1 s measured at a field site (Correction A9) | plan-142-I Phase 1: 254 pairs in 679.52 s |

## 3. Design

**Field sites in the harness.** `Site` gains S3, S4, S5, S6, S7, S9, S10, T1–T8.
A field site rewrites the case's statement from the binding `x` to the field:

- record sites: `x = f(x, …)` becomes `r = WITH r { b := f(r.b, …) }` (S4),
  `a := …` (S3), and so on;
- `STATE` sites: `x = f(x, …)` becomes `h.state.b = f(h.state.b, …)`.

The rewrite substitutes the identifier `x` with a word-boundary match. That is
safe because every `cases.tsv` statement names the binding `x`: check with
`grep -vc '\bx\b'` over the statement column → 0.

The templates come from plan-144's generators (findings Appendix C.3, C.10):

- record `Rec { a AS T, b AS T }` and `RecN { a, b AS T, n AS Integer }`;
- `STATE` payload `P { a AS T, b AS T, n AS Integer }`, on
  `RES h AS fs::File STATE P`;
- a callee `SUB g(RES h AS fs::File STATE P, …)` for T3/T4;
- `UNION Stream { fs::File, tcp::Socket }` for T8.

The value check is unchanged: the value printed before the loop equals the value
printed after it. For a record it prints `r.b` and a sibling field, so a dropped
sibling write fails the check.

**Expectations.** Add `tests/runtime/inplace_self_update/field_expect.tsv` with
columns `signature \t site \t expect`. Generate it once, in Phase 1, from the
findings' §1 and §2 cell tables:

| findings cell | `expect` |
|---|---|
| `y` | `arm` |
| `n (…)` | `copy:<letter>` naming the plan-145 letter that closes it (the table in §"plan-145 as a whole") |
| `n/a (…)` | `na:<reason>` |

plan-142's 11 `exempt` lines (`awk -F'\t' '$2=="exempt"' cases.tsv | wc -l` → 11:
`reduce`, `reduceRight`, 6 `compress`, 3 `crypto` overloads) map to `rebuild:new-value` at every field site. At a plain binding they
were exempt because there is no copy of `x` to avoid. At a field there is still
the record rebuild: their result is a new value of unrelated size. That makes them
Open Decision 3's case, not an arm's.

The other exception is the bug-671 cells. The prerequisite is met, so they are
re-measured and filed as `copy:D`/`copy:E`, not `na`. Commit the generator
script next to the file (`field_expect_gen.py`), so the mapping can be reviewed.

**Field kinds.** Add `tests/runtime/inplace_self_update/field_kinds.tsv` with
columns `kind \t setup \t statement \t bound(arm|arm+value) \t per-site expect`.
It has one line per field kind in the Measured populations table: each scalar,
each package record type, a nested user record, a variable-size nested record
(`{ s AS String }`), `json::Json`, `String`, `AttributedString`. The rows in the
findings' F2–F5 families differ by builtin but not by lowering: no arm names
them (findings Appendix B.3), so one representative statement per kind measures
the same path.

**Census.** A unit test, `field_kind_census_covers_every_record_field_type` in
`self_update.rs`, enumerates:

- every record type any registry package exports;
- the builtin scalar types;
- `String`, `List`, `Map` and `Set`.

It classifies each as `Scalar`, `Pointer`, `InlinedFixed`, `InlinedVariable` or
`Collection`, and asserts that each has a `FIELD_KIND_TABLE` row. A row is
`Arm(letter-landed)`, `Rebuild { reason, proof }` or `Deferred(plan)`. A black-box
twin in `tests/guards/inplace_self_update_census.rs` asserts that every type on
every `mfb man <pkg> types` "Records" section has a `field_kinds.tsv` line. So a
new package record type fails both censuses until someone classifies it.

**Matrix.** The matrix's `Site` gains the field sites. At the end of A, every
`(ArmId, field site)` pair is in `FIELD_PENDING` with the letter that lands it,
and letters B–H remove entries. The matrix asserts both directions:

- a pair not pending fires;
- a pending pair does **not** fire, so a letter that lands a pair early without
  removing its entry fails.

The 9 record and 8 `STATE` arms are outside the seam until B, so in A their pairs
are pending on B.

**Running cost.** 79 lines × 15 field sites is 1,185 pairs, 53 minutes at 2.7 s
per pair. Each letter runs only the pairs it flips:

- `MFB_SELF_UPDATE_FILTER` already narrows by line;
- this letter adds `MFB_SELF_UPDATE_SITES=<list>`.

The whole matrix runs twice: once in this letter, to prove every expectation
against today's compiler, and once in letter I.

Rejected alternatives:

- **One `field_expect` rule computed from the site** (e.g. "S4 is `arm` for the
  10 arm rows"): the rules are what this plan changes letter by letter. A computed
  rule would move with the code and could not catch a letter that changed more
  than it claimed.
- **Only the findings' marker method** (codegen inspection): the findings already
  have it, with 0 disagreements. The runtime alloc count is the only check that
  measures "no rebuild" instead of "the arm ran", and plan-142 rejected the
  marker-only guard for the same reason.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. `- [~]` partial. Moot tasks struck through with evidence,
> never deleted. **An unticked box means NOT DONE.**

### Phase 1: Measure

- [x] Re-run the findings' record and `STATE` probes for the 11 bug-671 rows
      against the fixed compiler (`gen_state.py` + `fill_state.py`, findings
      Appendix C.10/C.14) and record their T-site verdicts here. All are expected
      to be `n (no arm)`. A `y` is a finding to investigate, not a pass.
      Run on a copy of the probes (`/tmp/p145-probes`, `exclude.txt` without its 88
      `TYPE_CALL_ARGUMENT_MISMATCH` lines): `gen_state.py` → "2558 called functions,
      14 excluded"; `mfb build state --ncode` → "Wrote native code plan", no
      diagnostic; `markers.py` + `fill_state.py` → "rows 345 cells 2760
      disagreements 0". `diff` against the findings' `state/table.md` changes exactly
      the 11 rows (`difference distinct drop intersection merge sort sortBy
      symmetricDifference take union mapValues`), and every one of their 88 cells
      (T1–T8) is now `n (no arm)` (`ARM=- L1=- REPL=y`; `FREE=-` at T7, the §3.5
      leak). No `y`.
- [x] Classify every package record type by compile-time byte size: a record is
      `InlinedFixed` when every field is a fixed-width scalar or an
      `InlinedFixed` record. Record the list and the command. The list sizes
      letter F.
      Command: `cargo test --bin mfb field_kind_census` (`field_kind_census` in
      `self_update.rs` classifies every EXPORTed record of a program importing every
      package with the compiler's own `record_field_is_inlined` /
      `record_field_is_pointer`; Correction A7). 68 exported record types: **31
      `InlinedFixed`** — `astrings::AttrFlag AttrNumber`, `audio::AudioEnvelope
      AudioNote`, `canvas::Bounds GradientStop MouseEvent Point Size TextMetrics
      Transform`, `color::Color Hsl`, `datetime::Date Duration Instant Time`,
      `json::JsonBool JsonNull JsonNum`, `term::MouseEvent TermSize`, and the nine
      `vector::` types; **35 `InlinedVariable`** (a `String`, a collection or a data
      union inside); **2 `Pointer`** (`json::JsonArr`, `json::JsonObj`: they hold a
      `json::Json`, which is not memcpy-copyable, so they are not inlined).
- [x] Write `field_expect_gen.py` and generate `field_expect.tsv`. Record the line
      count, and the count per `expect` value, each with its command.
      `python3 tests/runtime/inplace_self_update/field_expect_gen.py >
      tests/runtime/inplace_self_update/field_expect.tsv`; `grep -vc '^#'` → **960**
      lines (64 `cases.tsv` lines × 15 sites; Correction A2). `grep -v '^#' | cut -f3
      | sort | uniq -c` → 40 `arm`, 20 `copy:C`, 342 `copy:D`, 18 `copy:E`, 104
      `copy:F`, 52 `copy:G`, 156 `copy:H`, 165 `rebuild:new-value`, 48
      `rebuild:not-last-grow` (Correction A3), 13 `deferred:string`, 2
      `na:TYPE_FOR_EACH_REQUIRES_COLLECTION`. No `copy:B`: B is byte-identical.

Acceptance: the three results are recorded here with their commands (est. 30 min).
Commit: `6529c1384` (with Phases 2 and 3)

### Phase 2: Harness field sites

- [x] `rt_inplace_self_update.rs`: the 15 field sites and their templates, the `x`
      rewrite, the `field_expect.tsv` reader and its four statuses, and
      `MFB_SELF_UPDATE_SITES`. A line or site missing from `field_expect.tsv`
      panics with its name. (`FieldSite`, `FieldExpect`, `field_frame`/
      `field_program`/`field_result_program`, `check_field`; the field programs
      also print a sibling field before and after, and a missing or stale
      `field_expect.tsv` line panics before any filter applies.)
- [x] `field_kinds.tsv` and its reader. It includes the `arm+value` control
      program: ~~the same expression bound to a fresh `LET`~~ the same expression
      handed to a no-op `SUB kindSink` (Correction A5). 79 kinds (Correction A4);
      the `canvas::` kinds build `-app` and run headless.
- [x] RED proof: flip one `copy:` line to `arm` (for example
      `collections::filter` at S4) and confirm the bound fails, naming the line.
      Then flip S4 `collections::append` to `copy:B` and confirm the "still
      copies" assertion fails. Restore both.
      `MFB_SELF_UPDATE_FILTER='collections::filter|collections::append(value AS
      List OF T, item AS T)' MFB_SELF_UPDATE_SITES=S4 cargo test --test
      rt_inplace_self_update every_self_update_case` → "2 of 2 case/site pair(s)
      failed": "…filter… at S4: marked `arm`, but 2000 more runs allocated 4000
      more blocks (want < 250) — the statement rebuilds the owner" and "…append…
      at S4: marked `Copy('B')`, but 2000 more runs allocated only 2 more blocks
      (< 250) — the update is in place now; flip the line to `arm`". Restored
      (`diff` against the saved copy empty).

Acceptance: `cargo test --test rt_inplace_self_update` passes, with every field
pair run (est. 55 min: 1,185 pairs at 2.7 s each. This is the one run that
checks every expectation against today's compiler. A subset would leave some
expectations untested, and letters B–H flip them on the assumption they held).
`MFB_TEST_EXE=target/release/mfb cargo test --test rt_inplace_self_update` (the
compiler with bug-673/674/675) ran every pair in 5,585.89 s:
`every_self_update_case_meets_its_allocation_bound … ok` (the 254 plain pairs and
all 960 field pairs); `every_field_kind_meets_its_expectation` "6 of 1185 case/site
pair(s) failed", all six the `Byte` line at T1–T5/T8 (Correction A16). With the
`Byte` bound corrected, `MFB_SELF_UPDATE_FILTER='Byte' … every_field_kind` → ok
(its 15 pairs); no other line changed.
Commit: `6529c1384`

### Phase 3: Unit census and matrix

- [x] `FIELD_KIND_TABLE` and `field_kind_census_covers_every_record_field_type`
      in `self_update.rs`. The population is read from the compiler, not listed:
      `field_kind_census` lowers (in app mode — `canvas` requires it) a program that
      imports every package and declares one record holding each builtin kind, and
      classifies every EXPORTed record type plus those fields with
      `field_kind_class` (`record_field_is_inlined`/`record_field_is_pointer`, then
      fixed size). 80 rows: 6 scalars `Arm('C')`, `String`/`AttributedString`
      `Deferred("string")`, `json.Json` and the two json pointer records
      `Arm('C')`, the three collections `Arm('E')`, 31 fixed-size records
      `Arm('F')`, 35 variable-size records `SIZE_VARIES`. A row must fit its class
      (an `Arm` for a variable-size kind fails), and a row naming no kind fails.
- [x] The black-box twin in `tests/guards/inplace_self_update_census.rs`:
      `every_package_record_type_has_a_field_kind_line` reads every `mfb man <pkg>
      types` Records section and requires a `field_kinds.tsv` line per type (and no
      stale `pkg::` line). `cargo test --test inplace_self_update_census` → "2
      passed".
- [x] The matrix's field sites and `FIELD_PENDING` (every field pair pending on
      its letter), with the pending pairs asserted **not** to fire. The unit
      `Site` gained the 15 field sites; `Probe::field_source` builds the same
      templates as the runtime harness. `FIELD_PENDING` lists `(ArmId, sites,
      letter)`; `FIELD_NEVER` lists the probes an arm never fires for by design
      (Correction A14). `field_pending_and_never_name_field_sites_once` checks
      both tables. With every field pair pending, the matrix passing proves no
      field pair fires today and every pair is listed.
- [x] RED proof: delete the `vector::Float3` row, and confirm both censuses fail
      naming it. Remove one `FIELD_PENDING` entry, and confirm the matrix fails
      naming the pair. Restore both.
      Row removed → `cargo test --bin mfb field_kind_census` → "vector.Float3
      (InlinedFixed) has no FIELD_KIND_TABLE row — classify it: …", FAILED. Line
      removed from `field_kinds.tsv` → `cargo test --test inplace_self_update_census
      every_package_record_type` → FAILED listing `vector::Float3`. `(ArmId::Append,
      LAST, 'B')` removed → `every_arm_row_fires` → "collections::append at S4: no
      probe fired Append" (and T2, T4, T8), FAILED. All restored (`git diff` of the
      two files empty against the saved copies).

Acceptance: `cargo test --bin mfb self_update && cargo test --test inplace_self_update_census`
→ pass; RED results recorded here (est. 10 min).
`cargo test --bin mfb self_update` → "6 passed" (174.70 s);
`cargo test --test inplace_self_update_census` → "2 passed".
Commit: `6529c1384`

## Validation Plan

- Tests added: field sites in `rt_inplace_self_update`, `field_expect.tsv`,
  `field_kinds.tsv`, the two field-kind censuses, the matrix's field sites.
- Per-letter unit gate: `cargo test --bin mfb` (`.ai/testing-gates.md:10`).
- Doc sync: none in this letter (letter I).

## Open Decisions

1. **`String` and `AttributedString` fields (findings §3.2 item 2: 65 rows, all
   `n`).** Recommended: out of plan-145. The field harness marks them
   `deferred:string` and asserts they still rebuild. The `String` fix plan,
   written from plan-143's findings, owns `String` arms at every site. Two things
   support this:
   - plan-143's findings show that plain `String` locals are not in place either
     (only `&` has an arm), so a field arm would need a lowering that does not
     exist yet.
   - An inlined `String` has no capacity word (findings Appendix B.5), so even `&`
     needs a representation decision, and that decision belongs to that plan.

   Letter B's seam serves field sites for every arm that declares its reallocation
   class. So once the `String` plan adds an arm, flipping its field lines is that
   plan's job, not new work here.
   Alternative: bring `&` on a last-inlined `String` field into this plan with a
   per-field capacity shadow. That is a new hidden slot per field and an
   `InlineGrow` for `String`.
   DECISION: the recommendation (adopted by the executor, 2026-09-21: the user asked
   for the plan to be run to completion). plan-146-A Open Decision 1 names the
   owner: a follow-up plan written after plan-145 and plan-146, so the tag is
   `deferred:string` (`FIELD_KIND_TABLE` row `Deferred("string")`).
2. **A cannot-reallocate arm at a not-last field (S3/T1/T3).** Recommended: admit
   it if letter D's Phase 1 shows that record size, copy and free read each
   field's stored offset and never sum field sizes. That result is recorded in D.
   Then shrinking or rewriting a middle field leaves slack that nothing reads.
   Otherwise `G17` stays as it is for every arm.
   DECISION: the recommendation (executor, 2026-09-21): admit it iff D's Phase 1
   measurement shows offsets are read, not summed. A lists those pairs as `copy:D`;
   if D's measurement fails, D re-files them as `rebuild:not-last` in Corrections.
3. **A variable-size inlined non-collection field** (`big::Int`,
   `http::Response`, a nested record holding a `String`, and the findings' F5
   replacements of an inlined field). Its new value may differ in size, and it has
   no capacity to grow into. Recommended: a `Rebuild` row with the proof "the new
   value's size is known only after it is built, and an inlined field without a
   capacity word cannot take a larger value in place". The harness asserts
   `rebuild:size-varies`.
   Alternative: at a last-inlined field, reallocate the record's tail to the new
   size. That reallocation copies the record's prefix, which costs about as much
   as the rebuild it replaces.
   DECISION: the recommendation (executor, 2026-09-21): `rebuild:size-varies`, row
   `SIZE_VARIES` with that proof.
4. **A pointer field (`json::Json`) in a record.** Recommended: in letter C.
   Compute the new value first, then free the old pointee and store the pointer
   into the slot. That is a type-driven store like Layer 1, with one free added.
   `json::Json` cannot be a `STATE` type (findings §2b), so this is records only.
   DECISION: the recommendation (executor, 2026-09-21), letter C. Measured: the two
   records holding a `json::Json` (`json::JsonArr`, `json::JsonObj`) ARE valid
   `STATE` field types (a `STATE` payload with such a field compiles), so C's
   pointer replace serves `STATE` owners too; `field_kinds.tsv` lists them `copy:C`
   at the T sites, and only the bare `json::Json` line is `na:TYPE_STATE_INVALID`.
5. **The `RES` parameter that declares an invalid `STATE` type** (findings §3.5:
   `SUB g(RES h AS fs::File STATE P_json__Json, …)` compiles). This is not a
   self-update problem, so it is not in this plan. Recommended: file it with
   `/write-bug`.
   DECISION: the recommendation (executor, 2026-09-21); the project rule is also to
   fix a bug once found, so it is filed and fixed as its own bug (Corrections).

## Corrections

- **A1 — prerequisite satisfied in-session.** bug-671 was Open when the plan was
  started; the user directed it fixed first. It was fixed and merged to `main`
  (`bf85f4900`, `4e0c50a8b`) before any plan-145 work, and the row re-checked MET.
- **A2 — `field_expect.tsv` has 960 lines, not 1,185.** `cases.tsv` has 79 lines of
  which 15 are the comment header (`grep -vc '^#' cases.tsv` → 64), so the file is
  64 × 15 = 960 (`grep -vc '^#' field_expect.tsv` → 960). The plan's "79 × 15"
  counted the header.
- **A3 — multi-statement lines take their worst statement.** 22 `cases.tsv` lines
  run two statements (`grep -v "^#" cases.tsv | awk -F"\t" "$4 ~ / ; /" | wc -l` → 22;
  `x = append(x, 1) ; x = distinct(x)`). Each statement is
  matched to its own findings row and the line gets the worst verdict. A line whose
  helper statement is a growing arm (`append`, `add`, Map `set`) is
  `rebuild:not-last-grow` at S3/T1/T3 (48 lines), because a grow at a not-last
  field is letter E's non-goal — the partner op there is never measured in place.
- **A4 — `field_kinds.tsv` has two more columns and 79 lines.** Columns `build`
  (`console`/`app`: `canvas` requires app mode; the harness builds `-app --debug`
  and runs headless with `MFB_MACAPP_HEADLESS` etc.) and `check` (the value
  check needs an expression per kind). Lines: 6 builtin scalars (the census's
  scalar population is `Integer Float Fixed Money Boolean Byte`, not the four
  among the findings' rows), `String`, `AttributedString`, `json::Json`, the 68
  package record types, and the user records `KFix`/`KVar`. It is generated by
  `field_kinds_gen.py` from `mfb man`, committed beside it.
- **A5 — the `arm+value` control is a sink call, not a `LET`.** Measured: a `LET`
  (or an assignment to `x`) of a value borrowed from the field copies it, one
  allocation per iteration — exactly what the owner's rebuild adds — so the control
  equalled the rebuild (`json::JsonNum at S4: … 2000 more blocks (<= 1.125 x the
  control's 2000)`) and the bound passed a rebuild. The control hands the value to
  a no-op `SUB kindSink(v AS T)`, which counts the value's own allocations only.
  The bound itself is the plan's (`<= 1.125 ×` the control's growth).
- **A6 — kind statements must make the bound decidable.** `json::parse(json::stringify(x))`
  allocates about 49 blocks per iteration, so the 12.5 % slack swallowed the
  rebuild's extra one or two. Every record kind uses `x = kindSame(x)` (returns its
  argument; a vector kind uses `vector::max(x, x)`), a pointer kind `x =
  kindFresh(x)` (a freshly built default value: storing a borrowed pointer value
  deep-copies it under any lowering), and `json::Json`, which has no default,
  `x = kindJson(x)` (`json::JsonNum[1.0]`).
- **A7 — the fixed-size classification is the compiler's.** A script over the
  `mfb man` field lists first found 30 fixed-size types; the compiler's
  `field_kind_class` finds 31 (`json::JsonNull`'s `value AS Nothing` is an 8-byte
  scalar slot) and classes `json::JsonArr`/`JsonObj` as pointer fields (a
  `json::Json` inside is not memcpy-copyable, so they are not inlined). The census
  and `field_kinds_gen.py` now apply the same rules.
- **A8 — three compiler bugs found by the harness, fixed as their own commits.**
  bug-673 (`364b2131a`): a builtin package's globals initialized after the
  program's, so a top-level `json::parse` failed ("nested too deeply"); the S5
  `json::Json` line hit it. bug-674 (`b3790a364`): Open Decision 5's parameter
  `STATE` check. bug-675 (`ff66de4ec`): `Float` arithmetic on a global's or a
  `STATE` field failed to build ("has no data object"); the `Float` kind at S5 and
  every T site hit it. Each has a RED-then-GREEN fixture and a bug record in
  `bugs/completed/`; the artifact gate stayed at 0 diffs across all three.
- **A9 — a field pair costs about 1 s, not 2.7 s.** `append` at all 15 field sites
  ran in 14.12 s; the full field run is recorded in Phase 2.
- **A10 — `In` is a keyword.** The `STATE` nested payload's inner type is `PIn`,
  not `In`.
- **A11 — Open Decisions adopted as recommended** (recorded under each).
- **A12 — the `String` deferral tag stays `string`.** plan-146-A Open Decision 1
  puts `String` fields in a follow-up plan written after plan-145 and plan-146, so
  the rows are `Deferred("string")` / `deferred:string`, not plan-146.
- **A13 — the T sites open `/dev/null`**, which every Unix host has (the harness
  is `#![cfg(unix)]`).
- **A14 — `FIELD_NEVER` beside `FIELD_PENDING`.** Some (arm, probe, site) triples
  never fire by design: a growing arm at a not-last field (Open Decision 2's
  non-goal half, E's non-goal) and the `String` concat arm at any field (Open
  Decision 1). A pending entry needs a letter that lands it, so these are a second
  table, asserted not to fire. `collections::set`'s Map probe is listed per probe
  type: its List probe is `copy:D` at S3.
- **A15 — non-defaultable and unconstructible kinds.** A `STATE` payload must be
  defaultable, so the kinds whose type has no default (16: the three `astrings`
  attributes, `http::Route`, `net::PingResult`, `term::MouseEvent`, 10 canvas
  records) are `na:TYPE_STATE_INVALID` at every T site. `canvas::Text` and
  `canvas::Picture` hold a live `RES` font/image: no default, and no literal can
  name the resource, so every site is `na:TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE` —
  both are `SIZE_VARIES` kinds, which rebuild by design anyway.
- **A16 — the `Byte` kind's bound is `arm+value`.** Its statement
  `x = toByte(len(toString(x)))` allocates a `String` each run on its own, so the
  plain `arm` bound failed at the T sites although Layer 1 stores the field in
  place ("Byte at T2: marked `arm`, but 2000 more runs allocated 2000 more
  blocks"); the value's allocation is exactly what `arm+value` accounts for.

## Summary

A extends plan-142's harness to the 15 field sites, generates one expectation per
(case, site) from plan-144's cell tables, adds a census over field *kinds*, and
opens the matrix to field sites with every pair pending on its letter. It changes
no codegen. Every later letter flips the lines it lands, and nothing can flip
silently.
