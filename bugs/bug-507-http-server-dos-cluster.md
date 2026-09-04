# bug-507: HTTP server DoS cluster — one malformed chunk kills the process, no timeout (slowloris), quadratic head rescan, no caps (OS-51/52/56)

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (denial of service)

Status: Open (found in audit-3, Surface 4 OS-51/52/56; mechanisms code-verified, OS-56 measured by the agent)

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
