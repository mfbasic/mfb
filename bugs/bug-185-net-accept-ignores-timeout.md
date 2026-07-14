# bug-185: net.accept ignores its timeoutMs argument → indefinite block / DoS

Last updated: 2026-07-14
Effort: medium (1h–2h)
Severity: HIGH
Class: Security

Status: Open
Regression Test: tests/rt-behavior/net_accept_timeout (to be added)

`net.accept(listener, timeoutMs)` accepts a timeout argument, stores it to a
stack slot, and then never reads it — the accept is a bare, unconditionally
blocking `accept(fd, NULL, NULL)`. A caller that passes a finite `timeoutMs`
expecting the call to return after that deadline instead blocks forever when no
client connects. Because thread cancellation is cooperative-only (OS-08), a
worker parked in this syscall cannot be interrupted, so a remote party that
simply never completes a connection can wedge a server's accept loop
indefinitely. The single correct behavior a fix produces: a positive `timeoutMs`
bounds the wait and returns a catchable timeout error (mirroring `net.read`);
`timeoutMs <= 0` retains today's blocking semantics.

This is the still-open audit-1 finding **OS-02**, re-verified against current
code. See `planning/audit-2-fs-net-thread.md`.

References:

- `planning/audit-2-fs-net-thread.md` (OS-02), `planning/old-plans/audit-1-fs-net-thread.md`
- Correct sibling: `net.read` implements the timeout it advertises — EAGAIN →
  `ERR_READ_TIMEOUT_CODE` (`src/target/shared/code/net/io.rs:454-483`), and
  `net.poll` bounds its wait with `poll(..., timeoutMs)`
  (`src/target/shared/code/net/poll.rs:61`).

## Failing Reproduction

```
mfb init /tmp/acceptproj
cat > /tmp/acceptproj/src/main.mfb <<'EOF'
IMPORT net
FUNC main() AS Integer
  LET l AS net::Listener = net::listen(0)   ' bind an ephemeral port, no client
  ' Expect this to return a timeout after ~500ms; instead it blocks forever.
  LET c AS net::Connection = net::accept(l, 500)
  PRINT "accepted"
  RETURN 0
END FUNC
EOF
mfb build /tmp/acceptproj && /tmp/acceptproj/target/*/acceptproj &
sleep 3; echo "still blocked after 3s (expected: timed out at 0.5s)"; kill %1
```

- Observed: the process blocks indefinitely; `accepted` is never printed and no
  timeout error is raised.
- Expected: `net::accept` returns/raises a catchable timeout error at ~500ms.

Contrast: `net::read(sock, n, 500)` on an idle socket *does* time out at ~500ms
via the EAGAIN path — accept is the outlier.

## Root Cause

`src/target/shared/code/net/io.rs:21` `lower_net_accept_helper`. At `:47` the
timeout arg `ARG[1]` is stored to `TIMEOUT_OFFSET` on the stack, but no later
instruction reads it. The accept loop (`:55-61`) issues `accept(fd, NULL, NULL)`
directly — there is no preceding `poll(fd, POLLIN, timeoutMs)` gate and no
`SO_RCVTIMEO` set on the listening fd — so the syscall blocks until a connection
arrives or a signal interrupts it (the `:88-100` EINTR retry re-issues the
blocking call, it does not honor the deadline). The stored timeout is dead.

## Goal

- `net.accept(l, timeoutMs)` with `timeoutMs > 0` returns a catchable timeout
  error if no connection arrives within the deadline; `timeoutMs <= 0` blocks as
  today.

### Non-goals (must NOT change)

- Do not change the `net.accept` signature or its blocking behavior when
  `timeoutMs <= 0`.
- Do not alter the EINTR retry semantics for the blocking case (bug-115).
- Do not change `net.read`/`net.poll`, which are already correct — reuse their
  pattern.

## Blast Radius

- `net/io.rs` `lower_net_accept_helper` — fixed by this bug.
- `net.read`/`net.write`/`net.poll` — already honor timeouts; unaffected,
  serve as the template.
- `net.connect` default (unbounded when `timeoutMs <= 0`) — related OS-05
  concern, tracked separately in `planning/audit-2-fs-net-thread.md`; not fixed here.

## Fix Design

Before the blocking `accept`, when `timeoutMs > 0`, gate the listening fd with a
`poll(fd, POLLIN, timeoutMs)` exactly as `net.read` does: on `poll` returning 0
(no readiness) raise the timeout error; on readiness fall through to `accept`
(which then returns immediately). Keep the EINTR-retry structure by re-issuing
`poll` with the remaining deadline. The `net.poll` helper
(`net/poll.rs`) already encodes the pollfd + EINTR logic and can be shared.
Rejected alternative: `SO_RCVTIMEO` on the listener — less portable for accept
and does not compose with the existing poll helper.

## Phases

### Phase 1 — failing test + audit
- [ ] Add the idle-listener reproduction as a rt-behavior test asserting a
      timeout return within a tolerance window; confirm it currently hangs.

### Phase 2 — the fix
- [ ] Insert the `poll`-gated wait into `lower_net_accept_helper`, guarded by
      `timeoutMs > 0`, reusing the `net.poll` pollfd/EINTR pattern.

### Phase 3 — validation
- [ ] Full acceptance suite green; verify the blocking (`timeoutMs <= 0`) path is
      unchanged and the timed path returns on both Linux and macOS.

## Validation Plan

- Regression test: idle-listener accept with a finite timeout returns a timeout
  error within tolerance.
- Runtime proof: the reproduction prints the timeout path instead of hanging.
- Full suite: `scripts/test-accept.sh`.

## Summary

The engineering risk is the syscall-level timing (poll deadline arithmetic +
EINTR retry), all of which already exists in `net.read`/`net.poll` and can be
reused. The blocking default must stay bit-for-bit unchanged.
