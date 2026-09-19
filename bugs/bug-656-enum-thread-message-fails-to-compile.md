# bug-656: an `ENUM` thread message type does not compile — `native inlined field size not available`

Last updated: 2026-09-19
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (codegen)

Status: Open
Regression Test: none yet — see Phase 1

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

Not yet localized. The failure is on the RECEIVE side (`bind m AS Colour`), not the send —
`thread::send` of an enum reaches the send emitter fine and takes the scalar path there
(the bug-650 sweep recorded no `unsized-send` line for the enum, because the build died
first at the worker's bind).

The message is `emit_inlined_field_size`'s "not available" arm. An enum is a scalar at
runtime but is a `ParameterType::Named` nominal that `record_fields` does not know, so the
receive-side bind asks for an inlined field size the model cannot answer. Phase 1 should
confirm whether the right answer is "an enum is a scalar, answer 8" or "an enum is not a
sendable boundary type, reject it earlier".

## Goal

- The reproduction builds and runs, or is rejected with a located developer diagnostic.
- No other message type's lowering changes.

### Non-goals (must NOT change)

- The sendability of the types that already work (String, record, List, Map, Set, data
  union, `AttributedString`, nested collections — all measured working in the same sweep).

## Phases

### Phase 1 — failing test + decide the branch

- [ ] Fixture under `tests/rt-behavior/threads/`; confirm RED. Localize the
      "not available" arm and decide build-it vs reject-it against the spec's sendability
      rules (`src/docs/spec/language/16_threads.md`).

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — full validation

Commit: —

## Summary

An enum message type passes the frontend's sendability check and then has no answer at the
receive-side bind.
