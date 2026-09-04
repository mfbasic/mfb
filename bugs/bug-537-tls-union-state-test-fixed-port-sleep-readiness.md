# bug-537: `rt_macos_d4_union_state_tls` listens on a fixed port and takes a fixed sleep as readiness — fails under load or beside a peer's run

Last updated: 2026-09-04
Effort: small (<1h)
Severity: LOW
Class: Footgun (flaky test — a real fix is masked as environment noise, or a green tree reads red)

Status: Open (observed once in a full `cargo test --no-fail-fast` on `worktree-B-509` at `fec4ceddc` while probe builds ran alongside; green in isolation immediately after, and green in the previous full run of the same test on the same code)
Regression Test: the test itself, run under load (`for i in $(seq 8); do (while :; do :; done) & done`) twenty times

`tests/rt_macos_d4_union_state_tls.rs` starts an MFB TLS server on `PORT: u16 =
18461`, `sleep(1000 ms)`, then runs `openssl s_client -connect` and asserts the
server echoes `ABCDE`. Observed failure: `peer did not receive the STATE bytes
[65, 66, 67, 68, 69] … got []` — the client connected to nothing (or to a
different process) and read EOF. Nothing about the union/STATE mechanism the
test proves was involved: the same binary passed the test alone 1.8 s later.

Two independent hazards, both named in `.ai/testing-gates.md`:

- **Fixed sleep as readiness.** Under CPU load (a concurrent artifact gate, a
  peer's `cargo test`, a release build) the server may not be listening within
  1000 ms; the doc's rule is "the fix is not a bigger sleep — make a live child
  part of the readiness condition".
- **Fixed port.** Any two copies of this test on the machine (two sessions, two
  worktrees) race for 18461; the loser's `s_client` talks to the winner's server
  or to a closed port. The sibling `rt_tls_connect_allow_self_signed.rs` had the
  same class (bug-477/488) and moved to a live-child readiness probe with retry.

The single correct behaviour a fix produces: the test passes 20/20 under an
eight-hog CPU load and with two copies running concurrently.

## Failing Reproduction

```
cargo test --test rt_macos_d4_union_state_tls   # while another full suite / gate runs
```

- Observed (once, 2026-09-04, `/tmp/b509-full2.log`): `got []`.
- Expected: `ok`.

## Root Cause

`tests/rt_macos_d4_union_state_tls.rs:macos_union_state_over_live_tls_socket_does_not_corrupt_the_handle`
— `const PORT: u16 = 18461` and `std::thread::sleep(Duration::from_millis(1000))`
before the client connects; no readiness probe, no retry, no ownership check.

## Goal

- Readiness = the server child is alive AND a `TcpStream::connect` to its port
  succeeds (poll with a deadline), as `rt_tls_connect_allow_self_signed.rs` does.
- The port is chosen per run (the MFB server takes it from an env var or
  argument) rather than fixed, so two copies cannot collide.

### Non-goals (must NOT change)

- The assertion itself (the exact STATE bytes echoed over a live `tls::Socket`)
  — it is the plan-80 D4 proof and must keep exercising the real handle.
- A longer sleep is explicitly not the fix.

## Blast Radius

- `tests/rt_macos_tls_write_capacity.rs` — the doc says this test "mirrors the
  loopback harness" of that sibling; audit it for the same fixed port / sleep.
- `tests/rt_tls_connect_allow_self_signed.rs` — already fixed (bug-477).

## Fix Design

Pass the port through the program's environment (`os::getEnvOr("PORT", …)`),
pick it with a bind-then-release helper guarded by the `try_wait` + connect probe
from `rt_tls_connect_allow_self_signed.rs`, and hold that test's process-wide
port mutex through the bind.

## Phases

### Phase 1 — failing test + audit
- [ ] Reproduce under the eight-hog load; audit `rt_macos_tls_write_capacity.rs`.
Commit: —

### Phase 2 — the fix
- [ ] Ephemeral port via env + live-child readiness probe with retry.
Commit: —

### Phase 3 — validation
- [ ] 20/20 under load; two concurrent copies both green.
Commit: —

## Validation Plan

- Regression: the test under load, twenty runs.
- Doc sync: none (test-only).
- Full suite: `cargo test --no-fail-fast -- --skip artifact_gate_all`.

## Summary

Harness-only. Found while landing bug-509 (whose files it never touches); recorded
so the next red is not mistaken for a regression.
