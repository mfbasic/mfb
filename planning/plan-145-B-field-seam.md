# plan-145-B: One seam for fields

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-145-A

Prerequisites: see plan-145-A (they gate every letter).

Today a field self-update has its own dispatch, separate from plan-142's seam:

- a record `WITH` runs the 8-link `try_inplace_record_field_*` chain
  (`builder_control.rs:1255`);
- a `STATE` write runs `try_inplace_state_collection_assign`'s 8 arms (`:351`).

Both chains are hand-written, and together they hold 17 arms that duplicate the
plain-local ones op for op (findings Appendix B.1, B.6: "the same after the
container"). The plan-142 seam is entered by a local record's `WITH`, but it
declines at `G2` because the value is a `WithUpdate` (B.2). A `STATE` write never
reaches the seam.

After B, both field forms build a `SelfUpdateSite` and call
`try_inplace_self_update`. The 17 arms are gone: their bodies become the
`Inlined` branch of the seam arm for the same op. Codegen does not change.

## 1. Goal

- `NirOp::Assign` with `r = WITH r { f := v }` and `NirOp::StateAssign` with a
  single-field `WITH` over `h.state` each call `try_inplace_self_update` with a
  field site. No `try_inplace_record_field_*` or Layer 2 `try_inplace_state_*`
  function exists: `grep -c 'fn try_inplace_record_field_\|fn try_inplace_state_collection\|fn try_inplace_state_\(remove\|set\|insert\|prepend\|splice\)' src -r`
  → 0.
- The matrix fires `Append BulkAppend Set SetAdd RemoveKey RemoveAt Insert Prepend
  SetRemove` at S4, T2, T4 and T8. Their `FIELD_PENDING` entries for those sites
  are removed.
- Every committed golden is byte-identical.

### Non-goals

- No new in-place case. An arm that was not field-capable before B declines at a
  field site. That covers all 16 plan-142 arms and `Concat`.
- Layer 1 (`try_inplace_state_scalar_assign`) is unchanged here. Letter C
  generalizes it.
- No change to `G14`, `G15`, `G16`, `G17` or `G25` semantics.

## 2. Current State

(Read 2026-09-21.)

- `SelfUpdateSite { name, type_, dest, by_ref }` (`self_update.rs:95`).
  `is_self` matches only `Local(name)` or `Global(name)`, and `read_by` walks
  for that name (`:108-146`).
- `resolve_self_update` (`inplace_dest.rs:237`) runs `G2`–`G7`/`G10` and the
  plan-142-H `G-global-operand`. It matches the call through
  `self_update_builtin`. The field containers instead use
  `inplace_call_args` (`:435`), which matches through
  `native_builtin_target`, so they never see the `#collections_X$T` spelling.
  No field arm serves a `Body::Mfb` member today.
- `resolve_inplace_record_field` (`:310`) runs, in order: `G2` `WithUpdate`,
  `G13`, `G14`, `G17`, {`G1`, `G15`, `G10`}, `G2`/`G3`/`G4`.
  `resolve_inplace_state_field` (`:371`) runs `G13` on `h.state`, `G14`, `G16`,
  `G17`, `G10`, `G2`/`G3`/`G4`, `G25` (findings B.6).
- The STATE destination emits when it opens: `open_inplace_state_dest` (`:459`)
  loads the `STATE` pointer. It must run after every gate (`O-order-1`) and
  before the operand is lowered (`O-order-4`).
- `try_inplace_self_update` opens `Ref`/`Global` destinations **before** the arms
  run (`self_update.rs:243`). That is sound for them only because
  `is_self_update_call` pre-filters. A `STATE` open cannot move there, because the
  STATE arms open after their op-specific gates today.

## 3. Design

**The site.** `SelfUpdateSite` gains `field: Option<FieldSite>`:

```rust
pub(crate) struct FieldSite<'a> {
    pub(crate) container: FieldContainer<'a>, // Record { local } | State { resource }
    pub(crate) field: &'a str,
    pub(crate) field_index: usize,
    pub(crate) record_type: ParameterType,
}
```

The two properties arms already consult change to match:

- `site.type_` is the **field's** type;
- `is_self(v)` matches `MemberAccess(Local r, f)` or
  `MemberAccess(MemberAccess(Local h, "state"), f)`, which is exactly `G18`
  (`value_is_record_field` / `value_is_state_field`). `read_by(v)` answers
  "reads the owner at all" (`r`, or `h`'s `.state`). That is conservative, and it
  is exactly the self-alias test `G12` makes today.

**Where the WITH is unwrapped.** The site builder in `NirOp::Assign` and in
`NirOp::StateAssign` does not pass the arm the `WithUpdate`. It checks three
things first:

- `G2` the value is a `WithUpdate`;
- `G13` its target is the owner;
- `G14` it has exactly one update.

If they hold, it builds the site from that update's field and passes the
update's **value** to `try_inplace_self_update`. The arms then see
`f(r.f, …)`, the same shape as `f(x, …)`. `G14` stays a site gate, as the
findings' §3.5 note on the seam requires. Letter C relaxes it deliberately.

**The destination.**

- A record field is `InPlaceDest::Inlined { block_slot: <the local's slot>,
  field_index, write_back: None }`. That is exactly what the record arms build
  today.
- A `STATE` field is a new, **unopened** `InPlaceDest::StateField { resource,
  field_index }`. `resolve_self_update` returns it unopened. The arm opens it with
  a new `open_inplace_dest(&dest) -> InPlaceDest` at the exact point where the
  STATE arm called `open_inplace_state_dest` today. That keeps emission order, and
  so bytes, identical. `close_inplace_dest` publishes it as now.

**Container gates in `resolve_self_update`.** It branches on `site.field`:

- a record field runs `G15` and `G17`;
- a `STATE` field runs `G16`, `G17` and `G25`;
- a plain site runs `G7`.

`G1` is unchanged. For a field site the call is matched through
`self_update_builtin`. That widens what the old `inplace_call_args` accepted: a
monomorphized `Body::Mfb` spelling now matches. None of the 9 field-capable ops is
a `Body::Mfb` member (they are all `abi_inline`/`Intrinsic`), so this changes no
codegen. Phase 1 verifies that claim.

**Field capability.** Each `SELF_UPDATE_ARMS` entry gains a
`fields: FieldReach`. In B the value is `None` or `Existing`. D and E replace it
with a reallocation class. An arm with `None` declines at a field site, which
keeps the 16 plan-142 arms and `Concat` out of fields until their letter.

**Folding the 17 arms.** Each record/STATE arm's body after its container is
moved, unchanged, into the seam arm of the same op as that arm's `Inlined`
branch:

- `append`/`bulk_append` → `lower_inplace_inlined_list_grow`;
- `add` and Map `set` → the `InlineGrow` route;
- `removeKey`, `removeAt`, Set `remove` and fixed-width List `set` → the
  sub-block route;
- `insert`/`prepend` → the splice body, unified for both containers.

The two container matchers and `inplace_call_args` are then deleted. So are the
8-link chain, `try_inplace_state_collection_assign`, and the 17 functions.

**Byte-identity.** This is code motion, so the gate is byte-identity of every
committed golden. The risk is emission **order**: the plain-local arm and the
field arm must allocate registers and stack slots in the same sequence the old
field arm did. A diff means a refactor bug. Objdump one fixture, fix it, and
re-run. The plan's design is not in question.

Risk: the `STATE` open point (`O-order-4`) and the gate order. Between them they
decide the bytes and the soundness of every STATE arm.

Rejected:

- **Keep the field chains and add plan-142 arms to them one by one:** 16 more
  duplicated arms × 2 containers. That is the drift plan-121-A and plan-142-A
  removed.
- **Open the STATE destination eagerly in `try_inplace_self_update`**, as `Ref`
  and `Global` are opened. It would emit the STATE load for statements every arm
  declines, which is not byte-identical and wastes the load.

## Phases

### Phase 1: Verify the spellings

- [ ] Confirm that none of the 9 field-capable ops reaches codegen as a
      `#collections_X$T` monomorph. Build one probe calling each on `r.b`, run
      `--nir`, and `grep -oE '"target": ?"[^"]*"'`. Store the output here.

Acceptance: output recorded; every target is `collections.<op>` (est. 5 min).
Commit:

### Phase 2: Field sites through the seam

- [ ] `FieldSite`, `FieldContainer`, `InPlaceDest::StateField` and
      `open_inplace_dest`, in `self_update.rs`/`inplace_dest.rs`.
- [ ] `resolve_self_update`: the container-gate branch above.
- [ ] `NirOp::Assign` and `NirOp::StateAssign`: build the field site and call
      the seam (after Layer 1, for `StateAssign`).
- [ ] Move the 17 bodies into the seam arms, and give each entry a `FieldReach`.
      Delete the chains, both container matchers, `inplace_call_args` and the 17
      functions.
- [ ] Matrix: remove the `FIELD_PENDING` entries for the 9 ops at S4/T2/T4/T8.

Acceptance: codegen is unchanged. `cargo build --release && cargo test --test golden`
→ `0 diff(s)` (est. 20 min: the artifact gate is the only check that sees every
arm's emission at every committed fixture). Then
`cargo test --bin mfb self_update && cargo test --test codegen_inplace_record_field --test rt_res_state_inplace_mutation`
→ pass (est. 10 min).
Commit:

## Validation Plan

- Byte-identity: the artifact gate (Phase 2).
- Behavior: `rt_inplace_self_update` with `MFB_SELF_UPDATE_SITES=S4,T2,T4,T8`.
  No line changes status (est. 8 min: 79 lines × 4 sites at 2.7 s).
- Per-letter unit gate: `cargo test --bin mfb`.

## Corrections

## Summary

B turns the three dispatches into one. It is the enabler for C through H: after
it, a field-capable arm is automatically an arm at every field site. It is
byte-neutral, and its risk is emission order.
