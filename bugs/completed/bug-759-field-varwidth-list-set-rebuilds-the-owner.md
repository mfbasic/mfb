# bug-759: `set` on a variable-width list in a record or `STATE` field rebuilds the whole owner every statement

Last updated: 2026-09-28
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Other (performance — an O(n) rebuild per statement where the in-place contract promises O(1)-ish)

Status: Fixed (2026-10-03, bug-staging-5)
Regression Test: `tests/runtime/rt_inplace_field_varwidth_set.rs`; `tests/codegen/codegen_inplace_record_field.rs` (`set_on_a_variable_width_record_list_grows_the_record`)

`r = WITH r { xs := collections::set(r.xs, j, v) }` and `h.state.xs = collections::set(h.state.xs, j, v)` run in
place when `xs` is a `List OF Integer`, and when `xs` is a plain local `List OF String`. When `xs` is a
**variable-width** list (`List OF String`, and any element type `list_element_is_fixed_width` rejects) held in a
record field, a nested record field or a `STATE` field, the self-update arm declines, and the statement takes the
general path. It rebuilds the owner (`with_target`; for `STATE` also `state_assign_value`) and copies the whole
list, **every statement**. A loop that rewrites every element is therefore O(n²). The program's output is correct;
only its cost is wrong, which is why no test caught it. The self-update harness's only `List` `set` line is
`List OF Integer`.

**The single correct behaviour a fix produces:** at the owner's **last inlined** field, a variable-width list
`set` runs in the list's own sub-block (no `with_target`, no `state_assign_value`, no whole-list copy). Its per-call
cost is flat in the list's length, as it is for a plain local, and when the data region must grow it grows the
**record** block through `InlineGrow`, as `append` / `insert` / Map `set` already do.

References:

- `.ai/collections.md`, "In-place mutation", the `FieldReach` table: the `Realloc` / growing route row already lists
  "the variable-width kinds of the `NoRealloc` list/map arms (their repack or reserve)" as growing the record
  through `InlineGrow` at the last inlined field. The code does not do this for `set`; the notes describe the
  intended design and this bug is the gap.
- `.ai/collections.md`: "In-place `MUT` self-update is a guaranteed cost bound, not an optimization (owner decision,
  bug-652): an arm that declines anywhere else … is a bug."
- `mfb spec memory collections`, *Cost*: "A self-update costs the elements it adds, removes or moves … It never costs
  the size of the part of `x` it leaves alone."
- Found during plan-149-C3 (`planning/completed/plan-149-C-catalog-docs-bench-gate.md`, Open Decisions, "The
  variable-width list `set` rows"): benchmark rows `test_lrd_set` / `test_lsd_set` were the only 2 of the 16
  `LET cur` rows plan-149's forward could not bring to direct-form speed, because the direct form itself rebuilds.
  Measured there: 23.268 → 17.011 ms (`list (Record-Dynamic) set`) and 22.787 → 17.105 ms (`list (State-Dynamic)
  set`) per run, against 0.008 ms for the fixed-width `set` rows.

## Failing Reproduction

`bugs/repro/bug-759-field-varwidth-list-set.mfb` times one `set` per element, as ns per call, at two sizes:

```
mkdir -p /tmp/b759/src && cp bugs/repro/bug-759-field-varwidth-list-set.mfb /tmp/b759/src/main.mfb
printf '{"name":"b759","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}\n' > /tmp/b759/project.json
target/release/mfb build -q /tmp/b759 && (cd /tmp/b759 && ./build/b759.out)   # run from the project: it opens project.json
```

Observed (macOS aarch64, `target/release/mfb` at `96cc421e1`, 2026-09-28):

```
n=1000 localDyn=92 fieldDyn=53267 stateDyn=41830 fieldFix=4 ns/set
n=4000 localDyn=26 fieldDyn=156119 stateDyn=156752 fieldFix=4 ns/set
```

- Observed: record-field and `STATE`-field `List OF String` `set` cost 42–156 µs per call and grow with n.
- Expected: flat in n and the same order as `localDyn` (tens of ns).

The emitted code shows why (`mfb build -ncode -target linux-x86_64`, per-function `stackSlots`):

| Function | `with_target` | `state_assign_value` | in-place slots |
|---|---|---|---|
| `localDyn` (plain local `List OF String`) | 0 | 0 | `inplace_set_index`, `inplace_set_item` |
| `fieldDyn` (record field, `List OF String`) | **1** | 0 | none |
| `stateDyn` (`STATE` field, `List OF String`) | **1** | **2** | none |
| `fieldFix` (record field, `List OF Integer`) | 0 | 0 | `inplace_inlined_subblock`, `inplace_recfield_set_index`, `inplace_recfield_set_item` |

Contrast cases, measured with a second probe (`/tmp/b759g`, same method):

| Site | n = 1000 | n = 4000 | Verdict |
|---|---|---|---|
| module-global `List OF String` (`gl = set(gl, …)`) | 333 | 25 | works ✓ (a plain block, not a field) |
| nested field `o.inner.xs`, last inlined | 78,437 | 206,118 | fails ✗ (same arm) |
| field `xs` followed by an inlined `String` field (not last) | 39,691 | 158,046 | fails ✗, but see Non-goals |

## Root Cause

`src/codegen/collection/assign/builder_inplace_assign.rs:lower_field_set` returns `Ok(false)` for any list whose
element is not fixed-width:

```rust
if crate::codegen::collection::layout::list_element_is_fixed_width(&element_type).is_none() {
    return Ok(false);
}
```

Its doc comment gives the reason: "a longer replacement makes that branch reachable, and it installs a fresh block no
sub-block can receive". "That branch" is `set_inplace_rebuild` in
`src/codegen/collection/list/list_mutate.rs:lower_list_set_in_place_with`. **That reason is stale.** Since bug-627,
a length-changing `set` on a list with an entry table never reaches `rebuild`:

- **shorter:** overwritten where it lies;
- **longer, at the data tail:** extended in place;
- **longer, elsewhere:** written at the aligned data tail;
- **no room:** `overflow`, which calls `emit_repack_list_data(…, None)`.

The only remaining `branch(&rebuild)` is in the `else` of `if entry_stride != 0`, reachable only by entry-free
(kind-2, fixed-width) lists, and the code comment there says that path "cannot reach the length-change path". So
the one place a variable-width field `set` can need a new block is the `overflow` repack. `emit_repack_list_data`
already takes `inline: Option<InlineGrow>` and implements the record grow: `emit_inline_grow_extend_size`,
`emit_inline_grow_split`, and `emit_inline_grow_free_old` instead of freeing a sub-block. Its other callers already
pass it: `builder_inplace_rewrite.rs` (the tail-room helper) and `list_compact.rs`. `lower_list_set_in_place_with`
hard-codes `None` and has no parameter to carry one, so the field route cannot pass it, and the arm declines instead.

Why the contrast cases are immune:

- a **plain local** or **global** list is its own block, so the `None` repack is right for it;
- a **fixed-width** element never changes length, so `resize` / `overflow` are unreachable and the sub-block route
  suffices (the arm's first branch).

## Goal

- At a record field, a nested record field, a by-ref-captured record's field and a `STATE` field that is the
  owner's **last inlined** field, a variable-width list `set` emits no `with_target` and no `state_assign_value`,
  and reaches `lower_list_set_in_place_with` through the inlined sub-block.
- The repro's `fieldDyn` and `stateDyn` are flat in n, within 2× of `localDyn` at n = 4000.
- A `set` whose new payload forces the repack grows the **record** block (`InlineGrow`), and the owner's other
  fields read back unchanged. For `STATE`, the moved block is republished (`close_inplace_dest`, `O4`).
- Benchmark rows `test_lrd_set` and `test_lsd_set` drop from ~17 ms to the order of the other in-place `list
  (…-Dynamic)` rows (`test_lrd_removeAt`: 0.79 ms).

### Non-goals (must NOT change)

- **Output, at every level and every site.** This is a cost fix only.
- **A not-last inlined field keeps declining.** Growing an earlier sub-block would shift every later sibling's
  offset (the `G17` rule `lower_field_set_add` enforces through `field_is_last_inlined`). The shorter / tail /
  in-room paths would not need a grow, but which one runs is only known at run time, and emitting both an in-place
  and a rebuild path per site is a larger design (see Open Decisions). The not-last row in the contrast table stays
  slow here.
- **Record layout, the collection block format, and the plain-local arm** are unchanged. The plain call site keeps
  passing `None`.
- **The kind-2 `rebuild` label stays** (it must be defined and must not fall through); the fix must not make it
  reachable.
- **Tempting wrong fix, forbidden:** deleting the `list_element_is_fixed_width` check alone. The field route would
  then hand a sub-block to `emit_repack_list_data(…, None)`, which frees the "pre-grow buffer": a pointer into the
  middle of the record block (the hazard `InlineGrow`'s own doc names), a use-after-free at the first overflow.
  The regression test must drive the overflow path so this fix is caught red.
- **Also forbidden:** making the benchmark or harness avoid the shape (e.g. storing the list in a local first).

## Blast Radius

Found by `grep -rn "list_element_is_fixed_width" src/codegen/collection/assign/` (one hit: `lower_field_set`),
`grep -rln InlineGrow src/codegen`, and every `emit_repack_list_data(` call site:

- `builder_inplace_assign.rs:lower_field_set`, `List` branch — **fixed by this bug**. It serves every field site
  (`S3`–`S10`, `T1`–`T8`) because they all route through `SELF_UPDATE_ARMS`; the nested-path and `STATE` rows above
  are the same decline.
- `list_mutate.rs:lower_list_set_in_place_with` — **fixed by this bug**: gains the `inline` parameter;
  `lower_list_set_in_place` (the plain-local wrapper) passes `None`, so plain locals and globals are byte-identical.
- `lower_field_set`, `Map` branch — **unaffected**: already passes `InlineGrow` to `lower_map_set_in_place`.
- `list_compact.rs` and `builder_inplace_rewrite.rs` (`emit_repack_list_data` with `inline`) — **unaffected**:
  already correct; they are the model for the fix.
- `list_mutate.rs:3564` (`emit_repack_list_data` from the append / insert grow) — **unaffected**: its callers pass
  their own `InlineGrow` or `None` already.
- **Not-last inlined field** — **latent, out of scope** (Non-goals): needs a runtime dual path.
- **Harness coverage gap** — `tests/runtime/inplace_self_update/cases.tsv` has one `set` line per signature, and
  the `List` one is `List OF Integer`, so `field_expect.tsv` has never measured a variable-width field `set`. That is
  how this stayed invisible. Closed by the Phase 1 tests (see Open Decisions for the harness).

## Fix Design

1. `lower_list_set_in_place_with` gains `inline: Option<InlineGrow>` and forwards it to the `overflow` branch's
   `emit_repack_list_data`. `lower_list_set_in_place` passes `None`, so every existing caller is unchanged.
2. `lower_field_set`, `List` branch, for a variable-width element:
   - decline unless `self.field_is_last_inlined(site)`, and unless no operand is read from the owner (the
     `field_realloc_admitted` rule the other growing arms use; `set(r.xs, j, r.xs …)`-style pointer operands must
     be snapshotted or declined);
   - open the destination, then read the field offset **before** any grow (`open_inplace_inlined_field_offset`),
     then open the sub-block, exactly as `lower_field_set_add` does;
   - lower index and item into slots as the fixed-width branch does, and call `lower_list_set_in_place_with(…,
     Some(InlineGrow { block_slot, field_off_slot }))`;
   - `close_inplace_dest` (publishes a moved `STATE` block, `O4`) and `forget_field_owner_constant`.
3. Keep the fixed-width branch as is: it never grows, so it stays on the sub-block at any field, last or not.
4. Update `lower_field_set`'s doc comment (the stale reason), the `FieldReach` table in `.ai/collections.md` if its
   wording changes, and `mfb spec memory collections` (field sites) where it describes what a field `set` does.

Where the risk is: the `overflow` path. It frees the old record (not the sub-block), republishes a `STATE` block,
and must recompute the entry pointer from the **new** sub-block after the split (the `relocated` label reloads
`buffer_slot`; `buffer_slot` must be the slot the split repoints). Each of these is a crash or silent corruption if
wrong, which is why Phase 1's test forces the overflow and reads the siblings after it.

Rejected alternatives:

- *Handle it in plan-149's NIR row, or special-case the benchmark shape.* The direct form itself rebuilds. No NIR
  rewrite reaches this; it is the arm's decline.
- *Route through the `rebuild` label with a sub-block-aware free.* That is the O(n) path bug-627 removed for locals.
- *Always grow in place, even at a not-last field.* That shifts later siblings (`G17`).

Expected output shifts: `.ncode` changes only where a program does a variable-width list `set` at a last-inlined
field. The benchmark is not a golden. `grep` the fixtures for the shape in Phase 1 to predict which goldens move,
and objdump ONE if anything else does.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_inplace_field_varwidth_set.rs` (register in `Cargo.toml`). At a record field, a nested
      field and a `STATE` field (each last inlined), it writes every element of a `List OF String` with a longer
      string, then enough longer strings to force the `overflow` repack. It asserts the list's contents **and** the
      owner's other fields after each phase, and an allocation bound in the harness's style (`--debug`,
      `arena.<k>.alloc_calls`: `count(2N) - count(N) < N/8`). It fails today on the bound, not on output.
- [ ] Add codegen cases to `tests/codegen/codegen_inplace_record_field.rs`: for each site, `with_target` = 0 and
      `state_assign_value` = 0 and an `inplace_…_set_index` slot present. They fail today (the repro table above).
- [ ] Record the fixture census for the shape (which goldens can move).

Acceptance: the new tests fail today for the documented reason (rebuild, not wrong output); the audit list above is
complete.
Commit: —

### Phase 2 — the fix

- [ ] `list_mutate.rs`: add `inline: Option<InlineGrow>` to `lower_list_set_in_place_with` and pass it to the
      `overflow` repack; the plain wrapper passes `None`.
- [ ] `builder_inplace_assign.rs:lower_field_set`: the variable-width `List` branch per Fix Design §2; rewrite the
      doc comment.
- [ ] Docs: `.ai/collections.md` (the `FieldReach` row and the `set` note) and the spec's field-site bullets.

Acceptance: Phase 1 tests pass; the repro's `fieldDyn` / `stateDyn` are flat in n and within 2× of `localDyn` at
n = 4000; the not-last contrast row is unchanged (still declines); output of every program unchanged.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/artifact-gate.sh target/release/mfb all`; regenerate only goldens whose diff is this arm (NIR is
      unchanged by a codegen fix, so localize with the `.ncode` of ONE fixture against the pre-fix compiler).
- [ ] `cargo test --no-fail-fast -- --skip artifact_gate_all`, `cargo test --release --test rt_inplace_self_update
      -- --ignored`, `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- [ ] Re-run the benchmark rows `test_lrd_set` / `test_lsd_set` (before / after, `--run 10`) and the repro.

Acceptance: full suite green; golden deltas are exactly this arm; both benchmark rows at the order of
`test_lrd_removeAt`.
Commit: —

## Validation Plan

- Regression tests: `tests/runtime/rt_inplace_field_varwidth_set.rs` (bound + contents + siblings, overflow forced)
  and the codegen cases in `tests/codegen/codegen_inplace_record_field.rs`.
- Runtime proof: `bugs/repro/bug-759-field-varwidth-list-set.mfb` flat in n; benchmark rows `test_lrd_set` /
  `test_lsd_set`.
- Doc sync: `.ai/collections.md` (`FieldReach` table, the variable-width `set` note), `mfb spec memory collections`
  (field sites), `lower_field_set`'s doc comment.
- Full suite: as Phase 3.

## Open Decisions

- **Harness coverage.** `field_expect.tsv` keys by `(signature, site)`, so a second `List OF String` line for the
  same `set` signature would collide ("duplicate line"). Recommended: the dedicated regression test above, plus a
  note in `cases.tsv`'s header that variable-width `set` is covered there. Alternative: key the harness by a line
  label, not a signature. That is a broader harness change and belongs to its own plan.
- **Not-last inlined field.** Recommended: leave it declining (Non-goals) and record it as the `rebuild:not-last-grow`
  shape it already is for `append`. Alternative: emit an in-place fast path plus a runtime fallback to the rebuild
  when `overflow` would be reached. That doubles the code per site and is a design of its own.

## Summary

The engineering risk is the overflow path at a field (record grow, old-record free, `STATE` republish, entry pointer
recomputed after the split). Everything it needs already exists and is exercised by `append` / `insert` / Map `set`;
the bug is one hard-coded `None` and a decline justified by a branch bug-627 made unreachable. Left untouched: the
not-last field, the plain-local and global arms, and the fixed-width field route.

## Resolution (2026-10-03)

As designed: `lower_list_set_in_place_with` takes `inline: Option<InlineGrow>` and
hands it to the overflow repack (`None` from the plain wrapper and from
`list_repeat`). `lower_field_set` admits a variable-width list at the owner's last
inlined field, reads the field offset after the operands and before any grow, and
passes `InlineGrow { block_slot, field_off_slot }`.

On the operand rule (Fix Design §2): `field_realloc_admitted` declines any operand
that reads the owner, which would also decline this bug's own shape
(`set(rec.xs, j, get(rec.xs, j) & "!")`). The hazard is narrower: an item that is
still a pointer into the owner when the repack frees it. `lower_value_stored` does not
copy a flat value. So an item that reads the owner is admitted only when its top
node is a `Binary`/`Unary` operation, which always builds a new value; anything
else (e.g. `get(rec.xs, k)` itself) declines. The index is an `Integer`.

Repro (macOS, `target/release`):

```
n=1000 localDyn=51 fieldDyn=53 stateDyn=40 fieldFix=4 ns/set     (was 53267 / 41830)
n=4000 localDyn=27 fieldDyn=28 stateDyn=28 fieldFix=4 ns/set     (was 156119 / 156752)
```

`rt_inplace_field_varwidth_set`: record, nested and `STATE` sites rewrite every
element twice with longer literals (the second overflows and grows the record),
and the list and owner siblings read back exact. Allocation growth from N=400 to
N=800 is under N/8 (main: 10 403). An aliasing-item case declines and stays exact.

Test change, with proof: `codegen_inplace_record_field`'s
`set_on_a_variable_width_record_list_declines` (plan-121-C, 021ebf944) asserted the
decline because the rebuild branch was believed reachable; this bug's root-cause
analysis disproves that (only the overflow repack can move the block, and it now
grows the record). The test now asserts the route runs **with** `InlineGrow` (the
field-offset slot), so it still pins the hazard it was written for. The in-place,
list and self-update suites pass. Spec: `memory/05_collections.md`. Notes:
`.ai/collections.md`.
