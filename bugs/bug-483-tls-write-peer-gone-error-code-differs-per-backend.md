# bug-483: `tls::write` to a departed peer reports a different error code on each of the three TLS backends

Last updated: 2026-08-31
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (cross-platform contract divergence)

Status: Open
Regression Test: — (none yet; the plaintext half is pinned by
`tests/rt-behavior/tcp/tcp-write-peer-closed-raises-rt`)

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

- [ ] Run the reproduction on macos-aarch64 and on box 2230 (windows-x86_64) and
      fill in the matrix; determine which macOS failure path a departed peer takes.
- [ ] Add a `tests/rt-behavior/tls/` fixture asserting `ErrConnectionClosed`.
      It needs a certificate pair and a peer that really goes away, so it will
      look like the `tcp` twin plus `examples/network-server/certs`.

Acceptance: every row measured; the fixture fails on macOS with `ErrTlsFailed`.
Commit: —

### Phase 2 — the fix

- [ ] macOS: `nw_error` domain/code classification on the `CTX_ERROR` path.
- [ ] Windows: `WSAGetLastError` classification on the `send_all` path.

Acceptance: the Phase 1 fixture passes on every backend; a certificate failure
still raises `ErrTlsFailed` and a write deadline still raises `ErrTimeout`.
Commit: —

### Phase 3 — regenerate + validate

- [ ] `scripts/regen-ncodesum.sh` + `scripts/artifact-gate.sh all` to 0 diffs.
- [ ] `cargo test --release --no-fail-fast` and `scripts/test-accept.sh`.
- [ ] Re-run the reproduction on macOS and box 2230.

Acceptance: full suite green; the reproduction reports `ErrConnectionClosed`
everywhere.
Commit: —

## Validation Plan

- Regression test: the new `rt-behavior/tls` fixture, plus the existing
  `rt-behavior/tcp/tcp-write-peer-closed-raises-rt` as the plaintext mirror.
- Runtime proof: the reproduction above printing the `ErrConnectionClosed`
  message on macOS and Windows, as it already does on Linux.
- Doc sync: none expected — `mfb man tls write` already states the contract
  (bug-467); this bug makes two backends meet it.
- Full suite: `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`.

## Open Decisions

- **How wide to cast the "peer is gone" errno set.** Recommended: the explicit
  small set above (EPIPE/ECONNRESET/ENOTCONN, WSAECONNRESET/ABORTED/SHUTDOWN),
  because `tls`'s alternative code (`ErrTlsFailed`) is a meaningful diagnosis
  worth keeping accurate. Alternative: mirror `tcp::write` exactly and treat
  every transport errno that is not a timeout as `ErrConnectionClosed`, which is
  simpler and matches the plaintext socket precisely.

## Summary

`tls::write` names a departed peer three different ways on three platforms, and
after bug-467 only the Linux one matches the `tcp`/`tls` mirror contract that
`tcp::write` and both `read` calls keep. Found while fixing bug-467, which made
the Linux row observable for the first time and therefore made the disagreement
visible. The risk is in choosing which failures to reclassify — over-reaching
would report a certificate failure as a disconnect — not in the emission, which
is a short classification block on each backend modelled on that backend's own
`read` path.
