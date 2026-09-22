# plan-144-A: Audit which record-field self-updates the compiler performs in place

Last updated: 2026-09-21
Overall Effort: large (3h–1d)
Effort: medium (1h–2h)
Depends on: nothing

plan-144 is a research spike that **changes no code**. It is the record and
`RES … STATE` counterpart of plan-141 (collections at non-`STATE` sites) and
plan-143 (`String` at non-record sites). Those two audits left out
records, `RES` and `STATE`. plan-144 covers them. It reads the compiler and
records, for every self-update-shaped builtin overload (any value type) and for
the self-update operators, whether a field self-update is done **in place** or
**rebuilt as a copy**. It does this at every record-field site (this letter) and at
every `STATE` payload site (plan-144-B). Each verdict cites the code that decides it
and is cross-checked with `mfb build --ncode`. The output is one findings file, and
the fix plan will be written from it.

This letter builds the row census that both letters share and fills the
**record-site** table. plan-144-B adds the `STATE` table and the summary.

The rule measured against is the same one plan-141 and plan-143 use: **a `MUT`
updating itself mutates in place, with no copy, for every kind of value.** For a
record, "itself" means the field. `r = WITH r { f := op(r.f, …) }` should mutate
`r.f` inside `r`'s existing block and should not rebuild the record.

Why this is separate from plan-143: the user set plan-143's scope to non-record
self-updates and asked for this audit to cover records and `STATE`. plan-141
covered collection ops in record fields (§2, S3–S6) at `b6a10efbc`. It did not
cover `String` fields, fields of any other type, `STATE` payloads, or the plan-142
changes. This audit re-verifies plan-141's record cells at the current commit and
does not copy them.

References:

- `planning/plan-141-findings/inplace-audit.md`: the method, the site legend
  (S1–S9, and "last" meaning *last inlined*), `markers.py` (Appendix C.1), and §2
  (record updates R1–R6), which this letter re-verifies.
- `planning/plan-143-string-self-update-audit.md`: the `String` row list and the
  **form** column (§4), which this plan reuses for its `String` rows.
- `src/codegen/collection/assign/inplace_dest.rs:310`
  (`resolve_inplace_record_field`, gates G14 and G17). `:371`
  (`resolve_inplace_state_field`) is used by plan-144-B.
- `src/codegen/engine/control/builder_control.rs:295`
  (`record_collection_last_inlined`, G17), `:1192` (`NirOp::Assign`), and `:1076`
  (`NirOp::StoreGlobal`).
- `src/codegen/collection/layout/builder_collection_layout.rs:706`
  (`record_field_is_inlined`).
- `src/codegen/memory/value/builder_value_semantics.rs:703` (`lower_with_update`,
  the whole-record rebuild).
- `src/codegen/collection/assign/self_update.rs`: plan-142's `SELF_UPDATE_TABLE`
  (`:796`) and `try_inplace_self_update` (`:234`). This letter records whether any
  record path reaches the seam.
- `mfb man variable` §"Changing a record: WITH" and §"A handle can carry its own
  data: STATE".

## Prerequisites

This plan only reads code and writes one findings file. The only conditions are
that the compiler being read is the one being probed, and that the lowering code
has no uncommitted edits (so every citation resolves at a commit).

| Must be true | Command | Status |
|---|---|---|
| The release compiler exists and is newer than the last `src/` commit | `ls -l target/release/mfb` vs `git log -1 --format=%ci -- src` → binary newer | MET (2026-09-21, in worktree `P-144` at `efdb54bb7`: binary 14:13 vs `src/` 14:03 `6ed5cc234`. Main's binary (13:34) was older than `6ed5cc234`, so the worktree's was rebuilt first, Correction A2) |
| No uncommitted edits in the lowering code the audit cites | `git status --short src/codegen/collection src/codegen/engine src/codegen/memory src/ir src/ast src/target/shared \| wc -l` → 0 | MET (2026-09-21, worktree `P-144`: 0) |

Everything below assumes both conditions hold. The findings file records the HEAD
hash the audit read.

## 1. Goal

- `planning/plan-144-findings/record-state-self-update-audit.md` exists and holds
  §0, the shared row census, and §1, the record-site table. For every row, §1 gives
  a verdict at each record site in §5: `y`, `n (<gate or path>)`, or `n/a` with a
  reason. Each `y`/`n` cites the deciding code as `file:symbol` and is confirmed by
  a marker line from one `--ncode` probe build.

### Non-goals (explicit constraints)

- **No code, test, golden, spec or man-page changes.** Only the findings file and
  the plan-144 checkboxes are written.
- **No `STATE` sites in this letter.** They belong to plan-144-B.
- **No non-record sites.** Plain local, global, loop-live and captured bindings
  (S1, S2, S7, S9 on a non-record binding) belong to plan-141, plan-142 and
  plan-143. A row's non-record behavior is cited where it explains a record verdict
  and is never re-audited.
- **Not timing-based.** Verdicts come from reading code. The only cross-check is the
  compiler's own `--ncode` output.
- **No fix design** beyond the form column, which is copied from plan-143 §4 for
  `String` rows.

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| Builtin packages / overloads | 42 / 828 | Appendix census script → `packages 42 overloads 828` |
| Literal self-update overloads of **any** value type (first parameter type = return type) | 323 | the same script with the type test `if ft and ft == ret:` → 323 lines |
| … collection-typed | 59: `collections` 23, `math` 27, `compress` 6, `crypto` 3 | same output, return type starts `List`/`Set`/`Map` → `uniq -c` per package |
| … `String`/`AttributedString` | 44: `strings` 20, `encoding` 8, `fs` 6, `astrings` 4, `os` 3, `io` 1, `net` 1, `regex` 1 | same output (matches plan-143 §2) |
| … any other type | 220, over 20 types: `Integer` 25, `Float` 16, `Fixed` 16, nine `vector::*` types 123 (15×3 + 13×6), `big::Int` 13, `color::Color` 9, `Money` 6, `datetime::DateTime` 4, `datetime::Duration` 3, `datetime::Instant` 2, `json::Json` 2, `http::Response` 1 | same output, grouped by return type (the Appendix one-liner) |
| Generic overloads whose result can equal the first argument's type (`Var`/`Arg(n)`) | 18 candidates (every one in `collections`), of which 4 type-check as a self-update: `transform`, `mapValues`, `reduce`, `reduceRight` | `census.py … generic` → 18 lines; probe `gen` → 14 errors, one per other candidate (findings C.1) |
| Rows in the shared census (after the F4 guard split) | 326: F1 63, F2 44, F3 6, F4 206, F5 7 | `rows.py` → `Counter({'F4': 206, 'F1': 63, 'F2': 44, 'F5': 7, 'F3': 6}) total 326` |
| In-place recognisers (`fn try_inplace_*`) | 48, of which 9 are `try_inplace_record_field_*` and 11 are `try_inplace_state_*` | `grep -rhoE 'fn try_inplace_[a-z_]*' src --include='*.rs' \| wc -l` → 48; the same grep with `record_field_` → 9 and with `state_` → 11 |

### What is already known (re-verified here, not assumed)

- plan-141 §3.2 finding 3: a record-field self-update is in place only for a
  **function-local** record with **exactly one** updated field (G14), where the
  field is a **collection** that is the **last inlined** field (G17,
  `builder_control.rs:295`), and only for the 9 `record_field_*` arms'
  operations. S5 (global record) is `n (StoreGlobal)` and S6 (nested) is
  `n (G17)`.
- plan-141 §3.2 finding 4: **no record arm handles a scalar or `String` field**
  (R1). The `STATE` twin (`try_inplace_state_scalar_assign`, `builder_control.rs:144`)
  does handle one. That is plan-144-B's concern, but the contrast is noted here.
- plan-142 made **non-record** sites share a seam (`self_update.rs`) and left
  records out (plan-142-A non-goal: "Records (`WITH`) stay out of the whole
  plan"). Phase 2 checks whether any record statement reaches
  `try_inplace_self_update`. If one does, it is a new fact, because plan-141 ran
  before the seam existed.
- `r.prop = value` still does not parse for an ordinary record
  (`MFB_PARSE_RECORD_FIELD_ASSIGNMENT`, plan-141 C.5). That form stays a
  single `n/a` row.

### Verified properties

- **A `MUT` record captured by a non-escaping `forEach` `LAMBDA` can
  self-update.** Probe `/tmp/p144x/rec_lambda`, in which
  `collections::forEach([1,2], LAMBDA(v AS Integer) -> r = WITH r { s := r.s & "x" })`
  appears on a local `MUT r AS R`, builds (`Wrote executable`, 2026-09-21). S9 is
  therefore a real column.
- **A module-level `MUT` record self-update compiles.** The same probe's
  `gr = WITH gr { s := gr.s & "x" }` builds.

## 3. Design Overview

The table fill follows plan-141 §3 exactly. Rows are overloads and forms, and
columns are record sites. Each cell records the lowering path the statement takes
(`NirOp::Assign` → `lower_with_update`, or `StoreGlobal`), the recogniser on it (if
any), and the first gate that declines, in code order. One generated `--ncode` build
covers every cell. It is read with plan-141's `markers.py`, extended with a
`WITH`-rebuild marker (plan-141 C.2 already reports `WITH=y`).

**Rows, and why not 323 of them.** A record arm is specific to a builtin: it matches
a `native_builtin_target` name. So each overload whose type *has* a record arm
(collection-typed) or *could* get one (`String`, per plan-143) gets its own row.
The 220 overloads of the 20 other types get **one row per type**. The row claims
that no recogniser anywhere names a builtin of that type, so the verdict cannot
depend on which function is called. Phase 2 proves the claim with a grep of every
arm's builtin names against those 220. A failed grep splits that type back into
per-overload rows, which is not a reason to stop.

Row families (§0 of the findings):

| Family | Rows | Source |
|---|---|---|
| F1 collection overloads | 59 literal + the `collections` generics that type-check (4 known) | census |
| F2 `String`/`AttributedString` overloads | 44 literal + the Phase 1 generic count | census |
| F3 operators | `s & t` (and a chain `s & t & u`), scalar `x + k` / `x - k` on `Integer`/`Float` | plan-141 §1b |
| F4 per-type rows | 20 types | census |
| F5 non-self-update forms | a replacement not derived from the field, for each field kind (scalar, `String`, `List`, `Map`, `Set`, record), and `r.prop = value` (not expressible) | plan-141 R2 and R6 |

plan-144-B uses the same row list for its `STATE` table. That is why the census is
§0 of the shared findings file and not part of this letter's table.

**Where the risk concentrates:** false `y` verdicts, as in plan-141. There is also a
new risk, a **false per-type row**: if any arm names a function of an F4 type, one
row is hiding different verdicts. The Phase 2 grep is the guard against that.

**Byte-identity is not a gate here**, because no code changes. The only check is
that each cell's reading agrees with the probe's marker. When the two disagree,
both are recorded and the dump wins.

Rejected alternatives:

- **Copying plan-141 §2's record cells.** They were measured at `b6a10efbc`, before
  plan-142 added nine letters of arms and the seam. They are re-probed.
- **One row per overload for all 323.** The 220 F4 rows would repeat one verdict
  per type. The Phase 2 grep proves that they do, and it costs less than 220 rows of
  identical evidence.

## 4. Field-position rules

These are plan-141's rules, stated from the code, which is re-read in Phase 2:

- A field is **inlined** when `record_field_is_inlined`
  (`builder_collection_layout.rs:706`) says so. At plan-141, the inlined kinds were
  `String`, collections, nested records, data unions and `Result`, and the
  fixed-width scalars were not inlined. Phase 2 records how each F4 type is
  classified (for example, whether a `vector::Float3`, `color::Color` or
  `big::Int` field is inlined).
- **S3 / S4** for an inlined field: the field is not / is the last inlined field.
  For a non-inlined (fixed-width) field: the field is not / is the record's last
  field. Position cannot matter for a non-inlined field, and the probes confirm
  that.

## 5. Record sites (the columns)

| Site | Meaning | Statement shape |
|---|---|---|
| S3 local rec, not-last | field of a local `MUT` record that is not the last inlined field | `r = WITH r { f := op(r.f, …) }` |
| S4 local rec, last | the record's last inlined field (last field, for a fixed-width type) | same |
| S5 global rec | any field of a module-level `MUT` record | same, `r` global |
| S6 nested | a field of a record that is itself a field | `r = WITH r { in := WITH r.in { f := op(r.in.f, …) } }` |
| S7 loop-live | S4 inside `FOR EACH v IN r.f` | same, inside the loop; `n/a` for a non-iterable field type, with evidence |
| S9 captured | S4 on a `MUT` record assigned inside a non-escaping `forEach` `LAMBDA` | same, inside the lambda (Verified properties) |
| S10 two-field | S4 plus a second scalar update in the same `WITH` | `r = WITH r { f := op(r.f, …), n := k }` (plan-141 R4, as a column) |

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial work. Strike moot tasks through with
> evidence, and never delete them. **An unticked box means NOT DONE.**

### Phase 1: Row census (shared with plan-144-B)

This phase delivers the full row list that both letters fill, and it measures the
last UNMEASURED population first.

- [x] Measure the generic self-update overloads (the UNMEASURED row). Read the
      registry signatures of every package for `Var`/`Arg(n)` returns that can
      equal the first parameter's type. For each candidate, write one line in
      `/tmp/plan-144-probes/gen/` and use `mfb build` to decide whether the
      self-update type-checks, as plan-141 C.3 did. Record the count and names in
      Measured populations.
      — `census.py target/release/mfb generic` → 18 candidates, all `collections`;
      `mfb build gen` → 14 errors (2× `TYPE_CALL_ARGUMENT_MISMATCH` for `getOr`,
      12× `TYPE_ASSIGNMENT_MISMATCH`), so 4 type-check: `transform`, `mapValues`,
      `reduce`, `reduceRight` (findings C.1).
- [x] Create `planning/plan-144-findings/record-state-self-update-audit.md` with
      these sections: header (HEAD hash, compiler build time), site legend (§5 of
      this plan, plus a `STATE` placeholder that plan-144-B fills), §0 row census
      (F1–F5 with counts), §1 record table
      (`| row | form | S3 | S4 | S5 | S6 | S7 | S9 | S10 | evidence |`),
      §2 `STATE` (heading only, for plan-144-B), §3 summary (heading only), and
      the appendices (census script, arm inventory, probes).
      — assembled by `assemble.py` from `findings_template.md` (HEAD `efdb54bb7`,
      compiler 14:13); `grep -n '^## ' …` shows the site legend, §0, §1, §2, §3 and
      Appendices A, B, C.
- [x] Generate the §1 rows with the census script, which is kept in Appendix A.
      Take signatures verbatim from `mfb man <pkg> <f>`, one row per F1/F2
      overload and one per F4 type (with its overload count), then add the F3 and
      F5 rows. For F2 rows, copy the form column's value from plan-143 §4 if
      plan-143's findings exist. Otherwise classify the form from the function's
      lowering, citing it, and mark it "(plan-144)".
      — `rows.py` (findings C.2) → 326 rows. plan-143's findings do not exist
      (`ls planning/plan-143-findings` → absent), so all 44 F2 forms were
      classified from their lowering and are marked `(plan-144)`, with citations
      in findings A.2. The F4 rows are per type for 5 types and per overload for
      15 types (the Phase 2 guard failed, Correction A1).

Acceptance: every census row exists in §1.
  Check: `sed -n '/^## 1\./,/^## 2\./p' planning/plan-144-findings/record-state-self-update-audit.md | grep -c '^| F[1-5]'`
  → 59 + 44 + generic count + F3 rows + 20 + F5 rows, the total the §0 table
  states (est. 1 min).
  Measured 2026-09-21: → `326` = 63 + 44 + 6 + 206 + 7, the §0 total. (The F4
  term is 206, not 20: Correction A1.)
Commit: 20988bdcd (Phases 1–3 landed together: one findings file)

### Phase 2: Map the record paths

This phase records which code decides every record cell, before any cell is
filled.

- [x] Arm inventory (Appendix B). For each of the 9 `try_inplace_record_field_*`
      arms, record the builtin(s) and arity it matches, the dispatch site, and its
      ordered gate list (read `resolve_inplace_record_field`,
      `inplace_dest.rs:310`, and the arm body). Note every difference from
      plan-141 Appendix B.3.
      — findings B.1. `fndiff.py b6a10efbc HEAD …` → `same` for all 9 arms, RF,
      `inplace_call_args`, `admits` and `record_collection_last_inlined`. The only
      differences are line numbers and the new seam call before the arm chain
      (`bc:1240`).
- [x] Seam check: record whether any `WithUpdate` statement reaches
      `try_inplace_self_update` (`self_update.rs:234`), by grepping its callers and
      the construction sites of `SelfUpdateSite`. Cite the answer.
      — findings B.2. The two callers are `bc:1102` (`StoreGlobal`, gated on a
      `Call`/`&` shape) and `bc:1240` (`Assign`, unconditional); `SelfUpdateSite`
      is built only at `bc:1092`/`bc:1234`. A local `WITH` reaches the seam and
      every arm declines at `G2`. A global `WITH` never reaches it (`SUG=-` in all
      339 S5 probes).
- [x] F4 guard: grep every arm's matched builtin names (all 48 `try_inplace_*`,
      plus `SELF_UPDATE_TABLE` rows) against the 220 F4 overloads. Record the
      command and a result of 0. A non-zero result means that type's row is split
      into per-overload rows, and a Correction is logged.
      — findings B.3: the result is **71, not 0** (`math` 40, `vector` 27, `big` 3,
      `datetime::add` 1), so 15 types were split into 201 per-overload rows
      (Correction A1).
- [x] Inlined classification: for every field type in the rows (the 4 collection
      shapes, `String`, `AttributedString`, and the 20 F4 types), record
      `record_field_is_inlined`'s answer with the branch that gives it.
      — findings B.4, measured by probe `inl`. Inlined: `String`,
      `AttributedString`, collections, records (`Inner`, `big::Int`,
      `color::Color`, 3 `datetime`, `http::Response`, 9 `vector`). Not inlined:
      `Integer`, `Float`, `Fixed`, `Money`, and `json::Json` (a recursive union,
      stored as a pointer).
- [x] Record the two copy paths with citations: `lower_with_update` (every field
      gathered, a new block built, the old block freed), and `StoreGlobal` for S5.
      Also record whether a `String` field's self-concat capacity shadow
      (`string_capacity_slot_for`) can exist for a field (it is keyed by local
      name). This decides whether F3 `&` can ever be `y` at S3/S4.
      — findings B.5 and paths P1/P2. No shadow can exist for a field: it is keyed
      on a local name and created only for a `name & …` chain (`bc:2374`), so F3
      `&` is `n` at every record site.

Acceptance: Appendix B lists all 9 record arms with dispatch site and gates. The
F4 guard result, the seam answer and the inlined table are each cited.
  Check: `grep -cE '^\| try_inplace_record_field_' planning/plan-144-findings/record-state-self-update-audit.md`
  → 9, plus `grep -c 'F4 guard' …` ≥ 1 (est. 1 min).
  Measured 2026-09-21: → `9`, and `grep -c 'F4 guard'` → `4`.
Commit: 20988bdcd (Phases 1–3 landed together: one findings file)

### Phase 3: Fill the record table

- [x] Generate one `--ncode` build in `/tmp/plan-144-probes/rec/` with one SUB per
      (row, site). Use the probe types from §5 (`TYPE Rec { a, b }` with both
      fields the row's type, so that `a` is S3 and `b` is S4, then `n AS Integer`
      for S10, which is not inlined and so keeps `b` last; plus a nested and a
      global variant). Keep the generator in Appendix C.
      — `gen_rec.py` (findings C.3) → `2100 functions, 0 excluded`, built without
      diagnostics. The probes are FUNCs that return the record, so the update is
      live. S10 uses a separate `RecN { a, b, n }`, so S4's `b` is still the last
      field for a fixed-width type.
- [x] Read the build with plan-141's `markers.py` (Appendix C.1 of the plan-141
      findings), copied into Appendix C. Extend it to report the record arms'
      marker slots and the `lower_with_update` rebuild. Record the marker line next
      to each verdict.
      — `markers.py` (findings C.4) was extended with the 26 plan-142 arms, the
      seam-global and the STATE markers. `lambdas.py` maps each S9 lambda to its
      FUNC (339 of 339). Each row's evidence cell carries every site's marker
      line.
- [x] Fill every cell with `y`, `n (<first declining gate or path>)` or `n/a`
      (with the reason: for example, S7 for a non-iterable type, with the
      diagnostic from a probe). A `y` names every gate it passed.
      — `fill_rec.py` (findings C.9) → `rows 326 cells 2282 disagreements 0`. The S7
      `n/a` cells cite `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`, 23 of 23
      types). The `y` cells' path A names RF's gates and the arm's gates
      (findings B.1).
- [x] Re-verify plan-141 §2's 22 record cells against the new table, and list
      every cell whose verdict changed since `b6a10efbc`, with the commit that
      changed it (`git log -S` on the deciding symbol).
      — findings §1b. plan-141 C.4 was rebuilt verbatim: every `r1`–`r5` and
      `x_concat_S3/S4` marker is identical, so no record verdict changed. The one
      changed line, `x_concat_S2`, is a global-`String` cell, changed by `02692cd64`
      (plan-142-H).

Acceptance: no empty cell in §1, and every `y`/`n` cell has a probe marker.
  Check: `sed -n '/^## 1\./,/^## 2\./p' planning/plan-144-findings/record-state-self-update-audit.md | grep '^| F' | grep -cE '\|\s*\|'`
  → 0 (est. 1 min).
  Measured 2026-09-21: → `0`. Every `y`/`n` cell's marker is in its row's
  evidence, and `fill_rec.py` checked all of them against the dump.
Commit: 20988bdcd (Phases 1–3 landed together: one findings file)

## Validation Plan

- Tests: none (no code).
- Coverage check: the §1 row count equals the §0 census (Phase 1 check). The 9
  record arms are all in Appendix B (Phase 2 check).
- Runtime proof: not applicable. `--ncode` cross-checks are used instead.
- Doc sync: none. Contradictions go to plan-144-B's summary for the fix plan.
- Final gate: `git diff --stat HEAD -- . ':!planning'` shows nothing this plan
  wrote (the pre-existing dirty files in the prerequisite are unchanged) (est. 1 min).
  The full suite is not run: no code changes, so it cannot see this work
  (plan-141 Correction 8).

## Open Decisions

- **F4 per-type rows vs per-overload.** Recommended: per-type, guarded by the
  Phase 2 grep, which splits a type back out on any hit. The alternative, 220
  per-overload rows, adds no information unless the guard fails.
- **Re-verify plan-141 §2 or cite it.** Recommended: re-verify (Phase 3's last
  task), because plan-142 landed nine letters since. The alternative is to cite it
  and probe only the new rows.

## Corrections

- **A1 — the F4 guard failed, so 15 types are per-overload rows (206 F4 rows, not
  20).** §3's premise was that no arm names a builtin of an F4 type. Measured:
  the arm names hit 71 of the 220 F4 overloads (findings B.3: `math::abs/…/tan` on
  `Integer`/`Float`/`Fixed`/`Money` 40, `vector::abs/max/min` 27, `big::abs/add/pow`
  3, `datetime::add` 1). Following Phase 2's rule, the 15 types with a hit are
  split into 201 per-overload rows, and the 5 without one (`color::Color`,
  `datetime::DateTime`, `datetime::Duration`, `json::Json`, `http::Response`) keep
  one row each. Phase 1's acceptance total is therefore 63 + 44 + 6 + **206** + 7 =
  326, where the check text had `+ 20`. The design did not depend on the per-type
  claim. A name hit never fires an arm here: every hit's arm also gates the
  binding's collection type (`G9`/`G10`), and no seam arm runs at a record site
  (B.2). All 201 split rows show the same verdicts as the per-type reading
  predicted (`fill_rec.py` → 0 disagreements). The 5 unsplit types probed every
  overload, and each type's markers are identical across its overloads.
- **A2 — prerequisite 1 was stale when the plan started.** Main's
  `target/release/mfb` (13:34) was older than the newest `src/` commit
  (`6ed5cc234`, 14:03; app-mode, not lowering). The audit reads the worktree, so the
  worktree's compiler was built with `cargo build --release` (exit 0, 14:13) and the
  row re-measured → MET. The plan's quoted HEAD `9e728044e` is superseded by
  `efdb54bb7`, which is the HEAD the findings record.
- **A3 — plan-143's findings do not exist.** plan-143 has not been run
  (`ls planning/plan-143-findings` → no such directory), so no F2 form could be
  copied. All 44 were classified here from the lowering, with citations (findings
  A.2) and the `(plan-144)` mark, per Phase 1's fallback. Three are not what the
  name suggests. `upper`/`lower`/`caseFold` are `rewrite`, because the Unicode
  table can change the width. `pathDirName`/`pathNormalize` are `rewrite`,
  because `""` becomes `"."`. `repeat` is `rewrite`, because `times = 0` shrinks.
- **A4 — the citation `builder_collection_layout.rs:706` is the method wrapper.**
  The `record_field_is_inlined` logic is the free function at
  `builder_collection_layout.rs:3152`, which the findings cite.
- **A5 — "plan-141 §2's 22 record cells" means 22 rows (88 cells).** All 88 were
  re-verified (findings §1b).
- **A6 — `StoreGlobal` now reaches the seam, but not for a `WITH`.** §2 carried
  plan-141's "S5 is `n (StoreGlobal)`" with the path "`StoreGlobal` dispatches no
  recogniser". Since plan-142-H (`02692cd64`) `StoreGlobal` calls
  `try_inplace_self_update` for `g = f(g, …)` and `&` chains (`bc:1087–1102`). A
  `WITH` value matches neither shape, so the S5 verdict stands. The path is now
  "StoreGlobal: the seam's shape gate is false for a `WithUpdate`" (P2), confirmed by
  `SUG=-` in all 339 S5 probes.
- **A7 — the probe also needs a two-field record type distinct from `Rec`.** §5 put
  `n` in the same `Rec` as `a`/`b`. For a fixed-width `T` that would make `b` not the
  last field at S4. The probe uses `Rec { a, b }` for S3/S4/S5/S6/S7/S9 and
  `RecN { a, b, n }` for S10 only (findings C.3), which keeps §4's definition of S4
  exact.

## Summary

This letter is a read-only table fill over the shared row census (59 collection +
44 `String` overloads, 4 generics, 6 operators, 206 F4 rows after the F4 guard
split, and 7 non-self-update forms: 326 rows) at seven record sites. The main risk is false `y` verdicts, and a new risk
is a per-type row that hides a function-specific arm. The Phase 2 guard grep
handles the second. plan-144-B fills the same rows at `RES … STATE` sites and
writes the summary.

## Appendix: census

The census is plan-142-A's Appendix script (`planning/completed/plan-142-A-self-update-guard-and-seam.md`)
with the type test replaced by:

```python
if ft and ft == ret:
    hits.append(s)
```

It was run on 2026-09-21 against `target/release/mfb` (built 13:24, HEAD
`9e728044e`) and printed `packages 42 overloads 828` and 323 hits. Family split:

```python
import sys, collections
c = collections.Counter()
for l in sys.stdin:                      # census output minus its first line
    r = l.rsplit(" AS ", 1)[1].strip(); p = l.split("::")[0]
    k = "coll" if r.split()[0] in ("List", "Set", "Map") else (
        "str" if r in ("String", "AttributedString") else "other")
    c[(k, p)] += 1
for k, v in sorted(c.items()):
    print(v, k)
```

Results: coll 59, str 44, other 220. Per-type counts come from the same loop keyed
on `r`.
