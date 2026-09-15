# bug-626: `list = collections::append(list, <builtin call>)` copies the whole list on every append

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory and time)

Status: Open
Regression Test: tests/runtime/rt_scope_drop_leaks.rs or a codegen-inspection test (to add, Phase 1)

A loop that appends a builtin call's result straight into a same-function `MUT` list,
`keep = collections::append(keep, fs::readText(path))`, copies the entire list on every
iteration instead of appending in place. Memory and time grow with the square of the element
count. Binding the value to a `LET` first makes the same loop linear. The output is correct
either way, so no functional test notices; only memory and wall time show it. On the macOS
host, 5,000 appends of a 5,000-byte `fs::readText` result mapped 63 GB and peaked at 63 GB RSS.

**The single correct behavior a fix produces:** the append of a builtin call result takes the
same in-place path as the bound form, so allocated bytes grow linearly with the element count.

References:

- `src/docs/spec/language/05_collections.md` (in-place collection updates).
- Memory note `collection-set-in-place-only-for-same-function-local`: the same fallback for
  a *user* function call operand was fixed by `static_item_type`'s return-type lookup. Builtin
  calls were left out.
- Found by plan-133-C Phase 1 while building a many-grows workload for the arena memory series.

## Failing Reproduction

`/tmp/plan-133-c/nest{100,200}` and `/tmp/plan-133-c/bound{100,200}`, built with
`target/release/mfb build --debug` on macOS, main `14c9fc1ca` + plan-133 through `ab1a95b35`.
The input is a 5,000-byte file.

```
IMPORT collections
IMPORT fs
IMPORT io

FUNC main AS Integer
  MUT keep AS List OF List OF Byte = []
  FOR i = 1 TO {N}
    keep = collections::append(keep, fs::readBytes("/tmp/plan-133-c/block5000.bin"))
  NEXT
  io::print(toString(len(keep)))
  RETURN 0
END FUNC
```

- Observed (`arena.0.*` of the `--debug` report): N=100 `alloc_calls 502`,
  `alloc_bytes 26687264`; N=200 `alloc_calls 1002`, `alloc_bytes 104174464`. The allocation
  count doubles while the bytes rise ×3.90: each append makes one allocation as large as the
  whole list so far (Σ i × 5,000 B).
- Expected: bytes about ×2 for 2× N, as the bound form below measures.

| Shape | N=100 `alloc_bytes` | N=200 `alloc_bytes` | Ratio |
|---|---:|---:|---:|
| `append(keep, fs::readBytes(…))` into `List OF List OF Byte` | 26,687,264 | 104,174,464 | ×3.90 (quadratic) |
| `LET s AS String = fs::readText(…)` then `append(keep, s)` into `List OF String` | 2,386,832 | 5,401,056 | ×2.26 (linear; the excess is the list's own growth) |

At scale (`/tmp/plan-133-c/str5k`: `append(keep, fs::readText(…))`, N=5,000):
`alloc_bytes 63063560064`, `mapped_bytes 63023300608`, 44.5 s, maximum RSS 63,055,724,544 B.
The `List OF List OF Byte` form at N=5,000 (`big5k`) mapped 63,523,377,152 B in 51.4 s. **Do not
run these shapes at N ≥ 5,000 on a shared machine**; a 50,000 run was killed before it
exhausted memory.

## Root Cause

Confirmed by reading the code:

- `try_inplace_append_assign` (`src/codegen/collection/assign/builder_inplace_assign.rs`)
  takes the in-place path only when the appended value's item type is statically known and
  equals the list's element type (the G11 check):
  `match self.static_item_type(&target.args[1]) { Some(item_type) if item_type ==
  element_type => {} _ => return Ok(false) }`. `Ok(false)` falls through to the general
  path, which copies the collection.
- `static_item_type` (`src/codegen/memory/value/builder_value_semantics.rs`) first asks
  `static_type_name`, then, for a `NirValue::Call`, looks the target up in `self.functions`
  (user functions) and `self.package_return_types` (package functions).
- `static_type_name`'s `Call`/`CallResult`/`RuntimeCall` arm is a hand-written table of
  builtin names (`replace`, `toString`, `len`, `get`/`getOr`, a few `math.*`) ending in
  `_ => None`. `fs.readText` and `fs.readBytes` are not in it, and builtins are in neither of
  `static_item_type`'s maps, so the item type is `None` and the append copies.

The bound form is immune because `NirValue::Local` resolves through `self.locals`.

Re-measured at `af7d9b778` (`/tmp/b626/run626.sh`, `--debug` release builds): `nest` 100 → 200
`alloc_bytes` 26,687,264 → 104,174,464 (×3.90), `str` (`fs::readText` into `List OF String`)
26,471,264 → 103,342,464 (×3.90), `bound` 2,386,832 → 5,401,056 (×2.26).

### Phase 1 audit

- **Gates.** `static_item_type` has 15 callers (`grep -rn "static_item_type(" src/codegen`), all
  in-place recognisers (`builder_inplace_assign.rs` ×14, `builder_control.rs` bulk append ×1).
  Every one only compares the answer for EQUALITY with the element, key or collection type
  and declines otherwise, and each arm re-checks the lowered `type_` as a hard `Err`. A
  wider answer can therefore only admit an operand whose type genuinely matches: each gate is
  **safe to widen**, the single-element / bulk split included (a `List OF T` result still
  differs from `T`).
- **Aliasing.** A widened gate lowers the operand with `lower_value_stored`. No `Call`,
  `CallResult` or `RuntimeCall` is an aliasing source (`value_is_aliasing_source`), and user
  and package function calls already reach these arms through the return-type lookup, so a
  builtin result adds no new ownership case.
- **Untyped builtins.** Every builtin not in `static_type_name`'s table was untyped here (the
  whole `fs`, `strings`, `collections`, `json`, … surface). The registry resolver
  `builtins::resolve_call_return_type_typed` types all of them, and `static_type_name_for_fold`
  and `overload_arg_type` already use it.

## Goal

- `append(keep, fs::readText(…))` and `append(keep, fs::readBytes(…))` into same-function
  `MUT` lists grow `alloc_bytes` linearly (×2 for 2× N), like the bound form.
- No builtin whose return type is known is left off the in-place path by this lookup.

### Non-goals (must NOT change)

- Value semantics: the non-local, `by_ref` and live-`FOR EACH` exclusions stay (G1–G10).
- The bulk `append(list, otherList)` path.
- **Tempting wrong fix:** adding `fs.readText` and `fs.readBytes` to the hand-written table.
  It fixes the two calls found here and leaves every other builtin broken.

## Blast Radius

- Every gate in `builder_inplace_assign.rs` that asks `static_item_type` (single-element
  `append`, the record-field `WITH` append, bulk append, set `add`, splice) — the same
  missing-type fallback; audit in Phase 1.
- Every builtin call whose name is not in `static_type_name`'s table — affected when appended
  directly. Phase 1 lists them from the builtin registry.
- User and package function calls — unaffected (the return-type lookup).

## Fix Design

Resolve a builtin call's return type from the builtin registry (the same descriptor the type
checker uses) in `static_item_type`, instead of the name table. Then the table is only a fast
path, or can be removed.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] A test comparing `alloc_bytes` at N and 2N for `append(list, fs::readText(…))` (small N,
      a 5,000-byte input), asserting linear growth; confirm it fails.
- [ ] Audit every `static_item_type` gate and the builtin registry for untyped call results.

Acceptance: the test fails for the documented reason; the audit has a verdict per gate.
Commit: —

### Phase 2 — the fix

- [ ] Builtin return types in `static_item_type`, from the registry.

Acceptance: the Phase 1 test passes; the bound form is unchanged.
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Regenerate the goldens the in-place path shifts; full suite; `scripts/test-accept.sh`.

Acceptance: full suite green; the golden deltas are only in-place appends.
Commit: —

## Validation Plan

- Regression test: the Phase 1 N-vs-2N case.
- Runtime proof: the `nest`/`str5k` probes at small N.
- Doc sync: none expected.
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- None.

## Summary

A missing type lookup silently turns an idiomatic loop quadratic. The fix belongs in the
lookup, not in a longer name table.
