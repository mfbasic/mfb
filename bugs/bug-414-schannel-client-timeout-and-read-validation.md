# bug-414: Schannel TLS client defects — `tls::connect` ignores `timeoutMs` (unbounded block), and `tls::read` skips the `maxBytes > 0` check

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (unbounded blocking / DoS) + cross-platform divergence

Status: Open
Regression Test: tests/ — a Windows TLS connect to a blackhole host with a small
`timeoutMs` must fail within the timeout; `tls::read` with `maxBytes <= 0` must
raise `ErrInvalidArgument`.

Two Windows/Schannel client defects in `src/target/shared/code/tls/`, batched
(same subsystem):

### (1) `schannel_impl.rs:192` — `timeoutMs` silently discarded (MED)
`lower_tls_connect` stores `timeoutMs` at `TIMEOUT` (:188) then discards it with
`let _ = TIMEOUT;` (:192). The subsequent `socket_connect` (:10) is a plain blocking
`connect`, and the handshake `recv` loop (:296-311) blocks with no `SO_RCVTIMEO`/
poll bound. The Linux/OpenSSL path honors `timeoutMs` (non-blocking connect + poll,
then `SO_RCVTIMEO`/`SO_SNDTIMEO` around the handshake — `openssl.rs:145-324`), and
the Schannel `accept` path honors it via `WSAPoll` (`schannel_server.rs:655`). Only
the Windows client `connect` ignores it: a blackhole or slow-loris server hangs the
calling thread despite a caller-supplied timeout — the same forever-wait class as
bug-202/bug-386.
- Fix: honor `timeoutMs` on the Windows connect + handshake (non-blocking connect +
  `WSAPoll`, then `SO_RCVTIMEO`/`SO_SNDTIMEO`), mirroring the OpenSSL path.

### (2) `schannel_read_close.rs:33` — no `maxBytes > 0` validation (LOW)
`lower_tls_read` stores `maxBytes` at `MAX` (:35) but never checks `maxBytes > 0`.
OpenSSL rejects `maxBytes <= 0` with `ErrInvalidArgument` (`openssl.rs:1834-1836`).
On Schannel, `maxBytes == 0` runs a full blocking `recv`+`DecryptMessage` then serves
0 bytes as OK; a negative `maxBytes` (→ huge `NOUT`) routes to `alloc_fail`/
`ErrOutOfMemory`. No memory-safety impact, but the same call yields
`ErrInvalidArgument` on Linux/macOS vs empty-OK / `ErrOutOfMemory` on Windows.
- Fix: add the `maxBytes <= 0 → ErrInvalidArgument` guard at read entry, matching
  OpenSSL.

References: `src/target/shared/code/tls/schannel_impl.rs:192`,
`src/target/shared/code/tls/schannel_read_close.rs:33`; contrast `openssl.rs:145-324`
/ `:1834-1836`, `schannel_server.rs:655`. Found during goal-07.

## Failing Reproduction

Windows/Schannel-only; not reproducible on the macOS host. Confirmed statically:
`let _ = TIMEOUT;` discard + no poll/setsockopt-timeout on connect (item 1); no
`<= 0` guard on `MAX` in the read body (item 2).

- Observed: (1) connect blocks past `timeoutMs`; (2) `read(maxBytes<=0)` returns
  empty-OK or `ErrOutOfMemory`.
- Expected: (1) connect fails within `timeoutMs`; (2) `ErrInvalidArgument`.

## Root Cause

The Windows client connect path never wires `timeoutMs` into the socket, and the
read path omits the argument-range check both other backends apply.

## Goal

- Windows `tls::connect` bounds connect+handshake by `timeoutMs`; `tls::read`
  rejects `maxBytes <= 0` with `ErrInvalidArgument`.

### Non-goals

- The already-correct OpenSSL/accept timeout paths.

## Blast Radius

- `schannel_impl.rs:192` (connect timeout), `schannel_read_close.rs:33` (read
  validation). The accept path (`schannel_server.rs`) already honors the timeout.
