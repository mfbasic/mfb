# bug-253: a borrow may re-declare a different `STATE` type on a resource that already carries one, reading the payload at the wrong type

Last updated: 2026-07-16
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: — (Phase 1)

A resource's `STATE` payload is allocated once, by whichever `RES` binding first
observes a null state slot, and typed **by that binding's own type string**. Nothing
checks that a *later* binding or parameter declaring `STATE T2` on the same resource
agrees with the `T1` the payload was actually allocated as. `emit_resource_state_init`
deliberately skips re-initialization when the slot is non-null — correct for
preserving a moved resource's state — so the second declaration silently adopts the
first's block and reads it at the wrong type.

The result is a **type confusion at the `STATE` payload with no diagnostic**: a
`Cursor { pos AS Integer }` written as `42` and then read through a `RES p AS File
STATE Label` parameter is interpreted as `Label { name AS String }`, i.e. the integer
`42` is read as a `String` header. Observed as a bogus `Allocation failed` at
runtime. This is reachable **today**, from safe source, with no `LINK`, no threads,
and no unsafe construct — and it is reachable through the *only* pattern that
currently works for handing a stateful resource across a function boundary (bug-252
forces exactly this shape: callee returns a bare resource, caller attaches the STATE).

The correct behavior a fix produces: **a resource's `STATE` type is fixed at its
declaration site, and any binding, parameter, or return that declares a different
`STATE` type for the same resource is a compile error.** A program must never read a
state payload at a type it was not allocated as.

References:

- `./mfb spec language resource-management` §15 — "The state is owned by the
  resource, default-initializes when the resource is produced, rides through `RES`
  signatures (`RES s AS File STATE FileState`), and is freed when the resource drops
  or is closed." The spec assumes one state type per resource; it never says what
  happens when two declarations disagree, because it does not contemplate it.
- bug-252 (a `FUNC` cannot return a stateful resource) — **the companion bug.** 252's
  workaround (callee returns bare, caller attaches STATE) is the natural way to hit
  this; and 252's fix design must decide caller/callee STATE agreement, which is this
  bug. Fix 253 first, or fix them together — 252 alone widens the hole to returns.
- Found while auditing bug-252's fix design (`bindings/libsnd` `openFile` wrapper).

## Failing Reproduction

Built against `target/debug/mfb` on macOS aarch64, stock `kind: executable` project.
Verified with `rm -rf build` and a fresh binary timestamp — the first run of this was
polluted by a stale `build/statetest.out`, so the clean re-run is the one recorded here.

```basic
IMPORT io
IMPORT fs

TYPE Cursor
  pos AS Integer
END TYPE

TYPE Label
  name AS String
END TYPE

FUNC openPlain(path AS String) AS RES File
  RES f AS File = fs::openFile(path)
  RETURN f
END FUNC

' Borrows the SAME resource, but declares a DIFFERENT STATE type:
SUB readAsLabel(RES p AS File STATE Label)
  io::print("label.name = " & p.state.name)
END SUB

SUB main()
  RES h AS File STATE Cursor = openPlain("src/main.mfb")
  h.state.pos = 42          ' payload allocated as Cursor { pos: Integer }
  readAsLabel(h)            ' same payload re-typed as Label { name: String }
END SUB
```

- Observed: **builds clean, no diagnostic**, then at runtime:
  `Error: 7-701-0001` / `Allocation failed.` — the integer `42` interpreted as a
  `String` header.
- Expected: a compile error. `h` carries `STATE Cursor`; `readAsLabel` demands
  `STATE Label`; the call must be rejected (an argument/param STATE mismatch), in the
  same family as `TYPE_STATE_INVALID` / `TYPE_UNION_STATE_FORBIDDEN`.

The reverse direction (allocate as `Label { name AS String }`, read as
`Cursor { pos AS Integer }`) is the more dangerous shape — it discloses a raw arena
pointer as an `Integer` rather than faulting. **Not yet demonstrated**: the attempt to
build it hit an unrelated `native code data relocation target '_mfb_str_empty' is not
a data object or defined symbol` build error (the tree is mid-plan-50 and RED), and
the run that appeared to succeed was a stale binary. Phase 1 must build this variant
on a green tree and record the actual result before this doc claims a disclosure
primitive. The `Cursor`→`Label` direction above is confirmed and is sufficient to
establish the type confusion.

### Contrast cases that work correctly today (regression guards)

- **Consistent STATE type across binding and param** → correct, and the intended
  behavior: `tests/rt-behavior/resources/resource-state-field-assign-valid` borrows a
  `RES s AS File STATE Cursor` through a `SUB seek(RES s AS File STATE Cursor, ...)`
  and observes the owner's update after the call. The state must survive the call —
  any fix that re-initializes per binding breaks this.
- **A stateless resource newly bound with a STATE** → correct: the slot is null, so
  the payload is allocated at the binding's type. This is the allocate-once path and
  must keep working.
- **`STATE` on a resource union** → already rejected on a binding
  (`TYPE_UNION_STATE_FORBIDDEN`), so a union cannot be used to launder a state type.
- **Non-defaultable STATE type** → already rejected on a binding
  (`TYPE_STATE_INVALID`).

| Environment | Details | Result |
| --- | --- | --- |
| macOS aarch64 | `target/debug/mfb`, console executable, clean `build/` | fails ✗ (builds, then bogus `Allocation failed`) |

Platform-independent by inspection: the missing check is in the target-neutral front
end; every backend faithfully emits the confused read.

## Root Cause

`STATE` rides inside the type string (`"File STATE Cursor"`) and is recovered by
`crate::builtins::resource::state_type_name` (`src/builtins/resource.rs:231-233`).
The payload itself is a pointer in the resource record at `FILE_OFFSET_STATE` (= 16,
`src/target/shared/code/error_constants.rs:652`) — **it carries no type tag at
runtime.** The type is supplied entirely by whichever type string the reader happens
to hold.

Allocation is guarded by a null-check, not a type-check.
`emit_resource_state_init` (`src/target/shared/code/builder_value_semantics.rs:10-36`)
loads the slot, compares against 0, and branches past the init when it is non-null:

```rust
self.emit(abi::load_u64(&current, &ptr, FILE_OFFSET_STATE));
let done = self.label("resource_state_init_done");
self.emit(abi::compare_immediate(&current, "0"));
self.emit(abi::branch_ne(&done));          // already has a state -> keep it
```

Its call site (`src/target/shared/code/builder_control.rs:289-297`) states the intent:
"The owning binding allocates the state record on first bind; a moved/returned
resource that already carries a state keeps it (the slot is non-null)." That is
**correct and necessary** — it is how a state survives a move. But "keep it" is
unconditional: it keeps the block regardless of whether the current binding's declared
state type is the one the block was allocated as.

The read side then types the load purely from the string.
`src/target/shared/code/builder_value_semantics.rs:175-190` loads the pointer at
`FILE_OFFSET_STATE` and returns it as `ValueResult { type_: state_type, ... }` where
`state_type` came from `state_type_name(&target_value.type_)` — the *reader's* type
string. `src/ir/lower.rs:2188-2195` does the same for `.state` member typing. Neither
consults what was allocated.

The front end never closes the gap. The `STATE` verify rules
(`src/ir/verify/mod.rs:825-845`) check a declared state type in isolation — is it
defaultable, is the base a union — but **there is no rule that relates two
declarations of the same resource's STATE to each other.** Argument/parameter
compatibility likewise compares the resource type; a param typed `File STATE Label`
accepting an argument typed `File STATE Cursor` is not flagged.

Why the contrast cases are immune: they all declare **one** state type per resource,
so the string the writer used and the string the reader uses are identical, and the
untagged payload is read at the type it was allocated as. The bug needs two
disagreeing declarations of the same resource — which nothing forbids.

## Goal

- The reproduction is rejected at compile time with a STATE-mismatch diagnostic
  naming both types.
- `tests/rt-behavior/resources/resource-state-field-assign-valid` and the four sibling
  STATE fixtures still pass unchanged — a consistent STATE type across binding and
  borrow must keep working, including the owner observing a borrow's update.
- No program can read a `STATE` payload at a type other than the one it was allocated
  as.

### Non-goals (must NOT change)

- **`emit_resource_state_init`'s null-check.** "Allocate once; a carried state
  survives a move" is required — bug-252's fix depends on it. The fix belongs in the
  front end (reject the disagreement), not in codegen (do not re-initialize, and do
  not make init type-aware at runtime).
- **The resource record layout / `FILE_OFFSET_STATE`.** Adding a runtime type tag to
  the payload is a plausible *alternative* design but is an ABI change; see Fix Design.
- **The `" STATE "` type-string encoding.** Same rationale as bug-252 — a structured
  representation is a separate plan.
- **`STATE` being optional.** A resource with no STATE, borrowed by a param that
  declares one, is the legitimate allocate-on-first-bind path and must keep working.
- **Tempting wrong fix: rewriting the reproduction so the two types agree.** The
  program is *accepted today*; making the test consistent removes the demonstration
  without removing the hole. The fix must reject the inconsistent program.
- **Tempting wrong fix: making the mismatch a runtime check.** The payload has no
  type tag to check against, and adding one is an ABI change. This is statically
  decidable at the call/bind site — reject it there.

## Blast Radius

Found by grepping `FILE_OFFSET_STATE`, `state_type_name`, and the `STATE` verify
rules tree-wide — not from memory.

- `src/ir/verify/mod.rs:825-845` (the `STATE` rules) — **the gap**: they validate a
  declared state type in isolation and never relate two declarations. The new rule
  belongs here or beside it.
- **Argument→param STATE compatibility** — **the reproduction's vector**; in scope. A
  `RES` argument whose state type differs from the param's declared state type must be
  rejected.
- **Binding STATE vs. initializer STATE** — in scope, same hazard: `RES h AS File
  STATE Other = <something already carrying Cursor>`. Currently unreachable for a
  *call* initializer (bug-252 blocks stateful returns), but reachable via any other
  expression that yields a stateful resource; confirm in Phase 1.
- **Return STATE vs. returned value's STATE** — **latent, blocked by bug-252**, and it
  unblocks the moment 252 lands. 252's Open Decisions explicitly defer here. Whichever
  bug lands second must cover it.
- `src/target/shared/code/builder_value_semantics.rs:10-36` (`emit_resource_state_init`) —
  the mechanism, not the fault. Unchanged by the fix (see Non-goals).
- `src/target/shared/code/builder_value_semantics.rs:175-190` and
  `src/ir/lower.rs:2188-2195` (`.state` read typing) — consumers. They will read the
  right type once the front end guarantees agreement.
- `src/target/shared/code/builder_arena_transfer.rs:336-337` (`thread::transfer` copies
  the state pointer across the resource plane) — **latent, needs a Phase 1 verdict.**
  It moves the pointer without consulting either side's type string. If a worker's
  `ISOLATED FUNC` parameter can declare a different STATE type than the sender's
  binding, this is the same confusion across a thread boundary.
  `tests/rt-behavior/threads/thread-transfer-state-rt` covers only the consistent case.
- `src/target/shared/code/builder_codegen_primitives.rs:1512`
  (`emit_resource_cleanup_call`) — unaffected: it calls the close symbol and never
  touches the payload's type. The free is inside the close helper / arena, so a
  mis-typed state does not corrupt cleanup.
- `fs_helpers_io.rs:792`, `fs_helpers_atomic.rs:201,1772`, `net/mod.rs:275` (resource
  constructors zeroing the state slot) — unaffected and load-bearing: the zero is what
  makes "first bind allocates" well-defined.

## Fix Design

Make the resource's STATE type part of what the front end checks at every site where
two declarations of the same resource meet — argument→param first (the demonstrated
vector), then binding→initializer and return→value.

The natural shape: extend the existing STATE verification in
`src/ir/verify/mod.rs:825-845` with a mismatch rule (e.g. `TYPE_STATE_MISMATCH`) and
apply it wherever a `RES` value with a known state type flows into a declaration with
a different one. The information is already present — both sides' type strings carry
their STATE — so this is a comparison the verifier can make today without new
plumbing.

The correctness risk concentrates in **not over-rejecting the allocate-on-first-bind
path**. `RES p AS File STATE Cursor` accepting an argument typed bare `File` (no state
yet) is legal and must stay legal — that is the null-slot case, and
`resource-state-field-assign-valid`-style code depends on a param declaring the STATE.
So the rule is asymmetric: *stateless → stateful* is fine (allocate), *stateful T1 →
stateful T2 where T1 ≠ T2* is an error, *stateful T1 → bare* is the bug-252 Open
Decision (recommend allowing; the payload is still freed by the close op).

Rejected alternatives:

- **Runtime type tag on the payload.** Store a type id beside the state block and check
  it on `.state`. Robust, but it is an ABI/layout change (Non-goals), costs a load and
  a compare on every state access, and turns a statically-decidable error into a
  runtime failure. Only worth revisiting if the static rule proves undecidable for some
  flow.
- **Re-initialize on every binding that declares a STATE.** Trivially "fixes" the type
  confusion by making the second declaration allocate its own block — and silently
  breaks the whole feature: the owner would stop observing a borrow's update
  (`resource-state-field-assign-valid`), and a moved/returned state would be discarded.
  Directly contradicts the spec's "a state update made through a borrowed RES parameter
  is visible to the owner after the call."
- **Documenting it as a footgun.** Neither the resource's producer nor its consumer can
  see the other's state declaration in general; this is not an author-avoidable mistake.

Expected output shift: no existing fixture declares two disagreeing STATE types
(confirmed: all 14 `STATE` uses under `tests/` are consistent), so goldens should not
move. Verify with the artifact gate rather than assuming.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [ ] Add `tests/syntax/resources/resource-state-param-mismatch-invalid/`: the
      reproduction. Confirm it builds clean today (the fixture fails by *not* erroring).
- [ ] Build the reverse direction (`Label`→`Cursor`, String read as Integer) on a green
      tree and record the actual observed result; update the Failing Reproduction with
      it. Do not claim a pointer-disclosure primitive until this is observed.
- [ ] Add a runtime fixture proving the confusion is a real wrong-type read, not just a
      missing diagnostic.
- [ ] Confirm the two flagged audit items: the binding→initializer vector, and whether
      `thread::transfer` (`builder_arena_transfer.rs:336-337`) admits the same mismatch
      across a thread boundary. Write a verdict for each into this file.
- [ ] Re-confirm the allocate-on-first-bind contrast (`stateless → stateful` param)
      so Phase 2 cannot over-reject it.

Acceptance: the new fixtures fail for the documented reasons; every blast-radius entry
has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] Add the STATE-mismatch rule and apply it at argument→param, binding→initializer,
      and (if bug-252 has landed) return→value.
- [ ] Keep *stateless → stateful* accepted (allocate-on-first-bind).
- [ ] Extend to the thread-transfer plane if Phase 1 finds it admits the mismatch.

Acceptance: the reproduction is rejected with the new diagnostic; all five existing
STATE fixtures pass unchanged; nothing in Non-goals moved.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/artifact-gate.sh`; confirm the codegen delta is nil.
- [ ] Regenerate any goldens the new rule shifts; diff and confirm the delta is only that.
- [ ] `scripts/test-accept.sh` green; `cargo test --bin mfb` green.
- [ ] Re-run the reproduction; confirm it no longer builds.

Acceptance: full suite green; golden deltas are exactly the intended change; the
reproduction is a compile error.
Commit: —

## Validation Plan

- Regression tests: `tests/syntax/resources/resource-state-param-mismatch-invalid/`
  (rejection), plus the runtime fixture proving the wrong-type read, plus the
  allocate-on-first-bind contrast.
- Runtime proof: **required for Phase 1's demonstration** — the point of this bug is
  that the build succeeds, so only running the binary shows the payload is read at the
  wrong type (`.ai/compiler.md` runtime completion gate). After the fix the proof
  inverts: the program must not build at all.
- Doc sync: `./mfb spec language resource-management` §15 should state that a
  resource's STATE type is fixed at its declaration site and that a disagreeing
  declaration is rejected — the spec is currently silent, which is why this shipped.
- Full suite: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo test --bin mfb`.

## Open Decisions

- **Is `stateful T1 → bare File` allowed?** Recommend **yes** (a caller that ignores
  the state; the close op still frees the payload). Shared with bug-252's Open
  Decisions — decide once, in whichever lands first.
- **Does `thread::transfer` admit the same mismatch?** Unconfirmed; Phase 1 resolves
  it. If it does, the fix must cover the resource plane, and the severity of the
  thread case should be assessed separately (a cross-thread type confusion).
- **Diagnostic code**: new `TYPE_STATE_MISMATCH` vs. reusing `TYPE_STATE_INVALID`.
  Recommend a new code — the existing one means "not a defaultable data type", a
  different failure the user fixes differently.

## Summary

The `STATE` payload is untagged at runtime and its type comes entirely from whichever
type string the reader holds, while allocation is guarded by a **null-check rather
than a type-check**. Two disagreeing declarations of the same resource's STATE are
therefore accepted, and the second reads the first's block at the wrong type — from
safe source, with no diagnostic. The codegen null-check is correct and must stay (bug-252
depends on it); the missing piece is a front-end rule relating declarations that the
verifier already has both sides' information to make. The real risk is over-rejecting
the legitimate allocate-on-first-bind path that every existing STATE fixture relies on.
