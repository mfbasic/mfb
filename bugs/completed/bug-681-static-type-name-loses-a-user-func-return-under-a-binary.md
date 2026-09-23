# bug-681: an `append` item that is arithmetic over a user `FUNC` call falls off the in-place path and the loop goes quadratic

Last updated: 2026-09-23
Effort: medium (1h–2h)
Severity: HIGH
Class: Footgun

Status: FIXED
Regression Test: `tests/runtime/rt_inplace_item_operand.rs` — one allocation-flatness
row per `G11` call site with a composite item operand, plus three contrast rows

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
- Every other arm that gates on `static_item_type` — **same hazard, confirmed by
  measurement, and fixed by the same one-place change.** Phase 1's audit found
  the query has exactly six `G11` call sites
  (`grep -n static_item_type src/codegen/collection/assign/builder_inplace_assign.rs`),
  and every one of them reachable with a composite operand reproduces. Measured
  as `alloc_calls` growth over an extra 2000 iterations
  (`tests/runtime/rt_inplace_item_operand.rs`), before → after:

  | arm | item operand | before | after |
  | --- | --- | --- | --- |
  | `append(list, item)` (`:48`) | `twice(i) * 2` | +4000 | +2 |
  | `append(list, item)` | `-twice(i)` | +4000 | +2 |
  | `append(list, item)` | `mkBox(i).n` | +8000 | +4002 (control +4000) |
  | `append(list, sublist)` (`:234`) | `mkBox(i).items` | +6000 | +4002 (control +4000) |
  | `add(set, item)` (`:125`) | `twice(i) * 2` | +4000 | +2 |
  | `remove(set, item)` (`:769`) | `twice(i) * 2` | +2000 | +0 |
  | `removeKey(map, key)` (`:186`) | `twice(i) * 2` | +2000 | +0 |

  The sixth site, `lower_field_splice` (`:1299`), is the field-site spelling of
  the same `G11` and takes the fix through the same query. So the bug was never
  `append`-specific: `add`, `remove` and `removeKey` all lost the path too, each
  as unboundedly as `append`.
- `collections::set` (`:278`) and `try_inplace_insert_assign` (`:805`) —
  **unaffected**: neither has a static `G11` gate (`insert` has no bulk form to
  distinguish), so neither consults the query.
- `examples/wind/src/grib.mfb` — **the discovery site, already worked around**
  (seven hoists). Unaffected once fixed; the hoists stay valid.
- Map-value reads (`collections.get` on a `Map`) deliberately return `None` from
  `static_type_name` — **unaffected**, that is a separate conservative choice
  documented in place, not this bug.
- `NirValue::Unary` and the other composite arms — **confirmed, not latent, and
  fixed.** `-twice(i)` reproduces exactly as `twice(i) * 2` does (+4000 blocks),
  and so does `NirValue::MemberAccess`: `mkBox(i).n` was +8000 against a +4000
  control, a shape the original write-up did not anticipate. `MemberAccess` is
  also the one composite that reaches the **bulk** `append` arm, since no
  operator yields a `List`: `append(xs, mkBox(i).items)` was +6000 against the
  same +4000 control. All four composite arms (`Binary`, `Unary`,
  `MemberAccess`, `ResultValue`) now share one implementation,
  `static_composite_type`, so the reach cannot drift apart again.
- `static_type_name`'s other consumers — **deliberately untouched**, which is
  where the fix deviates from the Fix Design below. See the `STATUS` block.

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

- [x] Add the allocation-flatness rows for a self-update whose item operand is a
      composite expression over a program `FUNC`, and confirm they fail today.
      They live in a new `tests/runtime/rt_inplace_item_operand.rs` rather than a
      `cases.tsv` line: `cases.tsv` is a census keyed by *builtin signature* and
      each row is also run at 15 field sites out of `field_expect.tsv`, so a
      second row for an already-listed signature would collide with the field
      expectations of the first. The new file reuses the same measure (build at
      `N` and `2N` with `--debug`, read the arenas' `alloc_calls`).
- [x] Add the three contrast rows (`f(i)`, `3 + i * 2`, `toFloat(i) + 1.0`) as
      guards that must stay flat. All three were flat before and after (+2).
- [x] Determine whether `-f(i)` (the `Unary` arm) and the `set`/map arms
      reproduce the same decline; write each verdict into Blast Radius. **They
      do** — and so does `MemberAccess`, which the write-up missed, including on
      the bulk-`append` arm. The table is in Blast Radius.
- [x] Confirm with the plan-147 owner whether that plan absorbs this. Landed
      **independently**, per this doc's Open Decision: plan-147's sites are
      S11/S12 (hand-over through a helper) and nothing in plan-147-A..F's text
      covers an item-operand shape. No file this fix touches is a plan-147 file.

Acceptance: met — the rows fail for the documented reason (`G11` declines
because `static_item_type` answers `None`), and the audit has a verdict per site.
Commit: `01f3ecaf6`

### Phase 2 — the fix

- [x] Reach the `self.functions` / `package_return_types` return-type lookup
      from inside a composite operand. **Deviation:** the lookup was *not* moved
      into `static_type_name`'s `Call` arm as designed, and it was not put on
      `static_item_type` either. Both have consumers outside the gates:
      `static_type_name` gates float-arithmetic lowering
      (`builder_numeric.rs:192`) and `is_function_value`
      (`operand_snapshot.rs:338`), and three in-code comments (bug-561, bug-626,
      the fold twin at `:1441`) warn against widening it; `static_item_type`
      feeds `nir_call_is_infallible_builtin`
      (`engine/control/builder_control.rs:808`), which types a builtin's
      arguments to decide whether a call can fail, so widening it changes which
      failure paths the optimizer may elide in programs with no self-update in
      them at all. The reach therefore went onto a new `static_operand_type`
      whose only callers are the six `G11` sites, with the shared derivations
      factored out so no query carries a copy: `static_composite_type` (the
      `Binary`/`Unary`/`MemberAccess`/`ResultValue` rules, shared with
      `static_type_name`) and `static_call_type` (the call lookups, shared with
      `static_item_type`).
- [x] Leave `static_item_type`'s answers exactly as they were — verified by the
      artifact gate, whose 18 diffs are unchanged with and without that query
      widened, i.e. none of them came from it.

Acceptance: met — every Phase 1 row passes, the three contrast rows stay flat,
and no diagnostic or program output changes anywhere (full suite below).
Commit: `2fe6c1e39`

### Phase 3 — regenerate expected outputs + full validation

- [x] Re-run the byte-identity / `.ncode` fixtures; for every shifted fixture,
      confirm by inspection that it contains an item operand of exactly this
      shape before re-baselining. **18 `.ncode` sums shifted across four
      fixtures** — the Fix Design predicted exactly this. Each was localized
      first: the gate is clean on the pre-fix compiler (`0 diff(s)`), and
      instrumenting the gates showed 1–4 newly-committed `G11` operands per
      fixture, every one a `Binary` over a call. The NIR names them, all in
      package-internal helpers whose `FUNC` `static_type_name`'s table does not
      name:

      | fixture | the newly in-place statement |
      | --- | --- |
      | `byte-identity/compress` | `o = append(o, #compress_reverseBits(c, l) * k + …)` — deflate's Huffman tables |
      | `byte-identity/crypto`, `rt-behavior/crypto-ec-valid` | `o = append(o, lo + bits.sl(hi, 8))` — `#crypto_unpack25519` |
      | `syntax/app/app-mouse-surface` | `o = append(o, #canvas_geoAt(offset, 16) + …)` — `#canvas_rememberScene` |

      So the delta is the fix working, and it means deflate, curve arithmetic and
      canvas scene accumulation were themselves rebuilding a list per element.
      Only `.ncode` moved: every `.run` behavior golden is unchanged, which is
      the "in-place is unobservable" contract holding. Re-baselined with the
      gate's own write half, `scripts/regen-native-goldens.sh`, over those four
      fixtures only.
- [x] `cargo test` full suite, plus the collection suites.
- [x] Re-run the reproduction and confirm the failing row drops to ~1 ms:
      **3759 ms → 1 ms** at `n = 16000`, release, macos-aarch64, alongside the
      contrast rows at 2 ms and 1 ms.
- [ ] Re-run `examples/wind` end to end. **Not runnable here:** `examples/wind`
      is not in this repository (the discovery site was an external project), so
      this step is unverified. The equivalent in-repo proof is the
      `mkBox(i).items` bulk-append row, the same shape the decoder hoisted.

Acceptance: full suite green; no expected-output delta to inspect; the
reproduction is linear everywhere it was quadratic.
Commit: see STATUS

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

## STATUS: FIXED (154dfb91b)

Landed on `main` from `worktree-B-681`. Commits: `01f3ecaf6` (Phase 1, the RED
rows), `2fe6c1e39` (Phase 2, the fix), `55753bd61` (audit + `.ai/collections.md`),
`995f7f5f1` (containment + golden re-baseline), `154dfb91b` (`cargo fmt` + this
record).

**Result.** `collections::append(xs, f(i) * 2)` at `n = 16000` went from 2798 ms
(measured 3759 ms on the run that reproduced it) to **1 ms**, release,
macos-aarch64. Full suite green: 223 targets, `test result: ok`, `cargo test`
exit 0, including the artifact gate at `2116 golden(s) checked, 0 diff(s)` and
the in-place census `rt_inplace_self_update` (8064 s).

**Deviations from the plan as written.**

1. *Not* `static_type_name`, and *not* `static_item_type` either. Both have
   consumers outside the gates — float-arithmetic lowering and
   `is_function_value` for the first, `nir_call_is_infallible_builtin` for the
   second — so the reach went onto a new `static_operand_type` with exactly six
   callers, all `G11`. The shared derivations were factored out
   (`static_composite_type`, `static_call_type`) so no query carries a copy.
2. The regression rows live in a new `tests/runtime/rt_inplace_item_operand.rs`,
   not in `cases.tsv`: that file is a census keyed by builtin signature whose
   rows also run at 15 field sites out of `field_expect.tsv`, so a second row for
   an already-listed signature would collide with the first's expectations.
3. `examples/wind` is not in this repository, so that Phase 3 step is
   **unverified** rather than done.

**What the write-up underestimated.** The bug was never `append`-specific:
`add`, `remove` and `removeKey` lost the path identically, and `MemberAccess` on
a call reproduces it too — a shape the doc did not anticipate, and the only
composite that reaches the bulk-`append` arm, since no operator yields a `List`.
Seven rows, each unbounded. The fix also lands in library code: the 18 shifted
`.ncode` sums are deflate's Huffman tables, `#crypto_unpack25519` and
`#canvas_rememberScene`, all of which were rebuilding a list per element.

**Watch for.** The reach is one query with one shared composite table. A new
composite `NirValue` added to `static_type_name` but not to
`static_composite_type` puts the two back out of step, which is exactly how this
bug arose.
