# bug-700: operations on a moved handle report `ErrResourceClosed`; the spec says `ErrResourceMoved`

Last updated: 2026-09-24
Effort: medium (1h–2h)
Severity: LOW
Class: Correctness (spec/code disagreement)

Status: Open
Regression Test: a runtime test transferring a handle through an untracked alias, then using it (Phase 1)

`mfb spec language resource-management` §15 says a moved handle that reaches an
operation is refused with `ErrResourceMoved`:

> "where a handle nonetheless reaches an operation moved (an alias those rules do
> not track), the operation is refused with `ErrResourceMoved` rather than
> `ErrResourceClosed`, because the handle is not closed — it belongs to another
> thread now." (`src/docs/spec/language/15_resource-management.md`, "A
> transferred handle is moved, not closed")

Only the `close` members do that. Every other handle operation tests the flag
word with `CLOSED != 0` — bit 0 closed, bit 1 moved, and a transfer sets both —
and raises `ErrResourceClosed` for either. Found during bug-694 (research on the
`fs` handle operations); the `fs` pages bug-694 wrote describe today's behavior
("`ErrResourceClosed` that `file` has been closed").

**Correct behavior.** Either every guarded operation reports `ErrResourceMoved`
for a moved handle (the spec's claim), or the spec states that only `close`
distinguishes the two. Code and spec agree.

References:

- plan-52-B (`planning/completed/plan-52-B-record-reclamation.md` §(b)): introduced
  the moved bit so "every existing guard" refuses a moved handle "with no new
  code", and made only the close helper report `ErrResourceMoved`. It records
  that a sender-side use after a move is **not constructible from source** under
  the static move rules (`TYPE_USE_AFTER_MOVE`), which is why no runtime test
  exists.
- `src/codegen/builtins/fs/gen_handle.rs:lower_fs_close_helper` — the one split.

## Failing Reproduction

Not yet constructed, which is Phase 1's first question. bug-694 showed a `RES`
parameter is an alias the static rules do not track: closing a `File` through
`SUB closeIt(RES x AS fs::File)` leaves the caller's binding usable, and later
operations on it reach the runtime guard. The candidate reproduction transfers
through the same kind of alias:

```
SUB giveAway(RES x AS fs::File, w AS <thread handle>)
  thread::transfer(w, x)
END SUB
' caller: giveAway(f, w) then fs::readLine(f)
```

- Expected per spec: `ErrResourceMoved` (77030009).
- Predicted today (guess, from the guard shape): `ErrResourceClosed` (77030004).

If the compiler rejects the transfer of a `RES` parameter, the operation is
unreachable and the fix is the spec sentence, not the code.

## Root Cause

`emit_closed_guard` (`fs/gen_handle.rs`, bug-694 C3) and the inline guards in
`fs/gen_read_write.rs`, and the `tcp`/`udp`/`tls`/`process`/`thread` handle
operations, branch on `CLOSED != 0` to a single `ErrResourceClosed` raise. Only
`lower_fs_close_helper` and the other `close` members test bit 1.

## Goal

- The spec and every handle operation agree on what a moved handle raises,
  pinned by a runtime test if the case is constructible, or by the spec's
  wording if it is not.

### Non-goals (must NOT change)

- The flag layout (bit 0 closed, bit 1 moved) and the `close` behavior.

## Blast Radius

- `fs` handle operations (readLine, readAll, readAllBytes, readBytesAt,
  writeAll, writeAllBytes, eof, size, flush, setBuffered, isBuffered).
- `tcp`, `udp`, `tls`, `process`, `thread` handle operations — same guard shape
  (grep `ErrResourceClosed` in `src/codegen/os/socket/` and `builtins/*/`).

## Fix Design

If constructible: split the guard's tail once (a shared "closed or moved" tail)
and use it everywhere, declaring `ErrResourceMoved` on each member (the bug-694
guard enforces the `fs` descriptors). If not: correct the spec sentence to say
only `close` reports it, and why.

## Phases

### Phase 1: can a moved handle reach an operation?

- [ ] Build the reproduction above; record the compiler's answer here.

Commit: —

### Phase 2: fix code or spec, per Phase 1

Commit: —

## Open Decisions

- Code vs. spec — decided by Phase 1's reachability answer.

## Summary

Small either way. The only real question is reachability, which plan-52-B
answered "not constructible" before `RES`-parameter aliasing was known to reach
the runtime guard.
