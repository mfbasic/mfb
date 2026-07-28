# bug-202: OpenSSL server TLS handshake (SSL_accept) has no timeout → a stalled client wedges the accept loop

Last updated: 2026-07-14
Effort: medium (1h–2h)
Severity: MEDIUM
Class: security (DoS)

Status: Fixed (2026-07-15) — lower_tls_accept_helper now bounds the blocking
server handshake with SO_RCVTIMEO/SO_SNDTIMEO on the accepted `connfd`, derived
from `timeoutMs` (tv_sec = ms/1000, tv_usec = (ms%1000)*1000) via the same
`emit_set_sock_timeouts` helper `connect` uses, and clears them once the handshake
completes so the returned socket's reads/writes stay unbounded. `timeoutMs <= 0`
skips both (blocking semantics unchanged, per the non-goals). A new
`TIMEVAL_OFFSET` (72) uses free space in the existing 96-byte frame.
Regression Test: 15 tls acceptance tests pass. NOTE: an end-to-end stalled-client
run could not be performed because `tls::listen` + `tls::accept` in one program
currently fails to build with "native code data relocation target
'_mfb_str_error_tls_failed' is not a data object or defined symbol" — verified
PRE-EXISTING (reproduced with this fix stashed), i.e. a separate latent defect in
the ERR_TLS_FAILED string's emission gate for the listen/accept path, filed as
bug-249.
Regression Test: tests/rt-behavior/ (Linux tls accept with a mid-handshake stall returns within timeout)

On Linux the server TLS handshake (`SSL_accept`) runs on a blocking socket with
no `SO_RCVTIMEO`/`SO_SNDTIMEO`, so `timeoutMs` bounds only the connection-wait
`poll`, never the handshake itself. A remote client that completes the TCP
connection then stalls mid-TLS-handshake blocks `SSL_accept` indefinitely,
wedging the single-threaded accept loop despite a finite `timeoutMs`. The macOS
accept path bounds this same handshake with its `DEADLINE`
(`macos.rs` `hs_loop`), and `connect` bounds `SSL_connect` via
`emit_set_sock_timeouts` — accept is the outlier. (Distinct from bug-185, which
is the plain-TCP `net.accept` path.)

## Failing Reproduction

Linux server: `tls::accept(l, 500)`. A client completes the TCP handshake
(satisfying `poll(POLLIN, timeoutMs)` at `:1387`) then sends nothing. Observed:
`SSL_accept` blocks forever; the accept loop never returns. Expected: the
handshake is bounded by `timeoutMs` and a stalled client yields a catchable
timeout.

## Root Cause

`src/target/shared/code/tls/openssl.rs:1470-1488` `lower_tls_accept_helper` —
`SSL_accept` on `connfd` with no `SO_RCVTIMEO`/`SO_SNDTIMEO` set from `timeoutMs`.

## Non-goals

- Do not change the macOS path or `connect` (already bounded).
- Do not change blocking semantics when `timeoutMs <= 0`.

## Blast Radius

- `lower_tls_accept_helper` (OpenSSL) only.

## Fix Design

Set `SO_RCVTIMEO`/`SO_SNDTIMEO` from `timeoutMs` on `connfd` around
`SSL_accept` (mirror the connect handshake-timeout wrapping), clearing them
after the handshake.
