# plan-52-C: STATE agreement at parameters — close the type confusion

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-52-A (the model + the pending fixtures this flips green)

Closes a live memory-safety hole. Nothing checks that a parameter's declared `STATE` type
agrees with the state the argument actually carries, so a borrow can **re-type** a payload
it should only observe — or **attach** one to a resource that never had it. Both compile
clean today, from safe source, with no `LINK`, no threads, and no unsafe construct.

The single outcome: **a resource's STATE type is fixed at its owning binding, and any
parameter declaring a different one — or demanding one the argument doesn't have — is a
compile error.**

References:

- `plan-52-A` §3 — the model table this implements (the param row). **Read first.**
- `planning/res.md` §2 — the verified behavior table this rests on.
- `./mfb spec language resource-management` §15 — as amended by plan-52-A.

## 1. Goal

- `test2(RES p AS File STATE Cursor)` given an argument carrying `Label` → **rejected**.
- `test2(RES p AS File STATE Cursor)` given a **stateless** argument → **rejected**
  (params observe; they do not attach).
- `test(RES p AS File)` given anything → **accepted**, `.state` inaccessible inside. The
  opt-out survives.
- Reading `.state` through a bare param produces a diagnostic that **names STATE**.
- plan-52-A's rows 4 and 5 flip from failing to passing; rows 1, 2, 3, 10 unchanged.

### Non-goals (explicit constraints)

- **The bare-param opt-out.** `RES p AS File` accepting a stateful argument with `.state`
  inaccessible is the model's param rule (plan-52-A §3), not laxity. Every close op depends
  on it — `FUNC close(RES db AS Db)` must accept the resource whatever state it carries.
  A "fix" that tightens bare params to stateless-only breaks every close op.
- **`emit_resource_state_init`'s null-check** (`src/target/shared/code/builder_value_semantics.rs:10-36`).
  Stays. "Allocate once; a carried state survives a move" is required by plan-52-D. The fix
  is a front-end rejection, **not** a codegen change — do not make init type-aware, and do
  not re-initialize per binding (see §3's rejected alternatives).
- **The binding and return rows** of the model table. Those are plan-52-D — the binding
  rule must land *with* the return rule, never before or after it (plan-52-D §3).
- **Runtime checks.** The payload has no type tag; there is nothing to check against, and
  adding one is a layout change (plan-52-B Non-goals). This is statically decidable —
  reject at the call site.
- **Tempting wrong fix:** rewriting plan-52-A's fixtures so the two STATE types agree. The
  programs are *accepted today*; making them consistent removes the demonstration, not the
  hole.

## 2. Current State

`STATE` rides inside the type string (`"File STATE Cursor"`), split back out by
`crate::builtins::resource::state_type_name` (`src/builtins/resource.rs:231-233`). The
payload itself is a pointer at `FILE_OFFSET_STATE` (= 16) and **carries no runtime type
tag** — its type comes entirely from whichever type string the reader holds.

`src/ir/lower.rs:739-742` carries a param's declared STATE into its local type string
("so `s.state` resolves inside the callee, matching `lower_param`"). Codegen's bind path
then runs `emit_resource_state_init`, which allocates **iff the slot is null**:

```rust
self.emit(abi::load_u64(&current, &ptr, FILE_OFFSET_STATE));
self.emit(abi::compare_immediate(&current, "0"));
self.emit(abi::branch_ne(&done));          // already has a state -> keep it
```

That null-check is correct for the *move* case it was written for ("a moved/returned
resource that already carries a state keeps it"). Nothing scoped it to that case. So a
param **attaches** when the slot is null and **re-types** when it is not.

The read side types the load purely from the string:
`src/target/shared/code/builder_value_semantics.rs:175-190` returns
`ValueResult { type_: state_type, … }` where `state_type` came from the *reader's* type
string. `src/ir/lower.rs:2188-2195` does the same for `.state` member typing. Neither
consults what was allocated.

The STATE verify rules (`src/ir/verify/mod.rs:825-845`) check a declared state type **in
isolation** — is it defaultable (`TYPE_STATE_INVALID`), is the base a union
(`TYPE_UNION_STATE_FORBIDDEN`). **No rule relates two declarations to each other.** That
is the gap.

**Verified today** (plan-52-A rows 4/5): a stateful param on a stateless owner allocates
(`a saw pos = 7`); a `Cursor{pos:Integer}=42` read through a `STATE Label` param is
interpreted as `Label{name:String}` and falls over with a bogus `Allocation failed` — the
integer 42 read as a String header.

**Not verified — do not claim it.** The reverse direction (allocate as
`Label{name:String}`, read back as `Cursor{pos:Integer}`) would disclose a raw arena
pointer as an `Integer` rather than faulting — a **disclosure primitive**, and a more
serious finding than the confusion above. It has **never been demonstrated**. The attempt
hit an unrelated `native code data relocation target '_mfb_str_empty' is not a data object
or defined symbol` build error on the mid-plan-50 tree; the run that appeared to succeed
was a **stale `build/*.out` from the previous test**. Nothing in this plan may describe
this as a disclosure primitive until it is built on a green tree and observed.

> **Trap, generally:** a failed `mfb build` leaves the previous `build/<name>.out` in
> place, and running it looks like a pass. `rm -rf build` and check the binary timestamp.
> This produced one false finding during this plan's research before it was caught.

The confirmed `Cursor`→`Label` direction is sufficient to establish the type confusion and
to justify every rule in §3; the disclosure variant would only raise the severity.

**Precedent to mirror:** the two rules at `:825-845` are the right shape and the right
home; they just need a *relational* sibling.

## 3. Design Overview

Add a `TYPE_STATE_MISMATCH` rule beside the existing STATE rules
(`src/ir/verify/mod.rs:825-845`), applied at the argument→parameter boundary. Both sides'
type strings are already in hand; this is a comparison the verifier can make today with no
new plumbing.

The param row, in full:

| argument | param `STATE T` | param **bare** |
|---|---|---|
| carries `T` | ✓ | ✓ (opt-out) |
| carries `T2 ≠ T` | ✗ `TYPE_STATE_MISMATCH` | ✓ (opt-out) |
| **stateless** | ✗ `TYPE_STATE_MISMATCH` | ✓ |

**The correctness risk is over-rejecting — but not where it first appears.** The obvious
reading is that *"stateless → stateful is fine (allocate)"* should stay legal, and that
rejecting it is the danger. That is backwards: param-attach is precisely what makes two
disagreeing borrows reachable **with no stateful binding anywhere**:

```basic
SUB a(RES p AS File STATE Cursor)   ' first call allocates a Cursor
SUB b(RES p AS File STATE Label)    ' second reads that block as a Label
RES h AS File = fs::openFile(...)   ' stateless owner
a(h)
b(h)                                ' -> confusion, no diagnostic
```

And no in-tree fixture depends on it — `resource-state-field-assign-valid`'s **owner**
declares `RES f AS File STATE Cursor`, so its param never allocates. plan-52-A Phase 3
confirms this by audit. The real risk is the opposite one: breaking the **bare-param
opt-out** (row 1/2/3), which every close op needs.

**Rejected: re-initialize on every binding that declares a STATE.** Trivially kills the
confusion by giving the second declaration its own block — and silently breaks the feature:
the owner would stop observing a borrow's update (`resource-state-field-assign-valid`), and
a moved state would be discarded. Contradicts §15's "a state update made through a borrowed
RES parameter is visible to the owner after the call."

**Rejected: a runtime type tag on the payload.** Robust, but it is a layout change, costs a
load and compare on every `.state`, and turns a statically-decidable error into a runtime
one.

**Rejected: documenting it as a footgun.** Neither the resource's producer nor its consumer
can generally see the other's state declaration. Not author-avoidable.

## 4. The `.state` read diagnostic

Secondary, same area, worth doing here. The **write** path is diagnosed precisely today:

```
error[2-203-0085 TYPE_STATE_INVALID]: `p` has no STATE to assign; declare the resource with `STATE T`.
```

The **read** path is not: `p.state.pos` degrades to `Unknown` and the error surfaces
wherever that Unknown lands — observed as `TYPE_CALL_ARGUMENT_MISMATCH` on `toString`
complaining about argument types, never mentioning STATE. It *is* rejected, so this is a
diagnostic gap, not a hole. Make the two paths say the same thing; flip plan-52-A row 3's
`TODO(plan-52-C)` golden.

## Compatibility / Format Impact

- **No layout, no `.mfp`, no API change.** Front-end rules only.
- **Source compatibility:** programs relying on param-attach or on a re-typed STATE stop
  compiling. Both are the bug. No in-tree fixture does either (plan-52-A Phase 3 audit).
- **Goldens:** plan-52-A rows 4/5 flip; row 3's message changes. No other movement expected
  — verify with the artifact gate rather than assuming.

## Phases

### Phase 1 — the mismatch rule

- [ ] Add `TYPE_STATE_MISMATCH` to `src/rules/table.rs` with a site-specific message
      naming **both** types.
- [ ] Implement the §3 table at the argument→param boundary, beside
      `src/ir/verify/mod.rs:825-845`.
- [ ] Keep bare params accepting anything — the opt-out is a Non-goal to preserve, and it
      is the easiest thing to break here.

Acceptance: plan-52-A rows 4 and 5 are rejected with `TYPE_STATE_MISMATCH`; rows 1, 2, 10
still pass unchanged; `resource-state-field-assign-valid` and its four siblings pass
unchanged.
Commit: —

### Phase 2 — the `.state` read diagnostic

- [ ] Give the read path a STATE-naming error, matching the write path's wording
      (`src/ir/verify/mod.rs` / `src/syntaxcheck/`).
- [ ] Update plan-52-A row 3's golden; drop its `TODO(plan-52-C)`.

Acceptance: `p.state.pos` on a bare param names STATE, not `toString`'s argument types.
Commit: —

### Phase 3 — validation

- [ ] `scripts/artifact-gate.sh`; confirm the codegen delta is nil (front-end rules only).
- [ ] Regenerate the goldens rows 3/4/5 shift; confirm the delta is only those.
- [ ] Re-run the two-disagreeing-borrows runtime fixture: it must now **fail to build**.

Acceptance: full suite green; golden deltas are exactly rows 3/4/5; the runtime confusion
proof is now a compile error.
Commit: —

## Validation Plan

- Tests: plan-52-A's `resource-state-param-mismatch-invalid` and
  `resource-state-param-attach-invalid` (flip to passing); rows 1/2/3/10 as guards; the
  five existing STATE fixtures unchanged.
- Runtime proof: **inverted** — before this sub-plan the proof is that the confusion
  program builds and misreads its payload; after, the proof is that it no longer builds.
  The five existing STATE fixtures still need their runtime pass (`.ai/compiler.md` gate),
  since over-rejection would show up there first.
- Doc sync: `src/docs/spec/language/15_resource-management.md` — the param row is
  plan-52-A's; add `TYPE_STATE_MISMATCH` to the diagnostics tables per
  `.ai/specifications.md`.
- Acceptance: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo test --bin mfb`.

## Open Decisions

- **One code or three?** One `TYPE_STATE_MISMATCH` across param/return/binding with a
  site-specific message, vs. a code per site. Recommend **one** — the user's fix is the
  same shape everywhere. plan-52-D reuses it for the return and binding rows.
- **`thread::transfer`** — still unresolved, audited in plan-52-A
  Phase 3. If an `ISOLATED FUNC` entry can declare a different STATE than the sender's
  binding, the same rule must cover the resource plane
  (`src/target/shared/code/builder_arena_transfer.rs:336-337`), and a cross-thread type
  confusion may warrant its own severity.

## Summary

A relational rule the verifier already has both sides' information to make. The engineering
risk is entirely in **what not to reject**: the bare-param opt-out must survive, or every
close op breaks. The intuitive rule — allow `stateless → stateful` so a param may
attach — is the unsafe one, and the fixture it was protecting turns out not to need it.
Attachment belongs at the owning binding, exactly once; params observe.
