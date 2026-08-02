# plan-76-B: tls::poll(sock[, timeoutMs]) AS Boolean

Last updated: 2026-08-02
Effort: large (3h–1d)
Depends on: nothing directly (see plan-76-A for the feature-wide Prerequisites). Independent of A.

The `tls` package has **no readiness primitive today** — no `poll`, no `setReadTimeout`. This
sub-plan adds the first: `tls::poll(sock AS TlsSocket[, timeoutMs AS Integer]) AS Boolean`, a
readiness query obeying the plan-73 timeout convention. It is the foundational primitive that
plan-76-C (`tls::poll(List)`) and plan-76-D (`http::pump`/`ready` over a TLS stream) build on.

The single behavioral outcome: `tls::poll(sock)` returns `TRUE` when the next `tls::read(sock, n)`
will return application bytes without blocking (or the connection is at a terminal readable state
— EOF/error), and `FALSE` when it would block; with an omitted timeout it blocks until that becomes
true, and honors the convention for `0` / `> 0` / `< 0`.

**Why this is not a plain `poll(fd)`:** a TLS socket may already hold **decrypted application bytes
buffered inside the TLS layer** with *nothing pending on the underlying fd* (a single TLS record can
carry many app bytes; one `SSL_read`/Network.framework receive drains a record, buffering the
remainder). Polling only the fd would report "not ready" while a byte is already available — a
correctness bug. Readiness therefore = **(TLS decrypted bytes already buffered) OR (raw fd
readable)**, and the "buffered" half is entirely backend-specific.

References (read first):

- `.ai/compiler.md`, `.ai/specifications.md`, `.ai/remote_systems.md` (TLS runtime proof needs the
  Linux/openssl and Windows/schannel boxes; macOS is local).
- `planning/completed/plan-73-D-tls.md` — the tls timeout migration; mirror its cross-backend rigor.
- `planning/completed/plan-73-A-…md` §"The convention" — the normative timeout table.
- `src/target/shared/code/net/poll.rs:17` (`lower_net_poll_helper`) — the fd-readiness / sentinel /
  EINTR structure the fd half of each backend reuses.
- Backends: `src/target/shared/code/tls/openssl.rs` (Linux/BSD; `SSL_read` at :2025, `SSL_pending`
  is the buffered-bytes query to add), `src/target/shared/code/tls/macos/{client,mod}.rs`
  (Network.framework — **no fd**; a pending-buffer ring, `mod.rs:106`), `schannel*.rs` (Windows;
  decrypted-record carry-over).
- `src/builtins/tls.rs` — descriptor tables (`TLS_FUNCTIONS` :133), `resolve_call` (:246), param
  arrays (:103), and `default_argument_padding` (:315) for the optional `timeoutMs`.

## Prerequisites

Feature-wide gate: see plan-76-A Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| Each TLS backend exposes a "decrypted bytes buffered?" query or an equivalent non-consuming peek | read `openssl.rs` (`SSL_pending`), `macos/mod.rs` (ring occupancy), `schannel_io.rs`/`schannel_read_close.rs` (carry-over buffer) | **PARTIALLY MET** — openssl YES (`SSL_pending`, symbol must be added), schannel YES (`STATE[LEFT_LEN]` @ STATE+64), **macOS NO** (no user-space decrypted-byte buffer — see Corrections B0-macOS; needs a receive-driven readiness shim). Phase 0 audit done. |
| Feature-wide gate (tree green, gate clean) | see plan-76-A Prerequisites | MET (tests 3750/0; gate baseline — see plan-76-A) |
| TLS runtime boxes reachable | `.ai/remote_systems.md` (Linux openssl, Windows schannel) | UNMEASURED — needed for Phase 2/3 runtime proof |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run and update
> before continuing and before stopping; report all rows if you stop.

## 1. Goal

- `tls::poll(sock AS TlsSocket) AS Boolean` — blocks until `sock` is readable, then `TRUE`.
- `tls::poll(sock AS TlsSocket, timeoutMs AS Integer) AS Boolean` — readiness query under the
  convention: `0` = one immediate check (`TRUE`/`FALSE` now); `> 0` = wait up to that long, `FALSE`
  on deadline; `< 0` = `ErrInvalidArgument`; omit = block until readable.
- "Readable" = a subsequent `tls::read` returns app bytes without blocking, **including** the case
  where bytes are already buffered in the TLS layer with an idle fd, and the terminal cases (peer
  close / connection error) where `read` returns promptly (`TRUE`, as `net::poll`/`io::pollInput`
  treat EOF as ready).
- Behavior is identical (per the convention) across all three backends: macOS Network.framework,
  Linux/BSD openssl, Windows schannel.

### Non-goals (explicit constraints)

- **No `tls::setReadTimeout`.** Only the readiness query is added. (Blocking `tls::read` timeouts
  remain out of scope, as today.)
- **No writability poll.** `POLLIN`-equivalent only.
- **No change to `tls::connect/accept/read/write/close`** or the `TlsSocket`/`TlsListener` types.
- **No list overload here** — that is plan-76-C.
- **No new error codes.** Reuse `ErrInvalidArgument` (`< 0`) and the existing network/closed codes.

## 2. Current State

- `tls` is a **Rust-native** package (`TLS.source = None`, `src/builtins/tls.rs:180`); every
  function is `Implementation::Same` with per-backend lowering. There is no `.mfb` source and no
  `poll` — the descriptor table `TLS_FUNCTIONS` (`tls.rs:133-142`) has connect/listen/accept/read/
  readText/write/writeText/close/closeListener and nothing else (confirmed: `rg -n 'poll'
  src/builtins/tls.rs` → none).
- Timeout support in tls today is only the blocking `timeoutMs` on `connect`/`accept`, padded via
  `default_argument_padding` (`tls.rs:315-337`) with `TIMEOUT_UNBOUNDED_SENTINEL` (`tls.rs:101`).
- Backends:
  - **openssl** (`tls/openssl.rs`): `SSL_read` at `:2025-2036`; a comment at `:1573` already
    references `poll(POLLIN, 0)` semantics for a connect path. `SSL_pending(ssl)` (returns count of
    already-decrypted, buffered app bytes) is the non-consuming buffered-bytes query to add; the fd
    half reuses `poll(2)` (`FIONBIO`/`O_NONBLOCK` handling at `:200`).
  - **macOS** (`tls/macos/`): Network.framework. There is **no raw fd**; decrypted data lands in a
    "single-producer/single-consumer ring of pending retained buffers" (`macos/mod.rs:106`), drained
    on the owning thread. Readiness = ring non-empty OR the connection is in a terminal readable
    state. `tls::poll` here is a ring-occupancy check plus a bounded `dispatch_semaphore` wait for
    the timeout form — NOT a `poll(2)`. This is the highest-risk backend.
  - **schannel** (`tls/schannel_io.rs`, `schannel_read_close.rs`): decrypted records carry over
    between reads (a partial-record buffer). Readiness = carry-over non-empty OR `WSAPoll(POLLRDNORM)`
    on the socket. `WSAPoll` scaffolding already exists (`schannel_impl.rs:33` `WSAPOLLFD`,
    `schannel_server.rs:610`).
- **net's `poll`** (`net/poll.rs`) is the reference for the fd half and the sentinel/clamp/EINTR
  block; the buffered half has no net analog (plaintext sockets have no user-space decrypt buffer).
- Checker path: tls flows through `check_table_builtin_call` → `resolve_call`; the new overload is a
  descriptor + `resolve_call` addition, no per-function checker edit.

### Measured populations

| What | Count | Command |
|---|---|---|
| `tls::poll` overloads today | 0 | `rg -n 'poll' src/builtins/tls.rs` → none |
| TLS backends needing a readiness path | 3 | `ls src/target/shared/code/tls` → openssl.rs, macos/, schannel* |
| Backends with an existing fd-poll scaffold to reuse | 2 (openssl `poll(2)`, schannel `WSAPoll`) | `rg -n 'WSAPoll\|POLLIN\|poll\(' src/target/shared/code/tls` |
| Backends with NO fd (buffer-only readiness) | 1 (macOS NW) | `rg -n 'no.*fd\|Network' src/target/shared/code/tls/macos/mod.rs` |
| tls rt-behavior tests to mirror | ≥1 | `ls tests/rt-behavior/tls \| rg 'timeout\|convention'` → `tls-timeout-convention-rt` |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| tls has no poll/readiness today | **CONFIRMED** | `rg -n 'poll' src/builtins/tls.rs` empty; `TLS_FUNCTIONS` list read. |
| macOS TLS has no raw fd (buffer-only readiness) | **CONFIRMED** | `macos/mod.rs:106` describes the pending-buffer ring; the backend is Network.framework (`NW_STATE_READY` `:69`), which exposes no socket fd. |
| A TLS socket can hold decrypted bytes with an idle fd | **CONFIRMED (by protocol)** | One TLS record → many app bytes; `SSL_read`/NW-receive drains a record and buffers the remainder, so `SSL_pending`/ring can be > 0 with the fd not readable. This is the whole reason fd-only poll is wrong. |
| openssl exposes a non-consuming buffered-count query | **UNVERIFIED** — Phase 0 | `SSL_pending(ssl)` is the standard call; confirm it is linkable/available in the vendored openssl and returns decrypted-app-byte count (not record bytes). |
| schannel carry-over buffer is queryable without consuming | **UNVERIFIED** — Phase 0 | Read `schannel_io.rs`/`schannel_read_close.rs` for the carry-over slot and whether its length is inspectable pre-read. |
| macOS ring occupancy is inspectable without dequeue, and a bounded wait exists | **FALSIFIED (bytes) / bounded-wait CONFIRMED** — Phase 0 | The `macos/mod.rs:104` ring is the **listener's pending-connection** ring (for `accept`), NOT a decrypted-app-byte buffer. The read path (`client.rs:820-936`) has a single-slot `CTX_CONTENT` consumed immediately — no user-space byte buffer exists, so there is NO non-consuming "bytes buffered?" query on macOS. A bounded wait DOES exist (`dispatch_semaphore_wait(sem, dispatch_time(...))`, connect path `client.rs:498-568`). See Corrections B0-macOS. |

## 3. Design Overview

One function, three backend implementations sharing a common contract; the risk is almost entirely
in the per-backend "is a byte available?" query, not in the language surface.

**Common contract (all backends):** `readable = buffered_app_bytes(sock) > 0 OR raw_layer_readable(sock)`.
`buffered_app_bytes` is backend-specific; `raw_layer_readable` is `poll(2)`/`WSAPoll` on the fd for
openssl/schannel and "connection terminal state reached" for macOS (which has no fd). The timeout
normalization (sentinel→block, `< 0`→invalid, `> 0`→clamp `INT_MAX`, `0`→immediate) is shared and
copied from `net/poll.rs`.

Per-backend readiness:
- **openssl:** `if (SSL_pending(ssl) > 0) return TRUE; else poll(fd, POLLIN, timeout)`. For the
  timeout form, if `SSL_pending == 0`, the fd `poll` carries the deadline; a wake means "raw bytes
  arrived" → readable. EINTR-retry as `net/poll.rs`.
- **schannel:** `if (carry_over_len > 0) return TRUE; else WSAPoll(fd, POLLRDNORM, timeout)`.
- **macOS:** `if (ring_non_empty) return TRUE`; else for `timeoutMs == 0` return `FALSE`
  immediately; for `> 0`/omit, wait on the read `dispatch_semaphore` bounded by the deadline
  (omit = indefinite) and re-check the ring / terminal state on wake. No `poll(2)`.

**Where design uncertainty concentrates (schedule FIRST):** whether each backend actually exposes a
**non-consuming** buffered-bytes query and a bounded wait (Phase 0). If openssl's `SSL_pending` is
usable, schannel's carry-over length is inspectable, and macOS's ring occupancy + semaphore wait are
reachable, the rest is mechanical. If any backend cannot answer "bytes buffered?" without consuming,
that backend's design must change (Phase 0 finds it before any lowering is written).

**Where correctness risk concentrates (schedule LAST):** the macOS backend — no fd, an async ring,
and a semaphore that `tls::read` already contends on (`macos/mod.rs:418` warns about
`dispatch_semaphore` release/leak on every read). A poll that waits on that same semaphore must not
steal a signal the pending read relies on, nor leak a semaphore. This is the bug-class the plan
guards with a buffered-then-idle runtime fixture and a repeat loop.

**Rejected alternatives:**

- *fd-only poll (ignore the TLS buffer).* Rejected — reports "not ready" while a decrypted byte is
  buffered; a caller then blocks in `tls::read`-gated logic forever. The buffered check is mandatory.
- *A peek that decrypts one byte and pushes it back.* Rejected — TLS record framing makes
  "un-reading" fragile and races the ring; use the backend's native pending-count instead.
- *Implement only macOS now, stub Linux/Windows.* Rejected — a readiness primitive that lies on two
  platforms is worse than none; all three ship together (plan-73-D precedent).

## 4. Detailed Design

### 4.1 Surface (Phase 1)

- `tls.rs`: add `const POLL: &str = "tls.poll";`, a `P_POLL = [req("sock", &[], TLS_SOCKET_TYPE),
  opt("timeoutMs", "Integer")]`, a `TLS_FUNCTIONS` entry `tf(POLL, "poll", &[ov(P_POLL, "Boolean")])`,
  a `resolve_call` arm (`POLL if exact([TlsSocket]) || exact([TlsSocket, Integer]) => "Boolean"`),
  `call_param_names`, `expected_arguments`, `argument_types`, and the omitted-`timeoutMs` padding in
  `default_argument_padding` (pad with `TIMEOUT_UNBOUNDED_SENTINEL`, mirroring connect/accept).

### 4.2 Backend lowering (Phases 2–4)

- Shared timeout normalization helper (copy `net/poll.rs:39-60`): sentinel→`-1`; `< 0`→
  `ErrInvalidArgument`; `> 0`→clamp `INT_MAX`.
- openssl (`tls/openssl.rs`): `lower_tls_poll_openssl` — `SSL_pending` fast-path, else `poll(fd)`.
- schannel (`tls/schannel*.rs`): `lower_tls_poll_schannel` — carry-over fast-path, else `WSAPoll`.
- macOS (`tls/macos/`): `lower_tls_poll_macos` — ring fast-path, else bounded semaphore wait.
- Dispatch from the tls lowering entry (where `read`/`write` fan out per target).

## Compatibility / Format Impact

- **Changed:** `tls` gains `poll` (one function, two overloads by timeout arity); tls man/spec add it.
- **Unchanged:** every existing tls function; `TlsSocket`/`TlsListener`; the resource registry; the
  `.mfp` encoding; blocking-read behavior.

## Phases

> Tick `- [x]` in the same commit as the work. An unticked box means NOT DONE.

### Phase 0 — backend readiness audit (design uncertainty first)

- [x] Per-backend non-consuming buffered-bytes query + bounded-wait primitive documented (recipe below).
      **Shared record layout** (POSIX family, `tls/mod.rs:24-28`): `TLS_OFFSET_FD=0`,
      `TLS_OFFSET_CLOSED=8`, `TLS_OFFSET_SSL=16` (openssl `SSL*`; schannel repurposes to the per-conn
      STATE block ptr), `TLS_OFFSET_CTX=24`. macOS differs (`macos/mod.rs:83-87`: `REC_CONN=0`,
      `REC_CLOSED=8`, `REC_QUEUE=16`, `REC_CTX=24`, **no fd**). Dispatch fan-out: `mod.rs:2131-2158`
      → `tls/mod.rs:373-408` by `PlatformFamily`.

  **openssl (Linux/BSD)** — buffered? **YES, non-consuming** via `SSL_pending(ssl)` (`ssl` @
  record+16). `SSL_pending`/`SSL_has_pending` are NOT yet in `TLS_SYMBOLS` (`tls/mod.rs:62-78`); add
  `"SSL_pending"` there + one `emit_dlsym` (pattern `openssl.rs:2016` for `SSL_read`) + a call. fd
  readable? → `poll({fd@record+0, POLLIN}, 1, ms)` (pattern `openssl.rs:1590-1607`). Readiness =
  `SSL_pending>0 || poll>0`.

  **schannel (Windows)** — buffered? **YES, non-consuming, already present.** `STATE = record[16]`,
  then `bytesBuffered = STATE[st::LEFT_LEN]` (STATE+64, `schannel.rs:63`); serve path at
  `schannel_read_close.rs:48-53` proves it is pre-read-inspectable and mutation-free. fd readable? →
  `WSAPoll({fd@record+0, POLLRDNORM=256}, 1, ms)` (pattern `schannel_server.rs:665-674`). Readiness =
  `STATE[LEFT_LEN]>0 || WSAPoll>0`.

  **macOS (Network.framework)** — buffered? **NO non-consuming query exists** (see Corrections
  B0-macOS). No fd; the read path posts `nw_connection_receive(min=1,max=maxBytes)` into a single
  `CTX_CONTENT` slot (`macos/mod.rs:99`) consumed immediately (`client.rs:912-936`) — there is no
  user-space leftover buffer for decrypted app bytes. The `mod.rs:104-115` ring
  (`LCTX_HEAD`@48/`LCTX_TAIL`@56/`LCTX_RING`@64, cap 16) is the **listener's pending-connection**
  ring for `accept`, NOT decrypted bytes. Terminal state: `CTX_STATE` (`macos/mod.rs:98`), ready=3,
  failed=4, cancelled=5 (`client.rs:864-868`). Wait primitive available: bounded
  `dispatch_semaphore_wait(sem, dispatch_time(NOW, ms*1e6))` (connect path `client.rs:498-568`; both
  symbols already in `SYMBOLS` `mod.rs:147-148`) — must respect the `emit_fresh_sem` release/pairing
  invariant (`mod.rs:412-427`). **Design revision (Phase 4):** macOS `tls::poll` needs a
  receive-driven readiness shim — post a bounded `nw_connection_receive`, stash the returned bytes in
  a NEW ctx pending-plaintext slot, and have `tls::read` drain that slot first (mirroring schannel's
  `LEFT` carry-over). This is the added scope Phase 4 must build; see Corrections B0-macOS.

  **Shared timeout plumbing to reuse:** `net/poll.rs:38-60` (sentinel→-1/block, negative→invalid,
  `>=0`→clamp INT_MAX) + EINTR retry `net/poll.rs:101-114`; `net_symbol(NetSymbol::Poll)` renames
  `poll`→`WSAPoll` on Windows (`net/mod.rs:59-88`); `emit_pollfd_events` writes the per-platform
  events field (`net/mod.rs:99-122`). Note `net.poll` reads `FILE_OFFSET_FD/CLOSED`; `tls::poll` must
  read `TLS_OFFSET_FD/CLOSED` (0/8) instead.

Acceptance: a written per-backend readiness recipe (query + wait), with the symbol/offset each uses.
No code yet. If a backend has no non-consuming query, its Phase design is revised here. **DONE** —
recipe above; macOS design revised (receive-driven shim, Phase 4).
Commit: (recorded next commit)

### Phase 1 — surface (descriptor + resolver + padding)

- [ ] Add `tls::poll` to `tls.rs` (all tables + padding), per §4.1.
- [ ] Tests: `tests/syntax/tls` accept `tls::poll(sock)` → `Boolean`, `tls::poll(sock, 100)` →
      `Boolean`; reject a non-`TlsSocket` arg and `< 0`-arity misuse at the checker level.

Acceptance: at `-ast -ir` the overloads resolve to `Boolean`; `cargo test --bin mfb` green (native
lowering will error if referenced at runtime — do not run a TLS program yet).
Commit: —

### Phase 2 — openssl backend (Linux/BSD)

- [ ] `lower_tls_poll_openssl`: `SSL_pending` fast-path + `poll(fd)` + shared timeout block + EINTR.
- [ ] Tests: `tests/rt-behavior/tls/tls-poll-rt` (run on the Linux/openssl box): connect to a test
      TLS endpoint, prove (a) after a partial `tls::read` that leaves buffered bytes, `tls::poll(sock,
      0)` is `TRUE` with the fd idle; (b) `tls::poll(sock, 0)` is `FALSE` before any data; (c)
      `tls::poll(sock)` (omit) blocks then returns `TRUE` on arrival; (d) `< 0` → `ErrInvalidArgument`.

Acceptance: the four cases hold on the openssl box, including the **buffered-with-idle-fd** case
(the correctness crux); ≥1000× loop leaks nothing.
Commit: —

### Phase 3 — schannel backend (Windows)

- [ ] `lower_tls_poll_schannel`: carry-over fast-path + `WSAPoll(POLLRDNORM)` + shared timeout block.
- [ ] Tests: the same four cases on the Windows box (codegen verified per the Windows
      codegen-verification convention; runtime on-box where available).

Acceptance: the four cases hold on Windows, buffered case included.
Commit: —

### Phase 4 — macOS Network.framework backend (largest risk, last)

- [ ] `lower_tls_poll_macos`: ring-occupancy fast-path; `0` → immediate `FALSE`; `> 0`/omit →
      bounded/indefinite semaphore wait re-checking the ring + terminal state; no semaphore leak, no
      stolen read signal.
- [ ] Tests: run `tls-poll-rt` locally on macOS; add a case that reads a large response in two
      `tls::read` calls and asserts `tls::poll(sock, 0)` is `TRUE` between them (buffered data,
      idle "fd"); ≥1000× connect/poll/read/close loop proving no `dispatch_semaphore` leak
      (`macos/mod.rs:418` hazard).

Acceptance: all readiness cases hold on macOS; the leak loop is flat; `cargo test --bin mfb` +
`artifact-gate.sh` green; goldens regenerated if tls codegen shifts.
Commit: —

### Phase 5 — docs

- [ ] Man page `src/docs/man/builtins/tls/poll.md` (new): both overloads, the timeout-convention
      row, and the explicit note that readiness includes TLS-buffered bytes (not just fd state).
      Cite `mfb spec language builtin-functions` §18.4.
- [ ] Spec: add `tls::poll` to the tls stdlib section and to §18.4's readiness-query list.

Acceptance: `mfb man tls poll` renders; man/spec-citation tests green.
Commit: —

## Validation Plan

- Tests: syntax accept/reject (Phase 1); rt-behavior readiness (buffered / not-ready / block / `< 0`)
  per backend (Phases 2–4) + leak loops.
- Coverage check: the readiness fixtures exercise every backend's `poll` lowering (each in the
  gate/acceptance denominator; TLS runtime runs on the respective box per `.ai/remote_systems.md`).
- Runtime proof: the **buffered-with-idle-fd** case — two-part read where `tls::poll(sock, 0)` is
  `TRUE` between reads — on every backend; this is the property fd-only poll would get wrong.
- Doc sync: `src/docs/man/builtins/tls/poll.md`, tls stdlib spec, §18.4.
- Acceptance: `cargo test --bin mfb`, `scripts/test-accept.sh` (tls glob, per-box), `artifact-gate.sh`.

## Open Decisions

1. **macOS timeout wait mechanism** — recommended: reuse the read path's `dispatch_semaphore`
   (bounded via `dispatch_semaphore_wait` with a `dispatch_time` deadline), re-checking the ring on
   wake, vs. a dedicated poll semaphore. Recommended reuse, but only if Phase 0 proves it does not
   race the pending read's signal; otherwise a dedicated wait object. (§3, Phase 4)
2. **Terminal-state = ready** — recommended: a closed/errored TLS connection polls `TRUE` (the read
   returns promptly), matching `net::poll`/`io::pollInput` EOF-is-ready. (§1)

## Corrections

- **B0-macOS (Phase 0 / §3 / Verified properties): macOS has NO non-consuming buffered-bytes
  query — the plan's "ring occupancy" premise is false for `TlsSocket` readiness.** The plan (§2,
  §3, Open Decision 1) assumed macOS Network.framework keeps decrypted app bytes in an inspectable
  "single-producer/single-consumer ring" at `macos/mod.rs:106`. That ring
  (`LCTX_HEAD`@48/`LCTX_TAIL`@56/`LCTX_RING`@64) is the **listener's pending-connection** ring used by
  `tls::accept`, not a decrypted-byte buffer. The read path (`client.rs:820-936`) posts
  `nw_connection_receive(min=1, max=maxBytes)` into a single `CTX_CONTENT` slot consumed immediately
  — there is no user-space leftover for decrypted app bytes, so "are decrypted bytes buffered?"
  cannot be answered non-consuming on macOS. **Design revision:** macOS `tls::poll` (Phase 4) must
  add a receive-driven readiness shim: a NEW ctx pending-plaintext slot filled by a bounded
  `nw_connection_receive`, drained first by `tls::read` (mirroring schannel's `LEFT` carry-over at
  `STATE+56/+64`). This makes `tls::read` on macOS buffered-aware (a change to the read path, not just
  a new poll), and is the added scope of Phase 4. Measured by the Phase-0 audit of
  `tls/macos/{mod,client}.rs` (no leftover/carry buffer; `CTX_CONTENT` single-slot at `mod.rs:99`).
  Open Decision 1 ("reuse the read semaphore") stands only for the WAIT half; the READINESS half needs
  the shim.
- **B0-tls.rs (Phase 1 surface): the optional `timeoutMs` should use `Fill(SENTINEL)`, not bare
  `Optional`.** tls (unlike net) HAS `default_argument_padding` (`tls.rs:315`), so the omitted
  `timeoutMs` is padded there with `TIMEOUT_UNBOUNDED_SENTINEL` (mirroring `accept`), not padded in
  `builder_values.rs`. §4.1 already says pad in `default_argument_padding`; recorded here so the param
  array uses the `fill`/`ACCEPT_DEFAULTS` shape, not net's `opt`.

## Summary

The risk is concentrated in the **buffered-bytes readiness** (Phase 0 proves each backend can answer
it non-consuming) and the **macOS Network.framework backend** (no fd, a contended semaphore — Phase 4,
last, behind a leak loop). openssl and schannel reuse existing `poll(2)`/`WSAPoll` scaffolds plus a
one-call pending check. Untouched: every existing tls function, the resource types and registry, the
`.mfp` encoding, and blocking-read semantics. This primitive is what plan-76-C and plan-76-D require.
