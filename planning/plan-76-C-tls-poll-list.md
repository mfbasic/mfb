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
| plan-76-B landed (scalar `tls::poll` works on all 3 backends) | `rg -n 'POLL' src/builtins/tls.rs` shows the descriptor; `ls tests/rt-behavior/tls/tls-poll-rt` | NOT MET until B lands |
| plan-76-A's borrowed-resource-return question resolved (return `RES <sock>` vs. index) | `planning/plan-76-A-*` Phase 1 Commit filled | NOT MET until A Phase 1 lands |
| Feature-wide gate (tree green, gate clean) | see plan-76-A Prerequisites | UNMEASURED |

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

- [ ] Add the list overload to `tls.rs` (descriptor/param/`resolve_call`/metadata), matching A's
      shipped return-shape decision (`RES TlsSocket` or `Integer` index).
- [ ] Tests: `tests/syntax/tls` accept `tls::poll(socks)` / `tls::poll(socks, 100)`; reject bare
      `List OF TlsSocket` (no `RES`) and non-list args; confirm the scalar overload still resolves.

Acceptance: overloads resolve at `-ast -ir`; `cargo test --bin mfb` green.
Commit: —

### Phase 2 — native list lowering (all backends)

- [ ] `lower_tls_poll_list_*`: empty guard; per-socket pre-scan via B's predicate; deadline-bounded
      wait+rescan; first-ready record-ptr return; expiry → `ErrTimeout`. openssl/schannel coalesce
      the fd wait if clean, else the portable scan-then-wait loop.
- [ ] Tests: `tests/rt-behavior/tls/tls-poll-list-rt` (per box): two TLS connections, data pushed to
      exactly one, `tls::poll([a,b])` returns it; a buffered-bytes case (one socket has buffered
      data, the other idle → the buffered one is returned even with both fds idle); timeout cases
      (`0` → `ErrTimeout` when idle; omit blocks then returns; `< 0` → `ErrInvalidArgument`); ≥1000×
      loop with no fd/semaphore leak.

Acceptance: the multiplex + buffered + timeout cases hold on every backend; leak loop flat;
`cargo test --bin mfb` + `artifact-gate.sh` green; tls goldens regenerated if codegen shifts.
Commit: —

### Phase 3 — docs

- [ ] Extend `src/docs/man/builtins/tls/poll.md` with the list overload (ownership note: borrowed
      pointer, list still owns/closes; buffered-readiness included). Update the tls stdlib spec.

Acceptance: `mfb man tls poll` shows all overloads; man/spec-citation tests green.
Commit: —

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

<!-- Filled in during execution. -->

## Summary

A thin generalization of B's scalar readiness to N sockets, sharing plan-76-A's borrowed-resource
list-return shape. Risk is the deadline-honoring wait+rescan loop and macOS's N-ring semaphore wait
(leak-loop guarded). Nothing here re-derives backend readiness — it reuses B. Untouched: the scalar
overload, other tls functions/types, and the `.mfp` encoding.
