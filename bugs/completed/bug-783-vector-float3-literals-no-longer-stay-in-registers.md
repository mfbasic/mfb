# bug-783: `vector::Float3[…]` literals and inlined vector ops no longer run register-native — the vector-shape check matches the bare name, the type is now `vector.Float3`

Last updated: 2026-09-29
Effort: medium (1h–2h) — guess
Severity: MEDIUM — a silent performance regression on every vector-heavy loop (one arena
block per literal); output unchanged
Class: Other (performance regression)

Status: Fixed
Regression Test: `tests/runtime/rt_vector_register_native.rs`

`src/codegen/builtins/vector/builder_vector_inline.rs:vector_field_count` decides whether
a type is one of the nine small vector shapes by matching the resolved nominal name
against bare spellings (`"Float2" | "Fixed2" | "Integer2"`, `"Float3" | …`). Since the
bug-480 package-name work (the vector types are now package-qualified,
`vector.Float3`), that match never succeeds for `vector::Float3`, so the
register-native vector carrier (bug-332 F3) and the inlined vector ops
(`vector_call_is_inlined`) stop applying: every `vector::Float3[…]` literal is built as
an arena block. Found by the bug-766 B agent: after bug-766 B removed the record-copy
allocations from `benchmark/mfb/src/recordbench.mfb:test_nbody_vec`, its remaining 10
allocations per step are these literals; with the prefix stripped as an experiment,
`nbody_vec` drops to 0 per step and ~63–70 ms (from ~83).

**The single correct behavior a fix produces:** a `vector::Float3[…]` (and every other
vector shape) literal bound to a local and read through its fields allocates nothing,
and the inlined vector ops run register-native again, as they did when the shapes were
bare names — output unchanged.

## STATUS: FIXED (`0f8dec276`, `1c1a1506e`; spec `c3ff53a24`; goldens `b9f387060`)

**Mechanism (confirmed).** `builder_vector_inline.rs:vector_field_count` matched the
bare spelling, but every vector type in the IR is `vector.Float3`
(`mfb build --ir`: 49 of 49 spellings qualified). So a `vector::Float3[…]` literal
was an arena block. The inlined ops built their result as a bare `Float3`
(`VECTOR_SHAPES`), which still matched, so they kept working. The bisect was not
needed.

**Also found.** The same bare match took a user's own `TYPE Float3` for the builtin:
`LET c AS Float3 = Float3["n", i, 0.5]` panicked the compiler (`%%vecnative:1 is not
a register operand (bug-721)`).

**The "complication" explained.** Once literals are native, every vector op's FUNC body
ends in `RETURN Float3[…]` over lanes. `lower_returned_value` did not recognise a
register-native value as fresh, so the exit built the block, copied it
(`materialize_inline_value_in_arena`), and freed the first: 2 allocations per return.
That is why the experiment saw `vector::scale` go 301 → 401.

**Found by the acceptance suite after the first fix: a use-after-free.**
`vector::clamp_length(vector::Float2[1.0, 1.0], 10.0)` returned `0.00 0.00`, and the
arena reported 2 double-free skips. The call takes the `$own1` variant, which is handed
the literal and returns it (`RETURN v`).
`builder_emit_helpers.rs:emit_prepared_call_args_with_site` claimed a handed-over
temporary *before* `materialize_value`. A register-native vector has no block and no
temp until then, so the claim matched nothing. The block was then registered as a fresh
temp, and the statement freed the block the callee owned. The same happens with a user
`FUNC pick(v …) … RETURN v`. The fix materializes a register-native vector before the
claim (`1c1a1506e`). This was latent: it needs a register-native vector handed over,
which the bare-name bug had stopped happening.

**Fix.**

- `VECTOR_SHAPES` and `vector_field_count` use the qualified identity
  (`vector.Float3`).
- `lower_returned_value` materializes a register-native vector once and claims the
  block (bug-536's shape for a fresh constructor temp).
- `emit_prepared_call_args_with_site` materializes a handed-over register-native
  vector before claiming its temp.

**Measured** (`rt_vector_register_native`, `--debug`, 50 vs 100 iterations):

| Case | Allocations per iteration, pre-fix | After |
|---|---|---|
| literal (5 shapes), or an inlined op over locals (7 ops) | 4–5 | 0 |
| `scale` with literal operands (FUNC path) | 7 | 3 (2 operands + result) |
| a user FUNC returning a vector literal | 5 | 1 |

`a_handed_over_vector_literal_is_freed_once` covers the handover case: it prints
`1.00 1.00 3.00 4.00 0.00`, with no double free and allocations equal to frees.
Without the claim change it prints all `0.00`.

**Benchmark** (`python3 /tmp/b783-bench.py`: the benchmark project with `main`
reduced to the nbody rows, pre-fix vs fixed compiler, `--run 10`):

- `nbody_vec` median 68.43 → 54.26 ms; `nbody_rec` 39.92 → 39.93 ms and `nbody` are
  unchanged. Checksums are identical.
- With `--debug` and `--run 1`, `arena.0.alloc_calls` goes from 1 000 443 to 447 for
  the three rows. `nbody_vec`'s 10 allocations per step are gone.

Gate: full suite 277 ok, artifact-gate 0 diffs, acceptance 1582 at -O1 and -O3.

The pre-fix column includes the test's four operand locals, which were also blocks.
The user `TYPE Float3`/`Integer2` now builds and prints 96. The repro prints
`5050.00` with `alloc_calls 1`.

## Failing Reproduction

`/tmp/b766B/vp.mfb` (bug-766 B agent):

```basic
IMPORT io
IMPORT vector

FUNC main() AS Integer
  MUT acc AS Float = 0.0
  FOR i = 1 TO 100
    LET c AS vector::Float3 = vector::Float3[1.0, 2.0, toFloat(i)]
    acc = acc + c.z
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
```

Measured (`mfb build -O 1 --debug`, main before bug-766 and the bug-766 branch alike):
prints `5050.00`, `arena.0.alloc_calls 101` — one block per literal. The bug-766 B agent
reports 1 with the name check patched to accept the qualified name (not re-measured).

## Root Cause (partly confirmed)

`vector_field_count` matches `name.resolve()` against bare names; the vector types'
identity became qualified with the bug-480 package-name work (candidate commit
`262a66bd2`, "a bare type spelling inside the compiler is always a user type" — **guess,
to bisect**). The `vector_promotion` unit test did not catch it: it inspects `main`,
while the literal's allocation happens in a separate constructor helper.

Complication (reported, not yet explained): stripping the prefix alone makes
`vector::scale` *worse* (301 → 401 allocations) — so other vector sites also key on the
name and must be fixed together; the fix is not the one-line match.

## Goal / Non-goals

- Every vector-shape site recognises the qualified identity (by type identity, not a
  bare-name match); the repro allocates nothing per iteration; `vector::scale` and the
  other inlined ops allocate no more than before bug-480.
- No language-spec or memory-model change; vector values' semantics and output
  unchanged. Forbidden: matching the bare spelling again (bug-480 made a bare spelling a
  user type — a user's own `TYPE Float3` must not be taken for the builtin).

## Phases

- [x] Phase 1 — RED runtime test (literal loop and `vector::scale` loop, `--debug`
      allocations N vs 2N), bisect the introducing commit, grep every name-keyed vector
      site (`"Float3"`, `vector_field_count`, `vector_call_is_inlined` callers).
- [x] Phase 2 — key the checks on the builtin vector type identity.
- [x] Phase 3 — goldens, full suite, `benchmark` `nbody_vec` / `vector` rows.
