# bug-257: `thread::transfer` admits a STATE type disagreement — cross-thread type confusion

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (memory safety)

Status: Fixed (plan-54)
Regression Test: `tests/syntax/threads/thread-transfer-state-mismatch` (the repro
below, now a `TYPE_STATE_MISMATCH` compile error) + `tests/rt-behavior/threads/
thread-transfer-state-rt` (an agreeing stateful transfer runs, plane STATE checked)

## Resolution

Fixed by **plan-54** (`planning/old-plans/plan-54-thread-plane-state.md`): the thread
plane type carries the resource's `STATE` on its `RES` element (`Thread OF RES File
STATE Cursor TO Out`). Both findings are closed:

1. **Type confusion (this bug's first half).** `thread::transfer` requires the moved
   resource's `STATE` to equal the plane's declared `STATE`; a stateful resource on a
   bare plane (the repro), a bare resource on a stateful plane, or two disagreeing
   states are each rejected with `TYPE_STATE_MISMATCH` (ir::verify, the sole rejecter).
   `thread::accept` returns the plane's stateful type, so the receiver cannot invent a
   different one. The repro below now fails to compile instead of dying at runtime.
2. **The cross-arena STATE pointer (the second finding, below).** The transfer now
   **deep-copies** the STATE record into the receiving thread's arena
   (`copy_resource_to_current_arena`, plan-54 §5) rather than aliasing the sender's
   pointer, so the accepted handle owns an independent payload with no cross-thread
   lifetime coupling.

`thread::transfer` moves a resource's `STATE` **pointer** to the receiver without
consulting either side's state type. The sender may attach a
`Cursor{pos AS Integer}`; the receiving `ISOLATED FUNC` may bind the accepted
resource as `STATE Label{name AS String}` and read that same untagged block back
as a `Label`. It builds clean, from safe source, with no `LINK`.

This is plan-52's type confusion (`planning/res.md` §2 fact #5) reached through the
**resource plane** instead of a parameter — and **plan-52-C and plan-52-D do not
close it**, by construction. See "Why plan-52 does not cover this" below.

The correct behavior a fix produces: **a resource's STATE type is fixed at its
owning binding, and a thread that accepts it cannot declare a different one** —
the same guarantee plan-52-C gives at a parameter, extended across the thread
boundary.

References:

- `planning/plan-52-C-state-param-agreement.md` §Open Decisions — flagged this as
  unresolved and asked for the audit; this bug is that audit's verdict.
- `planning/plan-52-A-state-model-and-fixtures.md` Phase 3 — the audit item.
- `./mfb spec language resource-management` §15.5 — the model this violates.
- `./mfb spec language threads` — the transfer/accept resource plane.
- `src/target/shared/code/builder_arena_transfer.rs` —
  `copy_resource_to_current_arena`.

## Failing Reproduction

A worker package declaring a state type that disagrees with the sender's:

```basic
' package xstate_workers
TYPE Label
  name AS String
END TYPE

EXPORT ISOLATED FUNC takeLabel(t AS ThreadWorker OF RES File TO String, seed AS String) AS String
  RES f AS File STATE Label = thread::accept(t, 1000)
  RETURN f.state.name
END FUNC
```

```basic
' consumer
TYPE Cursor
  pos AS Integer
END TYPE

FUNC main AS Integer
  LET t AS Thread OF RES File TO String = thread::start(xstate_workers::takeLabel, "seed")
  RES f AS File STATE Cursor = fs::openFile("src/main.mfb")
  f.state.pos = 99                    ' an Integer 99 in the payload
  thread::transfer(t, f)
  LET got AS String = thread::waitFor(t)
  io::print("worker read name=[" & got & "]")
  RETURN 0
END FUNC
```

Observed (macOS aarch64, green tree, fresh build):

```
Wrote executable to /tmp/xstate/app/build/xstate_app.out
Error: 7-701-0001
Allocation failed.
```

Builds clean; the worker reads the integer `99` as a `String` header and the
program dies with a bogus `Allocation failed`. (The bogus-allocation-failure
signature is the same tell as plan-52-A row 5.)

## Root Cause

`copy_resource_to_current_arena`
(`src/target/shared/code/builder_arena_transfer.rs`) allocates a fresh 80-byte
record in the **receiver's** arena and copies three words across:

```rust
// fd (0), closed flag (8), and the STATE pointer (FILE_OFFSET_STATE)
self.emit(abi::load_u64(&scratch10, &scratch9, FILE_OFFSET_STATE));
self.emit(abi::store_u64(&scratch10, abi::RET[1], FILE_OFFSET_STATE));
```

The STATE pointer is copied verbatim. Neither the sender's nor the receiver's
type string is consulted — nothing in the transfer path knows what the payload
*is*.

On the receiving side, `RES f AS File STATE Label = thread::accept(t, 1000)` runs
the ordinary bind path, which calls `emit_resource_state_init`. That helper
allocates **iff the slot is null**:

```rust
self.emit(abi::load_u64(&current, &ptr, FILE_OFFSET_STATE));
self.emit(abi::compare_immediate(&current, "0"));
self.emit(abi::branch_ne(&done));          // already has a state -> keep it
```

The transferred record's slot is **non-null** (the sender's pointer), so init is
skipped and the binding silently **adopts** the sender's payload — re-typed as
whatever the receiver declared. The payload carries no runtime type tag, so
nothing can detect the disagreement.

## Why plan-52 does not cover this

plan-52-C rejects an argument→parameter STATE disagreement; plan-52-D rejects
binding a *statically stateful* value to a bare binding. Both are static checks
over type strings. This path defeats both:

`thread::accept(t, 1000)`'s static return type is a **bare `File`** — the plane's
element type is `RES File`, which carries no STATE. So to plan-52-D's binding
rule, `RES f AS File STATE Label = thread::accept(...)` reads as *stateless init →
`STATE T` binding*, which is the **one true attach point** and legal. The STATE
arrives dynamically, in a pointer the type system never sees.

So this is not an omission in C/D — it is a hole in a different surface, and it
survives plan-52 in full.

## Fix Sketch (not implemented)

The disagreement is only checkable if the plane's type carries the state, because
sender and receiver are in different packages and never see each other's
declarations. Options, roughly in order of preference:

1. **Carry the STATE on the plane type.** `Thread OF RES File STATE Cursor TO
   Integer`, and require the accepting binding to agree. This is a language
   surface change (grammar + `.mfp` signature + `mfb spec language threads`), so
   it belongs in its own plan, not in plan-52.
2. **Forbid a STATE on an accepted binding** and require the worker to treat the
   resource as opaque (the bare-parameter reading). Cheap and sound, but it
   deletes a working, tested feature (`thread-transfer-state-rt` transfers a
   `Cursor` and reads `pos` on the far side).
3. **Zero the STATE pointer on transfer**, forcing the receiver to attach a fresh
   payload. Sound and cheap, but it silently drops state that
   `thread-transfer-state-rt` asserts survives — a behavior change to a passing
   test, and it leaks the sender's payload.

(1) is the only option that keeps the feature and closes the hole.

## Notes

`thread-transfer-state-rt` passes because its worker happens to declare a
structurally identical `Cursor` in its own package. Agreement there is a
coincidence of authorship, not a checked property — which is exactly why the hole
went unnoticed.

There is a second, independent issue visible at the same site, **not** covered
here: the copied STATE pointer still points into the **sender's** arena (only the
80-byte record is re-allocated in the receiver's). Arenas are per-thread, so the
receiver holds a cross-arena pointer whose lifetime is the sender thread's. This
also constrains plan-52-B: the sender's drop must not free a transferred STATE
payload (plan-52-B's `moved` bit must suppress the free, which is what that
sub-plan's Open Decision calls its sharpest edge).
