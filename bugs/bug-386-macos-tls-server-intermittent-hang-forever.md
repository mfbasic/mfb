# bug-386: macOS TLS server op intermittently blocks FOREVER, wedging the whole local `cargo test`

Last updated: 2026-07-25
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness (concurrency / liveness)

Status: Open
Regression Test: tests/macos_tls_write_capacity.rs (`macos_tls_write_sends_capacity_over_count_byte_list_exactly`)

The macOS TLS runtime waits on a `dispatch_semaphore` with a `DISPATCH_TIME_FOREVER`
deadline for its receive/send completion (`emit_wait`, `src/target/shared/code/tls/macos/mod.rs:489`).
When a Network.framework completion block does not fire on some path (a
handshake/accept/state-change error or cancellation race under load), that wait
never returns and the mfb TLS server process blocks **indefinitely**. The test
`macos_tls_write_sends_capacity_over_count_byte_list_exactly` stands up such a
server and has **no timeout on any wait**, so when the flake hits, the test hangs
forever and takes the entire local `cargo test` run down with it (observed: a run
parked on this test for 36+ minutes, cargo printing
`... has been running for over 60 seconds`, everything before it green). This test
is `#![cfg(target_os = "macos")]`, so CI (Ubuntu) never runs it — the hang is
invisible to CI and only bites local macOS development.

The single correct behavior a fix produces: the macOS TLS server never blocks
indefinitely — every semaphore wait is guaranteed to be signaled (on success AND
on every failure/cancellation/state-change path), so `readText`/`write`/`accept`
either make progress or raise a TLS error in bounded time; and the regression test
completes deterministically (server responds with the exact payload, both sides
exit) and can never wedge the suite.

References:

- `mfb man tls readText` — defines the short-read contract (see "disproven
  hypothesis" below): a read "returns as soon as any plaintext is available rather
  than waiting to fill the requested size."
- `src/target/shared/code/tls/macos/mod.rs:412-424` — the "exactly one wait
  balanced by exactly one signal" invariant this bug shows is violable.
- Found during bug-385 (the glibc trampoline fix): the full `cargo test` gate hung
  here, unrelated to that change. Cross-link: memory `glibc-musl-thread-entry-alignment`
  records the environmental note.
- Sibling prior macOS-TLS concurrency/lifetime bugs: bug-52 (readText encoding-error
  semaphore leak), bug-380 (failed-connect drop UAF).

## Failing Reproduction

Intermittent — surfaces under a full concurrent `cargo test` on macOS, not reliably
in isolation.

```
# On macOS (arm64), from repo root:
cargo test 2>&1 | tee /tmp/t.log
# Intermittently parks forever at:
#   test macos_tls_write_sends_capacity_over_count_byte_list_exactly has been running for over 60 seconds
# and never completes; the server child process is alive but idle (0% CPU).
```

- Observed: the test hangs indefinitely (36+ min before it was killed); the mfb TLS
  server child is alive at 0% CPU, parked on a `dispatch_semaphore_wait`.
- Expected: the test completes in a few seconds — the server reads the greeting,
  writes `ABCDE`, closes, and exits; the peer receives exactly `[65,66,67,68,69]`.

Contrast cases that work today (these bound the bug — it is NOT deterministic on
input shape):

- A prebuilt copy of the same server, driven by `openssl s_client` in a tight loop,
  completed **85/85** times across several batches (3-byte greeting, 16-byte
  greeting, immediate-stdin-EOF and lingering-stdin patterns) — the server responds
  and exits every time. The flake did not reproduce standalone.

| Environment | Details | Result |
| --- | --- | --- |
| macOS arm64, under full `cargo test` (many test binaries concurrent) | fresh `mfb build` per run, system under load | hangs ✗ (intermittent) |
| macOS arm64, prebuilt server + `s_client` loop, 85 iters | isolated, low load | works ✓ (0 hangs) |
| Linux (any) | test is `cfg(target_os="macos")` — not compiled | not run |

## Root Cause

**Unconfirmed** — the exact trigger was not pinned (intermittent; did not reproduce
in isolation). Hypotheses, most likely first:

1. **Unsignaled FOREVER wait on an error/cancellation path (most likely).**
   `emit_wait` (`src/target/shared/code/tls/macos/mod.rs:489`, called by
   `lower_tls_read_macos` at `client.rs:827` and `lower_tls_write_macos` at
   `client.rs:1253`) emits `dispatch_semaphore_wait(ctx->sem, DISPATCH_TIME_FOREVER)`.
   The invariant (mod.rs:412-424) is "exactly one wait balanced by exactly one
   signal from the completion block." If, under a handshake/teardown race, the
   `nw_connection_receive`/send completion block is never invoked — e.g. the
   connection transitions to `failed`/`cancelled` and Network.framework drops the
   pending receive completion instead of calling it — the signal never happens and
   the wait blocks forever. `accept` (`server.rs:1206`) waits similarly for an
   inbound connection and shares the hazard.
   *Confirm by:* attaching lldb to a hung server and reading the parked frame
   (which op's `dispatch_semaphore_wait`); or instrumenting each op to log
   enter/signal and running the test under load until it hangs.
2. **Listener accept vs. state-changed-handler race.** The listener context's ring
   + semaphore (mod.rs:65, 91-108) could drop or double-count a signal if a state
   transition and a new-connection callback interleave, stranding `accept`.
   *Confirm by:* same instrumentation, focused on `lower_tls_listen_macos` /
   `lower_tls_accept_macos`.
3. **`s_client` shutdown timing** interacting with the server read — least likely,
   since the isolated loop (including the immediate-stdin-EOF pattern the real test
   uses) never hung.

Disproven hypothesis (recorded so it is not re-chased): this is NOT a
short-read/`maxBytes` bug. An early lead — `readText(client, 16)` appearing to
block until 16 bytes arrive while the test sends only `"hi\n"` — was falsified:
`nw_connection_receive` is called with `min_length = 1` (`client.rs:821`), the
`readText` spec mandates a short read, and in isolation both a 3-byte and a 16-byte
greeting complete deterministically. The 3-byte "hang" seen once was the
intermittent flake coinciding with that run, not a consequence of the byte count.

## Goal

- The macOS TLS server cannot block indefinitely: every semaphore wait is either
  signaled on all completion/error/cancellation/state-change paths, or bounded by a
  deadline that raises `ErrTlsFailed` on expiry.
- `macos_tls_write_sends_capacity_over_count_byte_list_exactly` completes
  deterministically and can never wedge the suite: it gains a hard per-wait timeout
  that fails the test (rather than hanging) if the server stalls.
- The exact trigger from Root Cause is identified and named in the fix commit
  (not left as "made it more robust").

### Non-goals (must NOT change)

- **Do not weaken or delete bug-157's protection.** The byte-exactness assertion
  (`peer received [65,66,67,68,69]`) guards the CAPACITY-vs-COUNT write fix and must
  stay.
- **Do not "fix" this by only bounding the test's client-side wait.** Adding a
  timeout to the harness so the test *fails fast* is necessary (so a flake can't
  wedge `cargo test`) but is NOT sufficient — it masks the server hang. The server
  liveness bug itself must be fixed.
- **Do not change `readText` short-read semantics** (`min_length = 1`) — that is
  correct per spec; the disproven hypothesis is not the bug.
- No change to TLS wire behavior, the byte payload, or the resource/close model.

## Blast Radius

Found by grepping `emit_wait` / `dispatch_semaphore_wait` / the `lower_tls_*_macos`
entry points:

- `lower_tls_read_macos` (`client.rs:699`, waits at `client.rs:827`) — the path the
  test exercises; fixed by this bug.
- `lower_tls_write_macos` (`client.rs:1081`, waits at `client.rs:1253`) — same
  FOREVER-wait pattern; in scope (same hazard).
- `lower_tls_accept_macos` (`server.rs:1206`) — waits for an inbound connection;
  same hazard, in scope.
- `lower_tls_connect_macos` (`client.rs:3`) — already computes a `dispatch_time`
  deadline (`client.rs:470-487`: `timeoutMs > 0 ? NOW+ms : FOREVER`), so it has a
  timeout *mechanism*; audit whether every failure path still signals. Likely the
  reference for the fix.
- `lower_tls_listen_macos` (`server.rs:340`) / `lower_tls_close_macos`
  (`client.rs:1327`) — audit for the same signal-on-all-paths property.
- Other TLS tests that spawn a server/peer without a wait timeout — audit
  `tests/` for the same "hang the suite" exposure and give them bounded waits.

## Fix Design

Two layers, both required:

1. **Runtime liveness (the real fix).** Guarantee the semaphore is signaled on
   every exit of each async operation — success, error, `failed`/`cancelled` state
   transitions, and completion-block-never-called cases — OR give each op-level
   `emit_wait` a bounded `dispatch_time` deadline (as `connect` already has) and, on
   expiry, tear the connection down and raise `ErrTlsFailed`. Prefer the
   signal-on-all-paths fix once the exact unsignaled path is identified; use the
   bounded deadline as the defense-in-depth backstop. Correctness risk concentrates
   in the Network.framework state-changed handler ↔ receive/accept completion
   interleaving.
2. **Test robustness.** Give `macos_tls_write_capacity.rs` a hard wall-clock bound
   on the client wait (spawn a killer thread / bounded wait) so a future stall fails
   the test instead of hanging `cargo test` forever.

Rejected alternative: only adding the test timeout (masks the server bug — see
Non-goals). Rejected: switching `readText` to a fill-to-max read (wrong per spec,
and not the bug).

## Phases

### Phase 1 — reproduce reliably + audit (no behavior change)

- [ ] Get a deterministic (or high-rate) reproduction: run the test under load
      (parallel `cargo test`, or a stress harness that spawns the server + peer
      under contention) until it hangs; attach lldb to the parked server and record
      which `dispatch_semaphore_wait` frame it sits in. This identifies the op and
      the unsignaled path.
- [ ] Complete the blast-radius audit above: for each `lower_tls_*_macos` op, write
      down whether every failure/cancel/state path signals the semaphore.

Acceptance: the hung op + unsignaled path are named; the audit list has a verdict
per site.
Commit: —

### Phase 2 — the fix

- [ ] Fix the identified unsignaled path so the completion/error/state handler
      always signals; and/or add the bounded-deadline backstop to `emit_wait`.
- [ ] Apply the same guarantee to the in-scope sibling ops (read, write, accept,
      and any the audit flags).
- [ ] Add the hard client-wait timeout to `macos_tls_write_capacity.rs`.

Acceptance: the stress reproduction from Phase 1 no longer hangs; the byte-exactness
assertion still passes; nothing in Non-goals changed.
Commit: —

### Phase 3 — validation

- [ ] Run the full `cargo test` on macOS several times (it must never park on this
      test); run the stress harness for many iterations with zero hangs.
- [ ] Confirm no golden/expected-output churn (this is runtime-only, no goldens
      expected).

Acceptance: full macOS suite green and non-hanging across repeated runs; stress
harness 0 hangs.
Commit: —

## Validation Plan

- Regression test: `macos_tls_write_sends_capacity_over_count_byte_list_exactly`
  with a bounded client wait (fails fast instead of hanging), plus a stress loop.
- Runtime proof: server driven under contention completes deterministically; a hung
  server can no longer exist (lldb shows no permanently-parked wait; or the bounded
  wait raises `ErrTlsFailed`).
- Doc sync: none expected (behavior already matches the `readText` spec; the fix is
  a liveness guarantee, not a semantic change).
- Full suite: `cargo test` on macOS, repeated.

## Open Decisions

- Primary fix = signal-on-all-paths vs. bounded-deadline backstop. Recommend
  BOTH: fix the identified path, keep a deadline as defense-in-depth. (§Fix Design)

## Summary

The engineering risk is in Phase 1: reliably reproducing an intermittent
Network.framework liveness race and pinning the unsignaled completion path — the
fix itself is small once the path is known. The byte-correctness contract (bug-157)
and the `readText` short-read semantics are explicitly left untouched; the tempting
wrong fixes (test-only timeout, fill-to-max read) are forbidden above.
