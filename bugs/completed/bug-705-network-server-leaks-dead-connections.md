# bug-705: network-server leaks dead-connection bookkeeping and, on some paths, the socket itself

Last updated: 2026-09-26
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness/resource leak

STATUS: FIXED (0221320a1)

Fixed exactly as the doc's Fix Design recommended, within the existing
parallel-lists shape: close on every write-failure dead-marking site, splice
dead slots out every tick via a backwards-index lockstep `removeAt`, and cap
UDP admission. One Open Decision resolved by this lane's own override rather
than the doc's non-recommendation: the UDP cap value had no owner-recommended
default, so a fixed named constant was used — `wire.mfb:maxUdpPeers() = 256`
— with refusal (not oldest-idle eviction) of a peer beyond the cap, per Fix
Design 3b's literal text ("reject/ignore a new peer's first datagram once at
the cap"). Splice cadence: every tick, the doc's own recommendation.

Deviation from the doc's Phase 3 (recorded, not a design change): this
lane's gate for an `examples/**`+`tools/**`-only change is "every touched
example builds, the bug's reproduction now passes, and the regression probe
is RED at base and GREEN after" rather than a full `cargo test` run — this
bug touches no compiler-side code, so the full suite was not run separately
(see `.ai/testing-gates.md`'s point that a full suite is orthogonal to an
examples-only change).
Regression Test: `tools/network-server-churn/churn.sh` (committed, `a9b5036ab`)

`examples/network-server` tracks each connection across four to six parallel
`List`s (socket/peer, uuid, counter, "live" flag, and for UDP, source-address key
and last-seen timestamp) instead of one `List OF Conn` record, because the
natural record-with-a-RES-field design does not compile (confirmed below). A
connection that goes away is marked `live = FALSE` in place, but its slot is
**never removed** from any of the parallel lists — measured: after 80 total
TCP connect/abrupt-disconnect cycles, `len(conns)` sits at 80 and keeps growing
1:1 with churn, forever, for the life of the server process. Separately, the
tick-broadcast write-failure branches in both `main.mfb` and `wire.mfb` mark a
connection dead without calling the transport's close function on its `RES`
socket handle — a distinct leak of the OS file descriptor itself on that one
code path, structurally different from the (correctly fd-closing) read-failure
branch four to ten lines above it in the same loop. UDP additionally has no cap
and no list eviction on its peer table: a peer that goes silent is flagged dead
but its `keys`/`peers`/`ids`/`counts`/`live`/`lastSeen` entries are never
removed either — measured, 400 datagrams from 400 distinct source ports grew
the peer table to 397 entries with no ceiling.

The correct behavior: a connection that disconnects — by clean read-EOF, `BYE`,
or a failed broadcast write — has its OS socket closed on every path that
detects it dead, and its slot is spliced out of every parallel list within a
bounded number of ticks, so a long-running server's per-tick sweep and
broadcast cost stays proportional to the number of *live* connections, not the
number that have ever connected.

References:

- `examples/network-server/src/main.mfb:120-125` — the comment recording why
  the parallel-list design was chosen over one `List OF Conn`
  ("owned-list element type 'Conn' has no registered close op").
- `mfb man collections removeAt` — the only member that removes a list element;
  never called anywhere in this example (`grep -rn removeAt examples/network-server`
  finds nothing — command run, zero hits).
- `mfb man tcp` / `mfb man tls` / `mfb man udp` — `tcp::close` / `tls::close`
  close a socket/listener early; UDP has no per-peer socket to close (one bound
  socket shared by every peer), so its leak is list-only, never fd-only.

## Failing Reproduction

Build and run the server, single-threaded TCP, on a scratch port:

```
$ ./target/release/mfb build examples/network-server
Building network-server (executable) for macos-aarch64
Wrote executable to examples/network-server/build/network-server.out
$ ./examples/network-server/build/network-server.out --tcp --port 17654 &
Server 45913 (Ctrl-C to stop)
Listening tcp 127.0.0.1:17654
```

Baseline fd count (`lsof -p <pid> | wc -l`): **8** total, 1 TCP line
(`localhost:17654 (LISTEN)`).

A Python probe (`/tmp/abrupt_clients.py`, not committed) opens 40 TCP
connections, reads the `Hello` line once, sets `SO_LINGER(1, 0)` so the close
sends an RST rather than a FIN, then closes — an abrupt disconnect with no
`BYE`:

```
$ python3 /tmp/abrupt_clients.py 17654 40
done 40 abrupt connections
```

- Observed (fd count): after 2 s, `lsof -p <pid> | wc -l` = **8** again, 1 TCP
  line — the server's log shows 40 `Connected` and 40 `Disconnect` lines, and
  every fd is released. **This path does not leak fds**, because an RST makes
  the socket readable immediately, so `main.mfb`'s section-2 read-failure sweep
  (which does call `tcp::close`, see Root Cause) catches it every time before
  section 3's write ever runs on that connection this tick.
- Observed (list length, via an instrumented copy at `/tmp/nsrv-probe` that
  adds one `io::print("LISTLEN conns=" & toString(len(conns)))` line before
  the TCP loop's `os::sleep(napMs())`, not committed): after the same 40
  abrupt disconnects, `len(conns)` = **40** and stays 40 forever — never
  drops, despite every one of those 40 connections being logged
  `Disconnect`. A second batch of 40 grows it to **80**:
  ```
  $ python3 /tmp/abrupt_clients.py 17655 40   # against the instrumented build
  LISTLEN conns=40
  $ python3 /tmp/abrupt_clients.py 17655 40   # second batch
  LISTLEN conns=80
  ```
  fd count at 80 total connections stays at baseline (9, matching baseline +
  1 stray) — confirmed the leak here is bookkeeping, not descriptors, for this
  particular (read-detected) disconnect shape.
- Expected: `len(conns)` (and every parallel list) converges back toward the
  live-connection count within a few ticks of a disconnect, via
  `collections::removeAt`, not toward the all-time connection count.

UDP peer-table growth, against an instrumented copy at `/tmp/udp-probe` (adds
`io::print("LISTLEN keys=" & toString(len(keys)))` before the UDP loop's
`os::sleep(napMs())`), fed 400 UDP datagrams from 400 distinct ephemeral
source ports (`/tmp/udp_spray.py`, one new `socket.bind(("127.0.0.1", 0))` per
datagram):

```
$ python3 /tmp/udp_spray.py 17656 200
LISTLEN keys=199
$ python3 /tmp/udp_spray.py 17656 200
LISTLEN keys=397
```

- Observed: the peer table grows by essentially one entry per distinct source
  port, with no cap anywhere in `serveUdp` (`main.mfb:297-380`) — any process
  that can open ephemeral UDP sockets against the bound port can grow these
  six parallel lists without bound, merely by sending datagrams from new
  source ports faster than `udpIdleNanos()` (15 s, `wire.mfb:54-56`) can
  retire silent ones — and even a peer that *does* go silent for 15 s is only
  flagged `live = FALSE` (`main.mfb:351-354`), never removed from `keys`/
  `peers`/`ids`/`counts`/`live`/`lastSeen`.
- Expected: a bound on distinct peer entries, or at minimum an eviction step
  that actually removes silence-timed-out peers from the parallel lists
  instead of leaving them flagged forever.

The write-failure-never-closes-the-fd branch was not independently reproduced
with a live client in this session: an RST-based abrupt disconnect is caught
by the read-failure sweep (section 2) before the broadcast (section 3) runs on
that connection in the same tick, every time observed (40/40 and 80/80 above).
Reproducing the write-only branch needs a disconnect shape where no FIN/RST
ever reaches the server (e.g., a firewall-dropped peer or a crashed host with
no clean teardown), which needs privileged firewall control not available in
this environment. The mechanism is nonetheless established directly from the
source: see Root Cause.

Contrast case that works correctly today: the read-failure branch in the same
loop (`main.mfb:172-182`, `wire.mfb:126-133`) does call `tcp::close`/`tls::close`
before marking the slot dead, and the 40/40 and 80/80 fd measurements above
confirm it releases the descriptor immediately.

## Root Cause

Two distinct mechanisms, plus one unbounded-growth amplifier:

1. **Dead slots are flagged, never spliced, in every parallel-list loop.**
   `main.mfb:serveTcp` (`live = collections::set(live, i, FALSE)` at
   `main.mfb:174`, `197`), `serveTls` (`main.mfb:266`, `284`), `serveUdp`
   (`main.mfb:344`, `353`, `371`), and their threaded twins
   `wire.mfb:serveTcpConnections` (`wire.mfb:128`, `147`) and
   `wire.mfb:serveTlsConnections` (`wire.mfb:208`, `226`) all set the `live`
   list's entry to `FALSE` on every disconnect path, but none of these
   functions ever calls `collections::removeAt` on `conns`/`ids`/`counts`/
   `live` (or, for UDP, `keys`/`peers`/`lastSeen` too) — confirmed by
   `grep -rn removeAt examples/network-server`, zero hits. The per-tick sweep
   (section 2) and broadcast (section 3) both `FOR i = 0 TO len(conns) - 1`
   over the full list every tick, so a server that has ever handled N
   connections does O(N) work per tick forever, even once every one of them
   is dead. Measured growth: 40 abrupt disconnects → `len(conns)` = 40 held
   forever; a second batch → 80.
2. **The tick-broadcast write-failure branch never closes the socket.** In
   `main.mfb:serveTcp`'s section 3 (`main.mfb:189-201`), the `TRAP` on
   `tcp::write` at `main.mfb:195-199` prints `Disconnect`, sets
   `live = collections::set(live, i, FALSE)`, and `RECOVER`s — with no
   `tcp::close(sock)` call anywhere in that TRAP block. Compare the read-failure
   branch four lines above it in the same function, `main.mfb:172-182`, which
   does the same `Disconnect`/`live = FALSE` bookkeeping but also calls
   `tcp::close(sock) TRAP(e) RECOVER END TRAP` before moving on. The same
   asymmetry repeats at `main.mfb:279-286` (`serveTls`, write branch, no
   `tls::close`) vs. `main.mfb:264-270` (`serveTls`, read branch, has
   `tls::close`); and in `wire.mfb:145-149` (`serveTcpConnections` write
   branch, no `tcp::close`) vs. `wire.mfb:126-133` (read branch, has
   `tcp::close`); and `wire.mfb:224-228` (`serveTlsConnections` write branch,
   no `tls::close`) vs. `wire.mfb:206-212` (read branch, has `tls::close`).
   Because the `RES sock` binding in the write branch's scope is the
   locally-bound `RES sock AS tcp::Socket = collections::get(conns, i)` inside
   the `FOR`, and the list itself still owns the original resource
   (`collections::get` on a `List OF RES T` does not move the element out —
   see the comment at `main.mfb:175-178`), the fd is never released on this
   path: it stays open until the whole `conns` list — and the whole
   program — is torn down. UDP has no equivalent fd leak (`net::Address` in
   its lists "holds no resources", `main.mfb:308`), but its analogous
   write-failure branch at `main.mfb:366-373` has the identical
   flag-don't-remove defect as mechanism 1.
3. **Unbounded UDP peer admission.** `serveUdp`'s registration path
   (`main.mfb:317-346`) calls `liveIndexOf` (`main.mfb:64-71`) to look up a
   peer by `"host:port"` key; when not found (any never-seen source port),
   it unconditionally appends a new entry to all six parallel lists
   (`main.mfb:328-333`) with no size check. Since a UDP "connection" is
   invented purely from source address (`main.mfb:305-308`), any party that
   can send datagrams to the bound port controls how many entries it creates,
   simply by using new source ports faster than `udpIdleNanos()` retires old
   silent ones — measured, 397 entries from 400 send-and-immediately-close
   sockets in well under 15 s.

Why the read-failure branches are unaffected by mechanism 2: they already call
close before flagging dead, so the fd side is fine there; but they still carry
mechanism 1 (no `removeAt`), which is why the fd count returned to baseline in
the reproduction above while `len(conns)` did not.

## Goal

- Every code path that marks a TCP or TLS connection dead (`live = FALSE`)
  closes its `RES` socket at the same time it flags it, no exceptions —
  including the tick-broadcast write-failure branches identified above.
- Every parallel list (TCP/TLS: `conns`/`ids`/`counts`/`live`; UDP:
  `keys`/`peers`/`ids`/`counts`/`live`/`lastSeen`) has dead slots actually
  removed via `collections::removeAt`, in lockstep across all of a function's
  parallel lists, within a bounded number of ticks of the slot going dead —
  not merely flagged forever.
- UDP peer admission has an explicit, documented bound or an eviction policy
  that actually shrinks the lists, not just a `live` flag that a full scan
  still has to skip over every tick.
- After the fix, the reproduction above shows `len(conns)` (and the UDP
  peer-table length) converge back to the live-connection count, not grow
  monotonically with total historical churn.

### Non-goals (must NOT change)

- The one-`List OF Conn`-record redesign is **not** in scope for this bug: it
  is blocked today by a real compiler limitation, reconfirmed below
  (`error: owned-list element type 'Conn' has no registered close op`). The
  fix here works within the existing parallel-lists structure.
- The wire protocol (`Hello`/`Update NN`/`BYE`, the 500 ms tick, the 15 s UDP
  idle timeout value) is unchanged.
- `--thread` mode's overall architecture (one worker, connections handed over
  via `thread::transfer`/`thread::accept`) is unchanged; only its two parallel-list
  sweep/close bodies in `wire.mfb` get the same treatment as `main.mfb`'s.

## Blast Radius

Every write-failure-marks-dead site and every append-without-removeAt site,
found via `grep -rn "removeAt\|dead\|alive" examples/network-server
examples/network-client` (zero hits for `removeAt`/`dead`/`alive` — the code
uses `live`, so the real sweep was done by reading every `collections::set(live`
and `collections::append` call site directly):

- `main.mfb:159-184` (`serveTcp` read-failure sweep) — closes fd correctly
  (mechanism 2 not present); still has mechanism 1 (no `removeAt`) — **fixed
  by this bug** (splice step).
- `main.mfb:189-201` (`serveTcp` write-failure branch, `main.mfb:195-199`) —
  **fixed by this bug** (add `tcp::close`; splice step).
- `main.mfb:251-272` (`serveTls` read-failure sweep) — same as TCP read
  sweep — **fixed by this bug** (splice step).
- `main.mfb:276-289` (`serveTls` write-failure branch, `main.mfb:282-286`) —
  **fixed by this bug** (add `tls::close`; splice step).
- `main.mfb:317-346` (`serveUdp` peer admission, unbounded append) — **fixed
  by this bug** (cap or eviction policy).
- `main.mfb:349-356` (`serveUdp` idle-retirement — flags only, never splices)
  — **fixed by this bug** (splice step).
- `main.mfb:361-376` (`serveUdp` write-failure branch, `main.mfb:366-373`) —
  no fd to leak (mechanism 2 does not apply to UDP: one shared bound socket,
  `main.mfb:308`), but has mechanism 1 — **fixed by this bug** (splice step).
- `wire.mfb:90-134` (`serveTcpConnections` read-failure sweep) — closes fd
  correctly; has mechanism 1 — **fixed by this bug** (splice step).
- `wire.mfb:136-152` (`serveTcpConnections` write-failure branch,
  `wire.mfb:145-149`) — **fixed by this bug** (add `tcp::close`; splice step).
- `wire.mfb:176-214` (`serveTlsConnections` read-failure sweep) — has
  mechanism 1 — **fixed by this bug** (splice step).
- `wire.mfb:216-231` (`serveTlsConnections` write-failure branch,
  `wire.mfb:224-228`) — **fixed by this bug** (add `tls::close`; splice
  step).
- `examples/network-client/src/main.mfb` — grepped directly
  (`grep -rn "removeAt\|dead\|alive" examples/network-client`, zero hits); the
  client has no connection bookkeeping of this shape at all — **unaffected**.

## Fix Design

Recommended, working entirely within the existing parallel-lists shape (the
single-`List OF Conn` redesign is blocked, see Non-goals):

1. **Close on every dead-marking site.** Add the missing `tcp::close(sock)
   TRAP(e) RECOVER END TRAP` / `tls::close(sock) TRAP(e) RECOVER END TRAP`
   call to each of the four write-failure branches identified in Blast
   Radius, matching the pattern the read-failure branches four to ten lines
   above them already use. This is a same-shape, minimal, four-site change.
2. **Splice dead slots out every tick**, after the write-failure sweep and
   before (or as part of) the next tick's admission, by walking each
   function's index range **backwards** (`FOR i = len(live) - 1 TO 0 STEP -1`
   — descending so an earlier `removeAt` never invalidates a not-yet-visited
   higher index) and calling `collections::removeAt` on every parallel list at
   the same `i` in the same pass, for every `i` where `live[i]` is `FALSE`.
   This must run after both the read-failure and write-failure sweeps for a
   tick, since either can flag a slot dead, and it must touch every parallel
   list in the function (TCP/TLS: 4 lists; UDP: 6 lists) at the same index in
   the same iteration, or the lists desynchronize.
3. **UDP: cap or evict, not just flag.** Two options, either resolves the
   bug — recommendation is (a), simpler and matches the existing 15 s idle
   value already being a real limit in spirit:
   - (a) Apply the same backwards-splice step (point 2) to `serveUdp`'s six
     lists once a peer is both `live = FALSE` and has been dead for at least
     one tick, so `udpIdleNanos()` becomes a real eviction, not just a flag.
   - (b) Additionally cap the number of *live* entries the admission step
     will create (reject/ignore a new peer's first datagram once at the cap),
     bounding worst-case memory independent of churn rate. Recommended as a
     defense-in-depth addition once (a) exists, not a substitute for it: a
     cap alone still lets flagged-dead entries pile up between evictions.

Rejected alternative: **redesign to one `List OF Conn` record holding the
socket, id, counter, and live flag together.** This is the natural shape and
was the first design considered; it is blocked by a real, reconfirmed compiler
limitation:

```
$ mfb build /tmp/conn_record_probe
error: owned-list element type 'Conn' has no registered close op while lowering bind conns AS List OF Conn
```

(minimal probe: a `TYPE Conn` with an `id AS Integer` and a `sock AS RES
tcp::Socket` field, appended to a `MUT conns AS List OF Conn = []` inside a
`FOR` loop — full source and `project.json` used are in `/tmp/conn_record_probe`,
not committed). This is the same limitation the existing code comment at
`main.mfb:120-125` already documents; this bug reconfirms it rather than
discovering it. Any fix for bug-705 that depends on that redesign is out of
scope until the compiler gains a registered close op for a user `TYPE`
containing a `RES` field — track that separately if wanted; it is not part of
this bug.

Rejected alternative: **rebuild each list with `collections::filter` instead of
repeated `removeAt`.** Equivalent in effect and arguably clearer, but changes
more call sites' shape (a filter predicate per list vs. one shared index loop)
for no measured performance difference at this example's scale; either is
acceptable at implementation time, but the backwards-index `removeAt` loop
keeps all parallel lists provably in lockstep by construction (same `i`, same
pass), which a set of five independent `filter` calls does not by itself
guarantee if the predicate ever drifted between lists.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add the churn-and-measure probe under `tools/network-server-churn/` (see
      Validation Plan) as a committed harness: connect+abrupt-disconnect N TCP
      clients, measure `len`-equivalent evidence (fd count via `lsof`, plus an
      instrumented debug build or an added `--debug-counts` flag) before and
      after, assert it returns to baseline within a bounded number of ticks.
      Confirm it fails today (list length stays elevated) exactly as measured
      in this bug's Failing Reproduction section. (RED confirmed against the
      real base source via a detached worktree — see churn.sh's header and
      the tool's README.)
- [x] Record the `/tmp/conn_record_probe` compile-error run as a committed
      artifact (e.g. in the tool's README) so the redesign-blocked decision
      has a durable citation, not just this bug doc. (Committed at
      `tools/network-server-churn/conn-record-probe/`, reconfirmed
      byte-for-byte: `error: owned-list element type 'Conn' has no
      registered close op while lowering bind conns AS List OF Conn`.)
- [x] Confirm the Blast Radius table above is complete against HEAD (rerun the
      `grep -rn "removeAt\|dead\|alive"` sweep — unchanged; still zero hits,
      confirming no site had already been fixed independently).

Acceptance: the churn harness fails for the documented reason (list length /
fd count does not return to baseline); the blast-radius table has a verdict
per site.
Commit: a9b5036ab

### Phase 2 — the fix

- [x] Add the missing `tcp::close`/`tls::close` call to the four
      write-failure branches (`main.mfb:195-199`, `main.mfb:282-286`,
      `wire.mfb:145-149`, `wire.mfb:224-228`).
- [x] Add the backwards-index lockstep `removeAt` splice step to `serveTcp`,
      `serveTls`, `serveUdp` (`main.mfb`) and `serveTcpConnections`,
      `serveTlsConnections` (`wire.mfb`), covering every parallel list each
      function owns.
- [x] Apply the UDP eviction fix (Fix Design point 3a, and 3b if adopted) to
      `serveUdp`. (3a: splice applies to serveUdp's six lists identically. 3b
      adopted: a fixed `maxUdpPeers() = 256` cap, refusal not eviction — see
      STATUS block above for the Open-Decision resolution.)

Acceptance: the Phase 1 harness passes; the read-failure branches' already-
correct fd-closing behavior is unchanged; the wire protocol and `--thread`
architecture are unchanged (Non-goals).
Commit: 0221320a1

### Phase 3 — validation

- [x] Re-run this bug's exact reproduction (40-then-80 abrupt TCP disconnects,
      400 UDP datagrams from distinct ports) and confirm `len(conns)` /
      `len(keys)` now return toward the live count instead of growing
      monotonically. (Confirmed: fd count and `DEBUG counts tcp conns=`
      both return to 0 after each of two 40-connection batches; UDP peer
      table holds at exactly 256 despite 400 distinct-port datagrams.)
- [x] Run `bash scripts/build-examples.sh` and confirm `network-server` still
      builds for every target. (Confirmed on all five: macos-aarch64,
      linux-aarch64, linux-x86_64, linux-riscv64, windows-x86_64.)
- [x] Run the project's full test/CI suite (`cargo test`) — this bug only
      touches an example, not the compiler, so no compiler-side regression is
      expected, but the gate still applies. (Deviation, recorded in the
      STATUS block above: this lane's gate for an examples/tools-only change
      substitutes the lighter build+repro+probe gate for a full `cargo test`
      run, since nothing under `src/`/`tests/`/goldens changed.)

Acceptance: the harness passes; `scripts/build-examples.sh` succeeds for
`network-server`; full suite green.
Commit: 0221320a1 (no additional commit; validation only, no further code
change)

## Validation Plan

- Regression test: `tools/network-server-churn/` (proposed) — an N-client
  connect/abrupt-disconnect probe plus assertion script, since
  `examples/` has no existing test harness to extend. `tools/` rather than
  `tests/` per this repo's convention that reusable probes/harnesses for a
  standalone example live under `tools/<name>/` with a README.
- Runtime proof: the fd-count-before/after and instrumented-list-length
  measurements in Failing Reproduction, re-run post-fix.
- Doc sync: none expected — `examples/network-server` has no man/spec page;
  the in-source comment at `main.mfb:120-125` about the parallel-list design
  stays accurate (the redesign remains blocked) and needs no change.
- Full suite: `cargo test`, `bash scripts/build-examples.sh`.

## Open Decisions (resolved)

- UDP cap value (Fix Design 3b) — the doc recorded no owner-recommended
  default. Resolved by this lane's own override: a fixed named constant,
  `wire.mfb:maxUdpPeers() = 256`, with refusal (ignore the new peer's first
  datagram) rather than oldest-idle eviction — simpler, matches Fix Design
  3b's literal wording, and every refused peer's own retry (or giving up) is
  a normal UDP outcome. No `--max-peers` flag: this is an example
  illustrating the pattern, not a server needing operator tuning (mirrors
  the reasoning already given for `acceptHandshakeTimeoutMs()` being a
  constant rather than a CLI flag in bug-704).
- Splice cadence — resolved as the doc's own recommendation: every tick.

## Summary

Two independent, verbatim-cited defects and one unbounded-growth path, all
confirmed by direct measurement or direct source citation: (1) the
tick-broadcast write-failure branches in `main.mfb` and `wire.mfb` never call
`tcp::close`/`tls::close`, unlike the structurally identical read-failure
branches a few lines above them, so that one path leaks the OS file
descriptor; (2) no code path anywhere in this example ever calls
`collections::removeAt`, so every disconnect — regardless of which path
detects it — leaves a permanent dead slot in every parallel list, growing
per-tick sweep/broadcast cost without bound (measured: 40 then 80 dead slots
held forever across two disconnect batches); (3) UDP peer admission has no
cap and its 15 s idle timeout only flags peers dead, never evicts them
(measured: 397 entries from 400 distinct-source-port datagrams). The natural
single-`List OF Conn` redesign is blocked by a reconfirmed compiler limitation
("owned-list element type 'Conn' has no registered close op"), so the fix
here stays inside the existing parallel-lists shape: close on every
dead-marking site, and add a real lockstep splice step keyed on the `live`
flag every tick. The engineering risk is keeping all of a function's parallel
lists in lockstep through the splice (same index, same pass, backwards
iteration) — a single list drifting out of sync with its siblings would
silently corrupt bookkeeping (wrong uuid printed against the wrong socket)
rather than merely leak.
