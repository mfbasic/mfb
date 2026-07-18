# bug-317: TLS handshake-failure error paths leak native connection/session objects (macOS accept = remote server DoS)

Last updated: 2026-07-18
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Memory-safety (native-object leak) / Security (DoS)

Status: Open
Regression Test: tests/ (new) — repeated TLS handshake failures do not grow native-object count

The TLS connect/accept error exits cancel the connection but never release the
native framework objects they own, so each remotely-triggerable handshake failure
leaks one connection/session object. The standout is the macOS server-side
`tls::accept` path: an attacker flooding handshake-failing connections leaks one
`nw_connection` each, an unbounded server-side memory-exhaustion DoS. The success
and OOM paths free correctly (bug-55/236); only the handshake-failure exits were
missed. This surface is otherwise unusually well-hardened — verification is
fail-closed with no bypass (confirmed) — so these error-path leaks are the residual.

The single correct behavior a fix produces: every TLS connect/accept error exit
releases the native objects it owns (connection, session, context, queue), exactly
as the success/close and OOM paths do — so repeated handshake failures do not grow
native-object count.

References:

- `bugs/completed-bugs/bug-55-*` (fixed the connect success/OOM SSL+CTX/nw_release
  paths), `bug-202` (added the accept handshake timeout), `bug-236` (deferred macOS
  listen error-path releases as bounded/one-shot).
- `planning/old-plans/audit-2-crypto-tls.md`.
- Found during goal-06 review of `src/target/shared/code/tls/**` and
  `crypto_ec/macos.rs`.

## Items

### T1 — macOS `tls::accept` handshake-failure exits leak the accepted `nw_connection` (remote DoS, MEDIUM)
- `src/target/shared/code/tls/macos.rs:3254-3278` (`conn_fail`), `:3279-3303`
  (`hs_timeout`), in `lower_tls_accept_macos`.
- `accept` owns a `+1` retain on the popped connection (the new-connection trampoline
  `nw_retain`s it into the ring; `closeListener` drains with `nw_release`). The
  success path releases it at close (`nw_release(CONN)`), but `conn_fail`/`hs_timeout`
  only `nw_connection_cancel(CONN)` then `emit_fail` — cancel tears down network
  activity but does not drop the retain, so the `nw_connection` object leaks. A server
  looping on `tls::accept` catches the error and continues; an attacker floods
  handshake-failing connections → unbounded growth.
- Fix: in both `conn_fail` and `hs_timeout`, after `nw_connection_cancel(CONN)`,
  `nw_release(CONN)` (symbol already resolvable on this path).

### T2 — OpenSSL `tls::connect` `tls_fail` leaks the `SSL` session and per-connection `SSL_CTX`
- `src/target/shared/code/tls/openssl.rs:621-642` (`tls_fail`), in
  `lower_tls_connect_helper`.
- `tls_fail` only `close(fd)` + `emit_fail`. It is branched to after `SSL_new`,
  `SSL_set_fd`, `SSL_set1_host`, min-proto ctrl, `SSL_connect`, and
  `SSL_get_verify_result` — at all of which the `SSL` and the freshly-created client
  `SSL_CTX` exist and are owned. The success path and the sibling `alloc_fail` free
  both; `tls_fail` frees neither, leaking one `SSL` + one `SSL_CTX` (several KB) per
  failure (OpenSSL heap, not arena). A client reconnect loop against an
  expired/untrusted-cert host leaks unboundedly. (The OpenSSL `accept` `ssl_fail`
  path *does* `SSL_free` — the two are inconsistent.)
- Fix: give `tls_fail` a null-guarded `SSL_free(ssl)` + `SSL_CTX_free(ctx)` prologue,
  mirroring `alloc_fail`.

### T3 — macOS `tls::connect` `conn_fail`/`conn_timeout` leak the `nw_connection` and `dispatch_queue`
- `src/target/shared/code/tls/macos.rs:942-967` (`conn_fail`), `:968-994`
  (`conn_timeout`), in `lower_tls_connect_macos`.
- By these labels the endpoint/params are released, but `CONN` (owned `+1` from
  `nw_connection_create`) and `QUEUE` (`dispatch_queue_create`) are live. The success
  path releases them at close; the failure exits only `nw_connection_cancel(CONN)` +
  `emit_fail`, leaking the connection and queue. Unbounded under a client reconnect
  loop.
- Fix: in both, after cancel, `nw_release(CONN)` and `dispatch_release(QUEUE)`.

### T4 — macOS crypto `verify` omits the explicit public-key length precheck (defense-in-depth, LOW latent)
- `src/target/shared/code/crypto_ec/macos.rs:1096-1214` (`verify`).
- The OpenSSL backend guards `PUBLEN == point_len` before splicing; the macOS
  `verify` passes the raw public bytes straight to `SecKeyCreateWithData`, which
  validates the SEC1 point (wrong length/off-curve → NULL → `invalid_fail`). No OOB
  (the bytes are in an exactly-`PUBLEN` arena buffer). Purely a consistency gap.
- Fix (optional): add a matching `emit_len_check(PUBLEN, point_len, &invalid_fail)`
  for parity, or leave as library-validated.

## Goal

- Every TLS connect/accept error exit releases the native objects it owns; the macOS
  accept server DoS (T1) is closed.

### Non-goals (must NOT change)

- The (verified fail-closed) verification logic — no bypass exists; do not touch it.
- The success/close/OOM paths (already correct).
- The one-per-connection `dispatch_semaphore` accepted tradeoff (separate from the
  connection object).

## Blast Radius

- T1/T3 (`tls/macos.rs`), T2 (`tls/openssl.rs`), T4 (`crypto_ec/macos.rs`) — cited
  sites. The macOS `listen` error paths (bug-236, bounded/one-shot) are out of scope;
  confirm they stay so.

## Fix Design

Add the missing `nw_release`/`dispatch_release`/`SSL_free`/`SSL_CTX_free` calls to
each error exit, null-guarded, mirroring the success/OOM paths that already do it.
Rejected alternative: a shared cleanup label — the error exits reach different points
with different live objects; per-exit release is clearer and matches the existing
`alloc_fail` pattern.

## Phases

### Phase 1 — failing test + audit
- [ ] A test that drives repeated handshake failures (a stub server presenting a bad
      cert / stalling) and asserts bounded native-object growth. Audit all TLS
      connect/accept error exits for the same class.
### Phase 2 — the fixes
- [ ] Add the releases to T1/T2/T3 (and optionally T4's precheck).
### Phase 3 — validation
- [ ] Full suite green; success/close paths unchanged; no double-free.

## Validation Plan

- Regression: repeated-handshake-failure object-count test (macOS accept; OpenSSL
  connect; macOS connect).
- Runtime proof: server survives a handshake-failure flood without unbounded growth.
- Doc sync: none.

## Summary

The TLS error paths cancel but don't release native objects; the macOS accept path is
a remotely-triggerable server-side DoS (T1), with client-side reconnect-loop leaks in
OpenSSL/macOS connect (T2/T3) and a latent crypto consistency gap (T4). Adding the
missing releases — mirroring the success/OOM paths — closes them.
