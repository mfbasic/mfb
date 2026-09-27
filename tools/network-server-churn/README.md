# tools/network-server-churn

Regression probe for bug-705: `examples/network-server` never removed a
disconnected slot from its parallel-list bookkeeping (only ever flagged it
`live = FALSE`), so a long-running server's per-tick sweep/broadcast cost —
and, on one path, its open file descriptor count — grew with total historical
churn instead of tracking the number of connections still attached. UDP peer
admission additionally had no cap.

- **churn.sh** — builds `examples/network-server` with the given `mfb`
  binary, then:
  - **Phase A (TCP churn).** Connects 40 clients, reads each one's `Hello`
    line, then disconnects abruptly with `SO_LINGER(1, 0)` (an RST, not a
    FIN — this is the write-failure path from the client's perspective, but
    exercises the same splice code either sweep uses). Asserts the server's
    own `DEBUG counts tcp conns=` line (printed once per tick when the
    server is run with `--debug-counts`) converges to 0 within a bounded
    number of ticks. Repeats with a second batch of 40 and asserts
    convergence again — not just eventual convergence at the very end, which
    would hide a bug that only "resolves" for an unrelated reason.
  - **Phase B (UDP admission cap).** Sends 400 datagrams from 400 distinct
    ephemeral source ports and asserts the server's `DEBUG counts udp
    keys=` line never exceeds `wire.mfb:maxUdpPeers()` (256).

      bash tools/network-server-churn/churn.sh target/release/mfb

`--debug-counts` is not a private test harness bolted onto a copy of the
example — it is a real flag on the shipped `network-server` binary, added by
this bug's fix (`examples/network-server/src/main.mfb`), so this probe drives
the actual example's actual splice/cap logic through its actual CLI, not a
forked or instrumented duplicate.

Before the fix: phase A's `conns` count sticks at 40 after batch 1 and 80
after batch 2 (matching the bug doc's own measurement exactly); phase B's
peer count grows past the cap (measured 397-398 of 400 sent). Verified RED
against the real, unfixed source via a detached `git worktree add --detach`
at the pre-fix commit, with only the `--debug-counts` instrumentation (no
close/splice/cap changes) layered on top so the probe's own observability
hook could exist without smuggling in the fix.

This is a `tools/` probe, not a `scripts/` gate: like
`tools/network-starvation-probe`, it drives external OS processes (the
compiled example binary and raw Python sockets) that a single `.mfb` fixture
or `cargo test` cannot express, and it is meant to be re-run, not a one-off
tied to a single session.

Readiness note (see `.ai/testing-gates.md` "An ephemeral port picked with
bind-then-release hands a case the WRONG server"): the script picks a random
scratch port and confirms the SERVER PROCESS IT STARTED is the one that bound
it by grepping that child's own stdout for its "Listening" line, retrying on
a fresh port if the child exited first — not a bare `connect()` probe.

If `wire.mfb:maxUdpPeers()` ever changes, update the `CAP=256` line in
`churn.sh` to match.

- **conn-record-probe/** — a minimal project that reconfirms the compiler
  limitation bug-705's Fix Design cites as the reason `examples/network-server`
  cannot hold one connection's socket, id, counter and live flag together in a
  single `List OF Conn` record (the natural design, rejected as out of scope
  for this bug): a `TYPE Conn` with an `id AS Integer` and a
  `sock AS RES tcp::Socket` field, appended to a `MUT conns AS List OF Conn`
  inside a loop. Building it reproduces the exact error the bug doc quotes:

      $ mfb build tools/network-server-churn/conn-record-probe
      error: owned-list element type 'Conn' has no registered close op while lowering bind conns AS List OF Conn

  This is a durable citation for the redesign-blocked decision, not a probe
  this repo's gates run (a build that is EXPECTED to fail is not something
  `scripts/build-examples.sh` or any other build gate should touch — it lives
  under `tools/` and outside `examples/` for exactly that reason).
