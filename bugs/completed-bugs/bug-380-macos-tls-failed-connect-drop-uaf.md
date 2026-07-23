# bug-380: dropping a failed/refused macOS TLS connection use-after-frees its state-changed handler (intermittent SIGSEGV)

Last updated: 2026-07-23
Effort: large (3h–1d)
Severity: MEDIUM
Class: Memory-safety

Status: FIXED
Regression Test: `tests/rt-behavior/resources/closed-default-tls-drop-rt` (the plan-38 F7
fixture, previously flaky) — now stress-proven solid (8000/8000 concurrent runs).

## STATUS: FIXED (2026-07-23)

`tls/macos/client.rs:lower_tls_connect_macos` `conn_fail`/`conn_timeout` now **drains to
the terminal `cancelled` state** before returning: after `nw_connection_cancel`, a loop
`dispatch_semaphore_wait(ctx->sem, FOREVER)` then re-reads `ctx->state` until it is
`nw_connection_state_cancelled` (5). Because `cancelled` is terminal (no transition follows
it), no `state_changed_handler` invocation can run after the helper returns — so the async
handler can never dereference a freed `ctx`. The loop mirrors the connect wait loop above
and reuses its resolved `dispatch_semaphore_wait` (`WAITFN`); the semaphore's persistent
count means it never hangs, whatever the transition count or a leftover signal.

A single un-looped wait was tried first and was **insufficient** (crash rate fell from
~1/250 to ~1/1000): the handler can fire more than once, or a stale signal can be consumed
first, so only draining to the terminal state is correct.

Verified: **8000/8000** concurrent runs exit 0 (was ~1/250 SIGSEGV), no new
`DiagnosticReports/*.ips`, no hang (6000 runs in 8s). The only golden that shifts is
`cover-tls`'s `macos-aarch64.ncode` (the drain loop in the connect helper); regenerated,
`artifact-gate.sh` back to 0 diffs. Committed as its own change.

**Deferred (same class, unreproduced):** `lower_tls_close_macos` (`:1302`) cancels and
returns without the same drain, so a program that *successfully* connects, `close`s, then
exits immediately has the identical UAF window. It is not fixed here because reproducing it
needs a live TLS server (the fixture uses a refused connect, which never reaches `close`'s
cancel — the closed-default record short-circuits at `REC_CLOSED`). The same drain-to-
`cancelled` applies; file its own repro before landing it.

---


On macOS, a `tls::connect` that is **refused/failed** sets up an `nw_connection` with a
`state_changed_handler` on the `mfb.tls` dispatch queue, then — on the failure path —
cancels and releases the connection and its queue and returns *immediately*, without
waiting for Network.framework's asynchronous `cancelled`/`failed` state transition. That
transition still fires the `state_changed_handler` on the `mfb.tls` background queue **after**
the connect helper has torn everything down, and the handler dereferences the now-freed
`ctx` (the block-captured semaphore/context) → `EXC_BAD_ACCESS`. It is timing-dependent, so
it only manifests under load (the full acceptance suite, or concurrent runs), and passes on
a quiet retry — which is exactly why it reads as "flaky."

**The single correct behavior a fix produces:** dropping (or explicitly `close`-ing) a
TLS socket whose connect was refused/failed is memory-safe on every target and under any
timing — the `state_changed_handler` can never run against freed memory, so
`closed-default-tls-drop-rt` never SIGSEGVs, in isolation or under concurrent load.

This is **not** the plan-38 F7 bug (a *synchronous* `close` misreading a closed-default
record's offset-8 flag as a live pointer `0x1`). That fix is correct and stays. This is a
distinct, *asynchronous* mechanism: the fault address is a real heap pointer
(`0x1_04ef_0040`, not `0x1`) and the crashing frame is on the `mfb.tls` queue, not the main
thread.

References:

- `plan-38` F7 — the closed-default-flag offset fix and the fixture this bug re-opens.
- bug-317 — added the connect-failure `nw_release`(conn)/`release`(queue) so a reconnect
  loop against an unreachable host does not leak; this bug is the drop-safety half that
  release introduced a UAF window into.
- Memory note `bug-tls-drop-rt-segfault-flake` — the symptom triage that led here.
- Found while completing bug-331/bug-332 (2026-07-23); the full acceptance run tripped it
  once, which prompted the investigation.

## Failing Reproduction

Standalone, sequential runs do **not** fault (0/40). It reproduces under concurrency:

```
cd <repo>
./target/debug/mfb build tests/rt-behavior/resources/closed-default-tls-drop-rt
bin=tests/rt-behavior/resources/closed-default-tls-drop-rt/build/closed_default_tls_drop_rt.out
tmp=$(mktemp -d)
for b in $(seq 1 10); do for i in $(seq 1 25); do ( "$bin" >/dev/null 2>&1; echo $? >"$tmp/$b-$i" ) & done; wait; done
cat "$tmp"/* | sort | uniq -c        # observed: 249 "0", 1 "139"
```

- Observed: ~1 in 250 concurrent runs exits **139** (SIGSEGV); ~1 per full
  `scripts/test-accept.sh` run. Sequential/isolated runs pass (masking it as flaky).
- Expected: exit 0 every time (`tls-failed=TRUE` / `clean`), on every target and under load.

Backtrace (from `~/Library/Logs/DiagnosticReports/closed_default_tls_drop_rt.out-*.ips`):

```
Exception: EXC_BAD_ACCESS (SIGSEGV), fault addr 0x0000000104ef0040   [a heap pointer, not 0x1]
Crashing thread 2, queue "mfb.tls":
  closed_default_tls_drop_rt.out              0x…508c (+20620)   ← the state_changed_handler block
  libdispatch.dylib  _dispatch_client_callout
  libdispatch.dylib  _dispatch_lane_serial_drain
  libdispatch.dylib  _dispatch_lane_invoke
  libdispatch.dylib  _dispatch_root_queue_drain_deferred_wlh
  libdispatch.dylib  _dispatch_workloop_worker_thread
```

The crash is on the `mfb.tls` queue inside the app's own state-changed-handler block — not
the main thread — proving the async handler ran after teardown.

| Environment | details | Result |
| --- | --- | --- |
| macOS aarch64, Network.framework backend | under load / concurrency | fails ✗ (~0.4% concurrent, ~1/full-suite) |
| macOS aarch64 | sequential, isolated | works ✓ (0/40) — masks the bug |
| Linux OpenSSL backend | — | expected ✓ (no libdispatch handler; **confirm in Blast Radius**) |

## Root Cause

`src/target/shared/code/tls/macos/client.rs:lower_tls_connect_macos`:

1. `:431` sets `nw_connection_set_state_changed_handler(conn, &block)` on the `mfb.tls`
   queue (`:376` `nw_connection_set_queue`). The block captures `ctx` — the
   `dispatch_semaphore_create` semaphore (`:336-346`) and the `dispatch_semaphore_signal`
   fn pointer (`:357-367`) — and signals `ctx->sem` on each state transition.
2. On a refused/failed connect, control reaches `conn_fail` (`:568`), which
   `emit_cancel_and_release_conn` (`nw_connection_cancel` + `nw_release`, `:569`) and
   `emit_release_queue` (`:582`), then `emit_fail` returns `ErrTlsFailed` (`:595`) — **all
   synchronously, without waiting for the `cancelled` state transition.**

`nw_connection_cancel` schedules the `cancelled` transition *asynchronously* on the
`mfb.tls` queue; `lower_tls_close_macos` already documents this at `:1403-1410` ("ctx->sem
is intentionally NOT released here … cancel schedules the cancelled transition afterwards
and does `dispatch_semaphore_signal(ctx->sem)`"). But the *connect-failure* path never
arranges for `ctx` (and the queue) to outlive that final handler invocation: it releases
the connection and the queue and returns, and `ctx` is then reclaimed. When the pending
`cancelled`/`failed` handler fires on the background queue, it dereferences freed `ctx` →
use-after-free → SIGSEGV. Releasing the queue while a block is still enqueued on it is the
same hazard from the other side.

Why the contrast cases are immune: sequential/isolated runs almost always let the async
transition drain before the process moves on; only under scheduler pressure does the handler
land after teardown. The OpenSSL backend has no dispatch handler, so no async window.

## Goal

- `closed-default-tls-drop-rt` exits 0 across ≥100k concurrent runs (0 SIGSEGV), and a
  full `scripts/test-accept.sh` run never trips it.
- The `state_changed_handler` provably cannot run against freed `ctx`/queue: either the
  failure path waits for the `cancelled` state before releasing `ctx`+queue, or `ctx`+queue
  lifetime is tied to the connection's teardown (retain-until-handler-done).

## Goal — Non-goals (must NOT change)

- **Do not weaken or delete the fixture** (`closed-default-tls-drop-rt`) or make it stop
  exercising the failed-connect scope-drop. Masking the flake by muting the test is the
  explicitly-forbidden wrong fix.
- **Do not revert plan-38 F7** (the offset-8 closed flag) — it fixes a different,
  synchronous crash and is correct.
- **Do not reintroduce the bug-317 leak** — the connection and queue must still be released
  on connect failure; the fix must add synchronization, not drop the release.
- No change to the `TlsSocket` record layout (`REC_CLOSED`/`REC_CONN`/`REC_QUEUE`/`REC_CTX`)
  or the OpenSSL backend's behavior.

## Blast Radius

Found by searching `src/target/shared/code/tls/`:

- `tls/macos/client.rs:lower_tls_connect_macos` `conn_fail`/`conn_timeout` (`:568`) — the
  reproduced site; **fixed by this bug.**
- `tls/macos/client.rs:lower_tls_close_macos` (`:1302`) — the normal close path also cancels
  and returns; it relies on `ctx` outliving the cancelled transition (`:1403`). Audit
  whether the *same* UAF window exists when a successfully-connected socket is closed then
  its record dropped/arena-reclaimed — likely the same hazard, in scope.
- `tls/macos/server.rs` (`:854` `mfb.tls` queue) — the listener/accept side sets up the same
  queue+handler shape; audit its cancel/release/drop paths for the identical window.
- `tls/openssl.rs` — synchronous BIO/SSL, no libdispatch handler; **unaffected** (confirm no
  background callback captures freed state).

## Fix Design (hypotheses, to confirm before coding)

The Network.framework-correct shape is: after `nw_connection_cancel`, **wait for the
`state_changed_handler` to observe `nw_connection_state_cancelled`** (the handler already
signals `ctx->sem`) *before* releasing `ctx` and the queue. The connect-failure path should
reuse the same wait-then-release the close path uses, rather than releasing eagerly. Options
to weigh:

1. On `conn_fail`, after `cancel`, `dispatch_semaphore_wait(ctx->sem)` for the terminal
   transition, then release conn/queue/ctx (mirrors `close`). Simplest; keep the leak fix.
2. Give the handler block its own retain on `ctx`/queue and release inside the handler on
   the terminal state, so main-thread teardown order stops mattering.

Rejected: dropping the release (reintroduces bug-317); muting/re-baselining the test.

## Phases

### Phase 1 — make the failure deterministic + capture the frame

- [ ] Add a stress harness (or a loop wrapper) that reproduces the SIGSEGV reliably enough
      to gate a fix (the concurrent-loop above hits ~1/250; tighten if needed).
- [ ] Symbolicate the `+20620` frame to the exact handler block and confirm the freed field
      (`ctx`/`ctx->sem`/`ctx->signal`) it dereferences.
- [ ] Audit `close` (`:1302`) and `server.rs` for the same window; record verdicts here.

Acceptance: a command that faults within N runs on a clean tree, and the exact freed field named.
Commit: —

### Phase 2 — synchronize teardown

- [ ] Make `conn_fail`/`conn_timeout` (and any confirmed sibling) wait for the terminal
      state transition before releasing `ctx`+queue, per the chosen option — **no artifact
      change is the goal only if codegen for other TLS calls is untouched; the connect
      helper's own `.ncode` WILL shift and that golden is regenerated deliberately here.**

Acceptance: the Phase-1 harness shows 0 SIGSEGV across ≥100k runs; `artifact-gate.sh` shows
exactly the intended connect-helper delta and nothing else.
Commit: —

### Phase 3 — validation

- [ ] `scripts/test-accept.sh` green; run `closed-default-tls-drop-rt` under heavy
      concurrency (≥100k) with 0 failures.
- [ ] `no_libm_math_imports`-style invariants unaffected; OpenSSL backend unchanged.

Acceptance: full suite green; the fixture is provably solid under load.
Commit: —

## Validation Plan

- Regression: the existing `closed-default-tls-drop-rt` fixture, plus the Phase-1 concurrent
  stress loop as the real guard (a single acceptance run is too coarse to catch a ~1/250
  fault).
- Runtime proof: `~/Library/Logs/DiagnosticReports/*.ips` must stop appearing for the binary
  after the fix under the same stress.
