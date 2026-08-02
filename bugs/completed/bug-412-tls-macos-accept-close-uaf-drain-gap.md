# bug-412: macOS TLS `accept` failure paths and `closeListener` cancel without draining to `cancelled` → bug-380-class async-handler use-after-free

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Resource-safety (async completion-handler use-after-free; bug-380 class)

Status: FIXED (7dc352906)
Regression Test: the bug-380 stress harness (`closed-default-tls-drop-rt` under
heavy concurrent load on macOS aarch64) extended to exercise `tls::accept`
handshake failures + `closeListener` followed by process exit. In this
environment (no macOS runtime) the fix is pinned by two codegen emit-inspection
tests in `src/target/shared/code/tls/macos/tests.rs`
(`accept_failure_exits_drain_to_cancelled`, `close_listener_drains_to_cancelled`)
that assert each cancel-and-return exit emits a drain loop (back-edge label +
`ldr_u32 [ctx+CTX_STATE]` + `cmp_imm <cancelled>` + `b.ne` back), mirroring the
guards bug-380/bug-317 use for the connect path.

## STATUS: FIXED (7dc352906)

All three server cancel-and-return paths now drain to their terminal
`cancelled` state before returning, mirroring the connect-path drain bug-380
added:

- `lower_tls_accept_macos` `conn_fail` and `hs_timeout` drain `CCTX->state` to
  `nw_connection_state_cancelled` (5).
- `lower_tls_close_listener_macos` resolves `dispatch_semaphore_wait` and drains
  `LCTX->state` to `nw_listener_state_cancelled` (4), done while the listener,
  its queue, and ctx are still retained (before the bug-55 releases).

Implemented as a shared `emit_cancel_drain(ins, ctx_off, wait_off, label,
cancelled_state)` helper (connection and listener ctx share the `CTX_SEM` /
`CTX_STATE` prefix; only the terminal-state constant differs). The bug-317 leak
fixes on these paths are preserved (non-goal). Emitted macOS `.ncode` verified
to contain all three drains with stack layout identical to the working
`hs_wait` loop; the `macos-aarch64` tls byte-identity golden was regenerated and
the other six goldens are unchanged (the fix is scoped to the
Network.framework backend). Full `cargo test` green (3637 passed) and the tls
artifact-gate passes.

**Verdict for bug-380 Phase-1 audit item (`bugs/completed/bug-380-…md:197`,
"Audit `close` and `server.rs` for the same window"):** CONFIRMED — the server
accept-fail and `closeListener` paths had the identical async-cancel window and
now carry the same drain. The connect-path drain and the `conn_timeout` connect
exit were reviewed and left unchanged (connect never cancels before its wait
loop, so its exits cannot enter with the connection already `cancelled`).

bug-380 fixed a macOS-Network.framework use-after-free on the TLS **connect** path
by adding a synchronous cancel-drain loop (`client.rs:606-619`): after
`nw_connection_cancel` (which schedules the `cancelled` transition
asynchronously), it spins until the state handler reaches `cancelled` before
letting the arena be torn down, so a pending state-changed handler cannot
dereference a freed `CCTX`.

The **server** side never got the same treatment. bug-380's own Phase-1 audit item
(`bugs/completed/bug-380-…md:197`: "Audit `close` and `server.rs` for the same
window; record verdicts here") is left **unchecked**, and its Blast Radius (:169)
names `tls/macos/server.rs` as the listener/accept side with the identical window.

Two server paths cancel-and-return without a drain:

1. `lower_tls_accept_macos` `conn_fail` (`server.rs:1518`) and `hs_timeout`
   (`server.rs:1540`) call `emit_cancel_and_release_conn` then `emit_fail` and
   return immediately. The accepted connection runs its per-connection
   state-changed handler over the arena-allocated `CCTX` on the listener's shared
   `mfb.tls` serial queue; `nw_connection_cancel` schedules the `cancelled`
   transition asynchronously. If the server program exits (arena torn down) before
   that transition fires, the pending handler dereferences the freed `CCTX` →
   EXC_BAD_ACCESS. (The accept success path's drain loop lives in a *different*
   function, `lower_tls_close_listener_macos:1631-1710`, so these fail paths do not
   reach it.)
2. `lower_tls_close_listener_macos` (`server.rs:1712-1739`) cancels the listener
   and returns; its own comment (~:1758) notes the listener state handler over
   `LCTX` can still fire the `cancelled` transition, but adds no drain for it.

Structurally identical to the connect-path UAF bug-380 fixed. Timing/load-dependent
(matches bug-380's ~1/250-under-concurrency profile), macOS aarch64 only — hence
LOW/latent.

References:

- `src/target/shared/code/tls/macos/server.rs:1518`/`:1540` (accept fail paths),
  `:1712-1739` (closeListener), vs the connect-path drain
  `src/target/shared/code/tls/macos/client.rs:606-619` (bug-380 fix).
- bug-380 (`bugs/completed/`) — connect path FIXED; server/accept audit item :197
  left unchecked. Found during goal-07 (completes that deferred audit).

## Failing Reproduction

Requires macOS aarch64 + a live TLS client aborting handshakes under concurrent
load, then process exit before the async `cancelled` transition — not reproducible
in this environment (no macOS runtime). Static evidence: the accept fail paths and
`closeListener` lack the `client.rs:606-619` drain loop that bug-380 added to close
this exact window on the connect path.

- Observed: a queued `cancelled`-transition handler can run against freed arena
  `CCTX`/`LCTX` after accept-fail / closeListener + process exit.
- Expected: the fail/close paths drain to `cancelled` (like connect) before
  returning, so no handler fires on freed memory.

## Root Cause

`nw_connection_cancel` / `nw_listener_cancel` transition asynchronously; the accept
fail paths and `closeListener` release/return without waiting for the `cancelled`
state, leaving the arena-allocated context reachable by a pending handler.

## Goal

- `tls::accept` failure paths and `closeListener` drain to `cancelled` (mirroring
  the connect-path drain) before returning, so no state handler can dereference a
  freed context.

### Non-goals (must NOT change)

- The connect-path drain (already correct, bug-380). The leak fix (bug-317) on
  these paths must be preserved.

## Blast Radius

- `src/target/shared/code/tls/macos/server.rs:1518`/`:1540` (accept fail),
  `:1712-1739` (closeListener) — add the drain. Model: `client.rs:606-619`.
- Records the verdict bug-380's Phase-1 item :197 deferred.
