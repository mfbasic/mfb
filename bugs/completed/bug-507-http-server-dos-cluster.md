# bug-507: HTTP server DoS cluster — one malformed chunk kills the process, no timeout (slowloris), quadratic head rescan, no caps (OS-51/52/56)

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (denial of service)

Status: FIXED — see the STATUS block at the end (found in audit-3, Surface 4 OS-51/52/56; mechanisms code-verified, OS-56 measured by the agent)

Regression Test: fixtures asserting a malformed chunk-size returns an error to the client (not a process abort), an idle connection is dropped on a read timeout, and an oversized head is rejected.

## Summary

An MFBASIC HTTP server built on `http::handleRequest` can be taken down by any
anonymous client:

- **OS-51 — one malformed chunk-size line kills the whole process.**
  `__http_frameComplete` raises out of both untrapped read loops, so a 58-byte
  request (or a 4-byte response field on the client) aborts the process.
- **OS-52 — no read/write timeout (slowloris).** A single idle connection wedges
  the single-threaded accept loop indefinitely.
- **OS-56 — quadratic head rescan + no caps.** The head is re-scanned from the
  start on every read with no head/header-count/line/connection cap: 2 MiB → 0.7 s,
  8 MiB → 11 s, 64 MiB ≈ 12 min of pegged CPU (measured).

## Mechanism

- OS-51: `helper_hex_to_int.rs:21` / `helper_chunked_complete.rs:33` raise on a
  bad chunk size; `func_handle_request.rs:133` calls them inside a loop with no
  `TRAP`, so the raise propagates to process exit.
- OS-52: `func_handle_request.rs:123` reads with no deadline; `helper_limits.rs:20`
  defines no idle/read timeout.
- OS-56: `func_handle_request.rs:127` re-scans via `helper_index_of_bytes.rs:17`
  each read (O(n²)); `helper_limits.rs:11` caps nothing (no max head size, header
  count, line length, or concurrent connections).

## Reproduction

OS-56 measured by the agent (2 MiB→0.7 s … 64 MiB≈12 min). OS-51/52 code-verified
(the untrapped raise site and the deadline-free read). A full PoC needs a socket
peer; the parser-level facts are read directly.

## Best fix

- Wrap the per-connection read/parse in a `TRAP` that closes the connection and
  continues the accept loop instead of aborting the process (OS-51).
- Add a read/idle/total-request timeout and drop a connection that exceeds it
  (OS-52).
- Cap head size, header count, header-line length, chunk-size digits, and
  concurrent connections in `helper_limits.rs`; parse the head incrementally
  (track the scan offset) instead of re-scanning from 0 (OS-56).

## Non-goals

No MFBASIC surface change; keep behaviour for well-formed requests within the
caps; do not change the default response bytes.

## Prior art

audit-2 REPO-13 capped the *registry's* publish/validate (bug-188); the `http`
server package caps were uncovered. Searched `helper_limits`, `timeout`,
`chunk`, `slowloris`, `handleRequest`.

## Reproduction (2026-09-03, fix session)

All three reproduced against a scratch `http::handleRequest` server
(`/tmp/b506-repro`, release `mfb` at main `4efc93966`) driven by python sockets:

- OS-51: `POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\nZZ\r\n`
  → the server process printed `Error: 7-705-0003 / invalid chunk size` and
  exited 255; the client got an empty reply.
- OS-52: one connection sending `GET / HTTP/1.1\r\nHost: x\r\n` and going silent
  → a second client's well-formed request was not answered within 3 s (nor
  40 s in the RED test).
- OS-56: an unterminated head of 1 MiB → 0.24 s, 2 MiB → 1.16 s (≈4.8× for 2×:
  quadratic), and nothing capped the head, the header count, or a line.

## Fix

- OS-51: the per-connection read loop moved into `__http_readRequestNet`/`Tls`,
  which reports its outcome in `__http_ReadResult.status` (0 / 400 / 408 / 413
  / 431) and raises nothing; `__http_handleRequest`/`SSL` TRAP the read, the
  parse/dispatch (→ 500) and the head serialization, so a raise anywhere costs
  the one connection. `__http_frameComplete` (client `done`) traps a bad chunk
  size and reports the frame complete so `finish` raises the ordinary
  `ErrInvalidFormat` instead of `done` raising.
- OS-52: `tcp::setReadTimeout`/`tls::setReadTimeout` per read
  (`__HTTP_SERVER_IDLE_TIMEOUT_MS` = 10 s) plus a `datetime::monotonicNanos`
  whole-request deadline (`__HTTP_SERVER_REQUEST_TIMEOUT_MS` = 60 s); expiry is
  answered `408 Request Timeout` (silently closed if not a byte arrived).
- OS-56: `__http_frameAdvance` carries `scanFrom` (head search resumes three
  bytes back) and `cursor` (chunk walk resumes at the last complete boundary,
  `__http_chunkedScan`) across reads — one pass per byte. Caps in
  `helper_limits.rs`: `__HTTP_MAX_HEAD` 64 KiB, `__HTTP_MAX_HEADERS` 100,
  `__HTTP_MAX_HEADER_LINE` 8 KiB (also bounds a chunk-size line); each answered
  `431 Request Header Fields Too Large` as soon as it is seen; a `Content-Length`
  past the 64 MiB request cap is `413` before the body is read.
- An early rejection is followed by a bounded lingering close
  (`__http_lingerNet`/`Tls`: ≤ 4 MiB, 500 ms per read) so the 4xx reaches a client
  that is still sending instead of an RST (measured: without it the python peer
  saw `ConnectionResetError`, never the 431).

Not done, deliberately: a *concurrent-connection* cap — `handleRequest` is
single-threaded and serves one connection per call, so there is nothing to cap;
and a `tls::accept` handshake deadline — `tls::accept(listener, timeoutMs)`
bounds the wait for a client AND its handshake with one number, so bounding the
handshake would also turn "no client yet" into an error and change the
documented blocking-accept contract. A client that connects and never completes
the TLS handshake can still hold `__http_handleRequestSSL` in `tls::accept`; that
is a separate, TLS-layer hazard.

Regression test: `tests/rt_http_server_dos.rs` (RED on main for OS-51, OS-52 and
each 431 cap; the under-cap and tiny-chunk exchanges are pins). Docs:
`handleRequest` descriptor, `src/docs/spec/stdlib/05_http.md`, `.ai/net-tls.md`.

## STATUS: FIXED (624b2dd3f)

Fixed together with bug-506 in one commit: the two bugs share the server read
loop (`__http_readRequestNet`/`Tls` + `__http_frameAdvance`), so the strict
framing rules (506) and the trapped, bounded, incremental scan (507) were
restructured as one change rather than stacking an intermediate loop that
would have been rewritten a second time. Deviation from the fix-bug skill's
one-bug-at-a-time order, deliberate and reported.

Gates on the branch (worktree-B-506, main merged in at 01d1b8716):
`cargo test --no-fail-fast -- --skip artifact_gate_all` → 4598 passed, 0
failed, cargo exit 0 (`/tmp/b506-full.log`); the new RED tests green before and
after the merge; `cargo check --all-targets` clean; `test-accept.sh '*http*'`
14/14; `regen-ncodesum.sh` under bash refreshed 141 goldens of which only the 5
`byte-identity/http` sums moved; `artifact-gate.sh target/release/mfb all` — see
the landing report.
