# bug-702: operations on a moved handle report `ErrResourceClosed`; the spec says `ErrResourceMoved`

## STATUS: FIXED (1be50389c)

Code fixed, spec kept (Phase 1: the case is reachable through a `RES`
parameter). No deviation from the doc's fix design; the tls `close` members,
which the doc did not list, got the split too.

Last updated: 2026-09-24
Effort: medium (1h–2h)
Severity: LOW
Class: Correctness (spec/code disagreement)

Status: Fixed (see STATUS)
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

- [x] Build the reproduction above; record the compiler's answer here.

**Answer: reachable.** The compiler accepts the transfer of a `RES` parameter,
and the caller's binding stays usable. Measured 2026-09-26 at 0cd2db799
(`/tmp/lane702/r1`, the reproduction above with an `ISOLATED FUNC` that accepts
the file): `fs::readLine(f)` after `giveAway(f, t)` printed
`readLine code 77030004` (ErrResourceClosed). So the code is fixed, not the spec.
The same probe widened to every guarded family (all eleven `File` members,
`tcp::localAddress`/`accept` on a `Listener`, `udp::setReadTimeout`/`poll`,
`thread::isRunning` on a transferred `Thread`, and `tls::localAddress`/`accept`/
`close` on a `Listener`) reported 77030004 everywhere except `fs::close`.

Regression test: `tests/runtime/rt_moved_handle_guards.rs` (2 tests). RED at the
base compiler (`MFB_TEST_EXE=<base>/target/debug/mfb cargo test --test
rt_moved_handle_guards`: every line 77030004 except `close`), GREEN after.

Commit: see Phase 2 (test and fix land together)

### Phase 2: fix code or spec, per Phase 1

- [x] One shared guard, `resource::closed_or_moved_guard(flag, closed, moved)`:
      `flag == 1` → closed, any other non-zero → moved (bit 1 set). Every guard on a
      handle that can cross a thread uses it: `fs` (`gen_handle::emit_closed_guard`
      and the eight in `gen_read_write.rs`), `tcp` (`gen_io.rs` accept/read/write),
      the shared socket helpers (`os/socket/poll.rs` poll + set-timeout,
      `os/socket/shared.rs` address), `udp` (`gen_io.rs` receive/send), every `tls`
      backend (OpenSSL, Network.framework, Schannel: accept, read, write, poll,
      address, timeouts, close, closeListener — `tls::close` was the one close
      that did not split), and the `Thread` view (`control_block.rs`
      `emit_control_block_of` / `emit_view_closed_exit`).
- [x] Declare `ErrResourceMoved` on each such member (fs descriptors, forced by
      `every_fs_member_declares_exactly_the_errors_its_lowering_raises`; tcp/tls/udp
      descriptors and their pinned tables) and say so in the prose.
- [x] Spec: `memory heap-values` (the flag word), `threading control-block`,
      `threading queue-semantics` (which said the *receiving* thread finds the
      record moved — it is the sender's record), `language resource-management`.

Cross-target: the same probe programs (`/tmp/lane702/r2` — fs, tcp, udp, thread;
`/tmp/lane702/r3` — a `tls::Listener`) cross-built and run on Linux aarch64 glibc
(box 2226, OpenSSL) and Windows 11 (box 2230, Schannel) print `77030009` on every
line, as on macOS (Network.framework). Linux x86_64 boxes 2227/2228 were down.

Not changed, by design: `process::Process`, the audio streams, `canvas::Image`
and `canvas::Font` cannot cross a thread, so their records never carry the moved
bit and keep the single non-zero test. `pollList` has no per-socket flag guard.
The `thread` members declare no errors (`thread/mod.rs:overload`), so there is no
descriptor list to extend there.

Commit: 1be50389c (fix + test); 3ea85fa20 (the 40 native goldens it and bug-701
move: fs/tcp/udp/tls/thread/http/resource-xfer-slots covers, app-mouse-surface).
Gate: `cargo test --no-fail-fast` at 3ea85fa20 — 243 binaries `test result: ok`
(including `artifact_gate_all` and the `mfb` unit tests, 4395 passed), one red
(`codegen_helper_scratch_release`, bug-701's new helper unclassified, fixed in
41c9ed2f1 and re-run green alone).

## Open Decisions

- Code vs. spec — decided by Phase 1's reachability answer.

## Summary

Small either way. The only real question is reachability, which plan-52-B
answered "not constructible" before `RES`-parameter aliasing was known to reach
the runtime guard.
