# bug-766: a record updated through a List/Map element, a helper, or a callback is rebuilt in a fresh arena block on every update

Last updated: 2026-09-28
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Other (performance — per-update allocate/copy/free where the element-update and hand-over contracts promise none)

Status: Fixed (A, B, C, E; D for same-granule replacement — a size change keeps the rebuild by owner decision; `record nested` split to bug-784)
Regression Test: `tests/runtime/rt_record_update_forms.rs` (to be added, Phase 1); new unit cases in
`src/codegen/engine/analysis/borrow_get.rs` and `src/codegen/engine/analysis/handover.rs` (Phase 1)

Updating a record held in a collection, or threaded through a helper, allocates one or more arena blocks per update
— `_mfb_arena_alloc` for a copy of the element, `_mfb_arena_alloc` again for the `WITH` result, two byte-copy loops,
two `_mfb_arena_free` — **even though the compiler already has zero-allocation lowerings for the same update in a
slightly different spelling**. `MUT it = get(xs, j)` / `it = WITH it {…}` / `xs = set(xs, j, it)` costs 0 allocations
and 3.0 ns per update; the spelling the benchmark (and most users) write, `LET it = get(xs, j)` /
`xs = set(xs, j, WITH it {…})`, costs 2 allocations and 51 ns — 17× slower for the same observable result. The output
is always correct; only the cost is wrong, which is why no test caught it.

It is one family with five mechanisms (A–E below), all "a record value is materialised in a fresh arena block that
the next operation byte-copies and frees":

| Row (mfb -O1 / C -O0 / Python, ms, min column of `benchmark/baseline/*.log`) | Grade | Mechanism |
|---|---|---|
| `elem rec_set` 5.103 / 0.268 / 8.214 | D (19.0×) | A |
| `record update` 0.034 / 0.000 / 0.086 | D (34×, proxy only) | A |
| `record nested` 53.433 / 5.257 / 822.700 | C (10.2×) | A (write-back inside `IF`), C, D |
| `float nbody_rec` 113.952 / 12.194 / 528.918 (scalar `float nbody` 8.595) | C (9.3×) | B |
| `float nbody_vec` 194.649 / 17.198 / 686.586 | C (11.3×) | B |
| `record wide` 41.981 / 10.704 / 1788.423 | B (3.9×) | C, D |
| `elem rec_transform` 8.297 / 0.286 / 13.202 | D (29.0×) | E |

(Grades from `./benchmark/rank.py`; ratios mfb -O1 ÷ C -O0.)

**The single correct behavior a fix produces:** every update below allocates the number of arena blocks given in the
*after* column of the Goal table — for the list-element forms that is **zero per update** (flat in the iteration
count, exactly what the MUT spelling measures today), for a helper-threaded record accumulator zero per call when the
record's byte size does not change, and for `transform` over records one block per call (the result list) instead of
one per element.

## STATUS: FIXED (merged from `worktree-B-766`, `e5b2c933a`)

Measured by `tests/runtime/rt_record_update_forms.rs` (REPS 50 vs 100, `--debug`), every
case prints what its allocation-free contrast prints and leaks nothing:

| Mechanism | Before | After |
|---|---|---|
| A1–A5 (fused `WITH` item, LET-bound `WITH`, flat record, IF-arm write-backs, Map value) | 2 allocs/update | 0 |
| B (`nbody` pair step; `vector::Float3` fields) | 50 / 60 per rep | 0 / 0 |
| C (helper `RETURN WITH p { … }`) | 1 / call | 0 |
| D (same-granule `String` last-field replacement) | 1 / update | 0 |
| E (`transform` with a single-`WITH` record callback) | 1 / element | 0 (1 / call: the result list) |

Benchmark shapes (agents' measurements): `nbody_rec` 40 → 0 allocs/step, ~108 → ~52 ms;
`nbody_vec` 85 → 10 allocs/step, ~214 → ~83 ms (the remaining 10 are bug-783).

- **A** — `borrow_get.rs` recognises the fused, LET-temporary and IF-arm write-backs
  (`ElementBinding::other_sites`); `builder_control.rs` dispatches them.
- **B** — **deviation:** fixed-size non-escaping record temporaries live in a second
  per-function arena scratch block (the *record-temp scratch*), not the self-update
  scratch itself, because the self-update scratch can be regrown (moved) by a copying
  `sort` inside a temporary's live range. Same reservation code
  (`emit_reserve_scratch_block`), reserved once per call, freed on every exit; still arena
  memory, so within `mfb spec memory arenas`. `set`/`append` of a `WITH` item build the
  record in scratch then copy it in (one extra copy vs the design's direct build).
- **C** — `handover.rs:consuming_use` accepts `RETURN WITH p { <in-block fields> }`; the
  owned variant updates `p`'s block in place (`try_inplace_scalar_fields`) and moves it
  out. Also removed `handover.rs`'s file-level `#![allow(dead_code)]` (AGENTS.md).
- **D** — `try_inplace_scalar_fields` replaces a record's last inlined `String`/record
  field in place when its 16-byte granule is unchanged; a size change keeps the rebuild
  (1 alloc + 1 free), awaiting the Open Decision. STATE owners are refused
  (`rt_res_state_inplace_mutation::string_state_field_still_rebuilds`, bug-424).
  `field_kinds.tsv`: 144 `rebuild:size-varies` rows the harness reported now in place
  were flipped to `arm` (36 kinds × S4/S5/S9/S10), with proof in `b309807ec`.
- **E** — `func_transform.rs` evaluates a literal/direct single-`WITH` record callback into
  the accumulator.

**Owner decision (2026-09-29): a size-changing replacement keeps the rebuild.** No
realloc and no capacity field, so `record wide`'s alternating tag, which crosses a
granule, stays 1 alloc/call by design.

**Split out:** `record nested` is 2 allocs/update after this bug (not 3 — measured by
`bash /tmp/b784-measure.sh`, N = 100 000 vs 200 000: 200 407 vs 400 407). The two
sources are the new inner record built from a field read (`stepWide(h.inner, …)`) and
the replacement of a non-last inlined record field in the write-back. Neither is in
this bug's Fix Design; filed as bug-784.

**Found and fixed here:** a capturing `LAMBDA` passed to a callback leaked 32 B when the
callback failed and the error left through a function `TRAP` or a return
(`39088aa62`, test `rt_capturing_callback_failure_frees`); `09_closures.md` corrected.
**Found and filed:** bug-782 (a failing `set` on a copied list field leaks the copy),
bug-783 (`vector::Float3` literals no longer register-native).

References:

- `mfb spec memory collections`, "`get`" (the copy is skipped "when no program can tell the two apart", including
  "An element-bound update") and "Self-updates" (Cost: "It never costs the size of the part of x it leaves alone";
  "No per-statement allocation").
- `mfb spec language memory-semantics` §14.1 (a copy may be replaced by a move when the source is not used
  afterward), §14.6 (a buffer may be destructively updated by its one owner at its last use, including "a parameter
  the caller handed over"; "No program can observe the difference from a copy").
- `bugs/completed/bug-689-list-element-record-get-copies-whole-element.md` — introduced the element-bound update
  (`get → WITH → set`) and recognises exactly two spellings of it (its C3). This bug is the spellings it missed.
- plan-147 (`.ai/collections.md`, "An accumulator threaded through a helper is handed over, not copied") — the
  hand-over whose consuming-use set does not include `RETURN WITH p { … }`.
- Found while triaging the benchmark D/F rows (cluster D, "records held in collections or nested in records").

## Failing Reproduction

All numbers below are from `/tmp/dbA-cases/gen.sh <case> <P>` (built with
`/Users/justinzaun/Development/mfb/target/release/mfb build -O 1 --debug`, run, `arena.0.alloc_calls` read from the
`--debug` report) at `P = 10` and `P = 20` passes of 1000 updates: **allocs/update = Δalloc_calls / 10 000**. `ns/op`
is the same program built without `--debug` at `P = 100` (`./gen.sh <case> 100`), quietest of one run — treat as ±20 %.

The probe (`<BODY>` is replaced by each case's loop):

```
IMPORT io
IMPORT collections
IMPORT datetime
TYPE Item
  id AS Integer
  name AS String
  score AS Float
END TYPE
TYPE Body
  x AS Float
  vx AS Float
  mass AS Float
END TYPE
FUNC mk(n AS Integer) AS List OF Item
  MUT items AS List OF Item = []
  FOR i = 0 TO n - 1
    items = collections::append(items, Item[id := i, name := "item" & toString(i), score := 0.5])
  NEXT
  RETURN items
END FUNC
FUNC mkb(n AS Integer) AS List OF Body
  MUT bs AS List OF Body = []
  FOR i = 0 TO n - 1
    bs = collections::append(bs, Body[x := 0.0, vx := 1.0, mass := 2.0])
  NEXT
  RETURN bs
END FUNC
FUNC main() AS Integer
  MUT items AS List OF Item = mk(1000)
  MUT bodies AS List OF Body = mkb(1000)
  MUT mp AS Map OF Integer TO Body = Map OF Integer TO Body { }
  FOR q = 0 TO 999
    mp = collections::set(mp, q, Body[x := 0.0, vx := 1.0, mass := 2.0])
  NEXT
  LET t0 AS Integer = datetime::monotonicNanos()
  FOR pass = 1 TO <P>
<BODY>
  NEXT
  LET t1 AS Integer = datetime::monotonicNanos()
  io::print("ns/pass " & toString((t1 - t0) / <P>) & " " & toString(collections::get(items, 3).id))
  RETURN 0
END FUNC
```

| Case | `<BODY>` (inside `FOR j = 0 TO 999 … NEXT`) | allocs/update (P=10 → P=20) | ns/op |
|---|---|---|---|
| **A1 `itemset` (= `rec_set`)** | `LET it AS Item = collections::get(items, j)` / `items = collections::set(items, j, WITH it { id := it.id + 1, score := it.score + 0.5 })` | **2** (24 041 → 44 041) | **51** |
| **A2 `itemset_let2` (= `record update`)** | `LET it = get(items, j)` / `LET u AS Item = WITH it { id := it.id + 1 }` / `items = set(items, j, u)` | **2** | **41** |
| **A3 `bodyset`** (all-`Float` record) | as A1 over `bodies` with `WITH b { x := b.x + b.vx }` | **2** | **24** |
| **A4 `bodyset_if`** (write-back in `IF` arms, `record nested`'s shape) | `LET b = get(bodies, j)` / `IF j MOD 2 = 0 THEN bodies = set(bodies, j, WITH b {x := b.x + b.vx}) ELSE bodies = set(bodies, j, WITH b {x := b.x - b.vx}) END IF` | **2** | — |
| **A5 `mapset_let`** (Map value) | `LET b = get(mp, j)` / `mp = set(mp, j, WITH b { x := b.x + b.vx })` | **2** | — |
| contrast `itemset_mut` | `MUT it = get(items, j)` / `it = WITH it { id := it.id + 1, score := it.score + 0.5 }` / `items = set(items, j, it)` | **0** (4 041 → 4 041) | **3.0** |
| contrast `itemset_single` | `items = set(items, j, WITH get(items, j) { id := get(items, j).id + 1 })` | **0** | **2.8** |
| contrast `bodyset_mut` | MUT form of A3 | **0** | **2.8** |
| contrast `mapset_mut` | MUT form of A5 | **0** | — |
| contrast `itemget` / `bodyget` | `LET it = get(items, j)` / `acc = acc + it.id` (read only) | **0** | — |

Mechanism B (two elements live across a write), measured by `/tmp/dbA-nbody/gen.sh {rec|vec} <steps>`, which
extracts `test_nbody_rec` / `test_nbody_vec` verbatim from `benchmark/mfb/src/recordbench.mfb` with the step count
substituted (same checksum `-169079859` at 100 000 steps as the benchmark):

| Case | allocs per step (1000 → 2000 steps) | ms at 100 000 steps (no `--debug`) |
|---|---|---|
| `nbody_rec` | **50** (50 024 → 100 024) = 10 pairs × 4 + 5 drifts × 2 | **116.7** |
| `nbody_vec` | **85** (85 059 → 170 059) = 10 pairs × 7 + 5 × 3 | **212.1** |
| contrast: `float nbody` (scalar locals) | 0 | 8.6 (baseline log) |

Mechanisms C/D (`/tmp/dbA-wide/gen.sh <case> <N>`, `Wide` = `benchmark/mfb/src/recordbench.mfb`'s 10-field record
with two `String` fields; allocs per op from N = 100 000 → 200 000, ns/op at N = 1 000 000 without `--debug`):

| Case | Loop body | allocs/op | ns/op |
|---|---|---|---|
| `wide` (= `record wide`) | `w = stepWide(w, i, tagEven|tagOdd)` (`RETURN WITH w { count := …, …, tag := tag }`) | **1** | **45** |
| `notag` | `w = stepNoTag(w, i)` (`RETURN WITH w { count := w.count + 1, total := … }`, scalars only) | **1** | **37** |
| `inlinetag` | `w = WITH w { count := w.count + 1, tag := tagEven|tagOdd }` inline | **1** | **47** |
| contrast `inline` | `w = WITH w { count := w.count + 1, total := w.total + i MOD 7 }` inline | **0** | **2** |
| `nested` (= `record nested`) | the benchmark loop verbatim | **3** | **191** |
| `nestedmut` | the same with `MUT h` / `h = WITH h {…}` / `set(holders, k, h)` | **2** | **118** |

Mechanism E (`/tmp/dbA-cases/gen.sh`, allocs per `transform` of 1000 elements, P = 10):

| Case | Body | allocs/transform | µs/transform |
|---|---|---|---|
| `transform` (= `rec_transform`) | `transform(items, LAMBDA(it AS Item) -> WITH it { score := it.score * 2.0 })` | **1001** | **42.6** |
| `transform_id` | `transform(items, LAMBDA(it AS Item) -> it)` | **1001** | 43.6 |
| contrast `transform_int` | `transform(items, LAMBDA(it AS Item) -> it.id * 2)` | 1 | 14.4 |
| contrast `filter` | `filter(items, LAMBDA(it AS Item) -> it.score >= 0.25)` | 1 | 28.8 |

- Observed: the allocation counts above grow linearly with the update count for every non-contrast case.
- Expected: the Goal table below.

## Root Cause

### A — the element-bound recogniser accepts two spellings; the common ones are not among them

`src/codegen/engine/analysis/borrow_get.rs:collect_borrow_gets` (bug-689 Phase 3) makes `LET/MUT p = get(xs, i)` an
alias of the element (no copy, no free) only when the write-back is found by `write_back_index`, which requires the
`set`'s item to be the bare binding:

```rust
matches!(&args[2], NirValue::Local(p) if p == item).then_some(&args[1])
```

and the optional update to be the separate statement `p = WITH p { … }` immediately before it (so `p` must be `MUT`).
The other accepted spelling is `single_expression_update` (`set(xs, i, WITH get(xs, i) { … })`). Therefore:

- **A1/A3/A5** `set(xs, i, WITH p { … })` — the item is a `WithUpdate` whose target is `p`, not `Local(p)`:
  `write_back_index` returns `None`, the binding is not element-bound, and it is not read-only either (it is read
  as a `WITH` target, which `read_only`'s `members + scrut + temps` does not count), so it falls to the owning path.
- **A2** `LET u = WITH p { … }` / `set(xs, i, u)` — the update is a *bind* of a new name, not `p = WITH p`.
- **A4** the write-back is inside `IF`/`ELSE` arms: the search `(k + 1..block.len()).find(...)` scans the binding's
  own block only.

On the owning path (`.ncode` of `/tmp/dbA-cases/bodyset-10`, `_mfb_fn_main`, loop `for_loop_32`), each update emits,
in order: `list_get_done` → `bl _mfb_arena_alloc` + `flat_copy_wloop` (the owned copy of the element —
`src/codegen/memory/owned.rs:materialize_owned_element` → `copy_flat_block`); `bl _mfb_arena_alloc` +
`record_build_alloc_ok` (the `WITH` result — `src/codegen/memory/value/builder_value_semantics.rs:lower_with_update`
→ `emit_build_inlined_record`, which always builds a new block); `set_inplace_*` + `collection_copy_inline_wloop` (the
in-place `set` copies the new record into the slot — this part is already in place); then two `bl _mfb_arena_free`
(`owned_value_free_skip_166/167`). Two allocations, three record-sized copies, two frees — for a 24-byte record.

This is also why `record update` (A2 shape) is D.

### B — two elements of one list live across a write: every read and every `WITH` result is an arena block

`nbody_rec`'s pair step reads `bi = get(bodies, i)` and `bj = get(bodies, j)`, then writes slot `i` (reading `bj`)
and slot `j` (reading `bi.mass`). No element-bound form can apply as the rules stand — `bi` is read after its own
write-back and the container is written before `bj`'s — and the windowed borrow is refused because the window writes
the container. So each `get` takes `materialize_owned_element` (1 alloc) and each `WITH` item takes
`lower_with_update` (1 alloc): 4 per pair + 2 per drift = 50 per step, measured. `nbody_vec` adds one arena block
per `vector::Float3[…]` literal (a record literal is always `emit_build_inlined_record`; the contrast
`LET b AS Body = Body[…]` read only through fields measures 1 alloc per bind, `/tmp/dbA-cases` case `localwith`):
7 per pair + 3 per drift = 85, measured. The copies themselves are required by value semantics (`bj` must not see the
write to slot `i` if `i = j`); **only their placement in a fresh arena block per evaluation is not**.

### C — `RETURN WITH p { … }` is not a consuming use, so a record accumulator through a helper is rebuilt per call

`src/codegen/engine/analysis/handover.rs:consuming_use` (plan-147 P3) recognises `RETURN OP(p, …)`, `RETURN p`,
`LET y = p`, and the field form `RETURN WITH r { f := OP(r.f, …) }` for a *collection* field
(`returned_field_self_update_local`). `RETURN WITH w { count := w.count + 1, … }` over scalar fields is none of them,
so `consumable_params` is empty, no `$own` variant is emitted (`/tmp/dbA-wide/notag-1000000/w.ncode` has
`_mfb_fn_stepNoTag` and no `$own` symbol), the caller lends `w`, and the callee's `lower_with_update` builds a new
block: 1 alloc per call (`notag`, 37 ns) versus 0 for the same `WITH` inline (`inline`, 2 ns). `record nested` hits
the same thing through `stepWide(h.inner, …)`.

### D — replacing a `String` (or variable-width inlined) field in a `WITH` self-update always rebuilds

`src/codegen/engine/control/builder_control.rs:try_inplace_scalar_fields` serves a scalar, pointer or fixed-size
inlined field; for any other inlined field (`record_field_is_inlined && !record_field_is_inlined_fixed`) it returns
`Ok(false)`, and `try_inplace_mixed_with` only admits a *collection* inlined field. So `w = WITH w { tag := t }`
falls to `lower_with_update`: 1 alloc per op (`inlinetag`, 47 ns) even inline. In `record nested` the same decline
hits `WITH h { inner := … }` (`inner` is a variable-width inlined `Wide`), which is why `nestedmut` still costs 2.

The hazard that made this a decline is real: a record carries no size word (`mfb spec memory heap-values`, "a
record's total byte size is computed by walking the fixed slot region plus each inlined sub-block"), and
`_mfb_arena_free` is caller-sized with exact 16-byte quick bins (`mfb spec memory arenas`). A `String` field
rewritten in place to a different rounded size would make the walk disagree with the block's allocated size — a
wrong-size free. In-place is sound only when the rounded size is unchanged, or when the grow goes through a realloc.

### E — a record returned by a `transform` callback is a standalone block the loop copies and frees

`src/codegen/builtins/collections/func_transform.rs:lower_transform` calls the callback per element (`blr`), and the
callback's `RETURN WITH it { … }` (or even `RETURN it` — `transform_id`) returns a fresh arena block, because the
`FunctionRef` ABI hands the callback's result to the caller as an owned block (the bug-569/bug-677 comment above
`transform_result_kept`). The loop byte-copies it into the accumulator's spare slot and frees it
(`free_callback_result`): 1 alloc + 2 copies + 1 free per element (`.ncode` of `/tmp/dbA-cases/transform-10`:
`_mfb_fn__24lambda0` has one `bl _mfb_arena_alloc` → `record_build_alloc_ok_2`; the transform loop
`transform_call_loop_37` frees the result before `transform_result_kept_84`). That round trip is ≈ 29 ns of the
42.6 ns per element (`transform` 42.6 µs vs `transform_int` 14.4 µs per 1000 elements).

### Why the contrast cases are immune

The MUT and single-expression spellings are what `collect_borrow_gets` recognises; a read-only `get` binding is the
windowed/pinned borrow; `w = WITH w { <scalars> }` inline is `try_inplace_scalar_fields`; a scalar-returning
callback materialises nothing.

## Goal

Allocation counts are the checkable target (flat in the iteration count ⇔ 0 per update); the timing column is a
**guess** derived from the measured contrast of the same shape, not a gate.

| Case / row | allocs/update now → after | Justification | Time after (guess) |
|---|---|---|---|
| A1 `rec_set` | 2 → **0** | same update as `itemset_mut`, which measures 0 and 3.0 ns/op | ≈ 0.3 ms (C -O0 0.268) |
| A2 `record update` | 2 → **0** | same | ≈ C -O0 |
| A3/A4/A5 (fixed-width, `IF` arms, Map value) | 2 → **0** | MUT forms measure 0 | — |
| B `nbody_rec` | 50/step → **0/step** | copies stay (value semantics), but in frame/scratch storage, not per-evaluation arena blocks | ≤ 2× C -O0 (≤ 24 ms) |
| B `nbody_vec` | 85/step → **0/step** | same, plus `Float3` literals | ≤ 2× C -O0 (≤ 34 ms) |
| C `notag` (helper, scalar fields) | 1 → **0** | same update as `inline`, 0 | ≈ inline + call |
| C+D `record wide` | 1 → **0 when the record's rounded size is unchanged; ≤ 1 (a realloc) when it changes** | D's size rule | benchmark alternates tag sizes every call, so ≤ 1/call — see Open Decisions |
| `record nested` | 3 → **≤ 1** (the `stepWide` return; the Holder update in place or through `set`'s relocate) | A + C + D | guess ≤ 20 ms |
| E `rec_transform` | 1001 → **1** per `transform` | `transform_int` measures 1 | ≈ 3 ms (≈ 14 ns/element, `transform_int`) |

Every case must print exactly what it prints today (the RED test compares against the copying lowering's output).

### Non-goals (must NOT change)

- **Owner constraint: no change to the language specification or the memory-management model.** Value semantics
  stay: a `get`, a `LET`, an argument and a return still behave as copies; `WITH` / `collections::*` results,
  ordering, errors and error codes are unchanged; no new syntax, keyword, builtin or signature; `mfb man` and
  `mfb spec language` behaviour unchanged; the `mfb spec memory` copy/mutate/alias rules, the arena allocator and
  ownership/drop contract, the in-place self-update cost contract and RES handle semantics unchanged. Every change
  below is a lowering or analysis change inside §14.1 ("the compiler may replace a semantic copy with a move when it
  proves the source is not used afterward") and §14.6 ("No program can observe the difference from a copy"); a copy
  may be elided **only** where the source is provably dead or unaliased, and an in-place update only within the
  existing element-bound / self-update / hand-over contracts.
- The element-bound soundness rules of bug-689 stay: adjacency of the update and its write-back (C3), nothing may
  grow the element's block, `i` pure and unchanged, `p` dead after the write-back, failure atomicity.
- Failure atomicity: a `WITH` whose field expression fails (float overflow, a callback error) leaves the element /
  record / accumulator exactly as it was — the order is still "evaluate every update value, then write".
- The `FunctionRef` ABI (the base symbol a function value, `LINK` or a `.mfp` package sees) does not change.
- **Forbidden wrong fixes:** rewriting `benchmark/mfb/src/elem.mfb` / `recordbench.mfb` / `main.mfb` into the MUT
  spelling (the row measures the spelling users write); making a `get` of a record an alias whose lifetime is not
  proven (a UAF into the container — bug-538/601's class); freeing a record by anything other than its walked size.

## Blast Radius

Found with `grep -n "write_back_index\|single_expression_update" src/codegen/engine/analysis/borrow_get.rs`,
`grep -rn "fn consuming_use\|returned_field_self_update_local" src/codegen`, `grep -rn "lower_with_update\|emit_build_inlined_record(" src/codegen --include='*.rs'`,
and the probes above.

- `borrow_get.rs:write_back_index` / `collect_borrow_gets` — A1–A5 — **fixed by this bug**.
- Map element update (A5, `mapset_let` 2 → `mapset_mut` 0) — same recogniser — **fixed by this bug**.
- `getOr`-bound updates — excluded from element-bound by design (`get.get_or` check; bug-689 C4) — **unaffected**, not
  widened here.
- `builder_value_semantics.rs:lower_with_update` → `emit_build_inlined_record` — every `WITH` not served in place (B,
  E and the remaining D cases) — **fixed where the result is consumed by a copying sink** (a `set`/`append` item,
  a `RETURN` in an owned variant); a `WITH` bound to a long-lived owning binding keeps its block — **unaffected**,
  that block is the binding's storage.
- `memory/owned.rs:materialize_owned_element` — B (`get` copies of a binding not eligible for a borrow) — **fixed**
  for a flat, fixed-size, non-escaping binding (Phase 3); a String element or a graph-typed element — **unaffected**
  (a String `get` builds its block by design; a graph needs the graph copy).
- Record literals (`vector::Float3[…]`, `Body[…]`) bound to a non-escaping `LET` (`localwith`: 1 alloc per bind) —
  same storage question as B — **fixed with B**.
- `handover.rs:consuming_use` — C — **fixed by this bug** (add the scalar/`String`-field `RETURN WITH p { … }` form).
- `builder_control.rs:try_inplace_scalar_fields` / `try_inplace_mixed_with` — D — **fixed for a same-rounded-size
  replacement**; a size-changing replacement is the Open Decision below.
- `func_transform.rs:lower_transform` — E — **fixed by this bug**. Other HOFs that store a record-valued callback
  result per element (`mapValues`, `reduce` with a record accumulator, `zip`-style builders) probably take the same
  round trip — **guess, unmeasured**; `filter` does not (measured 1 alloc per call). Latent, out of scope until
  Phase 1's audit measures allocs/element for each.
- `elem rec_sortby` — a different mechanism (the `.mfb` `sortBy` body) — **bug-767**.

## Fix Design

**Phase 2 (A) — widen the element-bound recogniser; no codegen change.** In `collect_borrow_gets`:
1. accept a write-back whose item is `WithUpdate { target: Local(p), updates }` (A1/A3/A5): treat it exactly as the
   S1+W pair fused into one statement — the reads of `p` inside the updates are the S1 reads, the write-back is the
   same statement, so adjacency holds trivially. Lower it through the same element-field routes as
   `single_expression_update` (whose shape it is, with `p` for the `get`).
2. accept `LET u = WITH p { … }` immediately followed by `set(xs, i, u)` with `u` read exactly once (A2): `u` is the
   S1 result under a new name; classify it as the S1 of `p`.
3. accept write-backs in the arms of an `IF` whose every arm ends with a write-back of the same slot (A4), each arm
   checked with the existing per-block rule.
Every other bug-689 gate is unchanged. The element routes already refuse any growth of the element's block and
fall back to the written `set` (`ElementBinding::write_back_op`).

**Phase 3 (B) — non-escaping flat record temporaries without a per-evaluation arena block.** For a binding or
statement temporary whose type is a flat record of fixed size (no inlined variable-width field — `Body`, `VBody`,
`vector::Float3`) and that never escapes (read only through fields, as a `WITH` target, or as the item of a copying
`set`/`append` — the same classifier as the read-only borrow, minus the container-window condition), place the value
in storage reserved once per function (the self-update scratch, `emit_reserve_self_update_scratch`, already the
spec's "per-function scratch block, grown geometrically"), and lower `set(xs, i, WITH e { … })` as "evaluate every
update into its slot, copy `e`'s bytes into the element slot, overwrite the updated fields" (all values are computed
before the first write, so it is failure-atomic and correct even when `e` aliases the destination). See Open
Decisions for the storage question.

**Phase 4 (C) — `RETURN WITH p { <non-collection fields> }` is a consuming use.** Extend
`consuming_use` / `returned_field_self_update_local` so the scalar/pointer/fixed-inlined field form counts, and serve
it in the owned variant with `try_inplace_scalar_fields` over the parameter's block (then the existing move-out).
H1–H6 unchanged.

**Phase 5 (D) — same-rounded-size variable-width field replacement in place.** In `try_inplace_scalar_fields`, admit
an inlined `String` / variable-width field when it is the record's last inlined sub-block **and** the new record's
walked size rounds (16 B) to the old one: write the new block over the old span. A size change keeps the rebuild
(see Open Decisions).

**Phase 6 (E) — `transform` over a callback whose body is a single record expression.** When the callback is a
literal `LAMBDA`/direct `FUNC` whose body is one `WITH param { … }` (or `param`), evaluate it into the accumulator's
spare slot directly (the accumulator is private to the call, so a failure discards it — atomicity holds). Rejected
alternative: a callee-writes-into-caller-buffer internal variant for every record-returning function — larger, and
it touches the `FunctionRef` result contract; see Open Decisions.

Expected output shifts: `.ncode` of every program containing one of these spellings changes (fewer
`_mfb_arena_alloc`/`free` calls, fewer copy loops). Program stdout does not change anywhere.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/runtime/rt_record_update_forms.rs`, modeled on `tests/runtime/rt_list_element_record_borrow.rs`
      (build with `--debug`, run the loop `REPS` and `2 * REPS` times, require `alloc_calls` unchanged, `alloc_calls =
      free_calls`, `live_bytes 0`): one case per row of the Goal table (A1–A5, B `nbody_rec`/`nbody_vec` at 2 steps
      vs 4, C `notag`, D same-size `tag`, E `transform`), each also printing its values beside the MUT/copying
      spelling. Confirm every case except the contrasts fails today.
- [x] Unit cases in `borrow_get.rs` beside `get_with_set_on_the_same_slot_is_element_bound`:
      `a_with_over_the_binding_as_the_set_item_is_element_bound`, `a_let_bound_with_before_its_set_is_element_bound`,
      `write_backs_in_both_if_arms_are_element_bound`; and negatives — an `IF` with a write-back in one arm only, a
      read of `p` after the `WITH`-item `set`, a write to `xs` between the bind and the `set`.
- [x] Unit case in `handover.rs` beside `consumable_params_follows_the_hand_derived_table`:
      `RETURN WITH p { n := p.n + 1 }` makes `p` consumable; `RETURN WITH p { n := q.n }` plus a later read does not.
- [x] Audit the latent HOF sites (Blast Radius) with one measured allocs/element each; write the verdicts here.

Acceptance: the new runtime cases fail for the documented reason (allocation count grows with REPS); the value
comparisons pass; the audit list is complete.
Commit: see STATUS

### Phase 2 — mechanism A

- [x] `borrow_get.rs`: the three spellings above; element routes unchanged.

Acceptance: A1–A5 cases green; `rec_set` probe ≤ 0.5 ms at -O1 (measured contrast 0.30 ms); bug-689's tests green.
Commit: see STATUS

### Phase 3 — mechanism B (after the Open Decision on storage)

- [x] Flat fixed-size non-escaping record temporaries in function-reserved storage; `set(xs, i, WITH e {…})` built
      in the slot.

Acceptance: `nbody_rec`/`nbody_vec` cases green (0 allocs/step); output checksum `-169079859` unchanged.
Commit: see STATUS

### Phase 4 — mechanism C

- [x] `handover.rs` consuming use + owned-variant lowering.

Acceptance: `notag` case green; `tests/runtime/rt_owned_argument.rs` and the `rt_inplace_self_update` matrix green.
Commit: see STATUS

### Phase 5 — mechanism D

- [x] Same-rounded-size inlined field replacement in place (the size-changing replacement still rebuilds — owner decision 2026-09-29).

Acceptance: D case green; a size-changing replacement still frees the exact walked size (live_bytes 0).
Commit: see STATUS

### Phase 6 — mechanism E

- [x] Direct build of a single-expression record callback result into `transform`'s accumulator.

Acceptance: E case green; `tests/runtime/rt_transform_block_result_frees.rs` green.
Commit: see STATUS

### Phase 7 — regenerate expected outputs + full validation

- [x] Regenerate the `.ncode` goldens the change shifts; confirm by inspecting one fixture per mechanism that the
      delta is only removed alloc/copy/free sequences.
- [x] Full suite (`.ai/testing-gates.md`); re-run the benchmark rows in the header table.

Acceptance: full suite green; the benchmark rows meet the Goal table.
Commit: see STATUS

## Validation Plan

- Regression tests: `tests/runtime/rt_record_update_forms.rs`; the `borrow_get.rs` / `handover.rs` unit cases.
- Runtime proof: `/tmp/dbA-cases/gen.sh`, `/tmp/dbA-nbody/gen.sh`, `/tmp/dbA-wide/gen.sh` tables above, before/after;
  `benchmark` rows `elem rec_set`, `record update`, `record nested`, `record wide`, `float nbody_rec`,
  `float nbody_vec`, `elem rec_transform`.
- Doc sync: `mfb spec memory collections` — the `get` section's "An element-bound update (see Self-updates)" points at
  a Self-updates section that does not describe the element-bound update at all today (checked with
  `mfb spec memory collections | grep -n -i "element.bound"`: one hit, line 555); document the recognised spellings
  there. `.ai/collections.md` bug-689 section. The hand-over consuming-use list in `mfb spec memory
  native-calling-convention` if it enumerates them.
- Full suite: the project's `cargo test` gate per `.ai/testing-gates.md`.

## Open Decisions

- **B's storage — owner sign-off needed, potential blocker for the `nbody_*` rows.** The copies of `bi`/`bj` are
  required by value semantics; removing their *per-evaluation arena block* needs storage that is not a fresh block.
  Recommended: the per-function self-update scratch (arena-backed, reserved once, grown geometrically — the
  mechanism `mfb spec memory collections` Self-updates already specifies as "No per-statement allocation"). The
  alternative, frame (stack) slots, is cheaper but places a record outside the arena, which contradicts `mfb spec
  memory arenas` ("Every heap-backed value — strings, records, … — lives in" an arena) and `get`'s "copied into a
  fresh arena block"; under the owner constraint that is a memory-model question and must not be taken without
  sign-off. If neither is accepted, `nbody_rec`/`nbody_vec` can only lose the `WITH`-item block (2 of 4 per pair),
  since the `get` copies are then required as arena blocks.
- **D's size-changing replacement** (the benchmark `record wide` alternates `"even-step"`/`"odd"`, a 16-byte size
  change every call): in-place needs either an arena realloc on the owner (1 call, amortised by headroom — but a
  record block has no capacity field, and adding one is a layout change) or a capacity shadow like plan-146's for
  `String` locals. Recommended: realloc-in-place only when the rounded size shrinks or the next chunk is free; else
  keep the rebuild — `record wide` then stays ≤ 1 alloc/call. **Decided 2026-09-29: keep the rebuild.**
- **E's scope** — the literal-callback fast path (recommended) vs an internal "return into caller buffer" variant
  per record-returning function (plan-147's `$own<mask>` precedent: an internal symbol, base ABI unchanged).

## Summary

The engineering risk is soundness in Phase 2 and Phase 3: every widened alias is a potential use-after-free into
the container if a gate is wrong, so each new spelling needs its negative cases (bug-689's own C3/C5 traps). Phase 1
is cheap and fully specified by existing harnesses. The rest of the compiler — the in-place `set`, the field routes,
the hand-over machinery — already exists; this bug is about routing four more spellings into it. Semantics, layout,
the ABI and the memory model are untouched; B and D carry explicit owner decisions.
