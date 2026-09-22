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
| plan-142 is complete and archived | `ls planning/completed/plan-142-I-lock-and-docs.md` → exists | MET (2026-09-21) |
| plan-144 is complete and its findings exist | `ls planning/completed/plan-144-B-state-self-update-audit.md planning/plan-144-findings/record-state-self-update-audit.md` → both exist | MET (2026-09-21) |
| bug-671 fixed: a `STATE` field passed to a source-generic `collections` member type-checks | `ls bugs/completed/bug-671-*.md` → exists, **and** `scripts/test-accept.sh target/debug/mfb target/accept-actual 'rt-behavior/resources/state-field-source-generic-arg-valid'` → passes (the regression test bug-671 names) | NOT MET (2026-09-21: `bugs/bug-671-state-field-arg-to-source-generic-is-unknown.md`, Status: Open; in progress in worktree `B-671`) |
| The release compiler exists for `--ncode` probes | `ls target/release/mfb` → exists | re-run before starting |

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
| Which inlined record kinds have a compile-time fixed size | UNMEASURED | Phase 1 |
| Harness cost per (line, site) pair | 2.7 s | plan-142-I Phase 1: 254 pairs in 679.52 s |

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

- [ ] Re-run the findings' record and `STATE` probes for the 11 bug-671 rows
      against the fixed compiler (`gen_state.py` + `fill_state.py`, findings
      Appendix C.10/C.14) and record their T-site verdicts here. All are expected
      to be `n (no arm)`. A `y` is a finding to investigate, not a pass.
- [ ] Classify every package record type by compile-time byte size: a record is
      `InlinedFixed` when every field is a fixed-width scalar or an
      `InlinedFixed` record. Record the list and the command. The list sizes
      letter F.
- [ ] Write `field_expect_gen.py` and generate `field_expect.tsv`. Record the line
      count, and the count per `expect` value, each with its command.

Acceptance: the three results are recorded here with their commands (est. 30 min).
Commit:

### Phase 2: Harness field sites

- [ ] `rt_inplace_self_update.rs`: the 15 field sites and their templates, the `x`
      rewrite, the `field_expect.tsv` reader and its four statuses, and
      `MFB_SELF_UPDATE_SITES`. A line or site missing from `field_expect.tsv`
      panics with its name.
- [ ] `field_kinds.tsv` and its reader. It includes the `arm+value` control
      program: the same expression bound to a fresh `LET`.
- [ ] RED proof: flip one `copy:` line to `arm` (for example
      `collections::filter` at S4) and confirm the bound fails, naming the line.
      Then flip S4 `collections::append` to `copy:B` and confirm the "still
      copies" assertion fails. Restore both.

Acceptance: `cargo test --test rt_inplace_self_update` passes, with every field
pair run (est. 55 min: 1,185 pairs at 2.7 s each. This is the one run that
checks every expectation against today's compiler. A subset would leave some
expectations untested, and letters B–H flip them on the assumption they held).
Commit:

### Phase 3: Unit census and matrix

- [ ] `FIELD_KIND_TABLE` and `field_kind_census_covers_every_record_field_type`
      in `self_update.rs`.
- [ ] The black-box twin in `tests/guards/inplace_self_update_census.rs`.
- [ ] The matrix's field sites and `FIELD_PENDING` (every field pair pending on
      its letter), with the pending pairs asserted **not** to fire.
- [ ] RED proof: delete the `vector::Float3` row, and confirm both censuses fail
      naming it. Remove one `FIELD_PENDING` entry, and confirm the matrix fails
      naming the pair. Restore both.

Acceptance: `cargo test --bin mfb self_update && cargo test --test inplace_self_update_census`
→ pass; RED results recorded here (est. 10 min).
Commit:

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
   DECISION:
2. **A cannot-reallocate arm at a not-last field (S3/T1/T3).** Recommended: admit
   it if letter D's Phase 1 shows that record size, copy and free read each
   field's stored offset and never sum field sizes. That result is recorded in D.
   Then shrinking or rewriting a middle field leaves slack that nothing reads.
   Otherwise `G17` stays as it is for every arm.
   DECISION:
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
   DECISION:
4. **A pointer field (`json::Json`) in a record.** Recommended: in letter C.
   Compute the new value first, then free the old pointee and store the pointer
   into the slot. That is a type-driven store like Layer 1, with one free added.
   `json::Json` cannot be a `STATE` type (findings §2b), so this is records only.
   DECISION:
5. **The `RES` parameter that declares an invalid `STATE` type** (findings §3.5:
   `SUB g(RES h AS fs::File STATE P_json__Json, …)` compiles). This is not a
   self-update problem, so it is not in this plan. Recommended: file it with
   `/write-bug`.
   DECISION:

## Corrections

## Summary

A extends plan-142's harness to the 15 field sites, generates one expectation per
(case, site) from plan-144's cell tables, adds a census over field *kinds*, and
opens the matrix to field sites with every pair pending on its letter. It changes
no codegen. Every later letter flips the lines it lands, and nothing can flip
silently.
