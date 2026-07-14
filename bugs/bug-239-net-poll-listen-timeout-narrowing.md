# bug-239: net.poll/connect/listen narrow a 64-bit Integer timeout/backlog to a C `int` silently

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun

Status: Open

A 64-bit MFBASIC Integer timeout/backlog is passed unchanged in the arg register
to `poll`/`listen`, whose C prototypes read only a 32-bit `int`, so a large
positive value is silently narrowed.

Trigger: `net.poll(sock, 2147483648)` (or any `timeoutMs` with bit 31 set, e.g. a
computed ~24.8-day timeout) — poll's `int timeout` sees a negative value and
blocks forever instead of timing out; same class for the non-blocking-connect
poll timeout and the `listen` backlog.

Sites: `src/target/shared/code/net/poll.rs:66`, `net/io.rs:572` (connect poll),
`net/io.rs:476` (listen backlog).

Fix: clamp the timeout to `INT_MAX` (and floor at 0) before the poll/connect
call, or validate the 32-bit range at the language boundary.
