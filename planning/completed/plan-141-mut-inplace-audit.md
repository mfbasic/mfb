# plan-141: Audit which `MUT` self-updates the compiler performs in place

Last updated: 2026-09-20
Effort: medium (1h–2h)

A research spike that **changes no code**. It reads the compiler source and
records, for every `collections::*` function and for record-field updates,
whether a `MUT` binding that updates itself is mutated **in place** or
**rebuilt as a copy**. The result is one findings file of tables with columns
`function definition | binding site | in place y/n | evidence`. Every verdict
cites the code that decides it.

The rule the findings are measured against is the language owner's: **a `MUT`
updating itself mutates in place, with no copy, for every kind of value.** The
audit records where the compiler meets that rule and where it doesn't. Fixing
the gaps is a separate plan written from these findings.

Why now: while porting Brogue (`examples/brogue`), timing probes showed large
gaps. `collections::set` on a module-level `MUT List` cost 22,198 ns per write
against 36 ns on a local. On a module-level `Map` it cost 468,418 ns. On a
non-last list field of a local record it cost 70,231 ns
(`/tmp/inplace_probe`, 2026-09-20). Timings show *that* something copies, not
*which* code path decides it or what else is affected. This audit answers that
from the source.

References:

- `src/codegen/engine/control/builder_control.rs:1060` — `NirOp::StoreGlobal`
  lowering; `:1150–1260` — the local `NirOp::Assign` path and its
  `try_inplace_*` dispatch chain.
- `src/codegen/collection/assign/builder_inplace_assign.rs` — the in-place
  recognisers ("arms").
- `src/codegen/collection/assign/inplace_dest.rs` — `InPlaceDest` /
  `InPlaceGate`, the shared ownership and aliasing gates.
- `planning/completed/plan-121-gate-inventory.md` — the gate catalogue
  (G1–G24, E1–E2, O1–O4) and the arm-by-gate matrix. It is the starting map,
  **not** the answer: it was written at plan-121 and is re-verified against the
  code here.
- `.ai/collections.md` §"In-place mutation: one seam, one gate inventory".
- `mfb man collections` — the source of the function list and signatures.

## Prerequisites

None. This plan reads code and writes one findings file.

| Must be true | Command | Status |
|---|---|---|
| The release compiler exists, for the `--ncode` cross-checks | `ls target/release/mfb` → exists | MET (2026-09-20, re-run at follow-plan start: main checkout's `target/release/mfb` built 13:42, after the last `src/` commit at 12:32 — `git log -1 --format=%ci -- src`) |

## 1. Goal

- `planning/plan-141-findings/inplace-audit.md` exists and contains, for **every
  overload of every `collections::*` function** and for every record-update
  form in §4, a verdict per binding site (`y`, `n`, or `n/a` with a reason).
  Every `y`/`n` cites the deciding code as `file:symbol` and names the gate
  (`G1`–`G24`) or code path that decides it.

### Non-goals (explicit constraints)

- **No code, test, golden, spec or man-page changes.** The only files written are
  the findings file and this plan's checkboxes.
- **No `RES`, resources or `STATE`.** They are not ordinary variables. Every
  `STATE` arm (`try_inplace_state_*`, `builder_control.rs:139`, `:346–372`) is
  out of scope and appears in the findings only as "excluded".
- **Not timing-based.** A verdict comes from reading the code path. The one
  permitted cross-check is the compiler's own emitted native plan
  (`mfb build --ncode`) for a small probe, which is still reading code, not
  measuring time.
- No fix design. The findings may *name* the deciding gate, but proposing how
  to lift it belongs to the follow-up plan.

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| `collections::*` functions | 49 | `mfb man collections \| grep -oE 'collections::[a-zA-Z]+' \| sort -u \| wc -l` → 49 |
| Their overloads | 58 | parse each `mfb man collections <f>` page's `Overloads`/`Declaration` block (script in Phase 1) → 58 |
| Overloads returning a collection (a self-update `x = f(x, …)` is possible) | 34, in 32 functions | same script, return type starts `List`/`Set`/`Map` → 34 |
| …whose return type literally equals the first parameter's | 23, in 21 functions: `add append difference distinct drop filter insert intersection merge mid prepend remove removeAt removeKey replace set sort sortBy symmetricDifference take union` | same script → 23 |
| …generic-only (a self-update only when the type parameters coincide, e.g. `transform` with `T → T`) | 11 functions: `chunks flatten groupBy keys mapValues toList toSet transform values window zip` | same script → 11 |
| Overloads returning a non-collection (no self-update form; recorded `n/a`) | 24, in 17 functions | same script → 24 |
| In-place recogniser functions (`fn try_inplace_*`) | 30: 27 in `builder_inplace_assign.rs`, 3 in `builder_control.rs` (all 3 are `STATE`, excluded) | `grep -rn 'fn try_inplace_[a-z_]*' src --include='*.rs' \| wc -l` → 30 |

### What is already known (to be re-verified, not assumed)

- **Module-level `MUT`:** `NirOp::StoreGlobal` (`builder_control.rs:1060`)
  lowers every assignment as `lower_value_owned(value)` (deep copy of an
  aliasing source) plus a free of the old block. It has **no** `try_inplace_*`
  dispatch. Read; Phase 2 confirms that no other path reaches a global.
- **Function-local `MUT`:** `NirOp::Assign` tries, in order,
  `try_inplace_append_assign`, `…bulk_append…`, `…set_add…`, `…set…`,
  `…remove_key…`, `…prepend…`, `…remove_at…`, `…insert…`, `…set_remove…`,
  `…concat…`, then the `…record_field_*` family (`builder_control.rs:1166–1258`,
  read). An arm that declines falls through to the copying reassignment.
- **Record-field in place** requires the field to be the *last-inlined* `List`
  field (gate G17, gate inventory). Timing showed a first-field update copying;
  the audit finds the code that makes that true.
- **`r.prop = value`** does not compile for an ordinary record:
  `MFB_PARSE_RECORD_FIELD_ASSIGNMENT` (`src/rules/table.rs:187`), and
  `mfb man variable` says `WITH` is the only way to update an ordinary
  record's field. The findings record this form as "not expressible".

## 3. Design Overview

The audit is a table fill. The rows are the **overloads** (58, plus the record
forms in §4). The columns are the **binding sites** (§5). Each cell is found the
same way:

1. Find the lowering path the statement takes (`NirOp::Assign` vs.
   `StoreGlobal`, and for a record field the `WithUpdate` shape).
2. Find the arm that recognises that function on that path, if any.
3. Walk that arm's gates in the order the code checks them, and record the
   first gate that declines for this site, or `y` if none does.

For each **distinct** verdict (not each cell), one `--ncode` probe confirms the
reading: a small project, `mfb build --ncode`, then grep the dump for the arm's
in-place helper versus the copy path (`copy_collection_tight` /
`lower_value_owned`'s copy). A reading and a dump that disagree is a finding in
itself; record both, with the dump winning.

**Where the risk concentrates:** false `y`s. An arm that exists for a function
may still decline for a site through a gate far from the arm's own code (e.g.
G7 through a live `FOR EACH`, G12 through self-aliasing). Each `y` must name the
gate list it passed, not just the arm.

Rejected alternatives:

- **Timing probes per cell** — the approach that raised the question. The user
  asked for code verification. Timings also can't separate "copies" from "slow
  in-place path".
- **Trusting `plan-121-gate-inventory.md`'s matrix as-is** — it is an older
  snapshot, and arms were added after it (e.g. `remove_at`, `insert`,
  `set_remove` and the `record_field_*` family). It is the map; the code is the
  territory.

## 4. Record-update forms audited

| # | Form | Note |
|---|---|---|
| R1 | `r = WITH r { f := <new scalar> }` (f is `Integer`/`Float`/`Boolean`/`String`) | whole-record copy or field store? |
| R2 | `r = WITH r { f := <new List value not derived from r.f> }` | replacement, not self-update |
| R3 | `r = WITH r { f := collections::<op>(r.f, …) }` for every self-update-shaped overload | the `record_field_*` arms |
| R4 | `r = WITH r { f := …, g := … }` (two fields at once) | gate G14 "one update" |
| R5 | `r = WITH r { inner := WITH r.inner { f := … } }` (nested record) | |
| R6 | `r.prop = value` | not expressible today (`MFB_PARSE_RECORD_FIELD_ASSIGNMENT`); recorded with that evidence |

Each is evaluated at the record's position within its record (first, middle,
last field) and for a `List`, `Map`, `Set` and scalar field type.

## 5. Binding sites (the columns)

| Site | Meaning |
|---|---|
| S1 local | `MUT x` declared in a FUNC/SUB body |
| S2 global | `MUT x` at module level |
| S3 local rec, first | field that is not the last collection field of a local `MUT` record |
| S4 local rec, last | the record's last collection field |
| S5 global rec | any field of a module-level `MUT` record |
| S6 nested | a field of a record that is itself a field |
| S7 loop-live | S1 while a `FOR EACH` walks the same binding (G7) |

`STATE` sites are excluded (§1 non-goals).

## 6. Output format

`planning/plan-141-findings/inplace-audit.md`, in three sections:

1. **Collections**: one row per overload, grouped by function in `mfb man`
   order:
   `| function definition | S1 | S2 | S3 | S4 | S5 | S6 | S7 | evidence |`.
   Each site cell is `y`, `n (Gxx)` or `n/a`. The evidence cell names the arm
   (`file:symbol`) or states "no arm" and the path it falls to.
2. **Record updates**: one row per R-form × field type:
   `| form | field type | S3 | S4 | S5 | S6 | evidence |`.
3. **Summary**: counts of `y`/`n`/`n/a` per site, the list of deciding gates
   with the cells each decides, and every reading/`--ncode` disagreement.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` when the task's
> check has run and matched. `- [~]` for partial, with what remains. Moot tasks
> are struck through with evidence, never deleted. **An unticked box means NOT
> DONE.**

### Phase 1 — Row census into the findings file

- [x] Create `planning/plan-141-findings/inplace-audit.md` with the three
      section headings and the §5 site legend.
- [x] Generate the 58 collections rows (signature text copied verbatim from
      `mfb man collections <f>`) with a short script kept in the findings file's
      appendix. Pre-fill `n/a (returns <type>)` for the 24 non-collection
      overloads. — `census.py stats` → 49 functions, 58 overloads, 34 in 32
      collection-returning, 23 in 21 same-type, 24 in 17 non-collection. Pre-filled
      22, not 24: `reduce`/`reduceRight` return a free `U` and have a self-update
      form (Correction 2).
- [x] Add the R1–R6 × field-type rows. — 22 rows (`grep -c '^| R[1-6]'` → 22).

Acceptance: every overload has a row.
  Check: `grep -c '^| `collections::' planning/plan-141-findings/inplace-audit.md`
  → 58 (est. 1 min). **Ran: 58.**
Commit: 79a85fefa

### Phase 2 — Map the lowering paths

- [x] Confirm every assignment to a module-level `MUT` lowers through
      `NirOp::StoreGlobal` (`builder_control.rs:1060`) and that no
      `try_inplace_*` is reachable from it: grep every caller of each
      `try_inplace_*` and record them in the findings appendix. — Appendix B.1/B.2:
      `src/ir/lower.rs:1270`/`:2159` pick `IrOp::Assign` vs `IrOp::AssignGlobal`
      on `locals.contains_key`; `nir/lower.rs:328` → `StoreGlobal`; the caller grep
      finds arms only under `NirOp::Assign` (`bc:1166–1263`) and `NirOp::StateAssign`
      (`bc:1440`, `:1449`); global localization is not shipped
      (`optimizer/opt1/plans/globals.rs:43-48`).
- [x] List all ~~27~~ **19** non-`STATE` arms in `builder_inplace_assign.rs` with the
      function(s) and shape each recognises (`native_builtin_target` name,
      arity, `Call` vs `WithUpdate`), and the path that dispatches it. — Appendix
      B.3 (count corrected: Correction 1).
- [x] Record, per arm, the ordered gate list it checks (read
      `inplace_dest.rs:InPlaceGate::admits_with` and each arm's own checks).
      Note every difference from `plan-121-gate-inventory.md`'s matrix. — Appendix
      B.3 "ordered gates" column and B.4 (8 differences, incl. G17 widened to any
      inlined field, G24 lifted, new G26).

Acceptance: every one of the 30 recognisers is listed as audited (~~27~~ 19) or
excluded as `STATE` (~~3~~ 11), each with its dispatch site.
  Check: `grep -cE '^\| try_inplace_' planning/plan-141-findings/inplace-audit.md`
  → 30 (est. 1 min). **Ran: 30.**
Commit: cd5245ddc

### Phase 3 — Fill the collections table

- [x] For each of the 34 collection-returning overloads, fill S1–S7 with the
      verdict and the first declining gate, per §3. — also the 2 `reduce*`
      overloads (Correction 2) and a new column S9 (Correction 3). 10 overloads have
      arms (y at S1 and S4 only); 17 have no arm; 9 cannot type-check.
- [x] For each generic-only function (11), record whether the self-update form
      type-checks when the type parameters coincide, and its verdict. — 2 type-check
      (`transform` U=T, `mapValues` U=V; plus `reduce`/`reduceRight` U=List OF T):
      n (no arm). 9 do not: `mfb build gen` → 9 × `error[2-203-0008
      TYPE_ASSIGNMENT_MISMATCH]` (Appendix C.3).
- [x] One `--ncode` probe per distinct verdict (not per cell), in
      `/tmp/plan-141-probes/`; record the grep line that confirms it next to
      the verdict. — one build covered every cell (206 SUBs, Appendix C.2), plus
      `gen` (C.3). Every row's evidence names its probe SUBs; the marker lines are
      in C.2. Reading vs dump: 0 disagreements.
- [x] Add §1b for the two Open Decisions (String self-concat and scalar
      self-update rows; the S8 nested-collection row). — probes `x_concat_*`,
      `x_int_*`, `x_grid_S8` (Appendix C.4).

Acceptance: no empty cell in the collections table.
  Check: `grep -E '^\| `collections::' planning/plan-141-findings/inplace-audit.md | grep -cE '\|\s*\|'`
  → 0 (est. 1 min). **Ran: 0.**
Commit: 002f58f6d

### Phase 4 — Fill the record table and summarise

- [x] Fill R1–R6 × field type × S3–S6, with `--ncode` confirmation per distinct
      verdict. R6 cites `src/rules/table.rs:187` and a probe build's
      diagnostic. — 22 rows; probes `r1_*`…`r5_*` (Appendix C.4, 32 SUBs, every
      verdict matched); R6: `mfb build r6` → `error[1-102-0013
      MFB_PARSE_RECORD_FIELD_ASSIGNMENT]` (C.5).
- [x] Write the summary: per-site `y`/`n`/`n/a` counts, the deciding gates, and
      every reading/dump disagreement. — findings §3: 7 findings, 0 disagreements,
      4 `.ai/collections.md` contradictions, 2 leak observations for the follow-up.

Acceptance: no empty cell in the record table; the summary counts add up to the
table.
  Check: `grep -cE '\|\s*\|' <record table section>` → 0, and the summary
  total = cells in the two tables (count by hand from the file; est. 5 min).
  **Ran:** `sed -n '/^## 2. Record updates/,/^## 3. Summary/p' … | grep -cE '\|\s*\|'`
  → 0. Counts: computed by `summary.py` from the tables instead of by hand
  (Appendix C.7) — §1 58 × 8 = 464, §2 22 × 4 = 88, total 552 = the per-site
  columns' sum; §1b adds 3 × 8 = 24.
Commit: 1f1ccf5fe

## Validation Plan

- Tests: none. No code changes.
- Coverage check: the Phase 1 census (58 rows) against `mfb man collections`;
  the Phase 2 census (30 recognisers) against `grep 'fn try_inplace_'`. A
  function or arm missing from the findings fails those checks.
- Runtime proof: not applicable. Each verdict carries a `--ncode` cross-check
  instead.
- Doc sync: none. If a finding contradicts `.ai/collections.md` or
  `plan-121-gate-inventory.md`, list it in the summary for the follow-up plan to
  correct. This plan edits neither.
- Final gate: `git status --short` shows only `planning/plan-141-*` paths
  changed (est. 1 min). **Ran** before the Phase 4 commit: ` M
  planning/plan-141-findings/inplace-audit.md`, ` M planning/plan-141-mut-inplace-audit.md`;
  `git diff --stat main -- . ':!planning'` → empty.

## Open Decisions

- **`String` self-concat (`s = s & t`) and scalar updates.** Recommended:
  include one row each outside the collections table (the `concat` arm, G19–G21;
  scalars live in registers), because the rule covers "every kind of value".
  vs. collections and records only, as literally asked.
- **Collections nested in collections** (`grid = set(grid, i, set(get(grid, i), j, v))`
  on a `List OF List`). Recommended: one row, as a site S8, since it's the
  natural 2-D grid shape for game state. vs. leave it for the follow-up.

## Corrections

1. **Recogniser split (§2, Phase 2).** The plan said 27 in
   `builder_inplace_assign.rs` are non-`STATE`. 8 of those 27 are
   `try_inplace_state_*`: `grep -c 'fn try_inplace_state_' src/codegen/collection/assign/builder_inplace_assign.rs`
   → 8. So 30 = **19 audited + 11 `STATE` excluded** (8 there + 3 in
   `builder_control.rs`). Phase 2's "list all 27 non-STATE arms" is re-scoped to
   19. No other letter depends on the number; the Phase 2 check (30 rows) is
   unchanged.
2. **`reduce`/`reduceRight` are generic-only, not `n/a` (§2 populations).** Both
   return a free type parameter `U`, so `xs = collections::reduce(xs, init, f)`
   type-checks when `U = List OF T`. The census's "return type starts with
   List/Set/Map" rule counted them as non-collection. Their rows are audited like
   `transform`'s: 22 overloads pre-filled `n/a`, 36 audited
   (34 + these 2). The measured counts in §2 still hold for the rule they state.
3. **A binding site the plan did not list: S9, a `MUT` captured by a
   non-escaping `LAMBDA`.** `collections::forEach(xs, LAMBDA(v AS Integer) -> acc
   = collections::append(acc, v))` is a real, documented idiom
   (`tests/rt-error/functions/lambda-mut-foreach-valid`). Inside the lambda the
   capture is a `by_ref` local (`src/ir/lower.rs:5167`), which every arm declines
   (G1). Added as a column of §1 and §1b; probes `c_*_S9`.
4. **§5's S3/S4 definition is not the code's.** S4 is "the last *inlined*
   field", not "the last collection field": a `String`, nested-record, data-union
   or `Result` field declared after the collection field also blocks it (G17,
   `builder_collection_layout.rs:3152`). Probe `r3_list_then_string` → rebuild;
   `r3_list_then_int` → in place. The findings file states the code's definition.
5. **§2 "Record-field in place requires the field to be the last-inlined `List`
   field"** is out of date: since plan-121-C the record arms cover `Map` and `Set`
   fields too (`removeKey`, `set`, `add`, `remove`), and `insert`/`prepend`/`removeAt`
   on a `List`. The last-inlined requirement stands for all of them.
6. **Open Decisions resolved to the recommended options** (this skill does not
   negotiate scope): String self-concat and scalar self-update each get a row, and
   the nested-collection shape gets a row as S8, all in findings §1b.
7. **Probe scope.** Phase 3 asked for one probe per distinct verdict. One
   generated build covered every cell instead (206 SUBs, one `mfb build --ncode`,
   seconds), so no verdict rests on a representative. Same check, same tool.
8. **Finish-step CI command swapped for a scoped check.** The branch changes no
   code (`git diff --stat main -- . ':!planning'` → empty), so the project's full
   build-and-test run cannot see any of it; it would test `main`'s code again,
   for well over 10 minutes. The scoped check that catches the same mistake — an
   accidental non-`planning/` edit — is that diff, recorded under Validation Plan.

## Summary

A read-only table fill over 58 overloads, 6 record forms and 7 binding sites,
with every verdict tied to the gate that decides it and cross-checked against
the compiler's own emitted code. The known risk is false `y` verdicts from gates
that decline far from the arm. The output is the input to the follow-up plan
that makes every `MUT` self-update in place.
