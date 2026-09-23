# bug-681: an `append` item that is arithmetic over a user `FUNC` call falls off the in-place path and the loop goes quadratic

Last updated: 2026-09-23
Effort: medium (1h–2h)
Severity: HIGH
Class: Footgun

Status: Open
Regression Test: `tests/runtime/rt_inplace_self_update.rs` (allocation-flatness row), plus a
`tests/guards/` census row — see Phases

`xs = collections::append(xs, f(i) * 2)`, where `f` is a `FUNC` declared in the
program, does **not** take the in-place append path. Every iteration rebuilds the
whole list instead of writing into its spare slot, so a loop that should be
amortized O(1) per element is O(n) per element and O(n²) overall, and the
discarded blocks are never reused. Building a 260,000-element list this way does
not finish: it was killed by the OOM killer after allocating hundreds of
gigabytes.

The same expression with the call at the top level — `collections::append(xs,
f(i))` — is in place. So is arithmetic with no call in it, and so is arithmetic
over a *builtin* call. Only arithmetic wrapped around a **user-declared `FUNC`**
call loses the fast path, which is what makes this a footgun rather than a
visible limitation: the fast and slow spellings differ by one `* 2`, nothing
diagnoses the difference, and the penalty is unbounded memory rather than a
modest slowdown.

**The single correct behavior a fix produces:** `xs = collections::append(xs,
<any expression of the list's element type>)` stays on the in-place path
whenever the item cannot reach `xs`, regardless of whether a user `FUNC` call
appears inside the item expression. Allocation count stays flat in `n`, matching
the already-pinned `collections::append(xs, i)` row.

References:

- `.ai/collections.md` § "List memory management: headroom + amortized-O(1)
  append" — the contract this breaks, and § "In-place mutation: one table, four
  sites" for the arm/gate vocabulary (`G11` is the gate that declines here).
- `src/codegen/collection/assign/builder_inplace_assign.rs:24`
  `try_inplace_append_assign` — the arm, and its `G11` decline at lines 45–51.
- `src/codegen/memory/value/builder_value_semantics.rs:1189` `static_item_type`
  and `:1217` `static_type_name` — the root cause.
- `planning/plan-147-*` — actively working the same seam, but on a *different*
  site: plan-147 is about handing an owned argument over to a helper
  (`acc = helper(acc, i)`, sites S11/S12). This bug is the plain inline S1
  append whose *item operand* is compound. Confirm with the plan-147 owner
  whether it wants to absorb this; nothing in plan-147-A..F's text covers the
  item-operand shape.
- Found while writing `examples/wind` (GRIB2 decoder): the decode allocated
  without bound until every `collections::append` item was hoisted into a `LET`.

## Failing Reproduction

```basic
IMPORT collections
IMPORT datetime
IMPORT io

FUNC twice(i AS Integer) AS Integer
  RETURN i * 2
END FUNC

FUNC shapeCallArith(n AS Integer) AS Integer
  MUT xs AS List OF Integer = []
  FOR i = 1 TO n
    xs = collections::append(xs, twice(i) * 2)     ' <-- quadratic
  NEXT
  RETURN len(xs)
END FUNC

FUNC shapeCall(n AS Integer) AS Integer
  MUT xs AS List OF Integer = []
  FOR i = 1 TO n
    xs = collections::append(xs, twice(i))          ' <-- linear
  NEXT
  RETURN len(xs)
END FUNC
```

Measured at `n = 16000`, release build, macos-aarch64:

| item operand | shape | n=16000 |
| --- | --- | --- |
| `3 + i * 2` | arithmetic, no call | 1 ms ✓ |
| `twice(i)` | user `FUNC` call, top level | 1 ms ✓ |
| `toFloat(i) + 1.0` | arithmetic over a *builtin* call | 1 ms ✓ |
| `twice(i) * 2` | arithmetic over a *user `FUNC`* call | **2798 ms ✗** |

Scaling of the failing row confirms it is quadratic, not merely slower — time
roughly quadruples as `n` doubles, while the hoisted form stays at 0 ms:

| n | `append(xs, twice(i) * 2)` | `LET v = twice(i) * 2` then `append(xs, v)` |
| --- | --- | --- |
| 2000 | 22 ms | 0 ms |
| 4000 | 89 ms | 0 ms |
| 8000 | 488 ms | 0 ms |
| 16000 | 2443 ms | 0 ms |

- Observed: quadratic time and unbounded retained allocation. At the sizes a
  real decoder uses (260,000 elements) the process was killed by the OS: RSS
  passed 2 GB within 6 s and the original discovery run reached hundreds of GB
  of allocation before macOS became unresponsive.
- Expected: flat allocation count in `n` and the same ~1 ms as every other row.

The workaround, which is what `examples/wind/src/grib.mfb` does today, is to
name the value first — `LET value AS Integer = twice(i) * 2` — which restores
the fast path exactly.

| Environment | | Result |
| --- | --- | --- |
| macos-aarch64 | release, `mfb build` | fails ✗ |
| other targets | not yet measured | the cause is target-independent (a front-end type query), so expect ✗ everywhere |

## Root Cause

`try_inplace_append_assign`
(`src/codegen/collection/assign/builder_inplace_assign.rs:45-51`) commits only
when gate `G11` can prove the item is a single element of the list's element
type:

```rust
match self.static_item_type(&target.args[1]) {
    Some(item_type) if item_type == element_type => {}
    _ => return Ok(false),
}
```

`static_item_type` (`builder_value_semantics.rs:1189`) resolves a user `FUNC`
call, but **only at its own top level**:

```rust
pub(crate) fn static_item_type(&self, value: &NirValue) -> Option<ParameterType> {
    if let Some(type_) = self.static_type_name(value) { return Some(type_); }
    match value {
        NirValue::Call { target, args, .. } /* … */ => {
            if let NirValue::Call { .. } = value {
                if let Some(type_) = self.functions.get(target).map(|f| f.returns.clone())
                    .or_else(|| self.package_return_types.get(target).cloned())
                { return Some(type_); }          // <-- the user-FUNC lookup
            }
            /* … builtin resolution … */
        }
        _ => None,
    }
}
```

The fallback `static_type_name` (`:1217`) resolves a call against a **hardcoded
allowlist of builtin names** (`toFloat`, `len`, `math.*`, `collections.get`, …)
and returns `None` for every other target — including every user-declared
`FUNC`. Its `NirValue::Binary` arm recurses into **`static_type_name`, not
`static_item_type`**, so the user-`FUNC` lookup above is unreachable from inside
a binary operand.

The NIR confirms the shape. `twice(i) * 2` lowers to a `binary` node whose left
operand is the user call (`mfb build --nir`):

```json
{ "op": "assign", "name": "xs", "value": { "kind": "call", "target": "collections.append",
  "args": [ { "kind": "local", "name": "xs" },
            { "kind": "binary", "op": "*",
              "left":  { "kind": "call", "target": "twice", "args": [ … ] },
              "right": { "kind": "const", "type": "Integer", "value": "2" } } ] } }
```

That is exactly why each contrast case is immune:

- `twice(i)` — the call is the top-level value, so `static_item_type`'s own
  `Call` arm runs and the `self.functions` lookup answers.
- `3 + i * 2` — a `binary`, but every leaf is a `Const`/`Local`, which
  `static_type_name` resolves directly.
- `toFloat(i) + 1.0` — a `binary` over a call, but `toFloat` is on
  `static_type_name`'s builtin allowlist, so the recursion answers.
- `twice(i) * 2` — a `binary` over a call that is **not** on the allowlist and
  whose user-`FUNC` lookup lives one level up. `None` → `G11` declines → the
  general copying reassignment.

Declining is semantically correct (the contract is "in-place is an optimization
the program cannot observe"), so there is no wrong answer here — only an
unbounded cost with no diagnostic.

## Goal

- `static_type_name`'s `Call`/`CallResult` arm resolves a user-declared `FUNC`'s
  return type (the `self.functions` / `package_return_types` lookup), so every
  recursive caller — `Binary` above all — sees it.
- `xs = collections::append(xs, f(i) * 2)` has an allocation count flat in `n`,
  pinned by `tests/runtime/rt_inplace_self_update.rs`.
- The contrast rows above stay exactly as they are.

### Non-goals (must NOT change)

- **No semantic change.** In-place is unobservable except by timing and
  allocation count; this bug must not alter any program's output, diagnostics,
  or evaluation order.
- **Do not widen `G11` by weakening the type check.** Accepting an item whose
  type is unknown (rather than teaching the query to know it) would let a bulk
  `append(list, otherList)` onto the single-element path — `G11` exists to keep
  those apart (`builder_inplace_assign.rs:45-47`).
- **Do not relax any aliasing gate** (`G1`, `G7`, `G-global-operand`, the
  `operand_snapshot` rule). The item here genuinely cannot reach `xs`; the bug
  is that its *type* is unknown, not that its *aliasing* was in doubt.
- **Do not "fix" this by rewriting the reproduction** to hoist the value. The
  hoist is the user-space workaround, not the fix; the quadratic spelling must
  become linear.
- `examples/wind/src/grib.mfb` keeps its hoists and its comment either way —
  they are correct code, and reverting them is not part of the fix.

## Blast Radius

Searched for the same hazard with
`grep -rn "collections::append(" --include=*.mfb examples packages src/docs` and
by reading the two type queries' callers
(`grep -rn "static_item_type\|static_type_name" src/`).

- `src/codegen/memory/value/builder_value_semantics.rs:1217` `static_type_name`
  — **fixed by this bug**. This is the root cause.
- `src/codegen/collection/assign/builder_inplace_assign.rs:24`
  `try_inplace_append_assign` (`G11`) — **fixed by this bug**, via the query.
  No change to the arm itself is expected.
- Every other `SELF_UPDATE_ARMS` member that gates on `static_item_type` /
  `static_type_name` (the `set`/map arms in `builder_inplace_setmap.rs`, the
  shrink/sort/rewrite arms) — **same hazard, and fixed by the same one-place
  change**. Each needs its own census row asserting flatness with a user-`FUNC`
  call under a binary; Phase 1 enumerates them.
- `examples/wind/src/grib.mfb` — **the discovery site, already worked around**
  (seven hoists). Unaffected once fixed; the hoists stay valid.
- Map-value reads (`collections.get` on a `Map`) deliberately return `None` from
  `static_type_name` — **unaffected**, that is a separate conservative choice
  documented in place, not this bug.
- `NirValue::Unary` and other recursive arms of `static_type_name` — **latent,
  same mechanism** (they too recurse into `static_type_name`). In scope only to
  the extent the one-place fix covers them; Phase 1 records whether a `-f(i)`
  item reproduces.

## Fix Design

Move the user-`FUNC` return-type lookup down from `static_item_type` into
`static_type_name`'s `Call`/`CallResult`/`RuntimeCall` arm, as the fallback
after the builtin allowlist misses. `static_item_type` then keeps its current
behavior for free (its first line already delegates to `static_type_name`), and
every recursive arm — `Binary`, `Unary`, `ResultValue`, the `get`/`math.*`
argument recursions — gains the same reach.

The correctness risk is not in the query but in **what newly commits**: programs
that previously took the copying reassignment will now take an in-place arm, so
their emitted bytes change. `.ai/collections.md` warns that vreg/stack-slot
allocation order is observable in emitted bytes, so any byte-identity or
`.ncode` fixture covering such a program shifts. That delta is the fix working,
and Phase 3 must confirm each shifted fixture is a program of exactly this shape
— not an unrelated one.

Rejected alternatives:

- *Special-case the `Binary` arm to call `static_item_type`.* Fixes the measured
  case and leaves `Unary` and the other recursive arms broken, and splits the
  lookup across two functions that already differ confusingly.
- *Widen `G11` to accept an unknown item type.* Forbidden above — it collapses
  the single-element and bulk-append paths.
- *Diagnose the slow shape instead of fixing it.* A warning telling the
  programmer to add a `LET` is a worse contract than making the two spellings
  cost the same.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a `tests/runtime/inplace_self_update/cases.tsv` line and the matching
      `rt_inplace_self_update.rs` row for `append` with item `f(i) * 2` where
      `f` is a program `FUNC`; confirm it fails the allocation-flatness
      assertion today.
- [ ] Add the three contrast rows (`f(i)`, `3 + i * 2`, `toFloat(i) + 1.0`) as
      guards that must stay flat.
- [ ] Determine whether `-f(i)` (the `Unary` arm) and the `set`/map arms
      reproduce the same decline; write each verdict into Blast Radius.
- [ ] Confirm with the plan-147 owner whether that plan absorbs this; record the
      answer here.

Acceptance: the new row fails for the documented reason (`G11` declines because
`static_item_type` answers `None`); the audit has a verdict per site.
Commit: —

### Phase 2 — the fix

- [ ] Move the `self.functions` / `package_return_types` return-type lookup into
      `static_type_name`'s `Call` arm
      (`src/codegen/memory/value/builder_value_semantics.rs:1217`), after the
      builtin allowlist.
- [ ] Simplify `static_item_type` (`:1189`) to whatever remains once its
      duplicate lookup is redundant, without changing its answers.

Acceptance: Phase 1's row passes; the three contrast rows stay flat; no
diagnostic or program output changes anywhere.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Re-run the byte-identity / `.ncode` fixtures; for every shifted fixture,
      confirm by inspection that it contains an item operand of exactly this
      shape before re-baselining, per `AGENTS.md`.
- [ ] `cargo test` full suite, plus the collection suites named in
      `.ai/collections.md`.
- [ ] Re-run the reproduction above and confirm the failing row drops to ~1 ms.
- [ ] Re-run `examples/wind` end to end and confirm the GRIB decode is unchanged
      (125 ms for two 259,920-point fields) with the hoists still in place.

Acceptance: full suite green; every expected-output delta is a program of this
shape; the reproduction is linear everywhere it was quadratic.
Commit: —

## Validation Plan

- Regression test(s): the `rt_inplace_self_update.rs` flatness row + three
  contrast rows, and the `tests/guards/inplace_self_update_census.rs` entry.
- Runtime proof: the `n = 2000..16000` scaling table above, which must become
  flat; and `examples/wind` decoding a real 362 KB NOMADS GRIB2 file within a
  bounded RSS.
- Doc sync: add the item-operand rule to `.ai/collections.md` § "In-place
  mutation" — it is exactly the kind of gotcha that file exists for.
- Full suite: `cargo test` plus the canvas/collection suites.

## Open Decisions

- Does plan-147 absorb this, or does it land independently? Recommended:
  **land independently** — plan-147's sites are S11/S12 (hand-over through a
  helper) and this is S1 with a compound item, so the fix is one query arm and
  need not wait on a huge plan. (§References)

## Summary

The engineering risk is entirely in Phase 3: the fix itself is a few lines moved
between two functions, but it turns declines into commits, and every emitted-byte
fixture covering an affected program shifts. The gates, the semantics, and the
`G11` type check all stay exactly as they are — only the type *query* gets
smarter, and only about a return type the compiler already knows.
