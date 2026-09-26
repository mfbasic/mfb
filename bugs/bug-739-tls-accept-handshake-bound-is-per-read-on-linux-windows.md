# bug-739: on Linux and Windows, `tls::accept`'s handshake bound applies to each read, not to the whole handshake

Last updated: 2026-09-26
Effort: large (3h–1d)
Severity: MEDIUM
Class: Security/DoS

Status: Open
Regression Test: none yet. The repro is `bugs/repro/bug-739-tls-accept-trickle-server.mfb`
plus `bugs/repro/bug-739-tls-accept-trickle-client.py`. A fix phase should turn it into
a `tests/net/` case like `tests/net/rt_tls_accept_handshake_budget.rs`.

Found while fixing bug-736. `tls::accept(listener, timeoutMs)` now bounds two
steps: the wait for a connection, then the handshake. On macOS the handshake
step is one absolute deadline, so the whole handshake must finish within
`timeoutMs` of the connection being taken. On Linux (OpenSSL) and Windows
(Schannel) the handshake step is `SO_RCVTIMEO`/`SO_SNDTIMEO` on the accepted
socket. That timeout applies to each blocking `recv`/`send`, so it bounds how
long the peer may stay silent, not how long the handshake takes. A peer that
sends one byte of its ClientHello every `timeoutMs - ε` holds `SSL_accept`, or
the Schannel handshake loop, for as long as it keeps doing that. A server that
calls `tls::accept` from one thread is blocked for that whole time.
bug-202 bounded the handshake precisely to stop one peer from wedging the
accept loop. That fix stops a silent peer but not a slow one.

**The single correct behavior a fix produces:** on every backend the handshake
step of `tls::accept(listener, timeoutMs)` finishes, or raises `ErrTimeout`,
within about `timeoutMs` of the connection being taken, however the peer paces
its bytes. That is the macOS behaviour since bug-736.

References:

- `mfb man tls accept`: "…or if the handshake with the client that arrived runs
  out of time; a client that sends nothing for `timeoutMs` during its handshake
  is always dropped." That sentence is worded to hold on every backend today.
  Once this is fixed it can state the whole-handshake bound.
- `mfb spec stdlib transports`, "TLS specifics", the `tls::accept` bullet. It
  records the per-backend difference as-is.
- `bugs/completed/bug-736-tls-accept-shares-deadline-with-handshake.md`, where
  this was found.
- `bugs/completed/bug-202-*`: the handshake bound this narrows.

## Failing Reproduction

Build the repro server for both targets (it needs `cert.pem`/`key.pem` next to
it; any self-signed `CN=127.0.0.1` pair works):

```
mkdir -p /tmp/b739/src && cp bugs/repro/bug-739-tls-accept-trickle-server.mfb /tmp/b739/src/main.mfb
cp bugs/repro/bug-739-tls-accept-trickle-client.py /tmp/b739/client.py
# project.json: executable, entry main, src/**.mfb
mfb build /tmp/b739                          # macOS
mfb build --target linux-aarch64 /tmp/b739   # ship build/*-glibc.out + client + certs to box 2226
```

Run the server with `timeoutMs = 1000` on stdin. Then connect a client that
sends its first 20 ClientHello bytes one at a time, 200 ms apart, and then
finishes the handshake normally (`python3 client.py trickle 200 20`):

| Environment | Server result | Client |
| --- | --- | --- |
| Linux aarch64 glibc (box 2226, OpenSSL) | `result=OK 4619` (ms), a 1000 ms bound | `client OK after 4077ms` ✗ |
| macOS (Network.framework), bug-736 fixed | `result=FAILED 77050008 1411` | dropped at the bound ✓ |
| Windows (Schannel) | not run; same mechanism by code reading | expected ✗ |

- Observed (Linux): the handshake took about 4 s under a 1000 ms bound and
  succeeded. Each 200 ms gap reset the per-`recv` timeout.
- Expected: `ErrTimeout` about 1000 ms after the connection was taken, as on
  macOS.

Contrast cases that already agree on every backend (measured on macOS and box
2226 during bug-736):

- a peer that sends nothing (`client.py late 0 3000`, bound 1000):
  `FAILED 77050008` on both;
- a late connection with a delayed ClientHello (`client.py late 3000 4500`,
  bound 4000): `OK` on both;
- `tls::accept(l, 0)` with a silent peer queued: `FAILED 77050008` in 1 ms on
  macOS and 12 ms on Linux.

## Root Cause

- **Linux:** `src/codegen/builtins/tls/gen_openssl.rs:lower_tls_accept_openssl`
  sets `SO_RCVTIMEO`/`SO_SNDTIMEO` to `timeoutMs` on the accepted fd
  (`emit_set_sock_timeouts`) and then makes ONE blocking `SSL_accept` call.
  OpenSSL issues as many `read()`s as the handshake needs. Each one gets a fresh
  `SO_RCVTIMEO`, and nothing tracks the total time.
- **Windows:** `src/codegen/builtins/tls/gen_schannel_server.rs:lower_tls_accept`
  does the same with a DWORD `SO_*TIMEO`. Its handshake loop (`{symbol}_hs_read`)
  calls `recv` again each time `AcceptSecurityContext` reports incomplete input,
  and each `recv` gets the full timeout.
- **macOS is immune** because `dispatch_semaphore_wait` takes an absolute
  deadline, computed once per handshake by `emit_accept_deadline`
  (`gen_macos/server.rs`, bug-736).

## Goal

- On Linux and Windows, the handshake step of `tls::accept(l, timeoutMs)` raises
  `ErrTimeout` no later than about `timeoutMs` after the connection was taken,
  for a peer that trickles bytes. Measure it with the trickle repro on box
  2226/2223 (OpenSSL) and 2230 (Schannel).

### Non-goals (must NOT change)

- The two-step contract bug-736 settled: the connection wait and the handshake
  each get the full `timeoutMs`. Do not reintroduce one shared deadline.
- Omitted means block in both steps. `0` means one immediate attempt at each
  step.
- The returned socket's reads and writes stay unbounded. The handshake timeouts
  are cleared after the handshake, and that clearing must stay.
- `tls::connect`'s handshake has the same shape (`SO_*TIMEO` around a blocking
  `SSL_connect` / Schannel loop). It is a client-side bound, a different threat
  model, and out of scope unless the owner folds it in.

## Blast Radius

- `gen_openssl.rs:lower_tls_accept_openssl`: affected (all Linux targets:
  aarch64, x86_64, riscv64; glibc and musl).
- `gen_schannel_server.rs:lower_tls_accept`: affected (windows-x86_64), code
  reading only.
- `gen_macos/server.rs:lower_tls_accept_macos`: unaffected (absolute deadline).
- `examples/network-server` `serveTlsThreaded`/`serveTls`: exposed on Linux and
  Windows. A trickling peer holds the one accept thread past
  `acceptHandshakeTimeoutMs()`.

## Fix Design

- **Linux:** make the accepted fd non-blocking for the handshake and loop over
  `SSL_accept`. On `SSL_ERROR_WANT_READ`/`WANT_WRITE`, `poll` for
  `POLLIN`/`POLLOUT` with the time left (a `clock_gettime(CLOCK_MONOTONIC)`
  deadline taken when the connection is accepted). When the time runs out,
  raise `ErrTimeout` (a `WANT_*` at expiry, like today's `EWOULDBLOCK` path).
  Restore blocking mode afterwards. This replaces the `SO_*TIMEO` install and
  clear.
- **Windows:** before each `recv`/`send` in the handshake loop, set
  `SO_RCVTIMEO`/`SO_SNDTIMEO` to the time left (`GetTickCount64` deadline),
  raising `ErrTimeout` once none is left. That is the smallest change inside
  the existing loop.
- Rejected: `alarm()`/signals (process-wide, and they fight user code); leaving
  it as documented behaviour (it defeats bug-202's purpose).

## Phases

### Phase 1 — failing test + audit

- [ ] A `tests/net/` trickle case (Linux row; the Windows row runs on 2230 by
      hand) that fails today with `result=OK` after more than the bound.

Commit: —

### Phase 2 — the fix

- [ ] OpenSSL non-blocking handshake loop with a monotonic deadline.
- [ ] Schannel per-`recv` time-left timeout.

Commit: —

### Phase 3 — goldens + full validation

- [ ] Regenerate the four Linux and one Windows `tls` `.ncodesum` goldens; the
      diff is limited to `tls::accept`.
- [ ] Full `cargo test`. Run the trickle repro on 2226/2223 and 2230. Re-run
      `tools/network-starvation-probe/probe.sh`.
- [ ] Tighten the `mfb man tls accept` sentence and the spec bullet to the
      whole-handshake bound.

Commit: —

## Open Decisions

- **Is the whole-handshake bound the contract?** Recommended: yes. The owner's
  bug-736 decision ("follow Linux and Windows") assumed those backends bound
  the handshake. They bound each read, and only macOS bounds the whole
  handshake. The alternative is to document the per-read bound as the contract
  on Linux and Windows. That leaves the slow-peer DoS open and the backends in
  permanent disagreement.

## Summary

The risk is in the OpenSSL non-blocking loop, which is hand-emitted code for
three architectures, and in validating on remote boxes. The contract and the
macOS behaviour are settled by bug-736 and stay as they are.
