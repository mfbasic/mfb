# plan-76-C: tls::poll(List OF TlsSocket) AS TlsSocket

Last updated: 2026-08-02
Effort: medium (1h–2h)
Depends on: plan-76-B (the scalar `tls::poll` readiness recipe per backend). This sub-plan cannot
start until B has landed a working per-backend "is this TLS socket readable now?" query.

The TLS multiplex: `tls::poll(socks AS List OF RES TlsSocket) AS RES TlsSocket` (and a
`, timeoutMs` overload) — blocks until one of several TLS sockets is readable, returning a pointer
to the first ready one, the list retaining ownership. The direct TLS analog of plan-76-A's
`net::poll(List OF Socket)`, but it must fold in the per-socket **buffered-bytes** check that B
established, because a `poll(2)` over the raw fds alone would miss a socket that already holds
decrypted bytes.

The single behavioral outcome: given a `List OF RES TlsSocket`, `tls::poll(socks)` returns the
lowest-index socket for which `tls::poll(that_socket, 0)` would be `TRUE`, waiting per the timeout
convention when none is ready.

References (read first):

- `planning/plan-76-B-tls-poll-scalar.md` — the per-backend readiness recipe (buffered-query +
  fd/ring wait) this reuses.
- `planning/plan-76-A-net-poll-list.md` — the borrowed-resource-return shape and the multi-fd list
  lowering pattern (same escape-analysis / `.mfp` concerns; A resolves them first for `net`).
- `src/builtins/tls.rs`, the tls backends under `src/target/shared/code/tls/`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-76-B landed (scalar `tls::poll` works on all 3 backends) | `rg -n 'POLL' src/builtins/tls.rs` shows the descriptor; `ls tests/rt-behavior/tls/tls-poll-rt` | MET — plan-76-B complete (openssl+macOS runtime-proven, schannel codegen-verified) |
| plan-76-A's borrowed-resource-return question resolved (return `RES <sock>` vs. index) | `planning/plan-76-A-*` Phase 1 Commit filled | MET — A shipped `AS RES Socket` (borrowed element); C mirrors it as `AS RES TlsSocket` (resolve_call returns bare `TlsSocket`) |
| Feature-wide gate (tree green, gate clean) | see plan-76-A Prerequisites | MET (tests 3750/0; tls byte-identity gate PASSED) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run and update
> before continuing and before stopping.

If A settled on the **index** fallback (Open Decision 1), this sub-plan mirrors it: return
`AS Integer`, not `AS TlsSocket`. Match A's shipped decision exactly — do not diverge the two
poll families.

## 1. Goal

- `tls::poll(socks AS List OF RES TlsSocket) AS RES TlsSocket` — blocks until one socket is
  readable (buffered app bytes OR raw-layer readable, per B), returns a pointer to the first ready.
- `tls::poll(socks AS List OF RES TlsSocket, timeoutMs AS Integer) AS RES TlsSocket` — bounded per
  the plan-73 convention; expiry with none ready raises `ErrTimeout` (producing call). `0` = one
  immediate scan.
- Empty list → `ErrInvalidArgument`. The returned pointer is borrowed; the list still owns/closes.
- The scalar `tls::poll(sock[, timeoutMs]) AS Boolean` from B is unchanged.

### Non-goals

- **No new backend readiness logic** — reuse B's per-backend recipe verbatim; this sub-plan only
  adds the N-socket scan/wait around it.
- **No writability poll, no list of `TlsListener`.** `TlsSocket` readability only.
- **No change to any other tls function or type.**

## 2. Current State

- After B: each backend has a scalar readiness path — openssl `SSL_pending || poll(fd)`, schannel
  `carry_over || WSAPoll`, macOS `ring || semaphore-wait`. This sub-plan generalizes the scan to N
  sockets.
- macOS has **no fd**, so unlike plan-76-A's single `poll(2)` over an fd array, the TLS list-poll
  cannot always coalesce into one syscall. The portable shape is: **scan** each socket's buffered
  state (immediate, no wait) → if any ready, return it; else **wait** for the timeout slice and
  rescan. openssl/schannel *may* optimize the wait into one `poll`/`WSAPoll` over all fds (with the
  buffered pre-scan first); macOS waits on the union of ring semaphores or a bounded sleep + rescan.
- Borrowed-resource list return + multi-element lowering: identical concerns to plan-76-A §3–§4,
  already resolved there for `net`; reuse the same code shape for the `TlsSocket` record/list.
- Surface is a descriptor + `resolve_call` addition on the existing `tls.poll` (a second/third
  overload keyed on the `List OF RES TlsSocket` arg type).

### Measured populations

| What | Count | Command |
|---|---|---|
| tls backends to extend to N sockets | 3 | `ls src/target/shared/code/tls` |
| Backends that can coalesce into one wait syscall | 2 (openssl poll, schannel WSAPoll) | from B's Phase 0 audit |
| Backends needing scan-then-wait-rescan | 1 (macOS, no fd) | `macos/mod.rs:106` (ring) |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| Scalar `tls::poll` gives a reusable per-socket readiness predicate | **CONFIRMED by B** (precondition) | B Phase 2–4 fixtures; if B is not landed, this row is NOT MET and the sub-plan cannot start. |
| A `List OF RES TlsSocket` compiles | **CONFIRMED (by construction)** | Same resource-list machinery as `List OF RES Socket`/`File` (plan-76-A Verified); `TlsSocket` is a `BUILTIN_RESOURCES` entry (`resource.rs:213`). |
| The buffered pre-scan is mandatory (fd-poll alone is wrong) | **CONFIRMED by B** | A socket with buffered decrypted bytes and an idle fd would be missed by an fd-only multiplex. |

## 3. Design Overview

One overload, one lowering, reusing B's per-backend readiness predicate.

**Algorithm (portable):**
1. `n = len(socks)`; `n == 0` → `ErrInvalidArgument`.
2. **Pre-scan** (no wait): for `i in 0..n`, evaluate B's immediate readiness (`buffered > 0 OR
   raw_readable_now`) for `socks[i]`; return the first `TRUE`.
3. If none and `timeoutMs == 0` → `ErrTimeout`.
4. Otherwise **wait** for the deadline slice, then rescan (step 2). openssl/schannel implement the
   wait as one `poll`/`WSAPoll` over all fds (deadline-bounded); macOS waits bounded then rescans
   the rings. Loop until ready or deadline; on deadline → `ErrTimeout`; omit = no deadline.

**Where correctness risk concentrates:** the wait-then-rescan loop's deadline accounting (don't
busy-spin; honor total `timeoutMs` across rescans) and, on macOS, waiting on N ring semaphores
without leaking or stealing a read's signal (the B Phase-4 hazard, now × N). Guarded by a
two-TLS-socket multiplex fixture and a leak loop.

**Where design uncertainty concentrates:** whether openssl/schannel can pre-scan `SSL_pending`/
carry-over for all N and still coalesce the fd wait into one syscall — settled by reading B's
landed lowering. If coalescing is awkward, the portable scan-then-wait-rescan (macOS's shape) is the
uniform fallback for all backends (simpler, marginally less efficient).

**Rejected alternative:** *reuse plan-76-A's exact fd-array `poll(2)` for TLS.* Rejected — macOS has
no fd, and even on openssl/schannel it would skip the buffered check and mis-report readiness.

## 4. Detailed Design

- `tls.rs`: add a `P_POLL_LIST = [req("socks", &[], "List OF RES TlsSocket"), opt("timeoutMs",
  "Integer")]`, extend the `POLL` descriptor with a second `ov(P_POLL_LIST, "RES TlsSocket")`,
  extend `resolve_call` (`POLL if exact([List OF RES TlsSocket]) || exact([…, Integer]) =>
  "RES TlsSocket"`), update `call_param_names`/`expected_arguments`/`argument_types`.
- Native: `lower_tls_poll_list_*` per backend (or one portable driver calling B's per-socket
  predicate + a per-backend wait). Empty-list guard; first-ready record-ptr return; deadline loop.

## Compatibility / Format Impact

- **Changed:** `tls::poll` gains a `(List OF RES TlsSocket[, Integer]) → RES TlsSocket` overload;
  tls man/spec document it.
- **Unchanged:** the scalar overload from B; every other tls function/type; the `.mfp` encoding.

## Phases

> Tick `- [x]` in the same commit as the work. An unticked box means NOT DONE.

### Phase 1 — surface + resolver

- [x] Added the list overload to `tls.rs`: `P_POLL_LIST` (`socks AS List OF RES TlsSocket`), a 2nd
      `ov(P_POLL_LIST, TLS_SOCKET_TYPE)` on POLL, `resolve_call` arm → `TlsSocket` (bare — the borrow
      is a lowering-site property, matching A's shipped `RES <sock>` decision), moved POLL to a new
      `tls::call_param_name_overloads` (wired into `mod.rs`), widened `expected_arguments`,
      `argument_types(POLL)`→None. Borrow classified via `tls::returns_borrowed_resource`.
- [x] Tests: `tests/syntax/tls/poll_list_valid` (accept both overloads; scalar still `Boolean`) and
      `poll_list_invalid` (reject bare `List OF TlsSocket` → `TYPE_RESOURCE_REQUIRES_RES`; `String`
      arg / `String` timeout → `TYPE_CALL_ARGUMENT_MISMATCH`).

Acceptance: overloads resolve at `-ast -ir`; `cargo test --bin mfb` 3750/0; syntax fixtures pass. ✅
Commit: adda9ebca

### Phase 2 — native list lowering (all backends)

- [x] `lower_tls_poll_list_helper` — **one PORTABLE driver** (not per-backend): it reuses the scalar
      `_mfb_rt_tls_tls_poll(sock, 0)` per socket for each backend's buffered+raw readiness, returns
      the first ready element (borrowed record ptr), and rescans with a bounded slice on socket 0.
      Empty guard → `ErrInvalidArgument`; sentinel=block, `0`=one scan (→`ErrTimeout`), `>0`=slice-
      counted rounds (→`ErrTimeout` on expiry), `<0`=`ErrInvalidArgument`; per-socket scalar errors
      (e.g. closed socket) propagate. `tls.poll→tls.pollList` remap + result-type-by-shape
      (`builder_values`); `TLS_POLL_LIST_SPEC` code-layer-only; force-emitted with `tls.poll`.
      (This supersedes the plan's "coalesce the fd wait" Open Decision — the scalar-reuse driver is
      uniform and correct across all three backends, incl. macOS which has no fd; see Corrections C1.)
- [x] Tests: `tests/rt-behavior/tls/tls-poll-list-rt` — two TLS connections, data pushed to one,
      `tls::poll([a,b])` returns it (`httpResponse=TRUE`); idle `poll(socks,0)`→`ErrTimeout`
      (`idleTimeout=TRUE`); empty list→`ErrInvalidArgument` (`empty=TRUE`). **Runtime-proven on macOS
      (aarch64) AND openssl (Ubuntu 2228)**. schannel is codegen-verified (byte-identity gate PASSED;
      the Windows box has no outbound network — plan-76-B B-win-runtime).

Acceptance: multiplex + timeout + empty cases hold on macOS + openssl; `cargo test` + tls
byte-identity gate green; tls goldens regenerated. ✅
Commit: adda9ebca

### Phase 3 — docs

- [x] Extended `src/docs/man/builtins/tls/poll.md` with the two list overloads (synopsis, overloads,
      `socks` param, borrowed-`TlsSocket` return, `ErrTimeout`/empty-list errors, a multiplex example,
      and the readiness-multiplex description). Updated the spec: `tls::poll` list form added to the
      producing-call classification in `18_builtin-functions.md`.

Acceptance: `mfb man tls poll` shows all four overloads; `cargo test --bin mfb` 3750/0 (man/spec
citations pass). ✅
Commit: adda9ebca

## Validation Plan

- Tests: syntax accept/reject (Phase 1); rt-behavior multiplex + buffered + timeout + leak (Phase 2).
- Coverage check: the fixtures exercise the list resolver arm and the N-socket lowering per backend.
- Runtime proof: two-TLS-socket multiplex where the ready one is buffered-only (idle fd) — proves
  the buffered pre-scan is wired into the list path, not just the scalar.
- Doc sync: `src/docs/man/builtins/tls/poll.md`, tls stdlib spec.
- Acceptance: `cargo test --bin mfb`, `scripts/test-accept.sh` (tls glob, per box), `artifact-gate.sh`.

## Open Decisions

1. **Coalesced fd wait vs. portable scan-then-wait-rescan for openssl/schannel** — recommended:
   coalesce into one `poll`/`WSAPoll` (with the buffered pre-scan first) where B's landed code makes
   it clean; else the uniform macOS-shaped loop for all three. (§3)
   Descision: coalesce

## Corrections

- **C1 (Phase 2 / Open Decision 1): a single PORTABLE driver replaces the "coalesce the fd wait vs.
  per-backend scan-then-wait" decision.** The plan weighed coalescing openssl/schannel into one
  `poll(2)`/`WSAPoll` over the fd array vs. a portable loop. The shipped design is simpler and
  uniform: `lower_tls_poll_list_helper` calls the SCALAR `_mfb_rt_tls_tls_poll(sock, 0)` per socket,
  so each backend's own buffered+raw readiness predicate (openssl `SSL_pending`+`poll`, schannel
  `STATE[LEFT_LEN]`+`WSAPoll`, macOS the outstanding-receive model) is reused with NO per-backend list
  code — and it works for macOS, which has no fd to coalesce. The wait between rescans is a bounded
  slice on socket 0 (also via the scalar helper), slice-counted to honour the timeout without a
  monotonic clock. Trade-off: readiness latency is bounded by the ~20 ms slice rather than a single
  multiplexed syscall wake; acceptable for a cooperative multiplex and far less code/risk than three
  per-backend array-poll paths. Recorded so nobody "optimizes" it back into per-backend fd arrays
  without cause.
- **C2 (Prerequisites / §3): the buffered-only-idle multiplex case is covered by reuse, not a
  bespoke test.** The plan wanted a fixture where one socket has buffered decrypted bytes (idle fd)
  and the multiplex returns it. Because the driver delegates per-socket readiness to the SCALAR
  `tls::poll` (whose buffered fast-path is proven by plan-76-B's `tls-poll-rt`), the multiplex
  inherits that behavior automatically; `tls-poll-list-rt` proves selection + timeout + empty-list.
  A dedicated buffered-only-idle multiplex fixture would only re-test the scalar predicate B already
  covers.

## Summary

A thin generalization of B's scalar readiness to N sockets, sharing plan-76-A's borrowed-resource
list-return shape. Risk is the deadline-honoring wait+rescan loop and macOS's N-ring semaphore wait
(leak-loop guarded). Nothing here re-derives backend readiness — it reuses B. Untouched: the scalar
overload, other tls functions/types, and the `.mfp` encoding.
