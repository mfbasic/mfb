# bug-564: `tls` acceptance fixtures flake under a loaded full run — two sightings, neither reproducible

Last updated: 2026-09-06
Effort: unknown (one unreproduced observation)
Severity: LOW — but see "Why this is filed anyway"
Class: Runtime / teardown

Status: **OPEN — two sightings, different fixtures, neither reproduced**
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


## Second sighting (2026-09-06, same day) — a different fixture, a different shape

During bug-558's acceptance run:

    rt-behavior/tls/tls-write-peer-closed-raises-rt
      golden: write raised=TRUE
      actual: write raised=FALSE

Not a crash this time — a **behavioural flip**. The fixture starts a local
`tls::listen` on 127.0.0.1, spawns `openssl s_client` as the peer, closes the
peer, and asserts that the next `tls::write` raises. Whether it raises depends on
whether the peer's FIN has been processed by the time `write` runs, so the test is
**racy by construction**: it asserts a consequence of the peer's exit without
establishing that the exit has propagated.

### Attribution — bug-558 was EXONERATED by byte-identity, not by argument

Worth recording as a method. bug-558 changed only `src/cli/man.rs` (1 file, the
renderer), so the claim "it cannot affect a compiled program" is easy to *assert*.
It was instead **measured**: the fixture was built with the bug-558 compiler and
with a main compiler, and the two executables are byte-identical —

    558-binary: 9cb7623b2928326edacb9627b1969ba51e951072c7be7f2a6b83fe281a819194
    main-binary:9cb7623b2928326edacb9627b1969ba51e951072c7be7f2a6b83fe281a819194

A behavioural difference between two runs of the same bytes is not caused by the
change that produced them. Use this rather than "my diff looks unrelated".

### Did not reproduce

**16/16 green**, of which 8 were run with four concurrent `cargo build --release`
saturating the box specifically to provoke it. Also 6/6 green through
`test-accept.sh` in isolation.

### The pattern the two sightings share

Both are `tls` fixtures, both failed exactly once inside a **full** acceptance run
on a loaded box, and neither reproduces in isolation. That is the same profile as
bug-488 — a network-timing fixture whose failure needs the port and scheduling
pressure of hundreds of unrelated tests, which four copies of one test cannot
recreate.

**So the likely fix is in the fixtures, not the runtime**: a test that asserts a
consequence of a peer's exit should wait for the exit (or for a readable EOF)
rather than assuming it has landed. That would be a real fix rather than a
re-baseline, and it is worth doing before a third sighting costs another
investigation.
