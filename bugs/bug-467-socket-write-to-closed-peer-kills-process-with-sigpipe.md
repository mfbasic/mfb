# bug-467: writing to a socket whose peer has closed kills the process with SIGPIPE instead of raising

Last updated: 2026-09-01
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness + Availability (a remote peer can terminate any MFBASIC server)

Status: Closed
Regression Test: `tests/rt_sigpipe_socket_and_pipe.rs` (3 tests) and
`tests/rt-behavior/tcp/tcp-write-peer-closed-raises-rt`

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

## STATUS: FIXED (0a81c3846, 434c1be01, 7334b1606, 0c031db41, 5928cb3dd, 12621e706, a1aef8539, 272870ddd)

`lower_program_entry` installs `signal(SIGPIPE, SIG_IGN)` on every POSIX target
for every program, app mode included. The `io::` stdout/stderr write paths
classify `EPIPE` and restore `SIG_DFL` + `raise` so the pipeline convention is
untouched; `process::spawn`'s fork child restores `SIG_DFL` before `execvp`.

Deviations from the design above, all deliberate:

1. **No `SO_NOSIGPIPE` / `MSG_NOSIGNAL` was added.** The Fix Design's option 1+2
   would be dead weight under option 3 — see Open Decisions.
2. **`tls::write` on OpenSSL gained an `SSL_get_error` classification** that the
   bug did not ask for by name. It is required by Goal bullet 2: the path had
   never returned before, so it collapsed every failure into `ErrTlsFailed`, and
   leaving it there would have broken the `tcp`/`tls` mirror the moment the
   process stopped dying. The `WANT_READ`/`WANT_WRITE` -> `ErrTimeout` arm is
   part of the same requirement, not scope creep: without it a
   `tls::setWriteTimeout` deadline would have been reported as a closed
   connection.
3. **The `io::` output plan arms now import the errno accessor**, which they
   need to classify `EPIPE`. A side effect is that bug-62's documented gap — an
   output-only program could not classify a negative libc write and hard-errored
   instead of retrying `EINTR` — is closed.
4. **Filed rather than fixed: bug-483.** `tls::write` to a departed peer now
   raises `ErrConnectionClosed` on Linux but still `ErrTlsFailed` on macOS
   (Network.framework) and `ErrNetworkFailed` on Windows (Schannel). Neither
   layers TLS over a descriptor in the sense of this bug's goal, and each needs
   its own transport-error classification.

### Corrections found on landing (2026-09-01)

The branch above was written but never landed, and the gates it claimed were not
all met. Three things were wrong and are fixed in the commits appended to
the STATUS line.

1. **The regression fixture asserted nothing** (`5928cb3dd`). All four goldens of
   `rt-behavior/tcp/tcp-write-peer-closed-raises-rt` were committed as zero-byte
   placeholders, against the sibling `tcp-read-eof-raises-rt`'s 523 / 7004 /
   58725 / 0. `sync-goldens.sh` only ever *overwrites* an existing golden and
   never creates one, so the placeholders survived every regeneration and the
   fixture was a dead gate — the exact failure mode recorded for a new rt
   fixture. Regenerated; `build.log` now pins both output lines and `[exit 0]`.

   Its sensitivity is now measured rather than assumed: against a clean-`main`
   compiler it went RED on **8 of 10** runs, not 10. On the other two the peer's
   departure surfaces as `ECONNRESET` instead of `EPIPE`, which raises without a
   signal and looks identical to a pass.

   **And it was flaky in the other direction too, which the full sweep caught.**
   The claim that it "never false-REDs on a fixed one" was measured on an idle
   machine and was wrong. Under the load of the 1347-fixture acceptance run the
   fixture printed `write completed=TRUE`: with 32-byte writes every one was
   absorbed by the local send buffer before the peer's RST arrived, so nothing
   failed at all — a golden mismatch on a CORRECT build. A flaky golden is worse
   than a weak one, because it makes the whole suite untrustworthy.

   Fixed by changing what the loop writes (`5ca5d9f65`). The chunk is now 64 KiB
   rather than 32 bytes, which fills the send buffer within a few iterations so
   the write BLOCKS waiting for ACKs a departed peer will never send — and a
   blocked write is exactly where the failure surfaces. Measured on the fixed
   build: 12/12 idle and 15/15 under eight spinning CPU hogs, all `raised=TRUE`.

   That trade is deliberate and costs sensitivity: the large-chunk shape is RED
   on only ~4 of 10 runs against a broken compiler, because a blocked write tends
   to observe `ECONNRESET` rather than take the signal. So the two tests were
   given different jobs, which is the durable lesson here:

   * the **golden fixture** takes the shape that is stable, because its output is
     pinned byte-for-byte and must never vary on a correct build;
   * the **Rust test** (`rt_sigpipe_socket_and_pipe.rs`) keeps the small-write
     shape, which is likelier to take the signal per run, and now runs the
     program **10 times** asserting that no run is ever killed by a signal. At
     ~80% per-run detection that is ~1e-7 miss probability, against ~20% for the
     single run it did before.

   The Rust test deliberately does NOT assert that a raise happened on any given
   run — that is the assertion that made the golden flaky. It asserts the real
   contract on every run (no signal, exit 0) and, once across the ten, that the
   failure path was exercised at all, so the probe cannot silently stop
   reproducing the condition and pass for the wrong reason.

2. **`cargo test --release` was not green** (`a1aef8539`). Six tests pinned facts
   the fix deliberately changes — the five `io_*_imports_nothing` plan tests and
   `cli_linux_app_mode`'s console-handler assertion. Each went through the
   four-question gate; none of them disproves the fix, and all six were corrected
   to assert the new truth *more* strictly than before (exact import sets, and
   the handler symbol the app-mode test always meant instead of the `signal`
   string it used as a proxy). Details in that commit message.

3. **The branch predated 73 golden changes on `main`** (`12621e706`). Merging
   main conflicted on 73 `.ncode`/`.ncodesum` drift sentinels that both sides had
   regenerated. Resolved to main's values rather than either branch's, then
   re-derived from the merged source: `artifact-gate.sh all` went 84 diffs → **0**
   across 1825 goldens. Three tools were needed, which is worth recording —
   `regen-ncodesum.sh` (132, byte-identity only), `regen-outside-ncode.sh` (15),
   and `sync-goldens.sh` (40 `.nplan`/`.nobj`/`.mir`, which neither regen script
   sweeps).

A fourth item is *not* a defect: `cargo test`'s `artifact_gate_all` failed once in
0.26s with "another gate run holds the lock". That is the harness's contention
refusal, not a golden regression — nothing was checked. The standalone gate run
is the authority and it is clean.

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
| linux-x86_64 glibc | box 2228, raw `write` syscall | fails ✗ (exit 141) |
| linux-x86_64 musl | box 2227 | fails ✗ (exit 141) |
| linux-riscv64 musl | box 2229 | fails ✗ (exit 141) |
| linux-x86_64 TLS | box 2228, `tls::listen`/`accept` + `openssl s_client` peer | fails ✗ (exit 141) |
| windows-x86_64 | `send()`; Win32 has no SIGPIPE | ✓ immune (0 of 24 `.ncodesum` moved) |

MEASURED 2026-08-31, all rows, 3 runs each. The Linux rows are identical to
macOS, and the TLS row is the one that settled the mechanism: `SSL_write` on
Linux dies exactly the same way, and no call site can reach libssl's internal
`write(2)`.

### RE-MEASURED 2026-09-01, on landing

Re-run from scratch against two compilers built for the purpose — one from clean
`main` (no SIGPIPE code anywhere: `grep -rni sigpipe src/` is empty there) and
one from this branch — rather than trusting the rows above. Both binaries ran the
committed fixture; the "unfixed" column is the bug still alive at `main`.

| Target | Box | unfixed | fixed |
| --- | --- | --- | --- |
| macos-aarch64 | local | 8/10 exit 141 | 10/10 raises, exit 0 |
| linux-x86_64 glibc | 2228 | 5/5 exit 141 | 5/5 raises, exit 0 |
| linux-aarch64 glibc | 2223 | 3/3 exit 141 | 3/3 raises, exit 0 |
| linux-riscv64 musl | 2229 | 3/3 exit 141 | 3/3 raises, exit 0 |

The macOS row is 8/10 rather than 10/10 on purpose — see the note on the
fixture's sensitivity under Corrections. Every failing run produced **no output
at all**: the process dies inside the loop, so not even the first `io::print`
runs, which is the signal death rather than a wrong error code.

The two behaviours that had to survive were measured on both Linux write
conventions — x86-64's raw `svc` `write` (returns `-errno`) and riscv64's libc
`write` (needs the errno accessor):

| Check | Target | unfixed | fixed |
| --- | --- | --- | --- |
| `prog \| head -3` still dies by SIGPIPE | linux-x86_64 | exit 141 | exit 141 |
| `prog \| head -3` still dies by SIGPIPE | linux-riscv64 | exit 141 | exit 141 |
| spawned child's SIGPIPE disposition | linux-x86_64 | `DEFAULT` | `DEFAULT` |

Windows was not re-run: its entry is untouched and `artifact-gate.sh all`
reports **0** windows-x86_64 diffs, which is the prediction the fix makes about
itself.

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

- [x] Run the reproduction on linux-x86_64 and linux-riscv64 and fill in the
      matrix. Do the same through `tls::write` on Linux (OpenSSL) to confirm or
      eliminate the `SSL_write` row.
- [x] Add a `tests/rt-behavior/tcp/` fixture asserting the write raises
      `ErrConnectionClosed` and the process exits 0. It must write **at least
      twice** after the peer's close — one write succeeds, so a single-write test
      passes against the broken build.
- [x] Decide the mechanism (Open Decisions) before writing any fix.

Acceptance: the matrix is measured on every target; the new fixture fails for the
documented reason (killed by a signal, not a wrong error code). MET — the RED
test failed with `signal(13)`, and the `SSL_write` row was CONFIRMED, not
eliminated, which is what forced the process-wide mechanism.
Commit: e6402c7ab

### Phase 2 — the fix

- [x] Apply the chosen mechanism at every in-scope site from the audit.
- [x] Cover the `accept`ed descriptor explicitly if the per-socket route wins.
      N/A — the per-socket route did not win, so there is no per-descriptor flag
      to inherit. The disposition is process-wide and every descriptor, however
      obtained, is covered by construction.

Acceptance: the Phase 1 fixture passes on every target; `EAGAIN`/`EINTR`
classification is unchanged. MET — `write_fail` in `tcp/gen_io.rs` was not
touched at all; the fix only lets it run.
Commit: 0a81c3846, 434c1be01

### Phase 3 — regenerate + validate

- [x] Regenerate the drifted `.ncodesum` set and gate with `artifact-gate.sh all`.
- [x] `cargo test --release --no-fail-fast` and `test-accept.sh`.
- [x] Restore `tcp::write`'s documented raise (the sentence bug-465 softened) once
      the code delivers it.

Acceptance: full suite green; the reproduction passes on every row of the matrix.
MET, but only after the landing pass — see "Corrections found on landing". As
first written this acceptance was claimed, not met: the fixture's goldens were
empty and six tests were red. Now measured:
  * `artifact-gate.sh all` — 1825 goldens, **0** diffs, exit 0.
  * `cargo test --release --no-fail-fast` — a real `test result: ok`.
  * `test-accept.sh` — full acceptance sweep.
  * the reproduction, on all four POSIX targets, unfixed vs fixed, plus the
    `prog | head` and spawned-child behaviours that had to survive.
Commit: 7334b1606, 0c031db41, 5928cb3dd, 12621e706, a1aef8539, 272870ddd

## Validation Plan

- Regression test: the two-write rt-behavior fixture above, on every target.
- Runtime proof: the reproduction exits 0 with the `TRAP` message, rather than
  141.
- Doc sync: `tcp::write` DESC (restore the raise), and `tls::write` gains the
  matching sentence for parity — the gap bug-465 recorded and deliberately did
  not fill with an unverified claim.
- Full suite: `cargo test --release --no-fail-fast`, `test-accept.sh`,
  `artifact-gate.sh all`.

## Open Decisions — DECIDED

- **Which suppression mechanism.** DECIDED: **process-wide
  `signal(SIGPIPE, SIG_IGN)` at entry**, alone. The per-socket/per-call additions
  were dropped as pure redundancy — once the disposition is process-wide, a
  `SO_NOSIGPIPE` on each socket and a `MSG_NOSIGNAL` on each `send` change no
  observable behaviour and would only add drift and two more emitters to keep
  correct. The decision was forced by MEASUREMENT, not preference: the Linux TLS
  row was confirmed to die (box 2228, exit 141), and `SSL_write` reaches
  `write(2)` inside libssl through the socket BIO, where no call site can pass a
  flag and where Linux has no `SO_NOSIGPIPE` at all.
  It is `SIG_IGN` and deliberately NOT a handler: a handler is delivered, and
  delivery `EINTR`-interrupts blocking syscalls in every thread — including the
  `waitpid` in bug-474's `_mfb_rt_process_reaper`. `SIG_IGN` generates no
  delivery, so there is no interaction.
- **What `io::print` to a closed stdout should do.** DECIDED: keep the current
  behaviour exactly — `prog | head` still dies by SIGPIPE, measured at exit 141
  on macos-aarch64, linux-x86_64 (glibc and musl) and linux-riscv64, for the
  unbuffered, buffered (`io::setBuffered(TRUE)`) and stderr paths alike. It is
  kept the Go way: ignore SIGPIPE process-wide, then have the stdout/stderr write
  path classify its own `EPIPE`, restore `SIG_DFL` and re-raise. Every other
  errno still raises `ErrWriteFailed`. It does have its own test
  (`stdout_write_to_a_closed_pipe_still_dies_by_sigpipe`), which was GREEN before
  the fix and stays GREEN after it.
- **A third question the bug did not anticipate: spawned children.** POSIX
  carries an IGNORED disposition through `exec` unchanged — measured, not assumed
  (`sh -c 'trap "" PIPE; perl -e "print $SIG{PIPE}"'` prints `IGNORE`) — so every
  program `process::spawn` starts would have silently lost its own `prog | head`
  behaviour. DECIDED: the fork child restores `SIG_DFL` before `execvp`, with a
  test (`a_spawned_child_does_not_inherit_the_ignored_sigpipe`).

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
