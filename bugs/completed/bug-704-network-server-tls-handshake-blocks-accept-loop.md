# bug-704: `--tls --thread` accept loop blocks forever in one client's handshake

Last updated: 2026-09-26
Effort: small (<1h)
Severity: HIGH
Class: Security/DoS

STATUS: FIXED (6c1cadc85)

Fixed as designed: `serveTlsThreaded`'s `tls::accept` call now takes
`wire.mfb:acceptHandshakeTimeoutMs()`, mirroring `serveTls`'s existing
`budgetMs` pattern exactly as the doc's Fix Design recommended. One deviation
discovered during Phase 3 validation, not anticipated by the doc: the first
value tried (1500 ms) reliably starved *legitimate* clients too, because
`examples/network-client` probes TCP, then UDP, then TLS with a fixed
3000 ms `connectTimeoutMs`, and 1500 divides evenly into 3000 — the real TLS
attempt landed at the same point in every accept cycle, cycle after cycle
(reproduced 5/5 runs). Root cause: `mfb man tls accept`'s documented "one
deadline bounds both the wait for a connection and the handshake" means a
connection popped near the tail of that deadline gets only the leftover
sliver to complete its handshake in — correct, intentional API behavior, not
a library bug (confirmed: `tools/net-probes`-style manual timestamped
tracing showed the real connection was popped with a few ms left standing,
and the server's own `hs_timeout` path cancels the connection, which is what
the client observed as `ErrTlsFailed`). The fix stayed at the call site,
per the doc's non-goal ("the `tls::accept` API surface itself... is not a
library gap"): switched to 2200 ms, chosen so `3000 mod 2200 = 800` sits
comfortably mid-cycle rather than at either edge — verified 10/10 plain runs
and 5/5 attack-scenario runs clean, documented in `wire.mfb` at the constant.
**Correction (bug-736):** the shared deadline was a macOS `tls::accept` bug, not
intended API behaviour; bug-736 gives the handshake a fresh budget, so the
resonance rationale no longer applies and `acceptHandshakeTimeoutMs()` is 1000 ms.
Regression Test: `tools/network-starvation-probe/probe.sh` (committed,
`d90b604aa`)

`examples/network-server --tls CERT KEY --thread` runs one thread that does
nothing but accept connections and hand them to a worker
(`examples/network-server/src/main.mfb:serveTlsThreaded`). For TLS, "accept" is
`tls::accept(server)` called with no `timeoutMs` argument, and per `mfb man tls
accept` that call blocks until a connection arrives **and its handshake
completes**, with no bound when the argument is omitted. A client that opens a
plain TCP connection to the port and never sends any TLS bytes (an idle raw
socket, not a valid TLS peer) parks that single accept thread inside the
handshake read indefinitely. Because this is the *only* thread that ever calls
`tls::accept`, every other client — including ones that speak TLS correctly —
is starved: their connections may complete the OS-level TCP handshake (they
sit in the kernel backlog) but the application never reaches them, so their own
TLS handshake attempt times out. The correct behavior is that one
slow-or-hostile TCP peer can cost the accept thread no more than a bounded
amount of time, after which it must move on and let queued clients be served.

References:

- `mfb man tls accept` — the `timeoutMs` parameter and its semantics (quoted
  under Root Cause).
- `examples/network-server/README.md` (`## --thread` section) — already
  documents "For `--tls` the handshake still completes on the main thread" as
  a design fact, but does not flag the unbounded-block hazard.
- `examples/network-server/src/main.mfb:serveTls` (the non-threaded path,
  lines 224-238) — the sibling code that already bounds the same call with
  `budgetMs`, proving the fix is a known, already-used pattern in this same
  file.
- `bugs/completed/bug-464-sockets-and-listeners-not-thread-sendable.md` — the
  prior work that made handing an accepted TLS socket to a worker thread
  possible at all; this bug is in the accept step that precedes that handoff.

## Failing Reproduction

Build both example projects and locate the shipped self-signed cert:

```
$ ./target/release/mfb build examples/network-server
$ ./target/release/mfb build examples/network-client
$ ls examples/network-server/certs/            # cert.pem, key.pem already present
```

Pick a free port (verified free: `lsof -iTCP -sTCP:LISTEN -P | grep 18443` — no
output) and start the threaded TLS server:

```
$ cd examples/network-server
$ ./build/network-server.out --tls certs/cert.pem certs/key.pem --thread --port 18443
Server 44872 (Ctrl-C to stop)
Listening tls 127.0.0.1:18443 (threaded)
Worker tls ready
```

**Contrast case first (no idle connection): the client succeeds.**

```
$ cd examples/network-client
$ ./build/network-client.out --port 18443 --server-name localhost --allow-self-signed
Client 49864
[tcp 127.0.0.1:18443]
Connected 49864
Lost 49864 silent for 5000ms
Disconnected 49864
[udp 127.0.0.1:18443]
Failed 49864 no reply from 127.0.0.1:18443
[tls 127.0.0.1:18443]
Connected 49864
Data 49864 Hello f866ee08-a7fd-4d02-90c5-f1210e5bd657
Data 49864 Update f866ee08-a7fd-4d02-90c5-f1210e5bd657 01
...
Data 49864 Update f866ee08-a7fd-4d02-90c5-f1210e5bd657 05
Disconnected 49864
```

(network-client always tries TCP, then UDP, then TLS in turn against the same
port — the `[tcp ...]`/`[udp ...]` attempts fail because this server only
speaks TLS on this socket; that is normal client behavior, not the bug. The
`[tls ...]` attempt is the one that matters and it succeeds cleanly here.)

**Now open one idle, unauthenticated raw TCP connection to the same port and
never send anything on it:**

```
$ python3 - <<'EOF' &
import socket, time
s = socket.create_connection(("127.0.0.1", 18443), timeout=10)
time.sleep(3600)
EOF
```

**With that idle connection still open, run the client again against the same
server:**

```
$ ./build/network-client.out --port 18443 --server-name localhost --allow-self-signed
Client 50843
[tcp 127.0.0.1:18443]
Connected 50843
Lost 50843 silent for 5000ms
Disconnected 50843
[udp 127.0.0.1:18443]
Failed 50843 no reply from 127.0.0.1:18443
[tls 127.0.0.1:18443]
Failed 50843 Operation did not complete before its deadline.
```

- Observed: the TLS attempt — the one that worked a moment ago against the
  same server — fails with `Operation did not complete before its deadline.`
  (network-client's own bounded `tls::connect` timeout, 3000 ms, firing
  because the server-side accept thread never got back around to it).
- Expected: the client's TLS handshake completes and it receives `Hello`/
  `Update` lines exactly as in the contrast case, regardless of one other
  client holding an idle connection open.

**Recovery, bounding the bug:** killing the idle connection frees the accept
thread and the very next client run succeeds again:

```
$ kill -9 <idle-python-pid>
$ ./build/network-client.out --port 18443 --server-name localhost --allow-self-signed
...
[tls 127.0.0.1:18443]
Connected 51553
Data 51553 Hello 2fa1ae01-a67f-46a0-9a5f-ea824d8d89da
Data 51553 Update 2fa1ae01-a67f-46a0-9a5f-ea824d8d89da 01
...
Data 51553 Update 2fa1ae01-a67f-46a0-9a5f-ea824d8d89da 05
Disconnected 51553
```

This is the exact matrix run: baseline pass, then starved while the idle
connection is open, then recovered the moment it is closed — isolating the
idle connection, not general TLS brokenness, as the cause.

| Scenario | Idle raw connection held open | TLS client result |
| --- | --- | --- |
| Baseline | no | connects, receives 5 updates, clean disconnect ✓ |
| Under attack | yes | `ErrTimeout` — "Operation did not complete before its deadline." ✗ |
| Recovery | closed mid-run | connects, receives 5 updates, clean disconnect ✓ |

## Root Cause

`examples/network-server/src/main.mfb:serveTlsThreaded` (lines 422-448):

```
DO
  RES sock AS tls::Socket = tls::accept(server) TRAP(e)
    CONTINUE DO
  END TRAP
  thread::transfer(worker, sock) TRAP(e)
    ...
LOOP UNTIL FALSE
```

`tls::accept(server)` is called with the optional `timeoutMs` argument
omitted. `mfb man tls accept`:

> `timeoutMs (opt)`: "Optional. The maximum time to wait for a connection and
> its handshake, in milliseconds. Omit to block until ready; 0 is one
> immediate attempt; a positive value bounds the wait; a negative value
> raises ErrInvalidArgument."
>
> "The optional timeoutMs bounds how long accept waits for both an inbound
> connection and the handshake to complete... When it is omitted, `accept`
> blocks until a connection is ready and handshaken."

`serveTlsThreaded` is the *only* thread in `--tls --thread` mode that ever
calls `accept` at all (the comment at line 435-438 already says this
explicitly: "Unlike the single-threaded loop this one may block indefinitely
in accept, because nothing else runs on this thread that a tick could be late
for" — correctly describing the unboundedness, but not recognizing that
"late for a tick" understates the actual consequence: every *other* client's
accept, not just a tick, is what gets starved). Since the OS TCP accept queue
still completes the three-way handshake for new arrivals independently of the
application, a second client's `tcp::connect`/`tls::connect` can appear to
succeed at the kernel level while the process is permanently parked inside the
first, hostile connection's `SSL_accept` read — so the second client's own
handshake never gets serviced and eventually times out on its own client-side
deadline (`connectTimeoutMs`, 3000 ms in `examples/network-client`). One idle
peer therefore has unbounded leverage over every concurrently-arriving
legitimate client, not just a bounded delay.

The sibling, non-threaded function bounds the identical call
(`examples/network-server/src/main.mfb:serveTls`, lines 224-239):

```
LET budgetMs AS Integer = (nextTick - datetime::monotonicNanos()) / 1000000
IF budgetMs < minHandshakeMs() THEN
  EXIT DO
END IF
RES sock AS tls::Socket = tls::accept(server, budgetMs) TRAP(e)
  EXIT DO
END TRAP
```

`serveTls`'s poll loop still services already-connected clients' reads,
writes, and the 500 ms broadcast tick between accept attempts (`tickNanos()`
in `examples/network-server/src/wire.mfb`, 500,000,000 ns), and it caps each
handshake attempt to at most the remainder of the current tick
(`minHandshakeMs()` = 250 ms is the floor below which it skips the attempt
entirely rather than starting one it can't finish). So an idle connection
against the non-threaded server costs existing clients at most one tick
(≤500 ms) of delayed reads/writes/broadcasts, self-healing every loop
iteration — a bounded, already-designed-for cost, not the same hazard. I
confirmed this by reading the loop structure; I did not additionally
reproduce the (much smaller, non-security-relevant) bounded-delay behavior of
the non-threaded path, since Goal is scoped to the unbounded case.

## Goal

- `serveTlsThreaded`'s accept thread can never be blocked by one peer for
  longer than a fixed, small bound (a deadline passed to `tls::accept`,
  mirroring `serveTls`'s existing `budgetMs` pattern). A second, well-behaved
  client's TLS handshake must complete within its own client-side timeout
  even while an unrelated idle connection is open against the same server.

### Non-goals (must NOT change)

- The threading model itself (one accept thread, one worker thread) — this is
  not a redesign of `--thread`.
- The plaintext TCP (`serveTcpThreaded`) and UDP paths: `tcp::accept` has no
  handshake step (a TCP `accept()` either has a connection ready or it
  doesn't — there is nothing analogous to a TLS handshake to stall inside),
  and `--udp --thread` is rejected outright by the CLI (`main.mfb`, the
  `threaded AND mode = "udp"` check) — UDP has no per-connection accept at
  all. Neither is touched by this bug or its fix.
- `serveTls`'s existing bounded behavior (must not be weakened or "fixed"
  further — it is the correct pattern the fix should copy).
- The `tls::accept` API surface itself (its `timeoutMs` parameter already
  exists and already does what is needed; this is a call-site bug, not a
  library gap).

## Blast Radius

`grep -rn "tls::accept" examples/` — two call sites, both in
`examples/network-server/src/main.mfb`:

- `main.mfb:440` (`serveTlsThreaded`) — **fixed by this bug**: the only
  unbounded call, on the only thread that ever accepts in `--tls --thread`
  mode.
- `main.mfb:237` (`serveTls`) — **unaffected**: already passes a bounded
  `budgetMs`, and by construction (see Root Cause) the worst case is a
  ≤500 ms per-tick delay to existing connections, not an indefinite block of
  new ones. Not a call site sharing this hazard.

No other project under `examples/` calls `tls::accept`. `examples/browser`,
`examples/ai_chat`, etc. do not use `tls::listen`/`tls::accept` (grep above is
exhaustive across `examples/`).

## Fix Design

Give `serveTlsThreaded`'s call a bounded `timeoutMs`, the same parameter
`serveTls` already uses, so a stalled handshake fails fast with `ErrTimeout`
and the loop's `TRAP(e) CONTINUE DO` moves on to the next queued connection
instead of blocking on the current one forever. `mfb man tls accept` confirms
this is exactly what the parameter is for ("A positive value fails with
ErrTimeout if no connection arrives, or the handshake does not finish, within
that many milliseconds... the accepted connection is closed before the error
is returned; the listener stays open, so the server can continue accepting").
No library-level gap exists — `tls::accept` already supports everything the
fix needs; this is purely a missing argument at one call site.

Unlike `serveTls`, `serveTlsThreaded` has no tick to derive a budget from (the
thread only accepts; it does no polling or broadcasting), so the bound should
be a fixed constant analogous to `minHandshakeMs()` — e.g. a new
`acceptHandshakeTimeoutMs()` constant in `wire.mfb` (a few seconds is
generous for a local handshake; it should be well above realistic client
latency but small enough that one hostile connection cannot meaningfully
delay a legitimate one queued behind it). Rejected alternative: leaving the
call unbounded and instead running multiple accept threads — rejected because
it changes the documented "one accept thread, one worker thread" design for a
problem the existing `timeoutMs` parameter already solves with a one-line
change, and because a fixed pool still has the same hazard once all pool
threads are simultaneously occupied by hostile connections (whereas a bounded
timeout has no such saturation point).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Commit `tools/network-starvation-probe/probe.sh` (new tool directory,
      with a README per the repo's `scripts/`-and-`tools/` convention — this
      is a one-off-shaped probe that nonetheless needs to be reusable/rerun
      after the fix, and it drives OS-level processes (server binary, a raw
      idle socket, the client binary) rather than being expressible as a
      single `.mfb` file, which is why it belongs under `tools/<name>/`
      rather than `bugs/repro/*.mfb`). The script: builds both example
      projects, starts `network-server --tls ... --thread` on a scratch port,
      opens one idle raw TCP connection to it via `python3`, runs
      `network-client` with a bounded wall-clock timeout and asserts its TLS
      phase fails/hangs past a threshold (reproducing today's bug), then
      closes the idle connection and asserts a following client run succeeds
      within a bounded time (the recovery case), then kills every process it
      started.
- [x] Confirm it fails today for the documented reason (`ErrTimeout` /
      "Operation did not complete before its deadline." on the client's TLS
      phase while the idle connection is held).
- [ ] Blast-radius audit above is complete with a verdict per site.

Acceptance: the probe fails today, reproducing exactly the transcript above;
the audit table has a verdict for every `tls::accept` call site.
Commit: d90b604aa

### Phase 2 — the fix

- [x] Add a bounded handshake timeout constant to
      `examples/network-server/src/wire.mfb` (e.g.
      `PUBLIC FUNC acceptHandshakeTimeoutMs() AS Integer`).
- [x] Pass it to the `tls::accept` call in `serveTlsThreaded`
      (`examples/network-server/src/main.mfb:440`), matching the
      `TRAP(e) CONTINUE DO` handling already in place so a timed-out or
      failed handshake simply cedes the loop to the next connection.
- [x] Update the README's `## --thread` section and the inline comment at
      `main.mfb:435-438` — both currently assert unbounded blocking is
      acceptable/intended; both need to describe the new bounded behavior
      instead of documenting the hazard as a design fact.

Acceptance: the Phase 1 probe's starvation case now passes within its bound
(the second client's TLS handshake completes, or the idle connection is
evicted quickly enough that a subsequent attempt does) instead of hanging;
`serveTls`'s existing behavior is unchanged; `tcp`/`udp` paths are untouched.
Commit: 6c1cadc85

### Phase 3 — validation

- [x] Re-run `scripts/build-examples.sh` (builds every example, including
      `network-server`/`network-client`, for every registered target and
      bundles them into `target/examples.zip`; it fails non-zero if any
      per-target build fails) to confirm the fix doesn't break
      cross-compilation of the example. (11 failures reported, all
      pre-existing/unrelated: `audio` on every target — missing
      `libsnd.mfp` package — and `bugs`/`dungeon`/`emoji`/`water`/`wind` on
      `linux-riscv64` — app mode does not support riscv64. `network-server`
      and `network-client` both built clean on all five targets.)
- [x] Re-run the Phase 1 probe to confirm it now passes. (3/3 full runs
      green; separately, 10/10 plain client runs and 5/5 attack-scenario
      manual runs green — see the resonance note above.) (Correction: the
      resonance came from bug-736's shared accept deadline, fixed there.)
- [x] Manually re-run the exact repro transcript above end-to-end (baseline /
      attack / recovery) and confirm the "attack" row now also succeeds
      within a bounded time instead of timing out. (Confirmed: attack row now
      connects and receives all five Update lines, same as baseline; recovery
      row unaffected.)

Acceptance: `scripts/build-examples.sh`'s `network-server`/`network-client`
builds succeed on every target (the run's 11 failures are pre-existing and
unrelated — `audio` lacks an installed `libsnd` package on every target, and
app mode does not support `linux-riscv64` for `bugs`/`dungeon`/`emoji`/
`water`/`wind`; confirmed by inspection, not caused by this change); the
probe passes; the manual repro's "attack" row now matches "baseline" instead
of failing.
Commit: 6c1cadc85 (no additional commit; validation only, no further code
change)

## Validation Plan

- Regression test: `tools/network-starvation-probe/probe.sh` (Phase 1),
  failing today, passing after Phase 2.
- Runtime proof: the exact baseline/attack/recovery transcript in Failing
  Reproduction, re-run after the fix.
- Doc sync: `examples/network-server/README.md` `## --thread` section and the
  `main.mfb:435-438` comment (both currently document the unbounded block as
  intended).
- Full suite: `scripts/build-examples.sh` (the example-specific gate; this is
  example code, not compiler code, so `cargo test` is not expected to cover
  it — confirmed by `grep -rn "network-server" src/` finding no references
  from the compiler test suite).

## Open Decisions

- Exact timeout value for `acceptHandshakeTimeoutMs()` — recommend a few
  seconds (generous for any real client's handshake, small enough that a
  hostile connection's cost to the next queued client stays bounded and
  small) vs. making it a `--handshake-timeout` CLI flag for the example.
  Recommend the constant: this is an example program illustrating the
  pattern, not a production server needing operator tuning. (§Fix Design)

## Summary

The unbounded call is a one-line omission at a single call site
(`examples/network-server/src/main.mfb:440`) in an already-correct pattern
that this same file uses two hundred lines earlier (`main.mfb:237`); the
`tls::accept` library function already has everything the fix needs. The real
engineering risk is small and concentrated in Phase 1: building a reliable,
timing-based probe (external process orchestration: idle socket, two binaries,
bounded waits) that reproduces a starvation window deterministically enough to
gate on, without flaking on slower CI hosts. The fix itself and the doc sync
are mechanical once that probe exists.
