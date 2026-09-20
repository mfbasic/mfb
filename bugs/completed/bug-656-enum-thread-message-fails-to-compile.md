# bug-656: an `ENUM` thread message type does not compile — `native inlined field size not available`

Last updated: 2026-09-19
Effort: medium (1h–2h) — **actual: small**
Severity: MEDIUM
Class: Correctness (codegen)

Status: Fixed
Regression Test: tests/rt-behavior/threads/thread-enum-message-rt

## STATUS: FIXED (5b7499d31)

**The build-it branch, not the reject-it branch** — Phase 1's open question is
settled by two independent statements of the same rule, so no judgement call was
needed:

- `mfb spec language threads` §35: *"Primitive owned values, `String`, `Nothing`,
  records, unions, and immutable containers are sendable…"*
- `thread_unsendable_cause`'s own doc comment (`src/ir/verify/resources.rs`), which
  enumerates the classification and ends: *"…a record by every field, a union by
  every variant…; **enums yes**."*

So the frontend was right to accept the program and codegen was wrong to fail it.

**The fix:** `self.is_enum_type` joins `emit_thread_copy_real`'s scalar arm. An
enum's value is the member ordinal, in the slot, with no block anywhere — but it is
a `ParameterType::Named` nominal, so it fell PAST that arm into the
memcpy-copyable one below. `flatness_of_model_type` answers "flat" for a name it
does not recognise as a record, union or resource, which routed the enum to
`copy_flat_block`, which asked `emit_inlined_block_size_from_ptr_slot` for the block
size of a value that has no block. That is the reported error.

Same shape as the two accidents `flatness_walk`'s own comment already records
(bug-546's user-declared `RESOURCE`, bug-479's `ThreadHandle`): a type nobody had
classified taking the default, and the default being wrong for it.

**Proven at runtime, not by the build succeeding.** The fixture round-trips a
`Colour` through a worker in BOTH directions — `thread.send` parent→worker and
`thread.emit` worker→parent are separate copy sites — and asserts the ordinal that
comes back: Red→Green, Green→Blue, Blue→Red, `total=6`. A by-reference or truncated
copy would still have built. It also reports `live_bytes 0` with
`free_calls == alloc_calls` (38/38) and `double_free_skips 0`, confirming the
by-value path allocates nothing.

**No golden moved** (artifact-gate all, 2050 goldens). That is the expected result
and is itself informative: every program that reached the enum path previously
FAILED TO BUILD, so no committed fixture could have been taking it.

A thread whose message type is an `ENUM` is accepted by the frontend and then fails in
code generation with an internal error:

```
error: native inlined field size not available for type 'Colour' while lowering bind m AS Colour
```

Same class as bug-630 (`thread::waitFor(thread::start(…))`, "native inlined field size not
available for type 'Out'"): a valid program reaches codegen and dies there, with a message
written for a compiler contributor rather than a diagnostic written for the developer.

**The single correct behavior a fix produces:** a `Thread OF <Enum> TO …` builds and
round-trips an enum message, like every other sendable message type — or, if an enum is
deliberately not sendable, the frontend rejects it at the `thread::start` with
`TYPE_THREAD_BOUNDARY_NOT_SENDABLE` (`2-203-0063`), which already exists for exactly this.

References:

- Found by bug-650 Phase 1's `size_computable` enumeration sweep, which compiled one
  `thread::send` per message type. Every other type in that sweep built.
- bug-630 — the sibling shape, same "native inlined field size not available" failure.
- `src/rules/table.rs:703` — `TYPE_THREAD_BOUNDARY_NOT_SENDABLE`, the diagnostic that
  exists for the reject-it branch.

## Failing Reproduction

```
IMPORT io
IMPORT thread

ENUM Colour
  Red
  Green
END ENUM

ISOLATED FUNC wEnum(w AS ThreadWorker OF Colour TO Integer, n AS Integer) AS Integer
  LET m AS Colour = thread::receive(w, 20000)
  RETURN 1
END FUNC

SUB main()
  LET t AS Thread OF Colour TO Integer = thread::start(wEnum, 0, 4, 4)
  thread::send(t, Colour.Red)
  io::print("got=" & toString(thread::waitFor(t)))
END SUB
```

`mfb build --debug` at `3d49a969e`, macOS aarch64:

- Observed: `error: native inlined field size not available for type 'Colour' while lowering
  bind m AS Colour`, exit non-zero, no executable.
- Expected: it builds and prints `got=1` (or the frontend rejects the boundary type).

Verified on a clean main-tip compiler, not just the instrumented probe build.

## Root Cause

Localized: `emit_thread_copy_real` (`src/codegen/memory/arena/builder_arena_transfer.rs`)
dispatches the boundary copy, and its first arm — the by-value one — lists the scalar
`ParameterType` variants plus `Scalar` by name. An enum is none of those spellings; it is
a `Named` nominal. So it fell to the next arm, `type_is_memcpy_copyable`, which answers
TRUE for it: `flatness_of_model_type`'s final `else` returns `!record_field_is_pointer`,
and a name the model does not hold as a record, union or resource is not a pointer — so an
enum is reported "flat". `copy_flat_block` then asks
`emit_inlined_block_size_from_ptr_slot` for its block size, which has no arm for it and
returns the `Err` that surfaced as the internal error.

The failure is reported at the RECEIVE side's bind (`bind m AS Colour`) simply because
that is the first copy site reached; the send side has the same shape.

## Goal

- The reproduction builds and runs, or is rejected with a located developer diagnostic.
- No other message type's lowering changes.

### Non-goals (must NOT change)

- The sendability of the types that already work (String, record, List, Map, Set, data
  union, `AttributedString`, nested collections — all measured working in the same sweep).

## Phases

### Phase 1 — failing test + decide the branch

- [x] Fixture under `tests/rt-behavior/threads/`, confirmed RED for the documented
      reason. Branch decided: BUILD it — the spec and the verifier's own classifier both
      say an enum is sendable, so the frontend was right and codegen was wrong.

Commit: db95242fb

### Phase 2 — the fix

- [x] `is_enum_type` joins `emit_thread_copy_real`'s by-value arm.

Commit: 5b7499d31

### Phase 3 — full validation

- [x] Full suite; artifact gate (0 diffs, as expected — no committed fixture could
      have been on the enum path, since every program that reached it failed to build).

Commit: (see the merge commit)

## Summary

An enum message type passes the frontend's sendability check — correctly — and then has no
answer at the boundary copy, because the by-value arm lists spellings and an enum is a
nominal.
