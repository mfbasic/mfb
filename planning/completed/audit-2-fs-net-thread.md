# Audit 2 — Surface 4: Filesystem / network / thread runtime helpers

Last updated: 2026-07-14
Untrusted party: a remote net/http peer, or attacker-controlled paths/filenames.
Must not: escape an intended directory (traversal/TOCTOU), create world-writable
secrets, wedge a handler indefinitely (missing timeout), trigger SSRF, or corrupt
cross-thread ownership transfer.

Scope read: `src/target/shared/code/{fs_helpers_io, fs_helpers_paths,
fs_helpers_atomic, builder_fs_paths, os, stdin_broadcast}.rs`,
`src/target/shared/code/net/{io,mod,poll}.rs`,
`src/builtins/{fs,net,http,thread,os,io}*`, `src/builtins/http_package.mfb`,
`src/builtins/net_package.mfb`, the `*_specs.rs` runtime specs.

## Verdict on prior audit-1 findings (re-verified)

| ID | Prior sev | Verdict | Evidence |
|----|-----------|---------|----------|
| OS-01 | HIGH | **STILL PRESENT** | file-creating paths pass mode `"438"` (0o666): `fs_helpers_io.rs:700`, `fs_helpers_atomic.rs:920,1442`. Only `createTempFile`/`atomicWrite` are 0o600. → **bug-184**. |
| OS-02 | HIGH | **STILL PRESENT** | `net.accept` stores `timeoutMs` to a stack slot (`net/io.rs:47`) and never reads it; the accept is a bare blocking `accept(fd,NULL,NULL)` (`:55-61`). No poll/`SO_RCVTIMEO`. → **bug-185**. |
| OS-03 | MEDIUM | **STILL PRESENT** | no `openat2`/`RESOLVE_BENEATH`; `isWithin` is a `realpath`-based check (`fs_helpers_paths.rs:1410`); check-then-open TOCTOU. |
| OS-04 | MEDIUM | **STILL PRESENT** | `openFileNoFollow` adds `O_NOFOLLOW` to the terminal `open` only (`fs_helpers_io.rs:2191`); intermediate dir symlinks still followed. |
| OS-05 | MEDIUM | **STILL PRESENT** | `net.connect` with `timeoutMs<=0` is unbounded blocking (`net/mod.rs:493-499`); `net.read` allocates caller `maxBytes` (`net/io.rs:314-318`); the HTTP client connects with no timeout (`http_package.mfb:284`). → see OS-11. |
| OS-06 | LOW | **NOT DEMONSTRATED (fixed)** | `op_fail`/`connect_timeout` both `close` the fd (`net/mod.rs:695-742`); the in-code "leaks" comment (`:692-694`) is stale — the fd is stored before any branch and every path closes it. Recommend deleting the misleading comment. |
| OS-07 | LOW | **STILL PRESENT (by design)** | `fs::setCurrentDirectory` = process-global `chdir`; breaks per-thread CWD isolation. |
| OS-08 | LOW | **STILL PRESENT (by design)** | `thread.cancel/drop` are cooperative flags; a worker in a blocking syscall (accept/connect/read above) is not preempted — this is what makes OS-02/OS-05/OS-11 wedges unrecoverable. |

## New findings

### OS-09 — MEDIUM — HTTP request header CRLF injection (request splitting/smuggling)
- Location: `src/builtins/http_package.mfb:131` (`request = request & entry.key &
  ": " & entry.value & crlf`), request-target from `url.path`/`url.query`
  (`__http_requestTarget`, `:80-89`), `Host:` from `url.host` (`:72-77`).
- Threat/impact: a program forwarding attacker-influenced data into the `headers`
  map (or a `Url` built from an attacker `href`) lets the attacker embed `\r\n`
  and inject extra headers or a second request line — HTTP request
  splitting/smuggling against the upstream.
- Mechanism: `__http_normalizeMethod` (`:62-70`) rejects a space/empty in the
  *method* only. Header **names and values** and the URL **path/query/host** are
  concatenated verbatim with no CR/LF/control-char rejection (the only `\r\n` in
  the file are line terminators / response parsing).
- Reproduction:
  ```
  MUT h AS Map OF String TO String = {}
  h = collections::set(h, "X-Tag", "a\r\nX-Admin: true\r\nEvil: 1")
  LET r AS http::Response = http::read(net::toUrl("http://upstream/api"), h)
  ```
  Observed: the socket write contains `X-Tag: a\r\nX-Admin: true\r\nEvil: 1\r\n`.
  Expected: the value is rejected (or CR/LF stripped) before framing.
- Best fix (internal, no surface change): in `__http_buildRequest`, `FAIL error(...)`
  when any header name/value (and the derived request-target) contains a byte
  `< 0x20` (specifically CR/LF). Pure companion-source logic.
- Non-goals: restricting which hosts may be contacted (OS-10); normalization
  beyond control-char rejection. Fix is small → documented here, no separate bug doc.

### OS-10 — LOW — HTTP client SSRF: scheme-only URL validation, no internal-address guard (no redirect vector)
- Location: `src/builtins/net_package.mfb:86-165` (`__net_toUrl`); dial at
  `http_package.mfb:284`/`:308`.
- Threat/impact: `http::read`/`write` connect to any resolvable host including
  `127.0.0.1`, `169.254.169.254` (cloud metadata), RFC-1918/link-local; a program
  building a `Url` from untrusted input can be steered at internal services.
- Mechanism / scope-limiting: `__net_toUrl` validates the scheme only (`:91-93`),
  no host allow/deny or IP-range check. Importantly the client does **not** follow
  redirects — 3xx only appears in `__http_reasonPhrase` (`:936-956`); the response
  is returned verbatim — so there is **no redirect-based SSRF amplification**.
- Best fix: none required for correctness; any guardrail must be opt-in to avoid
  changing surface. Recommend documenting the absence of SSRF filtering.
- Non-goals: a default-deny host policy (would break legitimate localhost clients).

### OS-11 — LOW — HTTP client has no connect/read timeouts (thread wedge)
- Location: `http_package.mfb:284` (`net::connectTcp` — no `timeoutMs`), `:308`
  (`tls::connect(..., 0, ...)`), read loop `:288-303`.
- Threat/impact: a slow/malicious peer or stalled DNS blocks the calling thread
  indefinitely; with OS-08 (cooperative cancel) the thread cannot be interrupted.
  OS-05 realized on the HTTP surface.
- Mechanism: connect uses the `timeoutMs<=0` blocking branch (OS-05); the read
  loop caps total at 64 MiB (`__HTTP_MAX_RESPONSE`) but has no per-read deadline.
- Best fix (internal): thread a default connect/read timeout into
  `__http_exchangeTcp`/`Tls` (nonzero `connectTcp` timeout + `net::setReadTimeout`).
- Non-goals: a new public parameter.

## Stdin broadcast (plan-15) — no security finding
`stdin_broadcast.rs` frees log blocks off-arena on every exit path, releases the
mutex on every branch, and caps the log at `stdin_log_cap` (4 MiB). One
non-security correctness note: on subscriber-array exhaustion
`lower_stdin_subscribe` (`:684-685`) falls through without registering
(→ silent `ErrInvalidContext`), no memory/cross-thread impact.

## Verdict

Two prior HIGH findings still present (OS-01 world-writable modes → bug-184;
OS-02 accept ignores timeout → bug-185). OS-03/04/05 MEDIUM still present
(TOCTOU / symlink / unbounded — hardening, tracked in-file). New: OS-09 MEDIUM
(HTTP CRLF injection — actionable, small fix, documented here); OS-10/OS-11 LOW.
OS-06 not reproducible (fixed; stale comment).
