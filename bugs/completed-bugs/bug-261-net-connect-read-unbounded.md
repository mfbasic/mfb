# bug-261: `net.connect` with non-positive timeout blocks unbounded; `net.read` allocates caller-supplied `maxBytes` up front

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Security (availability)

Status: Fixed
Regression Test: `tests/rt-behavior/net/func_net_connectTcp_valid` (all connect
overloads, including the no-timeout forms, now route through the poll path and
succeed on loopback), `func_net_read_valid` / `func_net_readText_valid` (capped
read buffer returns the same bytes)

## Resolution

Connect (`net/mod.rs`): every connect now takes the non-blocking-connect + `poll`
path. A positive `timeoutMs` is honored as-is; a non-positive one (including the
omitted-argument overload, which passes 0) is replaced with the bounded
`DEFAULT_CONNECT_TIMEOUT_MS` (120 s) and raises `ErrTimeout` on elapse instead of
blocking indefinitely. The old unbounded blocking-connect path was removed.

Read (`net/io.rs`): the temporary read buffer is clamped to `READ_CHUNK_CAP`
(1 MiB) — used for both the allocation and the `read()` length — so a large or
attacker-influenced `maxBytes` no longer pre-commits that much memory for a
receive that delivers far fewer bytes. A single `read()` never returns more than
the socket receive buffer, so the documented "one underlying receive" semantics
are unchanged. `connectTcp` man page updated to describe the bounded default.

Note: the read-serialization/SSRF-adjacent items OS-08/OS-07 remain by-design and
are tracked in bug-268; this bug covers OS-05 (connect + read).

Two unbounded-resource footguns on the net surface. (1) `net.connect` called with
`timeoutMs <= 0` takes a fully blocking connect path with no ceiling — a stalled
DNS or a black-holed peer wedges the calling thread indefinitely, and with
cooperative-only cancellation (OS-08) the thread cannot be interrupted. (2)
`net.read(sock, maxBytes)` allocates `maxBytes` bytes eagerly before reading, so a
program that passes a large attacker-influenced `maxBytes` (or a fixed large cap)
commits that much memory regardless of how few bytes actually arrive. The single
correct behavior a fix produces: a connect has a bounded default deadline, and a
read grows its buffer to the bytes actually received rather than pre-allocating
the caller's ceiling.

References:

- `planning/audit-2-fs-net-thread.md` (OS-05; realized on HTTP as OS-11 →
  bug-268).
- `src/target/shared/code/net/mod.rs:493-499` — `timeoutMs <= 0` → unbounded
  blocking connect.
- `src/target/shared/code/net/io.rs:314-318` — `net.read` allocates `maxBytes`
  before the recv.

## Failing Reproduction

Connect: `net::connectTcp(host, port, 0)` against an IP that silently drops SYNs
→ the thread blocks past any reasonable deadline (minutes, OS-default). Expected:
a bounded default connect deadline with a clean timeout error.

Read: `net::read(sock, 1073741824)` on a socket that will deliver 10 bytes →
1 GiB is allocated immediately. Expected: allocation tracks the ~10 bytes read.

Contrast: a positive `timeoutMs` connect path already bounds the wait; the fix is
to give the non-positive case a sane default rather than "forever".

## Root Cause

`net/mod.rs:493-499` treats `timeoutMs <= 0` as "block forever" instead of
applying a default deadline. `net/io.rs:314-318` sizes the read buffer to the
caller's `maxBytes` argument up front rather than reading into a growable buffer
capped by `maxBytes`.

## Goal

- A `net.connect` with a non-positive timeout applies a bounded default connect
  deadline (e.g. via non-blocking connect + `poll`), returning a timeout error
  instead of blocking indefinitely.
- `net.read` allocates proportional to bytes actually received (chunked/growable,
  still capped by `maxBytes`), not `maxBytes` up front.

### Non-goals (must NOT change)

- The public `connect`/`read` signatures or the meaning of an explicit positive
  `timeoutMs`.
- The 64 MiB HTTP response cap (a separate ceiling).

## Fix Design

Connect: route the `timeoutMs <= 0` branch through the same non-blocking-connect
+ `poll(POLLOUT)` deadline machinery the positive branch uses, seeded with a
compile-time default (matching the accept-timeout work in bug-185). Read: read in
bounded chunks into a `Vec`-style growable buffer, stopping at EOF or `maxBytes`;
this also removes the pre-allocation amplifier. Pairs with bug-268 (OS-11 HTTP
client timeouts), which is the HTTP-surface realization of the connect half.
