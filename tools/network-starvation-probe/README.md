# tools/network-starvation-probe

Regression probe for bug-704: one idle, unauthenticated raw TCP connection against
`examples/network-server --tls --thread` must not be able to starve every other
client's TLS handshake by parking the server's single accept thread inside its
handshake read forever.

- **probe.sh** — builds `examples/network-server` and `examples/network-client`
  with the given `mfb` binary, starts the threaded TLS server on a scratch port,
  opens one idle raw TCP connection to it and never sends anything on it, then
  runs the real client and asserts it still completes its TLS handshake and
  receives all five `Update` lines within a bounded wall-clock time (phase A,
  "attack" — this is the case bug-704 documents as hanging). It then closes the
  idle connection and asserts a following client run also succeeds within a
  bound (phase B, "recovery" — guards against a fix that "solves" starvation by
  wedging the accept loop some other way). Kills every process it started
  (server, idle-socket helper, client) on exit, including on failure.

      bash tools/network-starvation-probe/probe.sh target/release/mfb

Before bug-704's fix, phase A fails (times out) because `serveTlsThreaded`'s
`tls::accept(server)` call has no `timeoutMs` and blocks forever inside the idle
peer's handshake. After the fix (a bounded `tls::accept(server, timeoutMs)`),
phase A passes because the accept call gives up on the idle peer and moves on to
the queued legitimate client.

This is a `tools/` probe, not a `scripts/` gate: it drives external OS processes
(the compiled example binaries and a raw Python socket) rather than being
expressible as a single `.mfb` fixture or exercised by `cargo test`, and it is
reusable/rerun after the fix rather than a one-off tied to a single session — the
two properties `scripts/README.md` uses to draw the `tools/` vs. `scripts/`
line.

Readiness note (see `.ai/testing-gates.md` "An ephemeral port picked with
bind-then-release hands a case the WRONG server"): the script picks a random
scratch port and confirms the SERVER PROCESS IT STARTED is the one that bound it
by grepping that child's own stdout for its "Listening tls" line, retrying on a
fresh port if the child exited first (lost the bind to a concurrent process on
this shared machine) — not a bare "is something listening" `connect()` probe,
which cannot tell this script's server from a sibling agent's.
