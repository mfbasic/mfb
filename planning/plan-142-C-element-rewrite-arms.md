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

- [ ] One shared arm `math_elementwise` covering the 16 functions' list
      overloads (pre-pass, then overwrite), dispatched by `self_update_builtin`.
- [ ] Flip the 16 rows and 27 `cases.tsv` lines; add `sqrt`-negative and
      `log`-zero atomicity cases to `rt_inplace_failure_atomic`.

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update --test rt_inplace_failure_atomic`
→ pass (est. 10 min).
Commit: —

### Phase 2 — `replace` and `transform`

- [ ] `replace` arm; `transform` arm with the infallible/fallible split.
- [ ] Rows, `cases.tsv`, atomicity case for a failing `f`.

Acceptance: same command → pass (est. 10 min).
Commit: —

### Phase 3 — `sort` and `sortBy`

- [ ] Permutation-apply primitive `lower_list_permute_in_place` (entry and
      fixed-width forms) in `list_mutate.rs` + a unit test that sorts lists of
      `Integer`, `String` and a record type and compares with the copying result.
- [ ] `sort` and `sortBy` arms (all element types the copying path accepts; the
      non-fast-path element types reuse the source-generic comparison through the
      permutation step).
- [ ] Rows, `cases.tsv`, atomicity case for a failing `keyFn`.

Acceptance: `cargo test --bin mfb permute_in_place self_update && cargo test --test rt_inplace_self_update --test rt_inplace_failure_atomic`
→ pass (est. 12 min).
Commit: —

### Phase 4 — Expected outputs

- [ ] Measured: `rg -lP '(\w+) = math::\w+\(\1\b' tests examples --glob '*.mfb'`
      → 2 files (`examples/brogue/src/terrain.mfb`, `examples/brogue/src/rooms.mfb`);
      `rg -lP "(\w+) = collections::(replace|transform|sort|sortBy)\(\1\b" …` → 0.
      Rebuild `examples/brogue` and confirm its output is unchanged against its
      oracle check.

Acceptance: `examples/brogue`'s own check passes (est. 5 min).
Commit: —

## Validation Plan

- Tests: matrix + harness rows; `rt_inplace_failure_atomic` cases; permutation unit test.
- Runtime proof: harness alloc counts; brogue output unchanged.

## Corrections

## Summary

Three mechanisms (pre-pass + overwrite, set-per-match, permutation apply) cover 20
functions. The permutation apply is the new code with real risk.
