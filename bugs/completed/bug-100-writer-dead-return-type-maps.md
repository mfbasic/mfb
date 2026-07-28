# bug-100 — dead per-function return-type maps in `lower_project_with_external_functions`

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G3).
**Severity:** LOW — dead code / wasted allocation, no runtime effect.
**Class:** dead-code.

## Finding

`src/binary_repr/writer.rs:176-190` — `function_return_types` and
`function_return_type_names` are populated (lines 181-182, 189-190) but never
read afterward; only `function_ids` is consumed (line 213). The
`types.type_id(...)` calls at 180/188 have an interning side effect, but the
same return types are re-interned in `lower_function` (line 558), so both maps
are pure dead storage (allocation + clones per build).

## Fix

Delete both maps; keep only the `function_ids` construction (and, if the
interning side effect at 180/188 is load-bearing for ordering, replace with a
bare `types.type_id(...)` discard — but line 558 already covers it).

## Verified

Grepped all references in writer.rs — the two maps have no read site after
construction.
