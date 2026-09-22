# plan-145-C: Scalar fields, pointer fields and the mixed `WITH`

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-145-B

Prerequisites: see plan-145-A.

This letter closes the findings' largest record-vs-`STATE` gap (§3.2 item 1: 68
rows, 408 cells):

- `h.state.hp = h.state.hp - dmg` is a single store (Layer 1,
  `try_inplace_state_scalar_assign`, `builder_control.rs:144`).
- `r = WITH r { hp := r.hp - dmg }` rebuilds the whole record through
  `lower_with_update` (`builder_value_semantics.rs:703`). No record arm handles a
  non-collection field (`G17`).

After C, a record local gets Layer 1's twin. Two more cases become in place:

- a pointer field (`json::Json`) is replaced in its slot (plan-145-A Open
  Decision 4);
- a two-field `WITH` that pairs one arm-served field with scalar fields (S10/T5)
  is in place. `G14` declines it today (findings §3.2 item 6).

## 1. Goal

- `field_kinds.tsv` lines `Integer Float Fixed Money` flip to `arm` at S3, S4 and
  S10. The `json::Json` line flips to `arm` at S3 and S4.
- `field_expect.tsv`: the 10 arm rows flip to `arm` at S10, T5 (a scalar update
  beside the collection update). The 68 scalar rows flip to `arm` at S10.
- Failure atomicity: `r = WITH r { hp := r.hp - big, xs := append(r.xs, k) }`
  inside a `TRAP` whose operand overflows leaves `r` fully unchanged, both fields.
- Order of effects: two updates whose values call a printing `FUNC` print in
  source order.

### Non-goals

- `WITH` semantics stay the same: every update expression reads the **old**
  record, and updates are evaluated in source order.
- S5 (global) and S9 (by-ref) are letters G and H. This letter declines them.
- Inlined non-collection fields are letter F (fixed-size) or `Rebuild`.

## 2. Current State

- Layer 1 admits a `STATE` update iff every updated field is
  `!record_field_is_inlined && !record_field_is_pointer`. That is exactly
  `Integer Float Fixed Money` among the rows (findings B.6). It computes every
  value first (source order, spilled), then loads the `STATE` pointer once and
  stores each value at `8 * index`. `G25` declines an operand that can reach a
  `STATE` assignment.
- The record side has no twin. In a record block, a scalar field is inline in its
  8-byte slot, the same as in a `STATE` block (the same `record_fields`
  layout). A pointer field holds an owned pointer that the record's drop frees
  (`record_field_is_pointer`, `builder_collection_layout.rs:2815`).
- Record locals can carry facts keyed on the local: `local.constant`,
  `len_of_local`, `provable_index_locals`, and promoted float locals. Which ones
  can name a record **field** is UNMEASURED (Phase 1).

## 3. Design

**One scalar-store routine for both containers.** Replace Layer 1 with
`try_inplace_scalar_fields(site_owner, value)`. `site_owner` is a record local
(block in its slot) or a `STATE` resource (block through
`RESOURCE_OFFSET_STATE`). The eligibility, the source-order compute and spill,
and the stores are Layer 1's, unchanged. Only the block pointer's source
differs. For a record local, the other gates are:

- `G1` declines (by-ref is letter H);
- live `FOR EACH` gates do not apply, because a scalar is never an iterable and
  storing one moves no block;
- `G25` is `STATE`-only.

For the `STATE` owner, the `.ncode` must stay byte-identical. That is checked on
`rt_res_state_inplace_mutation`'s fixtures.

**Pointer fields (records only).** Evaluate the new value owned, spill it, load
the old pointer from the slot, drop it with `emit_owned_value_drop`, then store
the new pointer. The new value is built before the old one is freed, so
`j := json::set(r.j, …)` still reads the live old value. The drop is the same drop
the record's scope exit would run on that field.

**The mixed `WITH` (relaxing `G14`).** Admit a `WITH` with any number of updates
when exactly one update is a non-scalar field whose field-capable arm accepts it,
and every other update is scalar- or pointer-eligible. Order:

1. evaluate each scalar value **that precedes** the arm's update in source
   order, and spill it;
2. run the arm (its operands evaluate here; the arm is failure-atomic);
3. evaluate the remaining scalar values;
4. store all scalars.

Step 3 runs after the arm has mutated the arm's field. So admit the statement
only if no update after the arm's update reads the arm's field. The site's
`read_by` is the test. Otherwise decline to the rebuild. A failure in step 1 or 3
must leave the record unchanged. Step 1 is safe: nothing has been stored yet.
Step 3 is not, because the arm has already written. So also require that every
update after the arm's update cannot fail. The predicate for "cannot fail" is
UNMEASURED; Phase 1 finds it (candidates: `inline_builtin_is_infallible`,
`ir::fallible`). If no usable predicate exists, admit only the order
`scalars…, arm` (the arm last). Record that as a Correction.

Risk: the mixed `WITH`'s ordering and atomicity rules. A wrong admit drops or
reorders a write. Each rule gets a runtime case that asserts the **decline**.

## Phases

### Phase 1: Measure

- [x] Which per-local facts can be keyed on a record field: read `len_of_local`,
      `provable_index_locals`, `promoted_float_locals` and `local.constant` for
      record-field keys. Record, with citations, which facts a field store must
      invalidate.
      None can name a field. `len_of_local` is recorded only for `LET n = len(L)`
      with `L` a bare `NirValue::Local` (the `Bind` arm of `lower_ops_inner`,
      `builder_control.rs`), and `resolve_len_list` accepts only `len(Local)` or
      such an `n`; `provable_index_locals` is keyed on a loop induction variable
      over such an `L`; `promoted_float_locals` is keyed on `Float` locals. A
      `r = WITH r { … }` store is still the op `Assign r`, so every
      reassigned-locals set that names `r` still names it. The only fact keyed on
      the owner is its `LocalValue::constant`, and `local_constant_value`
      (`builder_value_semantics.rs`) answers `Some` only for a constant, a constant
      local, `toString` or `typeName` — never a record value. The stores clear it
      anyway, as the record arms did.
- [x] Is there a codegen-visible "cannot fail" predicate for an arbitrary scalar
      expression? Record the answer and its evidence (see §3).
      No. `inline_builtin_is_infallible` (`codegen/builtins/mod.rs`) answers for a
      built-in CALL by name and argument types; `ir::fallible` works on HIR/IR per
      call (and vouches only for project functions) and is not available at
      codegen. Arithmetic can overflow, so no expression with `+`/`-`/`*` qualifies.
      C adds two conservative NIR predicates (`builder_control.rs`):
      `nir_value_is_effect_free` (reads, arithmetic, logic, and calls of
      infallible built-ins only — a user function may print) and
      `nir_value_cannot_fail` (reads, comparisons, `AND`/`OR`/`NOT`, infallible
      built-ins; not arithmetic, not negation). §3's ordering is realized with
      them (Correction C3).

Acceptance: both answers recorded here (est. 20 min).
Commit: `831e85027` (with Phases 2 and 3)

### Phase 2: Scalar and pointer fields

- [x] `try_inplace_scalar_fields` for both owners. Layer 1 becomes a call to it.
      (`builder_control.rs`; `STATE` keeps Layer 1's exact emission order and
      its `state_field_inplace` slot name, a record local uses
      `record_field_inplace`; `emit_field_owner_block` loads either owner's block.)
- [x] Pointer-field replace for records. — and for `STATE` payloads (Open
      Decision 4 note): the new value is taken with `lower_value_stored_field`
      (the ownership a `WITH` store takes), every value is spilled first, then
      each displaced pointee is dropped (`emit_owned_value_drop`) and the block
      reloaded after the drops (they call the arena), then stored.
- [x] `NirOp::Assign`: try it before the field seam.
- [x] Flip the `field_kinds.tsv` lines. ~~Remove the matching `FIELD_KIND_TABLE`
      pending entries.~~ — moot: `FIELD_KIND_TABLE` rows name the letter that
      lands them (`Arm('C')`) and have no pending state; the runtime file carries
      it. `field_kinds_gen.py` now marks Scalar/Pointer `arm` at S3/S4/S10/T1–T5/T8:
      9 lines changed (`Integer Float Fixed Money Boolean Byte`, `json::Json`,
      `json::JsonArr`, `json::JsonObj`; `grep -c copy:C field_kinds.tsv` → 0).
- [x] RED proof per `.ai/testing-gates.md:809`: revert the `Assign` call and
      confirm the flipped lines fail the bound. Restore.
      The B compiler (`e5a8b1a9d`, built before C) is exactly C without the call
      for records: `MFB_TEST_EXE=<B mfb> MFB_SELF_UPDATE_FILTER='Integer|json::Json'
      MFB_SELF_UPDATE_SITES=S3,S4 … every_field_kind` → "8 of 22 case/site
      pair(s) failed": `Integer at S3/S4: marked arm, but 2000 more runs allocated
      2000 more blocks`, `json::Json`, `JsonArr`, `JsonObj` at S3/S4 "allocated
      14000 more blocks (want <= 1.125 x the control's 6000)". With C: pass.

Acceptance: `rt_res_state_inplace_mutation` fixtures byte-identical
(`scripts/artifact-gate.sh target/release/mfb 'rt-behavior/resources/*'`, est. 5 min).
Then `MFB_SELF_UPDATE_SITES=S3,S4,T1,T2 cargo test --test rt_inplace_self_update`
filtered to the kind lines → pass (est. 3 min).
The gate cannot take that selector (Correction C1); the direct check —
`/tmp`'s `ncode_diff.sh <B mfb> <C mfb> tests/rt-behavior/resources/*`, a
`cmp` of each fixture's `-ncode -target linux-x86_64` dump — → "same=50 diff=0
fail=0". `cargo test --test rt_res_state_inplace_mutation` → 24 passed. The kind
lines at every flipped site: `MFB_SELF_UPDATE_FILTER='Integer|Float|Fixed|Money|
Boolean|Byte|json::Json|json::JsonArr|json::JsonObj'
MFB_SELF_UPDATE_SITES=S3,S4,S10,T1,T2,T3,T4,T5,T8 … every_field_kind` → ok
(900.88 s).
Commit: `831e85027`

### Phase 3: The mixed `WITH`

- [x] Relax `G14` as §3 describes, in the field-site builder, for both owners.
      `try_inplace_mixed_with` (`builder_control.rs`): one inlined collection
      update plus scalar updates; the scalars are evaluated at the arm's first
      emission (`open_inplace_dest` drains `field_pre_emit`), so a declining arm
      emits none of them, then stored after the arm (Correction C3).
- [x] Runtime cases in `tests/runtime/rt_inplace_field_mixed.rs` (+ stanza):
  - the atomicity case in §1; — record and `STATE` (a `RES` parameter,
    Correction C4): the handler and the caller both read every field unchanged,
    and nothing leaks;
  - the effect-order case; — before the arm: in place, prints in order; after
    the arm: declines, prints in order;
  - a later update reading the arm's field (asserts a decline and the correct
    value); — both: an effect-free read is admitted (it is evaluated before the
    arm, so it reads the old value, which is what `WITH` means) and a read through
    a user function declines; both print the old length (Correction C3);
  - a pointer and a scalar in one `WITH`. — in place against a sink control,
    `alloc_calls = free_calls`, `live_bytes 0`.
  RED: against the B compiler the four in-place assertions fail ("record: 2000
  more runs allocated 6000 more blocks — a rebuild", …) while every value check
  holds.
- [x] Flip the S10/T5 lines in `field_expect.tsv`. `field_expect_gen.py` gained
      `LANDED = {"B", "C"}`; regenerating changes exactly 20 lines (`diff … | grep
      '^>' | cut -f2,3 | sort | uniq -c` → 10 `S10 arm`, 10 `T5 arm`). The matrix's
      nine `MIXED`/`C` `FIELD_PENDING` entries are removed; `cargo test --bin mfb
      self_update` → 6 passed (the nine fire at S10 and T5).
- [x] The harness's S10/T5 second update is `n := k` (a local), not
      `n := r.n + 1` (Correction C2).

Acceptance: `cargo test --test rt_inplace_field_mixed` passes, and
`MFB_SELF_UPDATE_SITES=S10,T5 cargo test --test rt_inplace_self_update` passes
(est. 8 min).
`MFB_TEST_EXE=target/release/mfb cargo test --test rt_inplace_field_mixed` → "5
passed". `MFB_TEST_EXE=target/release/mfb MFB_SELF_UPDATE_SITES=S10,T5 cargo test
--test rt_inplace_self_update` → both tests ok (558.90 s): every `cases.tsv` line
and every kind at S10 and T5, the 20 flipped lines included.
Commit: `831e85027`

## Validation Plan

- Tests: the kind lines, S10/T5 lines, `rt_inplace_field_mixed`.
- Goldens: every committed fixture with a scalar record-field `WITH` is expected
  to diff (the rebuild becomes stores). Name them in Phase 2 by running
  `scripts/artifact-gate.sh target/release/mfb all` once. Objdump one per
  directory to confirm the diff is the store replacing `lower_with_update`, then
  regenerate only those. Record the count and the command.
  **Result:** `scripts/artifact-gate.sh target/release/mfb all` → "1484 tests, 1659
  build(s), 2096 golden(s) checked, 5 diff(s)", all five
  `byte-identity/audio/audio_codegen_cover_rt.<target>.ncodesum`. Localized
  (`/tmp` `ncode_fn_diff.py` over the B and C `-ncode -target linux-x86_64`
  dumps): 1 of 64 functions changed, `__audio_mmlApplyLegato` — 2349 → 2274
  instructions, the rebuild's slots (`with_target`, `with_old_field`,
  `record_build_*`, `reassign_value`) gone and `record_field_inplace` new — which
  is `ev = WITH ev { fadeIn := fi, fadeOut := fo }`
  (`helper_mml_apply_legato.rs`), two scalar fields now stored in place.
  Regenerated with `bash scripts/regen-native-goldens.sh target/release/mfb
  tests/byte-identity/audio` → "5 build(s), 5 golden(s) rewritten, 0 failure(s)".
- Per-letter unit gate: `cargo test --bin mfb`.

## Corrections

- **C1 — the artifact gate takes no fixture glob.** `scripts/artifact-gate.sh
  target/release/mfb 'rt-behavior/resources/*'` prints the usage and runs nothing:
  its selector is `all` or a byte-identity package. And no `rt-behavior/resources`
  fixture has an `.ncode` golden (`find tests/rt-behavior/resources -name '*.ncode*'`
  → none), so no gate run could see the `STATE` store's bytes. The byte identity was
  checked directly instead: each resources fixture built `-ncode -target
  linux-x86_64` by the B and the C compiler and compared with `cmp` (50 same, 0
  differ).
- **C2 — the S10/T5 harness statement matches plan-144's site legend.** The
  legend's S10/T5 second update is `n := k` (a parameter); plan-145-A's harness
  wrote `n := r.n + 1`, which can overflow. Under §3's rule an update after the arm
  that can fail must decline, so that statement would never have been in place,
  whatever C landed. The harness (and the unit probes) now bind `LET k AS Integer
  = 7` and write `n := k`.
- **C3 — the mixed `WITH` evaluates every scalar before the arm.** §3 evaluates the
  scalars after the arm's update after the arm, and so declines when one can fail.
  C evaluates ALL of them before the arm mutates anything, at the arm's first
  emission: `open_inplace_dest` drains `CodeBuilder::field_pre_emit`, which runs
  after every gate, so an arm that declines has emitted nothing and the statement
  falls back to the rebuild intact. A scalar written after the arm's update then
  runs earlier than source order, which is admitted only when that cannot be
  observed: it has no effect (`nir_value_is_effect_free`), and either it cannot
  fail (`nir_value_cannot_fail`) or neither can the arm (its operands are
  effect-free and infallible and the operation raises nothing beyond them:
  `append add prepend removeKey` Set `remove`, Map `set`). Two consequences: a
  failing scalar can never follow the arm's write (atomicity holds by
  construction), and a later update that READS the arm's field reads the old value
  — exactly what `WITH` means — so it is admitted, not declined; the runtime test
  asserts the old value in both the admitted (effect-free) and the declined
  (read through a user function) shape. A pointer field is not admitted in a
  mixed `WITH` (its new value would be built before an arm that can still fail,
  and leak on that path); a `WITH` of scalar and pointer fields only is
  `try_inplace_scalar_fields`'s.
- **C4 — a function-level `TRAP` handler may not read the function's own
  resource.** The atomicity test first read `h.state` in the handler of the
  function owning `h`, and crashed on every compiler: the route into the handler
  closes the function's resources. That is bug-676 (`1e5627b86`), now a compile
  error; the `STATE` half of the test takes `h` as a `RES` parameter, which is not
  closed, and its caller reads it again.

- **C5 — `rt_res_state_inplace_mutation` was not re-run at C, and one of its
  tests encoded the old `G14`.** `a_second_updated_state_field_declines_to_the_rebuild`
  (plan-121-D) asserted that a two-field `WITH` over `.state` takes no arm, because
  a sibling's value would be dropped; C's mixed `WITH` takes the arm and stores the
  sibling, so the test failed from `831e85027` on (`MFB_TEST_EXE=<C compiler> cargo
  test --test rt_res_state_inplace_mutation` → 1 failed). Found at plan-145-F. The
  protected behavior (the sibling's value lands) holds and is checked at run time by
  `rt_inplace_field_mixed`; the assertion was corrected to the mixed contract (the
  arm plus `mixed_with_scalar`, no `state_assign_value`) and renamed
  `a_second_updated_state_field_is_stored_beside_the_arm`. It fails on main's
  compiler (no mixed `WITH`) and passes now.
- **C6 — the record twin: `codegen_inplace_record_field`'s two-field decline.**
  `a_second_updated_field_declines_to_the_record_rebuild` (plan-121-C, `b8138f8bc`)
  asserted a two-field `WITH` over a record takes no arm; found by plan-145-I's
  full gate. Corrected as C5 was: the arm, `mixed_with_scalar`, and no
  `with_target` (`a_second_updated_field_is_stored_beside_the_arm`); passes on the
  final compiler, fails on main's.

## Summary

Layer 1 becomes a routine for both containers, a pointer field is replaced in its
slot, and a `WITH` may carry scalars beside one arm. The risk is the mixed
`WITH`'s ordering, and it is fenced by decline tests.
