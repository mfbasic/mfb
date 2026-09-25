# bug-689: `collections::get` of a record element copies the whole element, and a one-field element update cannot run in place

Last updated: 2026-09-24
Effort: large (3h–1d)
Severity: MEDIUM
Class: Other (performance: a self-update the compiler is expected to run in place instead rebuilds)

Status: Fixed
Regression Test: `tests/runtime/rt_list_element_record_borrow.rs`; the classifier's
unit tests in `src/codegen/engine/analysis/borrow_get.rs`

Reading a record out of a list copies the whole element into a fresh arena
block, every time. `collections::get(ps, i).lon` reads one `Float` field of
element `i`, but it allocates a copy of the whole element (and its trail) to
do it. That happens even when the element is a scalar-only record, and even
when the list is an immutable `LET`. Updating one field of one element has
the same problem.
`ps = collections::set(ps, i, WITH collections::get(ps, i) { f := … })` has
no in-place form: the element is copied out, updated in the copy, and copied
back. That is one allocation plus two element-sized copies per update.

The result is a large, silent cost on the natural record shape, `List OF T`
where `T` is a record. `examples/wind/src/flow.mfb` works around it with
nine parallel `List OF Float`/`Integer` module `MUT`s instead of a
`List OF Particle`. Its comment blames "a thousand small collections to
allocate". That is not the mechanism: records inline their collection fields
(`mfb spec memory heap-values`), so a `List OF Particle` is one block.

**Correct behavior after the fix.** Two changes:

1. A `get` of a record element whose result is only read (a field read, or a
   binding used only for field reads) does not allocate.
2. `xs = collections::set(xs, i, WITH collections::get(xs, i) { f := op(…) })`
   updates field `f` inside `xs`'s own block, the same way
   `r = WITH r { f := op(r.f, …) }` already does for a record local.

With both fixed, the probe's B, C, E, F and G variants allocate a bounded
number of blocks that does not grow with the iteration count, the same as A
and D do today.

## STATUS: FIXED (c0a6c1d97)

Both changes shipped, with two design deviations recorded in *Corrections*:
the list-element site is an element-bound `Record` owner rather than a new
`FieldContainer` variant (C2), and the two-statement form requires the `WITH`
and the `set` to be adjacent (C3). The probe's B, C, E, F and G no longer grow
with the frame count (C1). Also fixed here: a plan-86 E use-after-free over
`getOr` (C4). `examples/wind` now keeps its swarm as a `List OF Particle` (C6).
Found and filed: bug-695 (inline domain errors leak owned locals) and bug-696
(Windows marshalling buffers are never freed).

References:

- `mfb spec memory collections`, *Self-updates*: "the difference is that no
  second copy of `x` exists". The field-site list (record local, `STATE`,
  module-level record, nested record field) has no list element.
- `mfb spec language memory-semantics` §14: reads produce owned values. That
  is the language contract. Whether a read has to copy to honor it is up to
  the compiler.
- `.ai/collections.md` "get read-only borrow: pending-temp trap +
  MATCH-desugar chain": the plan-86 E precedent and its soundness gates.
- Found while reviewing `examples/wind` (parallel-array swarm) against a
  language-design review.

## Failing Reproduction

`bugs/repro/bug-689-list-element-record-copy.mfb` has seven variants selected
by `V`. Each runs 3200 elements × 2000 frames = 6,400,000 inner iterations.

```
mfb init /tmp/p && cp bugs/repro/bug-689-list-element-record-copy.mfb /tmp/p/src/main.mfb
mfb build -q --debug /tmp/p
for v in A B C D E F G; do V=$v /tmp/p/build/p.out 2>&1 | grep -E '^arena.0.alloc_(calls|bytes)'; done
```

Observed at `662910824`, macos-aarch64, `-O1` (and `-O3` gives identical
counts for E and F):

| V | shape | `arena.0.alloc_calls` | `alloc_bytes` |
| --- | --- | --- | --- |
| A | three parallel `MUT` lists, `set` self-updates | 47 | 1,088,192 |
| B | `MUT p = get(ps,i)`; `p = WITH p { lon, age, trail := set(p.trail,…) }`; `ps = set(ps,i,p)` | **6,416,019** | 1,234,246,144 |
| C | `LET p = get(ps,i)`; `ps = set(ps,i,p)` | **6,416,018** | 1,234,245,952 |
| D | `p = WITH p { … }` on one record local, 6.4M times | 7 | 656 |
| E | `sum = sum + get(ps,i).lon` (read only) | **6,416,018** | 1,234,245,952 |
| F | scalar-only `Dot{lon, age}`: `sum = sum + get(ds,i).lon` | **6,403,215** | 102,956,512 |
| G | immutable `LET ps`; `LET p = get(ps,i)`; `sum = sum + p.lon` | **6,406,418** | 514,485,056 |

- Observed: one allocation per `get` of a record element, sized as the whole
  element (192 B for B/C/E, 16 B for F).
- Expected: B, C, E, F and G allocate a bounded number of blocks, as A and D
  do.
- Wall clock, non-debug build (`time V=$v p.out`, user): A 0.10 s, B 0.52 s,
  E 0.36 s.

Contrast cases that are correct today:

- **D:** a `WITH` field self-update on a record local runs in place
  (plan-145). The `WITH` in B is not what costs.
- **A:** `set` self-updates on a `List OF Float` run in place.
- C's `set` back is a same-size in-place overwrite. C and E have identical
  counts, so all the cost is in the `get`.
- A `LET e = get(L, i)` used only as a `MATCH` scrutinee over an immutable
  plain-local `L` borrows today (plan-86 E).

## Root Cause

Two separate gaps.

**(1) `get` always copies a record element.** `lower_list_get_common`
(`src/codegen/builtins/collections/gen_list.rs`) loads the element as an
alias into the list's data region. It goes through
`materialize_owned_element` (`src/codegen/memory/owned.rs`), which, for any
`is_freeable_flat_value` type other than `String`, calls `copy_flat_block`
into a fresh arena block. The only way to skip that copy is the
`borrow_get_result` flag. It is set only for names in
`collect_borrow_get_locals` (`src/codegen/engine/function/function_lowering.rs`),
which requires all three of these:

- the binding is read only as a `MATCH` scrutinee (or through that match's
  `UnionExtract`s);
- the container is a plain `Local` that is bound at most once, never
  `Assign`ed, and not address-taken;
- the element is freeable-flat and not a `String`.

So every other read-only shape copies:

- a field read on the call result (`get(ps,i).lon`, E and F);
- a binding read only through field access (G);
- a `get` from a `MUT` container (C);
- a `get` from a module-level container (all of wind).

**(2) No list-element field site.** `FieldContainer`
(`src/codegen/collection/assign/self_update.rs`) has three variants:
`Record { local }`, `State { resource }` and `Global { name }`.
`field_self_update_site`/`peel_field_path`
(`src/codegen/engine/control/builder_control.rs`) recognise a `WITH` only
over one of those owners (`field_owner_is`). A
`collections::set(xs, i, WITH collections::get(xs, i) { … })` item operand is
none of them, so the `WITH` is built on a fresh copy of the element. That
copy is then passed to `set`. The `set` itself is an in-place same-size
overwrite (`lower_list_set_in_place`), but by then the copy has already been
made and paid for.

## Goal

- The probe's E, F and G variants report `arena.0.alloc_calls` below 100.
- B and C report `alloc_calls` below 100, with the element updated inside
  `ps`'s block.
- Output is unchanged, and `alloc_calls = free_calls` with `live_bytes 0` for
  every variant.
- `examples/wind` can be rewritten over `List OF Particle` with no
  per-particle-per-frame allocation. That is verified by the wind `--debug`
  report, not assumed.

### Non-goals (must NOT change)

- **Value semantics.** A `get` result that escapes still has to be an
  independent value. That covers a result that is stored, returned, passed to
  a call that may keep it, captured, or mutated, and one that stays live
  across a write to the container. The borrow is an optimisation that must
  not be observable.
- **Record and list memory layout**, and the fallible-call ABI.
- **`String` elements** keep their fresh-block materialisation. Skipping it
  leaks or dangles (`.ai/collections.md`).
- **Recursive (type-cycle) elements** keep the `needs_graph_copy` deep copy
  (bug-538).
- **Resource-bearing elements** keep today's alias behavior.
- **Tempting wrong fix: make `get` always return an alias.** That
  reintroduces bug-538/bug-601-class use-after-free. When the container is
  written or grown while an alias to one of its elements is live, the alias
  points at freed or overwritten memory. Every borrow needs a proven
  liveness bound.
- **Tempting wrong fix: change the probe or wind so they avoid `get`.** This
  bug exists to make the natural shape cheap.

## Blast Radius

Audit to complete in Phase 1. Known sites so far:

- `materialize_owned_element` (`src/codegen/memory/owned.rs`), used by
  list/map `get`/`getOr` — fixed by this bug, gap (1).
- Map `get`/`getOr` of a record value (`lower_map_get`,
  `src/codegen/builtins/collections/gen_map.rs`). Same copy, same fix; in
  scope.
- `FOR EACH e IN xs` over a `List OF Record`: the per-iteration element may
  be copied the same way. Measure in Phase 1 and classify.
- `builder_control` materialising bound elements (the other
  `materialize_owned_element` caller). Classify in Phase 1.
- `FieldContainer` (`src/codegen/collection/assign/self_update.rs`) — gap (2),
  fixed by this bug.
- `examples/wind/src/flow.mfb`. Its swarm comment states the wrong
  mechanism. Update the comment (and optionally the swarm) once the fix
  lands. Out of scope until then.

## Fix Design

**(1) Widen the read-only borrow.** Generalise `collect_borrow_get_locals`
from "MATCH scrutinee only" to "every read is borrow-transparent". A
borrow-transparent read is one of:

- a `MemberAccess` that yields a scalar, or a fixed-size record that is
  itself copied out;
- a `MATCH` scrutinee or `UnionExtract` (as today);
- a direct `get(ps, i).field` with no binding at all — a new classifier on
  the `MemberAccess` over a `get` call.

The hard part is the container-immutability gate. Today that gate is "bound
at most once and never assigned". That is sound but too coarse: it excludes
every `MUT` and module-level container, which is exactly where this pattern
lives. The replacement condition is that there is no write to the container,
and no call that can write it, between the `get` and the borrow's last read.
For the direct `get(ps, i).field` form that holds trivially, because the
borrow dies within its own expression. For a binding it needs a
straight-line liveness check. The `values_reach_store` / `StoreLeaf`
machinery already answers "can this operand store to X".

**(2) Add `FieldContainer::ListElement { list, index }`.** Recognise
`xs = collections::set(xs, i, WITH collections::get(xs, i) { f := … })`,
with the same `xs` and the same pure index expression `i`. Resolve the
element's block address with the existing `emit_element_value_offset`, then
reuse the existing field-site arms (scalar store, one-collection-beside-
scalars, fixed-size sub-record).

Two constraints:

- **Growth.** An operation that reallocates the field grows the element, and
  that changes the list's data layout. Admit it only where
  `lower_list_set_in_place`'s longer-payload rule allows: the element's span
  ends at `dataLength`, or the list is repacked. Otherwise rebuild, as
  `field_realloc_admitted` does for an inner field.
- **Failure atomicity.** A bad index must raise before anything is written.

Also accept the two-statement form `MUT p = get(xs, i)` … `xs = set(xs, i, p)`
when `p` is not otherwise escaped. That is what people actually write (B),
but it is harder: it needs `p` to be proven dead after the `set`. Stage it
after the single-expression form.

Rejected alternatives:

- **A new mutable `Array`/`Buffer` type.** It changes the language to work
  around a compiler gap, and it gives the language a second collection model.
- **A `collections::update(xs, i, fn)` builtin.** It is expressible today and
  would hit the same copy; it moves the problem rather than fixing it.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/runtime/rt_list_element_record_borrow.rs`, following
      `tests/runtime/rt_inplace_field_loop.rs`'s `--debug` report harness.
      Cover the E, F, G, C and B shapes, the map-value equivalent, and
      `FOR EACH`. Assert `alloc_calls` stays bounded as the iteration count
      grows (run N and 2N and require equal counts), plus
      `alloc_calls = free_calls` and `live_bytes 0`. Confirm the new
      assertions fail at HEAD. — 10 bounded positives (direct field, scalar
      record, binding over an immutable list, over a `MUT` list, over a
      module-level list, map value, C, B, B over a module-level list, the
      single-expression form); at `aa644f908` every one grew by exactly 64
      allocations per rep (one per `get`, `n() = 64`). `FOR EACH` measured
      separately: see the audit.
- [x] Add negatives that must still copy, and assert their output:
      - the result is returned;
      - it is stored into another list;
      - the container is written while a bound element is live;
      - the container is appended to (grown) between the `get` and the read;
      - a call that can write the module-level container runs between the two.
      — plus: an index operand that writes the module-level container (C5
      below), a read of the list between the `WITH` and the `set`, a read of
      the element after its write-back, a growing field update, and failure
      atomicity in both spellings. All passed at `aa644f908` except the
      failure-atomicity one, which leaked (bug-695, below).
- [x] Complete the blast-radius audit above, with a verdict per site. —
      `materialize_owned_element`, list and map `get`/`getOr`: fixed.
      `FOR EACH e IN xs` over a `List OF` record: unaffected — 138 allocations
      at 10 reps and at 20 (`FOR EACH p IN ps: sum = sum + p.age + len(p.trail)`),
      it walks the elements without copying them. "`builder_control`
      materialising bound elements": no such caller exists —
      `materialize_owned_element` is called only from `func_get`/`func_get_or`
      (`grep -rn materialize_owned_element src`); `owned.rs`'s module doc
      claimed otherwise and is corrected. `FieldContainer`: see C2.
      `examples/wind`: comment corrected; see C6.

Acceptance: the positives fail for the documented reason; the negatives
pass; the audit is complete.
Commit: c0a6c1d97

### Phase 2 — read-only borrow (gap 1)

- [x] Widen `collect_borrow_get_locals` and add the direct
      `get(...).field` classifier. Replace the whole-scope immutability gate
      with a no-write-between-get-and-last-read check that is sound for
      `MUT` and module-level containers. — `collect_borrow_gets`
      (`src/codegen/engine/analysis/borrow_get.rs`) keeps the plan-86 E form
      as *pinned* and adds the *windowed* one; `scalar_field_of_borrowable_get`
      (`builder_value_semantics.rs`) is the direct form.
- [x] Apply the same widening to map `get`/`getOr`.
- [x] Churn stress with interleaved allocations. Matching output does not
      prove the borrow fired; only the allocation count does
      (`.ai/collections.md`). — the negatives interleave list allocations
      after the write; the positives assert the count.

Acceptance: E, F and G are bounded; every negative still copies; no
leak.
Commit: c0a6c1d97

### Phase 3 — list-element field site (gap 2)

- [x] Add `FieldContainer::ListElement` and recognise the single-expression
      form, reusing the existing field arms. — as an element-bound
      `Record` owner instead of a new variant (C2).
- [x] Then the two-statement `get` → `WITH` → `set` form (B).

Acceptance: B and C are bounded; the failure-atomicity negatives (bad index,
a failing field operand) leave the list unchanged.
Commit: c0a6c1d97

### Phase 4 — expected outputs + full validation

- [x] Regenerate any `.ncode`/perf goldens the borrow shifts. Diff them and
      confirm the delta is only the removed copies. — 14 `.ncodesum`s on
      three fixtures, one function each, checked per function against the
      `aa644f908` compiler: `__audio_mmlApplyLegato` (the B shape; the `get`
      copy and the whole `set` gone, nothing added), `__regex_flatten` (the
      `firstLeaf` and `$match` copies and their frees), `__canvas_polygonEdges`
      (two `getOr` copies). `artifact-gate all`: 0 diffs.
- [x] Run the full suite. — `cargo test --no-fail-fast`: 228 binaries ok,
      `EXIT=0`. After merging `main` (no bug-689 code path touched): the golden
      gate over the merged tree, 2136 checked, 0 diffs.
- [x] Re-run the repro. Update `examples/wind/src/flow.mfb`'s swarm comment.
      If the swarm is converted to `List OF Particle`, compare its `--debug`
      allocation report before and after. — repro table in C1, on
      macos-aarch64, linux-aarch64 (2226) and windows-x86_64 (2230); comment
      updated; not converted (C6).
- [x] Doc sync: `mfb spec memory collections` *Self-updates* (the new field
      site, and the widened borrow in the `get` section); `.ai/collections.md`
      (the borrow gates).

Acceptance: full suite green; the repro is bounded for B, C, E, F and G.
Commit: c0a6c1d97 (fix, tests, goldens, docs); 83d5f25bc (wind comment);
5d876fbc9 (C8's golden); f414d4916 (merge of `main`)

## Corrections

- **C1 — the Goal's "below 100" cannot be met by this probe, under any
  lowering.** The probe builds 3200 elements, each with its own record (and
  for B/C/E a 12-slot trail list grown by `append`), so its setup allocates in
  proportion to the element count before a single frame runs. The property the
  bug is about is that the count does not grow with the FRAME count, and that
  is what the regression test asserts (N and 2N reps, equal counts). The probe
  after the fix (`mfb build -q --debug`, macos-aarch64, `V=$v p.out`):

  | V | `alloc_calls` before | after |
  | --- | --- | --- |
  | A | 47 | 47 |
  | B | 6,416,019 | 16,019 |
  | C | 6,416,018 | 16,018 |
  | D | 7 | 7 |
  | E | 6,416,018 | 16,018 |
  | F | 6,403,215 | 3,215 |
  | G | 6,406,418 | 6,418 |

  Every "after" row equals the probe's setup (5 per Particle, 1 per Dot, 2 per
  G element) plus a constant; `alloc_calls = free_calls`, `live_bytes 0` for
  all seven.
- **C2 — no `FieldContainer::ListElement`.** The element-bound binding `p`
  (or a hidden one for the single-expression form) holds the element's address
  in its slot, and the `WITH` is lowered with the ordinary
  `FieldContainer::Record { local: p }` — `emit_field_owner_block` already loads
  "the block" from the slot, so every field route works unchanged. What differs
  is only that nothing may grow that block: `field_is_last_inlined` answers no
  for the owner named in `element_updates.field_owner`, and every growing route
  asks it. The Fix Design's "admit growth where the span ends at `dataLength`"
  was not done: an update no route serves (a growing field, a `String` field,
  two collection fields) is lowered as the statement it stands for,
  `xs = set(xs, i, WITH p { … })`, whose in-place `set` already implements that
  rule.
- **C3 — the two-statement form needs the `WITH` and the `set` ADJACENT.** Not
  just "`p` dead after the `set`": between the `WITH` (which now writes the
  element early) and the `set` nothing may observe `xs`. Reads of `p`'s fields
  and of `xs` by lookup may come anywhere before the `WITH`.
- **C4 — a latent plan-86 E bug, fixed here.** A `MATCH` over
  `LET e = getOr(xs, k, <a default the statement builds>)` bound `e` to the
  default on a miss, and the statement freed the default at its end: `e` read
  freed memory (`a_match_over_a_get_or_miss_reads_the_default` printed `0` for
  `1230` at `aa644f908`). A binding over `getOr` now borrows only with an
  immutable local default.
- **C5 — the window includes the bind.** `LET p = get(gps, f())` with `f`
  storing to `gps` makes bug-496 snapshot `gps` into a statement temporary;
  a borrow would point into it after it is freed.
  `a_binding_whose_index_call_writes_the_module_level_list_is_a_copy` is RED
  without that check.
- **C6 — `examples/wind` converted after the fix landed (b6b04a811), measured
  through a harness, not the app's own report.** Wind's own `--debug` report
  is printed only on a normal exit (the SIGTERM handler that prints it is
  console-only), wind exits only on a key typed into its window, and headless
  it never gets past the "Fetching" screen (before or after the change). So the
  swarm was measured with a headless app-mode harness that compiles the
  example's own `flow.mfb`/`earth.mfb`/`grib.mfb` over a synthetic wind field
  with a fixed seed: old (seven parallel lists) and new (`List OF Particle`)
  print identical state at 100 and 200 frames and allocate identically per
  frame (+17,446,730 from 100 to 200 frames in both); `stepSwarm` has no
  allocation site left.
- **C7 — found: bug-695.** An inline domain error (a failing conversion, a
  division by zero, …) that leaves a function with no function-level `TRAP`
  skips the scope-drop walk and leaks every owned local. Pre-existing and
  independent of this bug; filed as `bugs/bug-695-inline-domain-error-skips-scope-drop.md`.
- **C9 — found: bug-696.** Cross-checking this fix on Win11 (box 2230) — the
  probe's B, C, E, F and G print the same values and the same bounded counts as
  on macOS and Linux aarch64 (box 2226) — showed 262,144 bytes live at exit on
  Windows only. It is the probe's one `os::getEnvOr`: the Windows backend never
  frees its UTF-16 marshalling buffers. Pre-existing and independent; filed as
  `bugs/bug-696-windows-marshal-buffers-leak.md`.
- **C8 — found: a stale golden.** `syntax/app/app-window-surface`'s
  `linux-x86_64.app.ncodesum` (`91b6f6cd…`, written by `b7abf9c0e`) is produced
  by no committed compiler: `694ce7095` (the implementation it pins) and
  `aa644f908` both build `d2ecd5f6…`, deterministically, and nothing between
  them touches codegen. It was the one diff in the `aa644f908` baseline gate;
  regenerated in its own commit.

## Validation Plan

- Regression tests: `tests/runtime/rt_list_element_record_borrow.rs`.
- Runtime proof: `bugs/repro/bug-689-list-element-record-copy.mfb`, table
  above, before and after.
- Doc sync: the spec memory/collections *Self-updates* and `get` sections,
  and `.ai/collections.md`.
- Full suite: the project's full `cargo test` gate (`.ai/testing-gates.md`).

## Open Decisions

- Should a self-update that misses its in-place form be reported to the
  user (a diagnostic or `--debug` note), so the next gap like this is
  visible without a probe? Recommended: yes, as a separate plan. Out of
  scope here.

## Summary

The risk is soundness in Phase 2. A borrow of an element that outlives a
write to its container is a use-after-free, and the present gate avoids that
by refusing every `MUT` and module-level container, which is where this
pattern lives. Phase 3 is mostly plumbing onto the plan-145 field arms. The
language, the layout and value semantics are untouched.
