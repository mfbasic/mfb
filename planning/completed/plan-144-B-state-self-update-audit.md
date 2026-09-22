# plan-144-B: Audit which `RES … STATE` payload self-updates the compiler performs in place

Last updated: 2026-09-21
Effort: medium (1h–2h)
Depends on: plan-144-A

This is the second half of the plan-144 audit, and it **changes no code**. It fills
§2 of `planning/plan-144-findings/record-state-self-update-audit.md`. For every row
of plan-144-A's shared census, §2 records whether a self-update of a
`RES … STATE` payload field, `h.state.f = op(h.state.f, …)`, mutates the existing
STATE block **in place** or **rebuilds** it. Each verdict cites the code that
decides it and is confirmed by an `--ncode` probe. This letter then writes the
summary for the whole audit (§3), which is the input to the fix plan.

The rule is the same as plan-144-A's: **a `MUT` updating itself mutates in place,
with no copy.** `STATE` is the one place where the language lets a field be
assigned directly (`mfb man variable` §"A handle can carry its own data: STATE").
The parser desugars that assignment into a single-field `WITH` over `h.state`
(`src/ast/stmt.rs:226`), so the question has the same shape as for a record.

References:

- plan-144-A, for the method, the census (§0) and the record table (§1), which §2 is
  compared against.
- `src/codegen/engine/control/builder_control.rs:1496` (`NirOp::StateAssign`). It
  tries Layer 1, `try_inplace_state_scalar_assign` (`:144`, bug-424), then Layer 2,
  `try_inplace_state_collection_assign` (`:351`: eight arms in order), then the
  whole-record replace, which frees the old block only under the bug-644 conditions.
- `src/codegen/collection/assign/inplace_dest.rs:371`
  (`resolve_inplace_state_field`: G13, G14, G16, G17, G10, G3/G4; "There is no
  `G1`").
- `src/codegen/collection/assign/builder_inplace_assign.rs`: the eight
  `try_inplace_state_*` collection arms (`:1140` `try_inplace_state_splice_assign`
  and its callers).
- `src/ir/shape.rs:1708`: the "State assignment target `…` is not a local binding"
  rejection.
- `planning/plan-141-findings/inplace-audit.md` B.3, which lists the 11 `STATE` arms
  as excluded and names gate G25 (`STATE`).
- Tests that pin today's STATE behavior, which are read and not edited:
  `tests/rt-behavior/resources/resource-state-field-assign-valid`,
  `state-scalar-inplace-decline-rt`, `bug487_state_mutating_operand`,
  `resource-union-state-drop-valid`, and `p121d-state-reach-rt`.

## Prerequisites

The plan-144-A prerequisites apply (see plan-144-A). In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-144-A is complete: §0 census and §1 table filled | `ls planning/completed/plan-144-A-*` → one file | MET (2026-09-21, worktree `P-144`: `planning/completed/plan-144-A-record-self-update-audit.md`; A's two rows re-checked at the same HEAD `efdb54bb7` and binary: MET) |

If plan-144-A is not complete, this plan cannot start.

## 1. Goal

- §2 of the findings file has one row per §0 census row, with a verdict at every
  `STATE` site in §4 (`y`, `n (<gate or path>)`, or `n/a` with a reason). Each
  `y`/`n` cites the deciding code and a probe marker.
- §2b records each **non-expressible** `STATE` site once, with its diagnostic.
- §3 summarizes plan-144 as a whole: per-site and per-family counts, the deciding
  paths, and every place the record and `STATE` verdicts differ for the same row.

### Non-goals (explicit constraints)

- **No code, test, golden, spec or man-page changes.**
- **No non-STATE sites.** Record sites are covered by plan-144-A. Plain bindings are
  covered by plan-141, plan-142 and plan-143.
- **The handle itself is not audited.** A `RES` handle is an alias, not a value
  (`mfb man variable` §"The exception: RES handles"). Only its `STATE` payload is
  audited. `List OF RES …` self-updates (`files = append(files, f)`) are F1 rows at
  plain-local sites and are out of scope.
- **Not timing-based.** No fix design.

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| Rows | 345 (§0 total, measured by plan-144-A Phase 1 and Correction A8) | `grep -c '^\| F[1-5]'` over §1 → 345 |
| `STATE` recognisers | 11: 3 in `builder_control.rs` (`scalar_assign`, `collection_assign`, `collection_append`) and 8 in `builder_inplace_assign.rs` | `grep -rhoE 'fn try_inplace_state_[a-z_]*' src --include='*.rs' \| wc -l` → 11 |
| Layer-2 dispatch order | 8 arms: `append`, `remove_key`, `set_add`, `set`, `remove_at`, `set_remove`, `insert`, `prepend` | read at `builder_control.rs:356-363` |

### Verified properties (probed 2026-09-21, in `/tmp/p144x/`)

These properties are expressible, and each probe returned `Wrote executable`:

- An owner local, `f.state.s = strings::trim(f.state.s)` and
  `f.state.s = f.state.s & "y"` (probe `nested_with`).
- The whole payload with two fields: `f.state = WITH f.state { pos := 1, s := f.state.s & "x" }`
  (`nested_with`).
- A nested payload record: `f.state.inner = WITH f.state.inner { x := 3 }`
  (`nested_with`).
- A `RES` parameter: `SUB g(RES s AS fs::File STATE Cur)` with
  `s.state.xs = collections::append(s.state.xs, 1)` and a two-field
  `s.state = WITH s.state {…}` (`state_param_multi`). The repo already covers the
  scalar form (`resource-state-field-assign-valid`, `SUB seek`).
- Loop-live: `FOR EACH v IN f.state.xs` with `f.state.xs = collections::append(f.state.xs, v)`
  inside the loop (`foreach_state`).
- A resource-union handle (`RES s AS Stream STATE Cursor`, a `{tag, record-ptr}`
  value, plan-74): `s.state.pos = 42`
  (`tests/rt-behavior/resources/resource-union-state-drop-valid/src/main.mfb:19-20`).

These properties are **not** expressible, and each diagnostic was observed:

| Shape | Diagnostic |
|---|---|
| module-level `RES g … STATE Cur`, `g.state.pos = …` in a FUNC | `error[2-203-0043 TYPE_UNKNOWN_VALUE]`, "State assignment target `g` is not a local binding." (`src/ir/shape.rs:1708`) (probe `global_res`) |
| `RES` field of a record: `r.h.state.pos = 3` | `error[1-102-0013 MFB_PARSE_RECORD_FIELD_ASSIGNMENT]` (`rec_field_res`) |
| deep payload path: `f.state.inner.x = 3` | `error[1-102-0013 MFB_PARSE_RECORD_FIELD_ASSIGNMENT]` (`nested_state`) |
| collection element: `fl[0].state.pos = 2` | `error[2-201-0015 SYMBOL_UNKNOWN_TYPE]` (`list_res`) |
| lambda capture: `forEach(…, LAMBDA(v AS Integer) -> f.state.pos = f.state.pos + v)` | `error[2-203-0019 TYPE_LAMBDA_CAPTURE_UNSUPPORTED]` (`lambda_res`) |

### What is already known (re-verified here, not assumed)

- Layer 1 stores a **fixed-width scalar** field in place, whatever the value
  expression is, so every F4 type that is not inlined should be `y` at an owner
  site. Phase 1 re-reads the eligibility test at `builder_control.rs:144`. The
  F4 types that plan-144-A found to be inlined (for example, a `vector::*` or
  `color::Color`, if inlined) take a different path.
- Layer 2 has arms for 8 collection operations and requires the last-inlined field
  (G17). A `String` field has no arm in either layer (the bug-644 comment at `:1496`
  mentions a `String` field that "never takes the inline-scalar fast path"), so a
  `String` payload field is expected to be `n`. This is the gap plan-143's rows
  carry over into `STATE`.
- The whole-record replace frees the displaced block only when the STATE type is
  sizable and no `FOR EACH` is walking the resource (bug-644, `:1496-1560`). Where
  the free is skipped, the cell is still `n`, and the leak is recorded in §3's
  "observed" list, as plan-141 §3.5 did.

## 3. Design Overview

The fill works the same way as plan-144-A. There is one generated `--ncode` build
in `/tmp/plan-144-probes/state/`, with one SUB per (row, site), each opening a
`RES … STATE P` handle. `P` is `a`, `b` (both the row's type), then
`n AS Integer`. `n` is not inlined, so `b` stays the last inlined field. T6 uses a
second payload type with `inner AS Inner` as its only inlined field. The build is read with plan-144-A's `markers.py`, extended with the `STATE`
arms' marker slots and a marker for the whole-record replace. Phase 1 identifies
that marker, because plan-141 never needed it.

**Where the risk concentrates:** as in plan-144-A, the risk is false `y` verdicts.
There is one extra `STATE` risk. Layer 1's scalar check reads each update's
**field type**, not the function, so a `y` there must be shown for an inlined-typed
F4 row as well as a fixed-width one. Otherwise a per-type row could claim `y` for a
type that is actually inlined.

Rejected: **inferring STATE verdicts from the record table.** The two paths share
`WithUpdate` but not a dispatcher (`StateAssign` vs `Assign`) or gates (no G1, and
G13 in place of the local-slot check). Only probing both can show where they
differ, and that difference is the finding the fix plan most needs.

## 4. `STATE` sites (the columns)

| Site | Meaning | Statement shape |
|---|---|---|
| T1 owner, not-last | owner-local `RES h … STATE P`, field `a` | `h.state.a = op(h.state.a, …)` |
| T2 owner, last | same, the last inlined field `b` (the last field, for a fixed-width type) | `h.state.b = op(h.state.b, …)` |
| T3 param, not-last | callee `SUB g(RES h AS … STATE P)`, field `a` | as T1 |
| T4 param, last | callee, field `b` | as T2 |
| T5 two-field | whole payload, two updates | `h.state = WITH h.state { b := op(h.state.b, …), n := k }` |
| T6 nested | a field of a record field of the payload | `h.state.inner = WITH h.state.inner { f := op(h.state.inner.f, …) }` |
| T7 loop-live | T2 inside `FOR EACH v IN h.state.b` | `n/a` for a non-iterable type, with evidence |
| T8 union handle | T2 on a resource-union handle (plan-74 `{tag, ptr}`) | as T2 |

§2b lists the five non-expressible shapes from Verified properties, each once, with
its diagnostic. They are not columns.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial work. Strike moot tasks through with
> evidence, and never delete them. **An unticked box means NOT DONE.**

### Phase 1: Map the `STATE` paths

- [x] Appendix B.2 of the findings. For each of the 11 `try_inplace_state_*`
      recognisers, record the builtin(s) and arity it matches, its position in the
      `StateAssign` order (`builder_control.rs:1496`, `:356-363`), and its ordered
      gates (`resolve_inplace_state_field`, `inplace_dest.rs:371`, plus the arm
      body). Pair each one with its `record_field_*` twin from plan-144-A
      Appendix B, and name every gate that differs.
      — findings **B.6** (B.2 was the seam check, Correction B5). The gates were
      read with `gates.py`. The arms match their twins gate for gate after the
      container. The containers differ in three ways: SC has no `G1`, `G16`
      replaces `G15` and runs before `G17`, and `G25` is added. Layer 1 has no
      twin.
- [x] Layer 1 eligibility: record, with a citation, which field types
      `try_inplace_state_scalar_assign` accepts. Check each of plan-144-A's inlined
      classifications against it.
      — B.6: `!inlined && !pointer` (`bc:186`) admits exactly `Integer`, `Float`,
      `Fixed` and `Money`; `json::Json` is refused as a pointer. Probe: `L1=y` in
      exactly the 68 ops of those four types, at T1–T5 and T8.
- [x] Record the whole-record replace path and the bug-644 free condition, and
      name the `--ncode` marker that identifies the replace. Confirm the marker on
      one probe (`h.state.s = strings::trim(h.state.s)`), and record the marker
      line.
      — §2 path legend and B.6. The marker is `state_assign_value` (`bc:1575`), and
      the free is `state_assign_replaced` (skipped under a live `FOR EACH`,
      `bc:1546-1560`). `r102o0_T2` (`h.state.b = strings::trim(h.state.b)`) →
      `ARM=- GLOBAL=- SUG=- WITH=y L1=- REPL=y FREE=y`.
- [x] Check whether any `StateAssign` reaches `try_inplace_self_update`
      (`self_update.rs:234`), and cite the answer.
      — B.6: no. The seam's only callers are `bc:1102` and `bc:1240`, and
      `StateAssign` (`bc:1496-1512`) calls Layer 1, then Layer 2, then the replace.
      Dump: the only `ARM=` values are `-` and the 8 `state_*` arms.
- [x] Record the parameter path: how `RES h` in a callee gets its STATE pointer,
      and whether any gate reads "is owner" (T3/T4 vs T1/T2). Do the same for the
      union handle's `+8` indirection (T8).
      — B.6: every path loads the handle's slot, applies `emit_resource_record_ptr`
      (`bvs:20`, `+8` for a union) and loads `RESOURCE_OFFSET_STATE`, and no gate
      reads ownership. The probe agrees: T3 = T1 and T4 = T8 = T2 in every row (0 of
      345 differ).

Acceptance: Appendix B.2 lists all 11 `STATE` recognisers with their order and
gates, and the replace marker is confirmed on one probe.
  Check: `grep -cE '^\| try_inplace_state_' planning/plan-144-findings/record-state-self-update-audit.md`
  → 11 (est. 1 min).
  Measured 2026-09-21: → `11` (the table is in findings B.6; Correction B5), and the
  replace marker was confirmed on `r102o0_T2`.
Commit: b1d5f93ce (Phases 1–3 landed together: one findings file)

### Phase 2: Fill the `STATE` table

- [x] Create the §2 table
      (`| row | form | T1 | T2 | T3 | T4 | T5 | T6 | T7 | T8 | evidence |`) with
      one row per §0 census row, generated by plan-144-A's census script.
      — `fill_state.py` over `rows.json` (the same `rows.py` census) → 345 rows.
- [x] Build one generated `--ncode` probe in `/tmp/plan-144-probes/state/` with one
      SUB per (row, site), and keep the generator in Appendix C. Read it with
      `markers.py`, extended with the STATE arm slots and the replace marker.
      Record each verdict's marker line.
      — `gen_state.py` (findings C.10) → `2470 called functions, 102 excluded`, and
      the `--ncode` build has no diagnostics. The 102 exclusions are 88 bug-671
      functions and 14 `json::Json` `TYPE_STATE_INVALID` functions (Corrections B1,
      B2). `markers.py` already carried the `STATE` markers from plan-144-A. Each
      row's evidence carries every site's marker line.
- [x] Fill every cell with `y`, `n (<first declining gate or path>)`, or `n/a`
      with a reason. A `y` names every gate it passed. When the reading and the
      dump disagree, record both, and the dump wins.
      — `fill_state.py` → `rows 345 cells 2760 disagreements 0`. The `y` paths are
      Layer 1 or the named Layer 2 arm, whose gates are in B.6.
- [x] Write §2b: the five non-expressible shapes, each with the probe source and
      diagnostic line (re-run them in `/tmp/plan-144-probes/state-na/`).
      — findings §2b and C.11 (`gen_statena.py`, one project per shape). All five
      fail with the diagnostic the plan recorded. `global_res` adds two follow-on
      errors from reading `g.state` (Correction B4).

Acceptance: no empty cell in §2, and the §2 row count equals §1's.
  Check: `sed -n '/^## 2\./,/^## 3\./p' planning/plan-144-findings/record-state-self-update-audit.md | grep '^| F' | grep -cE '\|\s*\|'`
  → 0, and the same `sed` piped to `grep -c '^| F'` gives the §1 count (est. 1 min).
  Measured 2026-09-21: → `0`, and `345` = the §1 count (`345`).
Commit: b1d5f93ce (Phases 1–3 landed together: one findings file)

### Phase 3: Summary for plan-144

- [x] §3.1: one paragraph stating when a record-field or `STATE` field
      self-update is in place.
      — findings §3.1 (one paragraph per kind of site: record, then `STATE`).
- [x] §3.2: findings, most consequential first. Each one names the deciding path
      and the cells it decides. Two must be included. The first is the
      **record vs `STATE` diff**: every row whose S4 and T2 verdicts differ, with
      the gate responsible (for example, a scalar field is `n (no arm)` in a
      record and `y` via Layer 1 in `STATE`). The second is the **`String` field
      gap** at both kinds of site, with the form column showing which `String`
      functions could be done in place at all.
      — findings §3.2, 7 findings. Finding 1 is the diff: 81 rows differ
      (`diff_s4_t2.py`). 68 are fixed-width scalars (`n (no arm)` vs Layer 1 `y`),
      and 13 are `STATE` `n/a`. Finding 2 is the `String` gap: 65 rows, 845 cells,
      with plan-143's forms shrink 22, same-len 4, grow 11, rewrite 21 and
      not-derived 5.
- [x] §3.3: the counts, per site and per family, for both tables. They are
      computed by a `summary.py` kept in the appendix, as plan-141 C.7 did, and
      the total must equal rows × (7 + 8).
      — findings §3.3 and C.15: §1 = 10 y / 2,120 n / 285 n/a; §2 = 438 y /
      1,941 n / 381 n/a; 5,175 cells.
- [x] §3.4: contradictions with `.ai/collections.md`, `mfb man variable`,
      plan-141's findings, and the STATE comments in `builder_control.rs`, for the
      fix plan to correct.
      — findings §3.4, 7 items: the `bc:1502-1508` "only `append`" comment, the
      `ipd:304-306` "last-inlined `List`" comment, `.ai/collections.md` "seven
      arms" and "every binding site", plan-141's `StoreGlobal` claims, this plan's
      own `json::Json` claim, and `mfb man variable` (no contradiction, noted).
- [x] §3.5: things observed that are not verdicts, such as leaks on the
      skipped-free replace path. Also record where plan-142's seam (`SelfUpdateSite`,
      `InPlaceDest::Inlined`) would and would not fit a record or `STATE` arm.
      — findings §3.5. It covers the S7 and T7 leaks (T7 `FREE=-` in 55 of 55),
      plan-141's S9 leak (now closed by `reassign_ref_old`), the invalid `STATE`
      type accepted on a `RES` parameter, and the seam fit.

Acceptance: the §3.3 total equals the cell count of §1 and §2.
  Check: `python3 <appendix summary.py> planning/plan-144-findings/record-state-self-update-audit.md`
  prints `total == rows*15` (est. 2 min).
  Measured 2026-09-21: → `total == rows*15: 5175 == 345*15 → True`.
Commit: b1d5f93ce (Phases 1–3 landed together: one findings file)

## Validation Plan

- Tests: none (no code).
- Coverage check: the §2 row count equals the §1 row count equals the §0 census, and
  all 11 `STATE` recognisers are in Appendix B.2.
- Runtime proof: not applicable. `--ncode` cross-checks are used instead.
- Doc sync: none. Contradictions are listed in §3.4 for the fix plan.
- Final gate (once, after Phase 3): `git diff --stat HEAD -- . ':!planning'` →
  nothing written by plan-144 (est. 1 min). The full suite is not run because no
  code changes (plan-141 Correction 8).

## Open Decisions

- **T3/T4 (parameter) as full columns, or one row of evidence.** Recommended: full
  columns. If no gate reads ownership (Phase 1 answers this), the columns will
  match T1/T2, which proves it cheaply. The alternative is a single note, which
  would be a guess until Phase 1 runs.
- **Inlined F4 types in `STATE`.** Recommended: probe every F4 type at T2, not a
  representative, because Layer 1's field-type test is exactly where a per-type row
  could be wrong.

## Corrections

- **B1 — 11 rows cannot be written at any `STATE` site (bug-671).** The first
  `STATE` probe build failed in 88 functions: `distinct`, `take`, `drop`, `sort`,
  `sortBy`, `union`, `intersection`, `difference`, `symmetricDifference`, `merge` and
  `mapValues` on `h.state.f`, at all 8 sites. The error is `error[2-203-0021
  TYPE_CALL_ARGUMENT_MISMATCH]` "cannot infer template arguments from `Unknown`".

  A minimal repro (`/tmp/plan-144-probes/stgen`) shows it is general: `LET ys = distinct(h.state.xs)`
  fails too, while `filter(h.state.xs, p)` compiles. The monomorphizer's
  `expression_type` has no `.state` arm, and its locals drop the `STATE` clause. This
  is a compiler defect, not a language rule, so it was filed as
  `bugs/bug-671-state-field-arg-to-source-generic-is-unknown.md` (`9db94aba4`) and
  not fixed here (this plan changes no code). Those cells are `n/a (does not compile:
  bug-671)`, and each row states the reading it would have if it compiled
  (`n (no arm)`).
- **B2 — `json::Json` cannot be a `STATE` field.** §2 "What is already known" said
  every non-inlined F4 type should be `y` at an owner site. `json::Json` is not
  inlined, but it is a pointer, so Layer 1 refuses it. More decisively, every handle
  declaration with it fails with `error[2-203-0085 TYPE_STATE_INVALID]` ("a copyable,
  defaultable data type"). Its 8 cells are `n/a`. The claim was corrected in findings
  §3.4 item 6.
- **B3 — the row count is 345, not "the §0 total as measured".** Following
  plan-144-A Corrections A1 and A8, §0 has 345 rows, and the §3.3 total is
  345 × 15 = 5,175.
- **B4 — `global_res` has two extra diagnostics.** The plan listed one,
  `TYPE_UNKNOWN_VALUE` "State assignment target `g` is not a local binding". That
  one reproduces at the assignment. Reading `g.state.pos` in the same function adds
  `TYPE_UNKNOWN_VALUE` and `TYPE_STATE_INVALID` ("`fs.File` here has no STATE to
  read"). Both are recorded in §2b.
- **B5 — the arm table is findings Appendix B.6, not B.2.** plan-144-A had already
  used B.2 for the seam check. The acceptance grep is location-independent and
  counts 11.
- **B7 — the bug was renumbered 670 → 671.** At merge time, the main checkout held
  another session's uncommitted `bugs/bug-670-metal-text-run-…md`, which claims the
  same number. This plan's bug document was renamed to
  `bugs/bug-671-state-field-arg-to-source-generic-is-unknown.md`, and every
  reference was updated (`grep -rc bug-670` over the plan-144 files → 0).
- **B6 — the final gate's scope.** `git diff --stat HEAD -- . ':!planning'` is empty
  at the end of B. The one file plan-144 wrote outside `planning/` is the bug-671
  document (`bugs/`, committed on its own as `9db94aba4`). It is a document, not
  code: the no-code non-goal holds (`git diff --stat efdb54bb7 HEAD -- src tests`
  → empty).

## Summary

This letter fills the `STATE` table for the same rows at eight payload sites,
records five non-expressible shapes once each, and writes the plan-144 summary. The
finding the fix plan most needs is the record-vs-`STATE` diff: the two paths share
the `WithUpdate` shape but have separate dispatchers and gate lists, so the same
field update can be in place in one and a copy in the other.
