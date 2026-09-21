# plan-142-C: In-place arms for `replace`, `transform`, `sort`, `sortBy`, and `math` element-wise

Last updated: 2026-09-20
Effort: large (3h–1d)
Depends on: plan-142-B

Prerequisites: see plan-142-A.

Self-updates that keep the list's length and rewrite or reorder its elements get
in-place arms: `collections::replace`, `collections::transform` (when `U = T`),
`collections::sort`, `collections::sortBy`, and the 27 `List` overloads of 16
`math` functions (`abs acos asin atan atan2 clamp cos exp log log10 max min pow sin sqrt tan`).
After C each allocates nothing at S1 in `rt_inplace_self_update`.

References: plan-142-A §3; B Phase 1's callback-fallibility finding;
`lower_list_set_in_place` (`list_mutate.rs:3332`, handles a longer
variable-width replacement in place).

## 1. Goal

- 4 `collections` rows and 16 `math` rows flip to `Arm`; their `cases.tsv` lines
  (4 + 27) flip to `arm`; matrix and harness pass for them.
- Failure atomicity (runtime cases): `math::sqrt` over a list with a negative
  element, `transform` with a failing `f`, `sortBy` with a failing `keyFn` —
  each leaves `x` unchanged.

### Non-goals

- Results unchanged: `sort`/`sortBy` stay stable with the same order.
- `transform` with `U ≠ T` is not a self-update (it cannot type-check) and is untouched.

## 2. Current State

| fn | lowering today |
|---|---|
| `replace` | `Body::Intrinsic`, `lower_replace` (`string/repr/builder_strings.rs:15`, shared with `strings::replace`) |
| `transform` | `Body::abi_inline(lower_transform)`, `func_transform.rs:131` |
| `sort` | `Body::mfb_with_fast_path`; native `lower_collection_sort_call` (`func_sort.rs:105`) for String/Integer/Fixed/Money, else the source generic |
| `sortBy` | `Body::mfb_with_fast_path`; `lower_collection_sortby_call` (`func_sort_by.rs:62`) |
| `math` list forms | `lower_math_call` (`builtins/math/gen_math.rs`), e.g. `func_sqrt.rs:48` |

`sort`'s native path builds an index permutation list
(`reserve_integer_index_list`, `func_sort.rs:37`) and then a new list.

## 3. Design

- **`math`** (fixed-width elements): pass 1 checks the domain of every element
  (the checks the copying lowering makes, same error, first offending element);
  pass 2 overwrites each element in place. Binary forms (`atan2`, `max`, `min`,
  `pow`) with a list second operand check lengths first. No allocation.
- **`replace`**: for each index whose element equals `old`, `lower_list_set_in_place`
  with `new` (already correct for a longer variable-width replacement). Cannot fail.
- **`sort`/`sortBy`**: keep the existing permutation computation (the index list is
  `n × 8` bytes of indices, not a copy of `x`; `sortBy`'s keys are computed first,
  so a failing `keyFn` fails before any write), then apply the permutation in
  place: for an entry-based list, permute the 40-byte lookup entries only
  (payloads do not move); for a fixed-width list, permute payloads by cycle-following
  with one element of scratch.
- **`transform` (`U = T`)**: per B Phase 1's answer. Infallible `f`: write each
  result with `lower_list_set_in_place` as it is computed. Fallible `f`: compute all
  results first into scratch — for a fixed-width `T` that scratch is `n × width`
  (the results, not a copy of `x`) — then write. Fallible `f` with a
  variable-width `T` declines at `G-atomic` (plan-142-A Open Decision 1, resolved), with a runtime case proving `x` unchanged after the failure.

Risk: the permutation apply (cycle-following correctness for fixed-width payloads)
and the `math` domain pre-pass matching today's error order exactly.

## Phases

### Phase 1 — `math` element-wise

- [x] One shared arm `math_elementwise` covering the 16 functions' list
      overloads (pre-pass, then overwrite), dispatched by `self_update_builtin`.
      `try_inplace_math_assign` (`collection/assign/builder_inplace_rewrite.rs`,
      `ArmId::Math`). No per-function pre-pass was written: every `math` array
      driver (all 8, `grep -rn emit_alloc_result_list src/codegen`) reduces its
      error mask and raises *after* its loop, and allocates its result through
      `emit_alloc_result_list`. The arm arms `simd_result_into`, which makes that
      function hand back the self-update scratch instead of allocating; the member's
      own lowering then writes its lanes there, and the arm copies them over `x`
      only after the lowering returned without raising (Correction C1). Matches
      `math.<f>` directly (`math_self_update_function`).
- [x] Flip the 16 rows and 27 `cases.tsv` lines; add `sqrt`-negative and
      `log`-zero atomicity cases to `rt_inplace_failure_atomic`. The math rows carry
      an `Integer` and a `Fixed` probe beside the `Float` one wherever the overload
      exists, so the matrix test compiles every overload's driver through the
      redirect (an unconsumed redirect is a hard `Err`).

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update --test rt_inplace_failure_atomic`
→ pass (est. 10 min).
Verified 2026-09-21: `self_update` → `4 passed`; `rt_inplace_self_update` (all 64 lines,
with the new result check) → `1 passed` (197.89s); `rt_inplace_failure_atomic` → `1
passed` (incl. `math::sqrt` over a negative element and `math::log` over zero).
Commit: 493f96d99

### Phase 2 — `replace` and `transform`

- [x] `replace` arm; `transform` arm with the infallible/fallible split.
      `try_inplace_replace_assign` / `try_inplace_transform_assign`
      (`builder_inplace_rewrite.rs`). `replace` uses the copying lowering's own
      compare (`emit_collection_payload_matches_value_branch`, as
      `lower_list_replace`). There is no infallible/fallible split: every callback
      is treated as fallible (plan-142-B Phase 1), so `transform` calls `f` for every
      element first, parking each result word (a scalar, or the pointer of a
      variable-width result) in the self-update scratch; a failure frees the parked
      `String` results and this element's copy, then routes the error. Both arms
      reserve tail room for longer payloads before their first
      `lower_list_set_in_place`, so no write repacks mid-pass (the only allocation,
      and so the only `ErrOutOfMemory`, precedes the first write). `G-atomic` never
      declines — Correction C3. String differential probe (longer/shorter/no-match
      `replace`, an out-of-order list, `transform` growing/shrinking/returning its
      own argument, a 300-iteration append/transform/replace/drop loop): 10/10 `ok`,
      `arena.0.alloc_calls 2183` = `free_calls 2183`, `live_bytes 0`.
- [x] Rows, `cases.tsv`, atomicity case for a failing `f`. Two cases: an `Integer`
      list and a `String` list whose results grow.

Acceptance: same command → pass (est. 10 min).
Verified 2026-09-21: `self_update` → `4 passed`; `rt_inplace_self_update` filtered to
`replace`/`transform` → `1 passed` (bound, `before` and result checks);
`rt_inplace_failure_atomic` → `1 passed`.
Commit: 692adba6a

### Phase 3 — `sort` and `sortBy`

- [x] Permutation-apply primitive `lower_list_permute_in_place` (entry and
      fixed-width forms) in `list_mutate.rs` + a unit test that sorts lists of
      `Integer`, `String` and a record type and compares with the copying result.
      In its own file, `src/codegen/collection/list/list_permute.rs`: one
      cycle-following pass over `block + HEADER + k * stride` (fixed-width lanes or
      40-byte entries alike), one element of temp, bit 63 of the permutation word
      as the visited mark. The comparison test is a runtime one (a unit test cannot
      run the result): `tests/runtime/rt_inplace_sort.rs` — `Integer`, `Float`,
      `Byte`, `String` elements; `sortBy` over `Integer`/`Float`/`String` keys, a
      key function returning its own argument, a record with equal keys
      (stability), an out-of-order list, one element, and `append`/`filter`/`drop`
      on the permuted payloads — 14/14 equal to the copying result, arena
      `alloc_calls` = `free_calls`, `live_bytes 0`. The codegen shape is pinned by
      `permute_in_place_reorders_lanes_and_entries_without_a_gather`
      (`src/codegen/builtins/tests/inplace_compact.rs`).
- [x] `sort` and `sortBy` arms (all element types the copying path accepts; the
      non-fast-path element types reuse the source-generic comparison through the
      permutation step). `src/codegen/collection/assign/builder_inplace_sort.rs`.
      The merge is the copying lowerings' own bottom-up stable merge (take the right
      head only when strictly less), over index words in the self-update scratch,
      with every loop variable in a slot so a comparison may call. Comparators: the
      native `String` byte compare (`emit_index_string_less_branch`, now
      `pub(crate)`), a signed word compare for `Integer`/`Fixed`/`Money`, and for
      every other ordered type and every `sortBy` key the `<` operator itself,
      lowered over two hidden locals — the generic body's comparison. `sortBy` calls
      `keyFn` for every element before reordering (Correction C5).
- [x] Rows, `cases.tsv`, atomicity case for a failing `keyFn`. The matrix rows carry
      `String`, `Float` and `Byte` probes for `sort` and `String`-element /
      `String`-key probes for `sortBy`, so each comparator path compiles.

Acceptance: `cargo test --bin mfb permute_in_place self_update && cargo test --test rt_inplace_self_update --test rt_inplace_failure_atomic`
→ pass (est. 12 min).
Verified 2026-09-21 (two filters are two runs — `cargo test` takes one name filter,
Correction C6): `cargo test --bin mfb permute_in_place` → `1 passed`; `cargo test --bin
mfb self_update` → `4 passed`; `rt_inplace_self_update` filtered to `sort`/`sortBy` →
`1 passed`; `rt_inplace_failure_atomic` → `1 passed` (incl. a failing `keyFn`);
`rt_inplace_sort` → `1 passed`. `grep -c pending:C cases.tsv` → 0.
Commit: —

### Phase 4 — Expected outputs

- [x] Measured: `rg -lP '(\w+) = math::\w+\(\1\b' tests examples --glob '*.mfb'`
      → 2 files (`examples/brogue/src/terrain.mfb`, `examples/brogue/src/rooms.mfb`);
      `rg -lP "(\w+) = collections::(replace|transform|sort|sortBy)\(\1\b" …` → 0.
      Rebuild `examples/brogue` and confirm its output is unchanged against its
      oracle check. The four brogue hits (`terrain.mfb:344-345`, `rooms.mfb:260-261`)
      are scalar `Integer` self-updates (`math::min`, `math::clamp`), which the arm
      declines (no collection layout, `G10`) — so no brogue codegen moves.

Acceptance: `examples/brogue`'s own check passes (est. 5 min).
Verified 2026-09-21: `MFB=<new release mfb> examples/brogue/check/check-terrain.sh 8 1`
→ `all 320 (level seed, depth) pairs match (125s, 12 jobs)`.
Full artifact-gate with every C arm: `2078 golden(s) checked, 0 diff(s)` — no
committed fixture self-updates a `math`/`replace`/`transform`/`sort`/`sortBy` list.
Commit: —

## Validation Plan

- Tests: matrix + harness rows; `rt_inplace_failure_atomic` cases; permutation unit test.
- Runtime proof: harness alloc counts; brogue output unchanged.

## Corrections

- **C1 (Phase 1): no domain pre-pass — the kernels' results go through scratch.** §3
  planned a pre-pass per function repeating each domain check. Every `math` array
  driver already accumulates a per-lane error mask and raises once, after its loop,
  so writing its lanes straight into `x` would clobber `x` before a later lane's
  error surfaced — but writing them anywhere *else* first is atomic for free. The
  arm redirects the member's result allocation into the self-update scratch
  (plan-142-B Correction B1) through the one allocation function all eight drivers
  share, and copies the lanes into `x` after the lowering returns. No kernel is
  duplicated, and a domain error provably precedes the copy.
- **C2 (Phase 1, affects every letter): the harness now checks results.** The
  allocation bound proves an arm ran and the `before` check proves it did not alias;
  neither proves it computed the right value. `rt_inplace_self_update` gains a third
  program per case: the statements once on `x`, and the same through chained `LET`s
  (not self-updates, so the copying lowering), rendered and compared. RED: with the
  math arm's copy-back removed, `math::sqrt(value AS List OF Float)` fails with
  "the self-update computed `0.10,0.20,0.30,`, the copying call `0.32,0.45,0.55,`".
- **C3 (Phase 2): `G-atomic` is never needed.** Open Decision 1 (plan-142-A) has a
  fallible `transform` over a variable-width `T` decline when its results "cannot be
  held without a copy of `x`". They always can: a variable-width result is one
  pointer, so the scratch holds `n` words — the results, not a copy of `x`. The arm
  declines for no callback shape.
- **C4 (Phase 2): `replace` needs the `ErrIndexOutOfRange` message.** The arm writes
  through `lower_list_set_in_place`, whose rebuild path (never taken from this arm,
  but emitted for every list type) calls `lower_list_remove_at`, which raises
  `ErrIndexOutOfRange`. A module calling only `collections::replace` had no data
  object for that message — the matrix test's first run failed with "native code
  string literal 'List or string index/range is outside valid bounds.' has no data
  object". `data_objects::string_symbols` now also registers it when
  `module_self_updates_with_replace` — only a module whose codegen the arm changes
  anyway, so no other module's data section moves. (`transform` was already on the
  message's list.)
- **C5 (Phase 3): the merge is the copying one, not a new sort.** §3 said "keep the
  existing permutation computation"; the native fast path only covers `String` and
  signed 8-byte elements (`sort_fast_path`), and everything else — `Float`, `Byte`,
  `Scalar`, and every `sortBy` shape the fast path declines — sorts through the
  `__collections_sort`/`__collections_sortBy` source generics. Both are the same
  stable bottom-up merge, so the arm runs that merge itself over index words, and
  compares with the generic body's own `<` (lowered on two hidden locals) wherever
  the fast path has no native compare. Same algorithm and comparison ⇒ same result,
  including for an order that is not a strict weak ordering.
- **C6 (Phase 3): `cargo test` takes one name filter.** The acceptance line
  `cargo test --bin mfb permute_in_place self_update` passes two; they were run as
  two commands (both recorded above).

## Summary

Three mechanisms (pre-pass + overwrite, set-per-match, permutation apply) cover 20
functions. The permutation apply is the new code with real risk.
