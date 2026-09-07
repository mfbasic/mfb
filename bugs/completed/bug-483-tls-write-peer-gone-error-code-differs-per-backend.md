# bug-483: `tls::write` to a departed peer reports a different error code on each of the three TLS backends

Last updated: 2026-09-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (cross-platform contract divergence)

Status: **FIXED** (2026-09-05, `7b0ab81be`)
Regression Test: `tests/rt-behavior/tls/tls-write-peer-closed-raises-rt`
(runtime, host backend — RED before the fix: `write raised=FALSE`), plus
`codegen::builtins::tls::gen_schannel::schannel_tests::write_classifies_the_winsock_error_behind_a_failed_send`
(the only instrument that reaches the Windows backend from a Mac) and
`codegen::builtins::tls::gen_macos::tests::{write_names_a_posix_send_failure_connection_closed,
the_send_completion_records_the_error_domain_itself}`. The plaintext half stays
pinned by `tests/rt-behavior/tcp/tcp-write-peer-closed-raises-rt`.

`tcp` and `tls` are documented drop-in mirrors — "the same 11 function names", a
socket that is "byte-for-byte interchangeable" — so a protocol package can be
written once against a transport shim. For a peer that has gone away, the three
`tls` backends name the identical event three different ways, and only one of
them matches what `tcp::write` and both `read` calls already promise:

| backend | `tls::write` after the peer closes | matches `tcp::write`? |
| --- | --- | --- |
| Linux, OpenSSL (`gen_openssl.rs`) | `ErrConnectionClosed` | yes (bug-467) |
| macOS, Network.framework (`gen_macos/client.rs`) | `ErrTlsFailed` | no |
| Windows, Schannel (`gen_schannel_io.rs`) | `ErrNetworkFailed` | no |

A program that traps `ErrConnectionClosed` around a write — which is exactly what
`mfb man tcp write` and now `mfb man tls write` tell it to do — therefore handles
a client disconnect correctly on Linux and re-raises an unrelated-looking
"TLS handshake … failed" on macOS. **The single correct behavior a fix
produces:** every backend raises `ErrConnectionClosed` when the transport under
the TLS session is gone, and keeps `ErrTlsFailed` for an actual protocol or
certificate failure.

This is the same shape as the deadline divergence `.ai/net-tls.md` records
("The deadline error code is `ErrTimeout` on every backend — and each one had to
be taught it"): the natural error path on each backend swallows the event as
whatever transport error that backend happens to produce, and each has to
classify it explicitly.

References:

- `.ai/net-tls.md`, "The deadline error code is ErrTimeout on every backend" —
  the precedent, and the reason a per-backend classification is expected work
  rather than a surprise.
- `bugs/completed/bug-465-*` — pinned `tcp::read`/`tls::read` on one shared
  `ErrConnectionClosed` at end of stream, which is the read-side half of the
  contract this bug breaks on the write side.
- bug-467 — found here. Until it landed, the Linux row could not be observed at
  all (libssl's internal `write(2)` delivered SIGPIPE and the process died), and
  the macOS/Windows rows were never compared against it.

## Failing Reproduction

An MFBASIC TLS server, a peer that connects and exits, then repeated writes.
`examples/network-server/certs/{cert.pem,key.pem}` are a usable pair. The client
is `openssl s_client` rather than `tls::connect` because an MFBASIC client
cannot accept a self-signed certificate (bug-477).

```
IMPORT io
IMPORT os
IMPORT tls

FUNC probe(port AS Integer) AS String
  RES server = tls::listen("127.0.0.1", port, "cert.pem", "key.pem")
  io::print("listening")
  RES conn = tls::accept(server)
  os::sleep(2000)
  FOR i = 1 TO 20
    tls::write(conn, "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx")
    os::sleep(200)
  NEXT
  RETURN "no-raise"
  TRAP(e)
    RETURN "raised: " & e.message
  END TRAP
END FUNC

FUNC main AS Integer
  io::print(probe(34572))
  RETURN 0
END FUNC
```

Driven with `./server.out & sleep 1; openssl s_client -connect 127.0.0.1:34572 </dev/null`.

- Observed (macos-aarch64, measured 2026-08-31):
  `raised: TLS handshake, certificate validation, SNI validation, or protocol operation failed.`
- Expected: `raised: Socket peer closed the connection or the connection is no longer usable.`
  — the message `ErrConnectionClosed` carries, which is what the same program
  prints on linux-x86_64.

Contrast cases, all measured on the same day and build:

- **Linux is already correct.** The same program on box 2228 (glibc x86_64)
  prints the `ErrConnectionClosed` message, because bug-467 taught
  `lower_tls_write_openssl` to classify `SSL_get_error`.
- **Both `read` sides already agree** on `ErrConnectionClosed` at end of stream
  (`rt-behavior/tcp/tcp-read-eof-raises-rt`,
  `rt-behavior/tls/tls-read-eof-raises-rt`). It is only the write direction that
  diverges.
- **`tcp::write` agrees on every target** — it maps every errno that is not
  `EAGAIN`/`EINTR` to `ErrConnectionClosed`
  (`tcp/gen_io.rs:lower_net_write_helper`), pinned by
  `rt-behavior/tcp/tcp-write-peer-closed-raises-rt`.

| Environment | Details | Result |
| --- | --- | --- |
| macos-aarch64 | Network.framework, loopback + `openssl s_client` | fails ✗ (`ErrTlsFailed`) |
| linux-x86_64 | OpenSSL, box 2228 glibc | works ✓ (`ErrConnectionClosed`) |
| linux-aarch64 / riscv64 | same OpenSSL emitter as x86_64 | expected ✓, not run |
| windows-x86_64 | Schannel | **not run** — read from source; expected ✗ (`ErrNetworkFailed`) |

The Windows row is a source reading, not a measurement, and must be measured on
box 2230 before the fix is designed.

### The measured matrix (2026-09-05)

All three rows run, with the literal message each backend printed. The probe is
the program above, driven by `openssl s_client -connect 127.0.0.1:PORT
-servername localhost </dev/null` (macOS host and box 2228) and by an MFBASIC
`tls::connect(..., allowSelfSigned := TRUE)` client on box 2230, which has no
`openssl`. The identity is a 397-day `CN=localhost` leaf with `serverAuth` and
`subjectAltName = DNS:localhost, IP:127.0.0.1`.

| backend | before | after |
| --- | --- | --- |
| **macos-aarch64** Network.framework (host) | `77070008` `ErrTlsFailed` — "TLS handshake, certificate validation, SNI validation, or protocol operation failed." | `77070004` `ErrConnectionClosed` — "Socket peer closed the connection or the connection is no longer usable." |
| **linux-x86_64** OpenSSL 3.5.6 (box 2228, Ubuntu glibc) | `77070004` `ErrConnectionClosed` — "Socket peer closed the connection or the connection is no longer usable." | unchanged, re-measured with the fixed compiler |
| **windows-x86_64** Schannel (box 2230, Win11 10.0.26100) | `77070003` `ErrNetworkFailed` — "Network operation failed **before a connection was established**." | `77070004` `ErrConnectionClosed` — "Socket peer closed the connection or the connection is no longer usable." |

The Windows message is worth reading twice: the connection *was* established and
had completed a handshake, so the string a program logs is not merely the wrong
code, it is a false statement about what happened.

The write **deadline** was measured on all three as well (`tls::setWriteTimeout`
+ a peer that stops draining), because a fix here must not swallow it:

| backend | `tls::setWriteTimeout` deadline, after the fix |
| --- | --- |
| macos-aarch64 | `77050008` `ErrTimeout` (3/3) |
| linux-x86_64 (2228) | `77050008` `ErrTimeout` |
| windows-x86_64 (2230) | `77050008` `ErrTimeout` |

That last row is a **second defect this bug fixes**: the Schannel write arm
consulted no `WSAGetLastError` at all, so `SO_SNDTIMEO` expiry (`WSAETIMEDOUT`,
10060) came out of the same blanket `ErrNetworkFailed` — contradicting
`mfb man tls setWriteTimeout`, which says a write reaching its deadline raises
`ErrTimeout`. The read side of that same backend was taught this in plan-110-D;
the write side never was. (The pre-fix Windows deadline value is a source reading
of `send_all(..., &fail)` → `ErrNetworkFailed`, not a measurement.)

### Which macOS failure path a departed peer takes — measured

Instrumented build, the two paths given distinct error codes:

- `CTX_ERROR != 0` after the send completion — **taken, 5/5**, on the first
  failing write.
- `CTX_STATE >= 4` terminal-state guard — **taken on every write after that**,
  and (2 in 5) on the first one too when Network.framework notices the dead peer
  before the program's next write. A second instrumented build split 4 from 5:
  the state is always **4 (`failed`)**, never 5 (`cancelled`).

So a fix that touches only the `CTX_ERROR` path reports `ErrConnectionClosed`
once and `ErrTlsFailed` for every retry after it — measured, per-write:
`ok 77070004 77020004 77020004 …` (77020004 was the instrumented state-guard
code). Both paths have to be classified. See the Corrections section for why
that is *not* the wholesale reclassification the non-goal forbids.

## Root Cause

Each backend collapses every write failure into one blanket code, chosen for the
failures that backend could historically produce:

- **macOS** — `src/codegen/builtins/tls/gen_macos/client.rs`, the `tls::write`
  emitter. Two failure sources both land on the single `write_fail` label
  (`ErrTlsFailed`): a connection already in a terminal state
  (`CTX_STATE >= 4`, failed/cancelled — the bug-386 guard) and a non-null
  `nw_error` left by the send completion in `CTX_ERROR`. Nothing reads the
  error's domain or code, so a POSIX `EPIPE`/`ECONNRESET` is indistinguishable
  from a certificate failure. Network.framework does expose it:
  `nw_error_get_error_domain` (1 = POSIX) + `nw_error_get_error_code`.
  Which of the two paths a departed peer actually takes is NOT yet measured and
  must be, since only one of them is safe to reclassify wholesale.
- **Windows** — `src/codegen/builtins/tls/gen_schannel_io.rs:lower_tls_write`.
  `send_all`'s failure and `EncryptMessage`'s negative return share one `fail`
  label (`ErrNetworkFailed`). No `WSAGetLastError` is consulted, so
  `WSAECONNRESET` (10054) / `WSAECONNABORTED` (10053) are not separated from a
  genuine protocol failure. The sibling read path already does classify — see
  `gen_schannel_read_close.rs:349`, which raises `ErrConnectionClosed`.
- **Linux** — already fixed by bug-467; kept here only as the reference shape.

## Goal

- `tls::write` raises `ErrConnectionClosed` when the peer has gone away, on
  macos-aarch64, windows-x86_64 and all three Linux targets.
- A certificate/protocol failure still raises `ErrTlsFailed`, and a write
  deadline still raises `ErrTimeout`, on every backend.

### Non-goals (must NOT change)

- **Do NOT "fix" this by relaxing the docs.** `mfb man tls write` now states the
  `ErrConnectionClosed` contract (bug-467) and `mfb man tcp write` states the
  same; the code must meet them.
- No change to the `tls::read` end-of-stream contract (bug-465 pinned it).
- No change to `ErrTimeout` for a `tls::setWriteTimeout` deadline
  (plan-110-D on macOS, bug-467 on OpenSSL).
- **Do not reclassify macOS's terminal-state guard wholesale.** `CTX_STATE >= 4`
  covers `failed` as well as `cancelled`, and a TLS handshake failure reaches
  `failed` too — turning that whole branch into `ErrConnectionClosed` would
  mislabel a certificate error as a disconnect, trading one wrong code for
  another.

## Blast Radius

Found by search (`grep -rn "ErrTlsFailed\|ErrNetworkFailed" src/codegen/builtins/tls/`).

- `gen_macos/client.rs` `tls::write` (`write_fail`) — **fixed by this bug.**
- `gen_schannel_io.rs:lower_tls_write` (`fail`) — **fixed by this bug.**
- `gen_openssl.rs:lower_tls_write_openssl` — already correct (bug-467);
  the reference for the other two.
- `tls::read` on all three backends — already classifies; unaffected.
- `tls::connect`/`listen`/`accept` — `ErrTlsFailed` there means a handshake or
  credential failure, which is correct; unaffected.
- `tcp`/`udp` — unaffected: they classify errno directly and already agree.

## Fix Design

Per-backend classification, mirroring what each backend's own `read` path
already does, with no shared abstraction (the three transports have nothing in
common at this layer):

- **macOS**: measure first — instrument which of `CTX_STATE >= 4` and
  `CTX_ERROR != 0` a departed peer takes. Then, on the `CTX_ERROR` path, dlsym
  `nw_error_get_error_domain`/`nw_error_get_error_code` and route domain
  POSIX(1) with code `EPIPE`(32) / `ECONNRESET`(54, **not** Linux's 104) /
  `ENOTCONN`(57) to a new `peer_closed` label.
- **Windows**: after `send_all` fails, call `WSAGetLastError` and route
  `WSAECONNRESET`(10054) / `WSAECONNABORTED`(10053) / `WSAESHUTDOWN`(10058) to a
  `peer_closed` label. `EncryptMessage`'s own negative return stays
  `ErrTlsFailed` — it is a protocol failure, not a transport one.

The correctness risk is in **which failures get reclassified**, not in the
emission: over-reaching turns a certificate error into a "peer closed", which is
a worse diagnosis than the current one. Expect `.ncodesum` drift on
macos-aarch64 and windows-x86_64 for every `tls`-importing fixture (`http`
imports `tls` transitively).

## Phases

### Phase 1 — measure all three rows, then a failing test

- [x] Run the reproduction on macos-aarch64 and on box 2230 (windows-x86_64) and
      fill in the matrix; determine which macOS failure path a departed peer takes.
- [x] Add a `tests/rt-behavior/tls/` fixture asserting `ErrConnectionClosed`.
      It needs a certificate pair and a peer that really goes away, so it will
      look like the `tcp` twin plus `examples/network-server/certs`.

Acceptance: every row measured; the fixture fails on macOS with `ErrTlsFailed`.
**Met.** The fixture mints its own identity at run time rather than carrying
`examples/network-server/certs` — see Correction 3. RED against the pre-fix
compiler: `write raised=FALSE` (3/3), with both positive assertions already TRUE
on that same build, so neither is satisfied by the fix rather than by the code
under it.

### Phase 2 — the fix

- [x] macOS: `nw_error` domain classification, recorded in the block trampolines
      (NOT on the `CTX_ERROR` path — see Correction 1, which disproves the
      design this line originally carried).
- [x] Windows: `WSAGetLastError` classification on the `send_all` path.

Acceptance: the Phase 1 fixture passes on every backend; a certificate failure
still raises `ErrTlsFailed` and a write deadline still raises `ErrTimeout`.
**Met**, measured on all three backends — see the matrix.

### Phase 3 — regenerate + validate

- [x] `scripts/regen-ncodesum.sh` + `scripts/artifact-gate.sh all` to 0 diffs.
- [x] `cargo test --release --no-fail-fast` and `scripts/test-accept.sh`.
- [x] Re-run the reproduction on macOS and box 2230.

Acceptance: full suite green; the reproduction reports `ErrConnectionClosed`
everywhere. **Met.** Golden delta: exactly **6** `.ncodesum` — {`byte-identity/tls`,
`byte-identity/http`, `byte-identity/resource-xfer-slots`} × {`macos-aarch64`,
`windows-x86_64`}. Predicted before regenerating and confirmed by building all
five targets of those three fixtures plus six untouched controls
(`strings`, `net`, `crypto`, `io`, `thread`, `os`): every Linux target and every
control was byte-identical. That is the expected set — the change is macOS-only
and Windows-only, and `http` reaches `tls` transitively through
`http::serverSSL`.

## Corrections

Written against the doc as it stood on 2026-08-31. Each is a claim that was
disproved by measurement while fixing it.

### 1. The Fix Design's macOS recipe would have shipped a use-after-free

The doc said: "on the `CTX_ERROR` path, dlsym
`nw_error_get_error_domain`/`nw_error_get_error_code` and route domain POSIX(1)
…". That cannot be done from `tls::write`. The `nw_error_t` a completion block is
handed is **borrowed for the block's duration**; Network.framework releases it on
return, so by the time `dispatch_semaphore_wait` returns, `CTX_ERROR` is a
dangling pointer. It has only ever been read as a non-null test, which is why
nothing had noticed.

Measured, not reasoned: a build that did exactly what the doc described SIGSEGV'd
in **2 of 6** runs, **3 of 5** under `MallocScribble=1`, and the runs that
survived disagreed about the answer for one identical scenario (POSIX 32, POSIX
54, and once a non-POSIX domain). That is a remotely-triggerable crash in a
server's write path.

The fix therefore records the domain **inside the block trampolines**
(`src/target/macos_aarch64/tls.rs`), on the dispatch queue, while the object is
alive: `nw_error_get_error_domain` is parked in the connection ctx
(`CTX_EDOMFN`) at setup and both `SEND_INVOKE` and `STATE_INVOKE` write the
domain into `CTX_EDOM`. `tls::write` classifies from that integer and never
touches the error object. The error *code* is not read at all — the domain alone
answers the question, and reading two values would double the trampoline's work
for no diagnosis.

### 2. The non-goal "do not reclassify the terminal-state guard" rested on a case that cannot happen at `tls::write`

The doc's reason was: "`CTX_STATE >= 4` covers `failed` as well as `cancelled`,
and a TLS handshake failure reaches `failed` too". A handshake failure cannot
reach `tls::write`'s guard: `tls::connect` (`gen_macos/client.rs`, the
`NW_STATE_READY` compare) and `tls::accept` (`gen_macos/server.rs`, same compare)
BOTH wait for the connection to reach ready and raise `ErrTlsFailed` on
failed/cancelled, so every `tls::Socket` that exists completed its handshake.

Measured, the guard is also not avoidable: against a departed peer it is what
every write after the first one hits, and 2 runs in 5 what the first one hits.
Leaving it alone means reporting `ErrConnectionClosed` once and `ErrTlsFailed`
forever after for one disconnect — a fixture that flaked 3 TRUE / 2 FALSE across
five runs on exactly that.

The guard is still **not** reclassified wholesale, which is the part of the
non-goal that survives: it raises `ErrConnectionClosed` only when a
`nw_error_domain_posix` error was actually recorded on that connection, and
`ErrTlsFailed` otherwise (including when no error was classified at all, which is
the pre-bug behaviour). A `dns`- or `tls`-domain failure keeps `ErrTlsFailed` on
both paths.

Splitting 4 from 5 was tried first and does not help: a departed peer always
produces state **4 (`failed`)**, never 5.

### 3. "An MFBASIC client cannot accept a self-signed certificate (bug-477)" is stale

bug-477 landed. `tls::connect` takes `allowSelfSigned`, and
`mfb man tls connect` documents it. That is what makes the Windows row
measurable at all — box 2230 has no `openssl` (`where openssl` → "Could not find
files for the given pattern(s)"), so `openssl s_client` is not available as the
peer there and an MFBASIC client is.

The regression fixture still uses `openssl s_client` as its peer, deliberately: a
proof where our client and our server agree with each other cannot tell a working
TLS implementation from two matching bugs. It also mints its own identity at run
time instead of reusing `examples/network-server/certs`, because every committed
certificate expires — and macOS refuses a server certificate whose validity
window exceeds ~398 days (`.ai/net-tls.md`), so a committed pair turns the
fixture red on a *date* rather than on a regression.

### 4. The Fix Design's Windows recipe was right about `send`, wrong about `EncryptMessage`

"`EncryptMessage`'s own negative return stays `ErrTlsFailed`" — it was never
`ErrTlsFailed`. Both arms shared one `fail` label emitting **`ErrNetworkFailed`**
(`gen_schannel_io.rs`). The fix leaves that untouched: reclassifying
`EncryptMessage` is a separate judgement this bug has no evidence for, and
changing it would be exactly the over-reach the doc warns about.

### 5. A second defect, in the same emitter: the Windows write deadline

`tls::setWriteTimeout` documents `ErrTimeout`. The Schannel write arm consulted
no `WSAGetLastError`, so `WSAETIMEDOUT` (10060) — which is what `SO_SNDTIMEO`
expiry reports on Winsock, not `EWOULDBLOCK` — was reported as
`ErrNetworkFailed`. Fixed in the same classification block and measured on box
2230: `77050008 ErrTimeout`.

### 6. Blast Radius: verified, and one entry corrected

`grep -rn "ErrTlsFailed\|ErrNetworkFailed" src/codegen/builtins/tls/` re-run on
the current tree matches the doc's list. The `gen_schannel_io.rs` line is
`:504` today (the doc cited the function, which is the durable citation). One
entry was wrong in substance rather than in location — see Correction 4.

## Validation Plan

- Regression test: the new `rt-behavior/tls` fixture, plus the existing
  `rt-behavior/tcp/tcp-write-peer-closed-raises-rt` as the plaintext mirror.
- Runtime proof: the reproduction above printing the `ErrConnectionClosed`
  message on macOS and Windows, as it already does on Linux.
- Doc sync: `mfb man tls write` already states the contract (verified by
  RENDERING it, not by grepping source) and needs no change — this bug makes two
  backends meet it. `src/docs/spec/stdlib/17_transports.md` did NOT state the
  write-side half of the mirror contract and now does, beside the end-of-stream
  paragraph it already carried. `.ai/net-tls.md` gains the per-backend row and
  the `nw_error` lifetime rule (Correction 1), which is the part a future session
  would otherwise rediscover with a segfault.
- Full suite: `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`.

## Open Decisions

- ~~**How wide to cast the "peer is gone" errno set.**~~ **Settled, differently
  per backend, because the two backends do not offer the same information.**
  - Windows keeps the explicit small set (`WSAECONNRESET` 10054 /
    `WSAECONNABORTED` 10053 / `WSAESHUTDOWN` 10058), plus `WSAETIMEDOUT` 10060 →
    `ErrTimeout`; anything else stays `ErrNetworkFailed`.
  - macOS classifies on the **domain**, not the code: `nw_error_domain_posix`
    is the transport, `dns`/`tls` are not. An errno list is the wrong instrument
    there — the code has to be read from a live object (Correction 1), so the
    cheapest safe read is the one that answers the question, and Apple's own
    domain split *is* the transport-vs-protocol distinction this bug needs.
    A measured aside: the POSIX code varies run to run between `EPIPE` (32) and
    `ECONNRESET` (54) for the identical scenario, so an EPIPE-only list would
    have been intermittently wrong.

## Summary

`tls::write` names a departed peer three different ways on three platforms, and
after bug-467 only the Linux one matches the `tcp`/`tls` mirror contract that
`tcp::write` and both `read` calls keep. Found while fixing bug-467, which made
the Linux row observable for the first time and therefore made the disagreement
visible. The risk is in choosing which failures to reclassify — over-reaching
would report a certificate failure as a disconnect — not in the emission, which
is a short classification block on each backend modelled on that backend's own
`read` path.
