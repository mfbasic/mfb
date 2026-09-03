# plan-121 gate inventory — the decline conditions of every in-place arm

Produced by plan-121-A Phase 1. This file is the **specification** that
plan-121-A Phase 2's `InPlaceGate` implements: a condition listed here must be
enforced by the seam, because a gate weaker in the seam than in the arm it
replaces silently un-protects an aliasing case in every container at once.

Population: 10 arms.

```
grep -rhoE "fn try_inplace_[a-z_]*" src/codegen/ | sort -u | wc -l   # → 10
```

Eight live in `src/codegen/collection/assign/builder_inplace_assign.rs`
(`wc -l` → 907); two — `try_inplace_state_scalar_assign` (`:51`) and
`try_inplace_state_collection_append` (`:212`) — live in
`src/codegen/engine/control/builder_control.rs`.

Dispatch: the eight local/record arms are chained at
`builder_control.rs:879-909` (short-circuit `&&` of negations, falling through to
the general copying reassignment). The two STATE arms are dispatched separately
at `builder_control.rs:1050` and `:1056`, off the `NirOp::StateAssign` path.

---

## G-codes: the condition vocabulary

Each condition is given a code so an arm's row can cite it and Phase 2 can be
checked line-by-line against the set.

| code | condition (arm declines when…) |
|---|---|
| **G1** | `by_ref` — the local's slot holds a pointer to the parent slot, not the buffer |
| **G2** | the NIR value is not the expected shape (`Call` / `WithUpdate` / string-concat chain) |
| **G3** | `native_builtin_target(target)` is not the expected operation name |
| **G4** | the call arity is not the expected one |
| **G5** | `args[0]` is not a bare `NirValue::Local` |
| **G6** | `args[0]` is not the *same binding* being assigned (self-update) |
| **G7** | a live `FOR EACH` iterates this binding — `for_each_iterable_locals` |
| **G8** | the local is not in `self.locals` |
| **G9** | the local's type is not a typed List / Set / Map, as the arm requires |
| **G10** | `CollectionTypeLayout::from_type` is `None` for the collection type |
| **G11** | the RHS's `static_item_type` is not the required type (element vs whole-collection) |
| **G12** | the RHS aliases the mutated collection itself (self-alias `f(x, x)`) |
| **G13** | `WithUpdate.target` is not this same local (record) / this resource's `.state` (STATE) |
| **G14** | `updates.len() != 1` |
| **G15** | a live `FOR EACH` iterates this **record field** — `for_each_iterable_record_fields` |
| **G16** | a live `FOR EACH` iterates this **state field** — `for_each_iterable_state_fields` |
| **G17** | the field is not a *last-inlined* List field — `record_collection_last_inlined` |
| **G18** | `args[0]` is not exactly this record/state field (`value_is_record_field` / `value_is_state_field`) |
| **G19** | no capacity shadow slot for this name — `string_capacity_slots` |
| **G20** | the value is not a recognised string self-append chain — `string_self_append_operands` |
| **G21** | the target name reappears in a later operand (`s = s & x & s`) — `nir_value_reads_local` |
| **G22** | an updated STATE field is inlined or a pointer composite (scalar arm only) |
| **G23** | the updated field name is not found in the record's field list |

Two conditions are enforced **after** lowering as a hard `Err`, not a decline —
they are type-invariant assertions, not gates, and Phase 2 must keep them as
errors, not convert them to declines:

| code | assertion (arm returns `Err`) |
|---|---|
| **E1** | the lowered index is not `Integer` (`set`, list overload) |
| **E2** | the lowered item/key/value type ≠ the collection's element/key/value type |

And one obligation is not a gate at all but a mandatory emission step:

| code | obligation |
|---|---|
| **O1** | `observe_float` on every value entering the collection (plan-17 finiteness boundary) |
| **O2** | `materialize_value` before the payload spill (`d`-native float → GP bits, plan-01) |
| **O3** | `locals.get_mut(name).constant = None` after a successful in-place mutation |
| **O4** | STATE only: write the (possibly reallocated) state pointer back through `RESOURCE_OFFSET_STATE` |

---

## The matrix — which arm enforces which condition

`✓` = enforced. `—` = not applicable to that arm's shape.

| condition | append | bulk&nbsp;append | set&nbsp;add | removeKey | set | prepend | concat | rec&nbsp;append | state&nbsp;scalar | state&nbsp;append |
|---|---|---|---|---|---|---|---|---|---|---|
| G1 by_ref | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | — |
| G2 shape | ✓ Call | ✓ Call | ✓ Call | ✓ Call | ✓ Call | ✓ Call | ✓ chain | ✓ With | ✓ With | ✓ With |
| G3 op name | `append` | `append` | `add` | `removeKey` | `set` | `prepend` | — | `append` | — | `append` |
| G4 arity | 2 | 2 | 2 | 2 | 3 | 2 | — | 2 | — | 2 |
| G5 arg0 Local | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | — | — | — |
| G6 self-update | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ (via G20) | — | — | — |
| G7 FOR EACH local | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | n/a¹ | — | — | — |
| G8 local exists | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | — | ✓² | ✓² |
| G9 typed elem | ✓ List | ✓ List | ✓ Set | ✓ Map | ✓ List\|Map³ | ✓ List | — | ✓ List | — | ✓ List |
| G10 layout | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | — | ✓ |
| G11 static type | ✓ elem | ✓ list | ✓ elem | ✓ key | ⁴ | ⁴ | — | ✓ elem\|list | — | ✓ elem\|list |
| G12 self-alias | ⁵ | ✓ | ⁵ | ⁵ | ⁵ | ⁵ | ✓ (G21) | ✓ | — | ✓ |
| G13 With target | — | — | — | — | — | — | — | ✓ local | ✓ `.state` | ✓ `.state` |
| G14 one update | — | — | — | — | — | — | — | ✓ | ✗⁶ | ✓ |
| G15 FOR EACH rec field | — | — | — | — | — | — | — | ✓ | — | — |
| G16 FOR EACH state field | — | — | — | — | — | — | — | — | ✗⁷ | ✓ |
| G17 last-inlined | — | — | — | — | — | — | — | ✓ | — | ✓ |
| G18 arg0 is the field | — | — | — | — | — | — | — | ✓ | — | ✓ |
| G19 shadow slot | — | — | — | — | — | — | ✓ | — | — | — |
| G20 concat chain | — | — | — | — | — | — | ✓ | — | — | — |
| G21 operand re-reads | — | — | — | — | — | — | ✓ | — | — | — |
| G22 inlined/pointer field | — | — | — | — | — | — | — | — | ✓ | — |
| G23 field found | — | — | — | — | — | — | — | ✗⁸ | ✓ | ✗⁸ |
| E1 index Integer | — | — | — | — | ✓ | — | — | — | — | — |
| E2 lowered type | — | ✓ | — | — | ✓ | ✓ | ✓ String | — | — | — |
| O1 observe_float | ✓ | ✗⁹ | ✓ | ✗¹⁰ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| O2 materialize | ✗¹¹ | ✗¹¹ | ✓ | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ |
| O3 clear constant | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✗¹² | ✗¹² |
| O4 state write-back | — | — | — | — | — | — | — | — | ✗¹³ | ✓ |

### Footnotes — every asymmetry, and whether it is deliberate

1. **`concat` has no `FOR EACH` gate and needs none.** A `String` can never be a
   `FOR EACH` iterable, so `for_each_iterable_locals` can never contain it. The
   doc comment states this. **Not a hole.**
2. **The STATE arms `?` on a missing local instead of declining** — they return
   `Err("native code state assignment unknown local …")` rather than `Ok(false)`.
   Equivalent in effect (the local must exist by then); Phase 2 must not silently
   convert this to a decline, which would change a compile error into a slow path.
3. **`set` handles two collection kinds in one arm** — List overload first, Map
   overload second, `Ok(false)` if the type is neither. It is the only arm whose
   G9 is a disjunction.
4. **`set` and `prepend` deliberately skip the static-type gate (G11).** Both are
   documented: the source checker enforces the item type, and the post-lowering
   `E2` check catches any mismatch as a hard error. `append` needs G11 because it
   must *distinguish* single-element from bulk; `set`/`prepend` have no bulk form.
5. **Only `bulk append`, `rec append`, `state append` and `concat` gate the
   self-alias (G12).** For the others it is unreachable-by-typing, not forgotten:
   - `append(x, x)` where `x: List OF T` fails G11 (item type is the list type,
     not the element type) — it falls through to the bulk arm, which *does* gate.
   - `add(s, s)` / `removeKey(m, m)` / `set(l, i, l)` fail the element/key type
     check for the same reason (a collection is not its own element type, absent
     a self-nesting `List OF List OF T` where `x` still is not `List OF T`).
   - `prepend(l, l)` likewise.

   **This is load-bearing and fragile.** A future arm that widens G11 (e.g.
   accepts `Unknown`) re-opens the self-alias hole for that operation. Phase 2's
   `InPlaceGate` must make G12 explicit and unconditional rather than inherit it
   as a side effect of typing.
6. **`state_scalar` permits multi-field updates (no G14)** — deliberately: it
   writes each scalar at its own fixed slot, so N independent stores are sound.
   Every *collection* arm requires `updates.len() == 1`.
7. **`state_scalar` has no `FOR EACH` gate and needs none:** a fixed-width scalar
   store never moves the block and never frees a buffer an iterator holds. Only
   the collection arm can realloc.
8. **G23 is subsumed for the record/state append arms** —
   `record_collection_last_inlined` returns `None` when the field name is absent,
   so G17 covers it. `state_scalar` checks it directly because it has no G17.
9. **`bulk append` does not call `observe_float`** — the RHS is a whole
   `List OF T`, not a scalar; its elements crossed the observation boundary when
   that list was built. Deliberate, not a hole.
10. **`removeKey` does not call `observe_float` on the key.** Every *other* arm
    observes a `Float` entering the collection. A key that is looked-up-and-removed
    does not *enter* the collection, so nothing new is stored; the surrounding
    `lower_value` of a `Float` key is the same value the map was built with.
    Recorded as deliberate-by-inspection. **Phase 2 must not "helpfully" add O1
    here** — it would change emission and break byte-identity.
11. **`append` and `bulk append` spill with a bare `abi::store_u64` after
    `materialize_value` / without it respectively**, where `set_add` and
    `removeKey` use `store_value_at`. `append` *does* call `materialize_value`;
    `bulk append` does not (its RHS is a collection pointer, never a `d`-native
    float). The store helper differs (`store_u64` vs `store_value_at`) and Phase 2
    must preserve the *exact* helper per arm — this is precisely the kind of
    detail that shows up as a byte-identity diff.
12. **Neither STATE arm clears `local.constant`.** The local is the *resource
    handle*, not the collection; its constant-ness is unrelated to the state
    block's contents, so there is nothing to invalidate. Deliberate.
13. **`state_scalar` needs no write-back (O4):** a scalar store never reallocates,
    so the STATE pointer cannot change. Only the collection arm can move the block.

---

## Ordering constraints Phase 2 must preserve

These are not conditions but sequencing facts; getting them wrong changes vreg
allocation order and therefore emitted bytes (`.ai/codegen-invariants.md`:
vreg-alloc order is observable).

- **O-order-1.** Every gate that can decline runs **before** any `lower_value`.
  No arm lowers a value and then declines — that would emit dead code and leak a
  stack slot. Phase 2's seam must keep gate-then-lower strictly separated.
- **O-order-2.** In `set` (list overload) the *index* is lowered and spilled
  **before** the item. In the map overload the *key* before the *value*. Source
  order.
- **O-order-3.** In `state_scalar`, **all** field values are computed (source
  order) and spilled **before** any store, "matching WITH so a field that reads
  another field's old value sees it". This is a semantics requirement, not a
  style choice.
- **O-order-4.** In `state_append`, the STATE pointer is loaded and spilled to
  `inline_state_ptr` **before** the RHS is lowered. Reversing it would let the
  RHS's own lowering observe a stale pointer.
- **O-order-5.** Stack-slot *names* passed to `allocate_stack_object` are
  per-arm strings (`"inplace_append_item"`, `"inplace_recfield_rhs"`,
  `"inline_state_rhs"`, …). They are debug labels, but the *number and order* of
  allocations is not — a seam that allocates one extra slot shifts every later
  frame offset.

---

## DEFECT FOUND — the STATE arm's G11 was never widened (plan-121-A Phase 2b)

Writing this inventory surfaced a live bug, not just an asymmetry.

`static_item_type` is a strict superset of `static_type_name`: it falls back to
the callee's declared `returns` when the hand-written builtin table misses
(`builder_value_semantics.rs:1067-1080`). It exists *only* because the narrow
helper made `list = append(list, someFunc(x))` fall off the in-place path and run
O(n²) — `tests/codegen_inplace_append_call_result.rs` records the measurement:
**3 ms vs 60 243 ms for 50 000 appends, a 20 000× cliff.**

That fix was applied to five gate sites. It missed the sixth:

```
grep -n "static_item_type\|static_type_name" \
  src/codegen/collection/assign/builder_inplace_assign.rs \
  src/codegen/engine/control/builder_control.rs
# builder_inplace_assign.rs:69,176,287,363,430  → static_item_type   (5 sites)
# builder_control.rs:281                        → static_type_name   (1 site)
```

`builder_control.rs:281` is `try_inplace_state_collection_append`'s G11. So

```basic
f.state.raw = collections::append(f.state.raw, someFunc(x))
```

still declines the in-place grow and rebuilds the whole STATE block per element —
the exact O(n²) the widening was written to delete — while the *record* form of
the identical program is fast. The precedent test's own doc comment claims the
fix covers "`try_inplace_append_assign` and its set/map/record-field siblings";
the STATE sibling is absent from that list and from the test file, which has
three cases, all plain-local (`grep "^fn " tests/codegen_inplace_append_call_result.rs`).

**Byte-identity impact of fixing it: none on the committed golden set.** No
fixture exercises the shape — every state-field append in `tests/` passes a bare
local or a builtin call:

```
grep -rn "state\.[a-zA-Z_]* = .*append(" tests/
# 7 hits: bug424_state_accum_inplace (a local `chunk`),
#         rt_res_state_inplace_mutation.rs (a local `chunk`),
#         rt_macos_d4_union_state_tls.rs ×5 (toByte(...), an rt test, not a golden)
```

**Scheduling.** Widening G11 is a *behavior* change to which programs take the
fast path, so it may not ride inside plan-121-A Phase 2, whose acceptance is
byte-identity — mixing them would make a clean gate unreadable. It is landed as
**Phase 2b**, immediately after Phase 2's gate passes, with a RED-first
codegen-inspection test. Recorded in plan-121-A Corrections.

---

## The `removeAt` asymmetry (forward reference, plan-121-B §3)

Not a condition of any arm today, because no `removeAt` arm exists. Recorded here
because Phase 3 of plan-121-A builds `removeAt` against this inventory:

`append` may proceed under a *record-field* `FOR EACH` in some situations because
it writes only **beyond** the count snapshotted at loop entry. `removeAt` shifts
entries **below** the snapshot, which a live iterator can observe. **`removeAt`
must therefore decline on G7/G15/G16 unconditionally**, and may not reuse
`append`'s reasoning. Same for `insert` (shifts up from an index *inside* the
snapshot) and Set `remove` (compaction shift — which is exactly why the existing
`removeKey` arm gates G7).
