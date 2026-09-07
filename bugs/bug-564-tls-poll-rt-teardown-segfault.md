# bug-564: `tls-poll-rt` printed all its output correctly and then SIGSEGV'd in teardown — one sighting

Last updated: 2026-09-06
Effort: unknown (one unreproduced observation)
Severity: LOW — but see "Why this is filed anyway"
Class: Runtime / teardown

Status: **OPEN — one sighting, did not reproduce**
Regression Test: — (none possible until it reproduces)

## The sighting

During bug-558's acceptance run (2026-09-06), `test-accept.sh` reported one
mismatch:

    rt-behavior/tls/tls-poll-rt/build.log:  [exit 0]  ->  [exit 139]

The program printed **all** of its expected output first —
`ready=TRUE httpResponse=TRUE loop=TRUE` — and only then took the SIGSEGV. So
the crash is **after** `RETURN 0`, in teardown, not in the logic under test.

Log: `/tmp/wt558-accept.log:575` (ephemeral; the line is quoted above because the
file will not survive).

## Why this is filed anyway

It did not reproduce: 3/3 green in isolation, and a full re-run was exit 0 with
1421 tests and 0 mismatches. One observation is thin, and the honest reading is
that it may be environmental — it ran while a peer session's `cargo test
--release` was saturating the box, and `tls-poll-rt` is a **live-network** fixture
(8.8.8.8:443).

It is filed because of its **shape**, not its frequency. `[exit 139]` *after* a
program has produced correct output is a teardown crash, and a teardown crash is
the one failure mode a `.run` golden cannot distinguish from success on a good
day. It is also the shape a double free takes, and this tree has an open cluster
of ownership work (bugs 560, 561, 562, and bug-536 shape C) that could produce
one. A second sighting with somewhere to land is worth more than a lost first.

**bug-488 is the precedent for how this closes**: a flake held open on a count,
and closed by a measured clean period rather than by a fix. Do not close this on
"could not reproduce" alone.

## What to record on a second sighting

- The full `build.log` diff, including which output lines DID appear.
- Whether the box was loaded, and by what.
- Whether the network was reachable — a live-network fixture failing to connect is
  a different bug.
- Whether any ownership change (560/561/562/536-C) had landed in between.

## Non-goals

- Do not disable or re-baseline the fixture. A golden that records `[exit 139]`
  pins a crash.
- Do not "fix" it speculatively. There is nothing to fix yet; there is something
  to watch.
