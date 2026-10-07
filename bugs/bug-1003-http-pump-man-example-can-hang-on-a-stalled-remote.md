# bug-1003: the `http::pump` man example can hang forever: it fetches a live remote (example.com) with no deadline

Last updated: 2026-10-06
Effort: small (<1h)
Severity: LOW
Class: Footgun (gate flake + missing deadline)

Status: Open
Regression Test: (proposed) the man example runs against a local peer, and/or a cooperative-read deadline test with a peer that accepts and never answers

`mfb man http pump` example 1 runs `http::startRead(net::toUrl("http://example.com/"))` and
spins on `http::done`/`http::ready`/`http::pump` with no deadline. When the remote stalls, the loop
never ends. The man-examples gate kills it at 60 s (`TIMED OUT after 60s — an example must
terminate`), failing the gate.

**Correct behavior:** a gating example must not depend on a live third-party host, and a cooperative
read should not spin forever on a silent peer. Decide which: give `startRead`/`pump` an optional
timeout like `tcp::`'s, or document that the caller owns the deadline and show one in the example.

## Measured

On 2026-10-06 the batch-17 full gate (staging dbfb7fa90) failed only `http::pump#1(run)` in
man-examples. The coordinator then ran the example's program in a loop on this Mac:
- main 468ff0f94 / 91ce9c697 (`/tmp/main-dbg-target/release/mfb`): 1 timeout (>60 s) in 13 runs; the
  others finished in 1–5 s printing `931 bytes accumulated`.
- staging: 0 timeouts in 3 runs.
The same example passed in batch 16's gates, so it is intermittent and depends on the network.
