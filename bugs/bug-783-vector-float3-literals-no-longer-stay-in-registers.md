# bug-783: `vector::Float3[…]` literals and inlined vector ops no longer run register-native — the vector-shape check matches the bare name, the type is now `vector.Float3`

Last updated: 2026-09-29
Effort: medium (1h–2h) — guess
Severity: MEDIUM — a silent performance regression on every vector-heavy loop (one arena
block per literal); output unchanged
Class: Other (performance regression)

Status: Open
Regression Test: to be added (Phase 1)

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

- [ ] Phase 1 — RED runtime test (literal loop and `vector::scale` loop, `--debug`
      allocations N vs 2N), bisect the introducing commit, grep every name-keyed vector
      site (`"Float3"`, `vector_field_count`, `vector_call_is_inlined` callers).
- [ ] Phase 2 — key the checks on the builtin vector type identity.
- [ ] Phase 3 — goldens, full suite, `benchmark` `nbody_vec` / `vector` rows.
