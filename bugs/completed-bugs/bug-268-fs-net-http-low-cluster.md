# bug-268: fs/net/http residual LOW cluster — SSRF, HTTP no-timeouts, cooperative-only cancel, process-global chdir, stale comment

Last updated: 2026-07-17
Effort: medium (1h–2h, several small items)
Severity: LOW
Class: Security / robustness / dead-code

Status: Fixed (OS-11/OS-06 landed; OS-10/OS-08/OS-07 documented)
Regression Test: OS-11 exercised by
`tests/rt-behavior/http/http_server_loopback` (the client now connects with a
bounded timeout + read deadline and completes the loopback exchange, 5/5)

## Resolution

- **OS-11 (HTTP client no timeouts) — FIXED.** `http_package.mfb` adds
  `__HTTP_CONNECT_TIMEOUT_MS`/`__HTTP_READ_TIMEOUT_MS` (30 s). `__http_exchangeTcp`
  connects with the bounded timeout and sets a per-read deadline
  (`net::setReadTimeout`); `__http_exchangeTls` connects with the bounded timeout
  (the TLS surface has no per-read timeout; the 64 MiB response cap bounds a
  stalled read's memory). A slow/black-holed peer now fails with a timeout instead
  of wedging the thread. Composes with bug-261 (the connect default deadline).
- **OS-06 (stale net-fd leak comment) — FIXED.** The comment in `net/mod.rs` that
  claimed the socket fd leaks on the error paths was corrected: `op_fail` closes
  the fd and `socket_fail` frees the addrinfo, so nothing leaks.
- **OS-10 (HTTP SSRF) — DOCUMENTED (by design).** No correctness change: a
  default-deny host policy would break legitimate localhost clients. The http
  package man page now states plainly that the client applies no SSRF filtering
  (scheme-only URL validation, no redirect following) and that a caller forwarding
  untrusted URLs must apply its own host allow/deny policy.
- **OS-08 (cooperative cancel) — DOCUMENTED (by design).** `thread::cancel`/`drop`
  remain cooperative flags; the man page already documents the cooperative model.
  The bounded HTTP/connect deadlines above make a stalled exchange fail on its own
  rather than requiring forced preemption.
- **OS-07 (process-global chdir) — DOCUMENTED (by design).**
  `fs::setCurrentDirectory` man page now states the working directory is
  process-global, not per-thread, so relative-path fs ops are not thread-isolated
  and a program needing per-thread resolution must build absolute paths.

A batch of individually-LOW residual findings on the fs/net/http surface from
audit-2 that lack their own bug docs. Each item is independently addressable; two
(OS-07, OS-08) are current design choices tracked here so they are not
re-discovered as "unknown". Grouped per the repo's low-severity-batch convention
(cf. bug-180).

References:

- `planning/audit-2-fs-net-thread.md` (OS-06, OS-07, OS-08, OS-10, OS-11).

## Findings

### OS-11 — HTTP client has no connect/read timeouts (thread wedge)
- Location: `src/builtins/http_package.mfb:284` (`net::connectTcp` — no
  `timeoutMs`), `:308` (`tls::connect(..., 0, ...)`), read loop `:288-303`.
- Symptom: a slow/stalled peer or DNS blocks the calling thread indefinitely; with
  OS-08 (cooperative cancel) it cannot be interrupted. The read loop caps total at
  64 MiB but has no per-read deadline. HTTP-surface realization of OS-05
  (bug-261).
- Fix: thread a default connect/read deadline into
  `__http_exchangeTcp`/`__http_exchangeTls` (nonzero `connectTcp` timeout +
  `net::setReadTimeout`). Internal; no public parameter.

### OS-10 — HTTP client SSRF: scheme-only URL validation, no internal-address guard
- Location: `src/builtins/net_package.mfb:86-165` (`__net_toUrl` validates scheme
  only, `:91-93`); dial at `http_package.mfb:284`/`:308`.
- Symptom: `http::read`/`write` will connect to `127.0.0.1`,
  `169.254.169.254` (cloud metadata), RFC-1918/link-local — a program building a
  `Url` from untrusted input can be steered at internal services. **Scope-limited:
  the client does not follow redirects** (3xx returned verbatim,
  `__http_reasonPhrase:936-956`), so there is no redirect-based amplification.
- Fix: no correctness change required; any host allow/deny must be **opt-in** to
  avoid breaking localhost clients. Document the absence of SSRF filtering; if an
  opt-in guard is added, key it off an explicit policy argument.

### OS-08 — thread.cancel/drop are cooperative flags only (by design)
- Location: `thread.cancel`/`thread.drop` set a flag; a worker blocked in a
  syscall (accept/connect/read) is not preempted.
- Symptom: this is what makes the OS-02/OS-05/OS-11 wedges unrecoverable. Tracked
  as a design limitation, not a defect to silently fix.
- Fix (if pursued): pair cancellation with the bounded default timeouts (bug-185,
  bug-261, OS-11 above) so a cooperative cancel is checkable at each deadline,
  rather than adding forced preemption.

### OS-07 — fs::setCurrentDirectory is a process-global chdir (by design)
- Location: `fs::setCurrentDirectory` → process-global `chdir`.
- Symptom: breaks per-thread CWD isolation (all threads share one CWD). Design
  limitation; documented so callers know relative-path fs ops are not
  thread-isolated.
- Fix (if pursued): prefer `*at`-family calls with an explicit dir fd for
  thread-scoped relative resolution; otherwise document the global-CWD contract.

### OS-06 — stale "leaks" comment on the net fd path (dead-code/cleanup)
- Location: `src/target/shared/code/net/mod.rs:692-694` (comment) vs `:695-742`
  (`op_fail`/`connect_timeout` both `close` the fd).
- Symptom: the in-code comment claims a leak that does not exist — the fd is
  stored before any branch and every path closes it (verified: OS-06 not
  reproducible). Misleading only.
- Fix: delete the stale comment.

## Goal

- HTTP client applies bounded default connect/read deadlines (OS-11).
- The SSRF exposure (OS-10) and the by-design cooperative-cancel/global-chdir
  limitations (OS-08/OS-07) are documented; the stale net-fd comment (OS-06) is
  removed.

### Non-goals (must NOT change)

- A default-deny host policy (would break legitimate localhost clients).
- Forced thread preemption / a per-thread CWD model as a prerequisite.
- Any public fs/net/http signature.
