# bug-467: writing to a socket whose peer has closed kills the process with SIGPIPE instead of raising

Last updated: 2026-08-30
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness + Availability (a remote peer can terminate any MFBASIC server)

Status: Open
Regression Test: — (none yet; see Phase 1)

`tcp::write` to a peer that has closed its end does not raise. The first write is
accepted by the local OS, the peer's stack answers with an RST, and the **second
write terminates the whole process with SIGPIPE** — no `TRAP` runs, no scope
drop runs, `main` never returns, and the exit status is `141` (128 + 13). The
program has no way to defend itself: nothing in the runtime installs a SIGPIPE
disposition, sets `SO_NOSIGPIPE`, or passes `MSG_NOSIGNAL`.

`tcp::write`'s own documentation states the opposite — "Writing to a socket whose
peer has already closed **raises** rather than silently discarding the data" — so
a program written to the documented contract (wrap the write in a `TRAP` and
handle the disconnect) is killed at exactly the moment its error handling was
supposed to run.

**Why this is worse than an ordinary wrong-error bug.** A server's peer is
untrusted input. Any client that connects and immediately disconnects can end the
server process, taking every other in-flight connection with it. That is a remote
denial of service reachable with two syscalls, and it needs no malformed data —
just a close at the wrong moment, which is also what an ordinary client crash or
a `curl` interrupted with `^C` produces.

**The single correct behavior a fix produces:** a write to a socket whose peer has
gone away returns an error to the MFBASIC program — `ErrConnectionClosed`, the
same code the read side already raises at end of stream — and never delivers a
signal. `TRAP` sees it, scope drop runs, and the process survives to serve its
other connections.

References:

- `src/codegen/builtins/tcp/func_write.rs` DESC — the contract this violates
  ("raises rather than silently discarding the data").
- `src/codegen/builtins/tcp/gen_io.rs:860` — `platform.emit_write` on the
  non-Windows path: a bare `write(2)` with no `MSG_NOSIGNAL`.
- `src/codegen/builtins/tcp/gen_io.rs:880-905` — `write_fail`, the errno
  classification that would already do the right thing if the syscall were
  allowed to return `EPIPE` instead of the process being killed first.
- bug-465 (`bugs/bug-465-tcp-tls-mirror-divergences.md`) — found while probing
  `tcp`/`tls` doc parity for that bug, which corrected the false `tcp::write`
  sentence to stop promising a raise this code cannot deliver. The raise itself
  is this bug.

## Failing Reproduction

macos-aarch64, `target/release/mfb`.

```
IMPORT net
IMPORT tcp
IMPORT io

FUNC probe AS String
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  RES conn = tcp::accept(server)
  tcp::close(client)                     ' the peer goes away
  MUT n = 0
  FOR i = 1 TO 20
    tcp::write(conn, "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx")
    n = n + 1
    io::print("wrote " & toString(n))
  NEXT
  RETURN "no-raise-after-" & toString(n) & "-writes"
  TRAP(e)
    RETURN "raised-after-" & toString(n) & "-writes: " & e.message
  END TRAP
END FUNC

FUNC main AS Integer
  io::print(probe())
  RETURN 0
END FUNC
```

- Observed:

  ```
  wrote 1
  [exit 141]
  ```

  Confirmed as the signal, not an exit code, under `lldb -b -o run`:
  `Process exited with status = 13 (0x0000000d) Terminated due to signal 13`
  (`kill -l 13` → `PIPE`). The `TRAP` never runs and `probe`'s `RETURN` line
  never prints.

- Expected: `raised-after-1-writes: <ErrConnectionClosed message>` — the write
  fails, the `TRAP` runs, the process exits 0.

Contrast cases, all measured on the same host and build:

- **A single write after the peer closes SUCCEEDS.** Only the second one dies, so
  a test that writes once cannot see this.
- **50 writes of 1 byte each survived** in an earlier probe; 32-byte writes die on
  the second. The trigger is the RST arriving, so it is timing-dependent — which
  makes it *intermittent in production and easy to miss in a test*, not rare.
- **`tcp::read` at the same point is correct**: it raises `ErrConnectionClosed`
  (bug-465, `tests/rt-behavior/tcp/tcp-read-eof-raises-rt`). Only the write
  direction is unprotected.
- **Empty writes are unaffected**: `tcp::write(sock, [])` / `tcp::write(sock, "")`
  return before any syscall (`gen_io.rs`, the `remaining == 0` early exit).

| Environment | Details | Result |
| --- | --- | --- |
| macos-aarch64 | `target/release/mfb`, loopback | fails ✗ (SIGPIPE, exit 141) |
| linux-x86_64 | not yet run — the write path differs (raw `write` syscall) | unknown, expected ✗ |
| linux-riscv64 | not yet run | unknown, expected ✗ |
| windows-x86_64 | `send()`; Win32 has no SIGPIPE | expected ✓ (immune) |

The Linux rows must be measured before the fix is designed, not assumed: CI runs
linux/DEBUG and a local macOS green proves neither.

## Root Cause

POSIX delivers `SIGPIPE` to a process that writes to a socket whose peer has sent
an RST, and the default disposition terminates it. Nothing in the generated
program overrides that:

- **No process-wide disposition.** `grep -rni "sigpipe\|nosignal\|SO_NOSIGPIPE" src/`
  returns nothing. The runtime never calls `signal`/`sigaction` for it.
- **No per-socket opt-out.** Every socket is created through
  `net_symbol(platform, NetSymbol::Socket)` (`os/socket/shared.rs:705`,
  `udp/gen_io.rs:130`, `net/gen_ping.rs:337`) or a direct `"socket"` call
  (`tls/gen_openssl.rs:174,1139`, `tls/gen_schannel_impl.rs:70`,
  `tls/gen_schannel_server.rs:349`). None of them sets `SO_NOSIGPIPE`, the
  macOS/BSD per-socket suppression. `listen` sets `SO_REUSEADDR` and the timeout
  setters set `SO_RCVTIMEO`/`SO_SNDTIMEO`, so the setsockopt machinery is present
  and used — this option is simply absent.
- **No per-call opt-out.** `tcp/gen_io.rs:860` writes with `platform.emit_write`
  (a plain `write(2)`, or on linux-x86_64 a raw syscall) on every non-Windows
  target. `MSG_NOSIGNAL` is a `send(2)` flag, and `write(2)` has no flags
  argument, so this call shape cannot express the suppression at all. Windows
  already uses `send(..., 0)` and is immune for the unrelated reason that Win32
  has no SIGPIPE.

The error path is otherwise correct and would need no change: `write_fail`
(`gen_io.rs:880`) already classifies errno and distinguishes `EAGAIN` (timeout)
from `EINTR` (retry) from a closed connection. It never runs here only because
the signal kills the process before `write` returns `-EPIPE`.

`udp` is immune in practice (a datagram socket has no peer to send an RST), and
macOS `tls` is immune (Network.framework owns the transport and reports send
failures through its completion handler rather than a raw `write`). Linux/Windows
`tls` layer over a plain descriptor: Schannel is immune with Win32, and the
OpenSSL path writes through `SSL_write`, which issues its own `write(2)` inside
libssl and so is exposed by the same mechanism — with the extra wrinkle that a
per-socket `SO_NOSIGPIPE` does not exist on Linux, so libssl's internal write
cannot be fixed from the call site.

## Goal

- `tcp::write` to a socket whose peer has gone away raises `ErrConnectionClosed`
  and the process survives, on macOS, Linux (x86-64 and riscv64) and Windows.
- The same holds for `tls::write` on every platform that layers TLS over a
  descriptor.
- No MFBASIC program can be terminated by a signal as a consequence of a peer's
  behavior on a socket it owns.

### Non-goals (must NOT change)

- **Do NOT "fix" this by deleting the documented raise from the docs.** bug-465
  already softened `tcp::write`'s sentence so it no longer promises behavior the
  code cannot deliver; that is a stopgap, not the fix. The contract is right and
  the code must meet it.
- No change to `tcp::read`'s end-of-stream contract (bug-465 pinned it).
- No change to the `EAGAIN`/`EINTR` classification in `write_fail`.
- **A process-wide `signal(SIGPIPE, SIG_IGN)` is not obviously the right answer**
  and must not be adopted without deciding the question below: it also changes
  what `io::print` does when its stdout pipe closes, which is how `prog | head`
  is supposed to end. Weigh it against per-socket/per-call suppression.

## Blast Radius

Found by search, not memory (`grep -rn "emit_write\|NetSymbol::Send" src/codegen/`).

- `src/codegen/builtins/tcp/gen_io.rs:860` (`lower_net_write_helper`) — the
  reproduction. **Fixed by this bug.**
- `src/codegen/builtins/tls/gen_openssl.rs` `SSL_write` path — same hazard on
  Linux, through libssl's internal `write(2)`. **In scope**; needs a different
  remedy from the call-site one.
- `src/codegen/builtins/tls/gen_macos/**` — unaffected: Network.framework owns
  the socket; no raw `write` is issued.
- `src/codegen/builtins/tls/gen_schannel*.rs` — unaffected: Win32 has no SIGPIPE.
- `src/codegen/builtins/udp/gen_io.rs` — unaffected in practice: a connectionless
  datagram socket has no peer that can RST it. Confirm rather than assume for a
  `connect`ed UDP socket.
- `src/codegen/builtins/fs/**`, `io::print`/`io::write` — same *class* (a plain
  `write(2)` to a pipe whose reader is gone), out of scope here because the
  correct behavior differs: a CLI dying on a closed stdout pipe is the
  conventional, wanted behavior for `prog | head`. Decide it separately; do not
  let a process-wide fix silently change it.

## Fix Design

Not settled — the three candidate mechanisms have materially different reach, and
the Linux `SSL_write` case is what makes the obvious per-call fix insufficient:

1. **Per-socket `SO_NOSIGPIPE`** (macOS/BSD only; the option does not exist on
   Linux) at every socket-creation site listed above, including the descriptor
   `accept` returns — the flag is per-socket and is **not** inherited across
   `accept`.
2. **Per-call `MSG_NOSIGNAL`**: switch the non-Windows write from `write(2)` to
   `send(fd, buf, len, MSG_NOSIGNAL)`, which the Windows arm already models. Fixes
   the direct writes on Linux, does nothing for libssl's internal write.
3. **Process-wide `signal(SIGPIPE, SIG_IGN)`** at program entry: covers every
   path including libssl, and is what most network runtimes do — but it changes
   `io::` pipe behavior, so it needs the Non-goals decision first.

The correctness risk is concentrated in **coverage, not in any one edit**: a fix
that lands on the direct `tcp::write` path and leaves `SSL_write` exposed still
lets a TLS server be killed by a client, which is the same bug with a narrower
trigger. Expect `.ncodesum` drift across every network fixture on the affected
targets whichever mechanism is chosen.

## Phases

### Phase 1 — measure the matrix, then a failing test

- [ ] Run the reproduction on linux-x86_64 and linux-riscv64 and fill in the
      matrix. Do the same through `tls::write` on Linux (OpenSSL) to confirm or
      eliminate the `SSL_write` row.
- [ ] Add a `tests/rt-behavior/tcp/` fixture asserting the write raises
      `ErrConnectionClosed` and the process exits 0. It must write **at least
      twice** after the peer's close — one write succeeds, so a single-write test
      passes against the broken build.
- [ ] Decide the mechanism (Open Decisions) before writing any fix.

Acceptance: the matrix is measured on every target; the new fixture fails for the
documented reason (killed by a signal, not a wrong error code).
Commit: —

### Phase 2 — the fix

- [ ] Apply the chosen mechanism at every in-scope site from the audit.
- [ ] Cover the `accept`ed descriptor explicitly if the per-socket route wins.

Acceptance: the Phase 1 fixture passes on every target; `EAGAIN`/`EINTR`
classification is unchanged.
Commit: —

### Phase 3 — regenerate + validate

- [ ] Regenerate the drifted `.ncodesum` set and gate with `artifact-gate.sh all`.
- [ ] `cargo test --release --no-fail-fast` and `test-accept.sh`.
- [ ] Restore `tcp::write`'s documented raise (the sentence bug-465 softened) once
      the code delivers it.

Acceptance: full suite green; the reproduction passes on every row of the matrix.
Commit: —

## Validation Plan

- Regression test: the two-write rt-behavior fixture above, on every target.
- Runtime proof: the reproduction exits 0 with the `TRAP` message, rather than
  141.
- Doc sync: `tcp::write` DESC (restore the raise), and `tls::write` gains the
  matching sentence for parity — the gap bug-465 recorded and deliberately did
  not fill with an unverified claim.
- Full suite: `cargo test --release --no-fail-fast`, `test-accept.sh`,
  `artifact-gate.sh all`.

## Open Decisions

- **Which suppression mechanism.** Recommended: **process-wide
  `signal(SIGPIPE, SIG_IGN)` at entry, plus `MSG_NOSIGNAL`/`SO_NOSIGPIPE` where
  it is free** — only the process-wide disposition reaches libssl's internal
  write, and leaving TLS servers killable is not an acceptable partial fix.
  Alternative: per-socket/per-call only, accepting that a Linux TLS server stays
  vulnerable until libssl is otherwise contained.
- **What `io::print` to a closed stdout should do** if the process-wide route is
  taken. Recommended: keep the current terminate-on-closed-pipe behavior for the
  `io::` path explicitly (it is what `prog | head` needs), rather than inheriting
  a change from the socket fix. Needs its own decision and probably its own test.

## Summary

A remote peer can terminate any MFBASIC TCP server by connecting and
disconnecting: the second write after the peer's close raises SIGPIPE and the
process dies without running a single line of the program's error handling, while
`tcp::write`'s documentation promises exactly the error the program is prevented
from receiving. Found while auditing `tcp`/`tls` documentation parity for bug-465,
which corrected the false sentence but could not deliver the raise. The
engineering risk is in coverage rather than in any individual edit — the Linux
OpenSSL path writes inside libssl, where a call-site fix cannot reach, so the
mechanism has to be chosen before any code is written.
