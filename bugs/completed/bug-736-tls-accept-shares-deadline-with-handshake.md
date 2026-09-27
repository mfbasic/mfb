# bug-736: `tls::accept`'s one deadline covers both the connection wait AND the handshake — on macOS only

Last updated: 2026-09-26
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness/Footgun

Status: Closed
Regression Test: `tests/net/rt_tls_accept_handshake_budget.rs` (runtime, macOS +
Linux) and `codegen::builtins::tls::gen_macos::tests::accept_handshake_gets_a_fresh_deadline_after_the_pop`
(codegen pin, runs everywhere). The original server repro
`bugs/repro/bug-736-tls-accept-shared-deadline.mfb` is kept; see the correction
under Failing Reproduction.

STATUS: FIXED (2143bc28d)

Owner decision (2026-09-26): Option A, "follow Windows and Linux". The fix
landed that way. `lower_tls_accept_macos` computes the connection-wait deadline
in `emit_accept_deadline` and calls it again right after popping a connection,
so the handshake gets a fresh `timeoutMs` measured from the pop. Omitted is
still `DISPATCH_TIME_FOREVER` and `0` is still `DISPATCH_TIME_NOW` in both
steps, matching Linux (`poll`, then `SO_*TIMEO` with 1 µs for `0`) and Windows
(`WSAPoll`, then `SO_*TIMEO` with 1 ms for `0`). Linux and Windows code is
unchanged. `mfb man tls accept` and `mfb spec stdlib transports` now describe
the two separately bounded steps. The man change is recorded as a
clarification, not a breaking change.

Deviations and findings:

1. **The doc's own repro (connect at 10 ms, ClientHello at 299 ms) is not the
   late-pop mechanism.** That client is popped at about 10 ms, so under the
   fix its handshake budget runs to about 310 ms, and a ClientHello at 299 ms
   still fails. That is correct under the owner's contract: the handshake is
   bounded from the pop. The measurement that separates buggy from fixed is a
   connection that ARRIVES late. With connect and handshake together at 280,
   290 or 295 ms of a 300 ms accept, the unfixed server reports
   `FAILED 77050008` while the client-side handshake reports OK. The fixed
   server reports `OK after 308/318/318ms` (`/tmp` probe, macOS). The committed
   test uses wide margins instead of that edge: connect at 3000 ms, ClientHello
   at 4500 ms, bound 4000 ms.
2. **Linux and Windows bound each handshake read, not the whole handshake.**
   This doc's Root Cause says both "reload the raw value" for the handshake.
   That is true, but `SO_RCVTIMEO` is per `recv`: a peer that trickles its
   ClientHello (1 byte every 200 ms, bound 1000 ms) completes in about 4 s on
   Linux (box 2226, measured), while fixed macOS drops it at the bound. Filed
   as **bug-739**. The man page is worded to be true on every backend until
   that lands.
3. **Example follow-up done.** `examples/network-server/src/wire.mfb`:
   `acceptHandshakeTimeoutMs()` is 1000 ms with a plain rationale (the handshake
   budget alone, well inside network-client's 3000 ms). `minHandshakeMs()` and
   `serveTls` comments are rewritten, and bug-704's resonance notes carry a
   correction. With 1000 ms, which divides 3000 exactly, the UNFIXED compiler
   fails `tools/network-starvation-probe/probe.sh` 3/3 and the fixed compiler
   passes (see Phase 3), so the real example is an end-to-end witness.

A server that loops on `tls::accept(listener, timeoutMs)` intermittently drops
a perfectly well-behaved TLS client with `ErrTimeout` (server side) / a reset
or truncated handshake (client side), even though nothing is hostile, slow, or
misconfigured. The mechanism: **on macOS (Network.framework)**, `tls::accept`
computes ONE absolute deadline when the call starts and reuses it, unmodified,
for both (1) the wait for a connection to be popped off the listener's queue
and (2) the wait for that connection's TLS handshake to reach `NW_STATE_READY`
— confirmed by reading `gen_macos/server.rs` (cited below) and reproduced at
runtime. A connection that is popped late in that window — because the
client's own network/CPU scheduling pushed its `ClientHello` out, or because
its retry cadence resonates with the server's cycle length (exactly
bug-704's finding) — inherits only whatever sliver of the deadline is left for
its handshake, and the server closes it out from under the client mid-flight.
**On Linux (OpenSSL) and Windows (Schannel), this specific mechanism does NOT
exist** — both back ends reload the full, unmodified `timeoutMs` value fresh
for the handshake phase, so the handshake is never starved by time spent
waiting for the connection. That is a real but DIFFERENT deviation from the
documented contract (see Root Cause) and is called out, not silently folded
into "same bug," to avoid over-fixing a phase-neutral platform.

The single correct behavior a fix produces: **a connection that is available
to accept gets a full, un-shrunk budget to complete its handshake in, on every
backend** — `timeoutMs` bounds "how long do I wait for a connection to hand
off to a handshake," and separately (however sized) "how long does that
handshake get," but a slow connection-wait must never eat into the
handshake's budget. The two Linux/Windows back ends already behave this way in
effect (each phase gets the full value independently) but not by contract —
today's per-backend behavior is undocumented and inconsistent, which this bug
also has to resolve (see Open Decisions).

References:

- `mfb man tls accept` — "The optional timeoutMs bounds how long accept waits
  for both an inbound connection and the handshake to complete... it is
  omitted, accept blocks until a connection is ready and handshaken."
- `bugs/completed/bug-704-network-server-tls-handshake-blocks-accept-loop.md`
  — the STATUS block that first surfaced this: `serveTlsThreaded`'s bounded
  `tls::accept(server, 1500)` reliably starved a legitimate client whose
  `connectTimeoutMs=3000` divided evenly into 1500, "reproduced 5/5 runs,"
  worked around by retuning to 2200ms ("phase-shifting, not a fix," per this
  bug's assignment) rather than fixing the shared-deadline mechanism itself.
- `examples/network-server/src/wire.mfb:acceptHandshakeTimeoutMs` (lines
  50-69) and `src/main.mfb:250-256` — both already document this exact
  mechanism in comments, citing `mfb man tls accept` verbatim, and both
  currently accept it as a documented limitation rather than a bug: "That is
  fine for a connection that arrives at ~random offset into the cycle... but
  [it] would land at the SAME point in every accept cycle."

## Failing Reproduction

Server (committed at `bugs/repro/bug-736-tls-accept-shared-deadline.mfb`):
listens on 127.0.0.1:18736 and loops `tls::accept(l, 300) TRAP(e) ... END TRAP`
40 times, printing each iteration's outcome and elapsed time. Needs a
self-signed cert next to it:

```
openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem -days 397 \
  -nodes -subj "/CN=127.0.0.1" -addext "subjectAltName=IP:127.0.0.1" \
  -addext "extendedKeyUsage=serverAuth"
mfb build .   # project.json: executable, src/ = the repro .mfb above
```

Harness (`client_probe3.py`, run against the built `./build/<name>.out`):
establishes the raw TCP connection early (`connect_delay_ms` after the
server's listener comes up — well inside the 300ms budget, so it is popped
from the listener's queue almost immediately) and then *deliberately delays
sending the TLS ClientHello* until `hs_delay_ms` after the same starting
point — simulating nothing more adversarial than a client whose handshake
bytes are a little late reaching the server once its connection is already
queued (slow scheduler, slow network, or exactly bug-704's resonance case).

```python
import subprocess, sys, time, socket, ssl, select

def run_trial(hs_delay_ms, connect_delay_ms=10, port=18736):
    proc = subprocess.Popen(["./build/bug736.out"], stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, cwd="/tmp/bug736")
    t_listen = None
    while True:
        line = proc.stdout.readline()
        if not line:
            break
        ts = time.monotonic()
        if b"listening" in line:
            t_listen = ts
            break

    target_connect = t_listen + connect_delay_ms / 1000.0
    if target_connect > time.monotonic():
        time.sleep(target_connect - time.monotonic())
    raw = socket.create_connection(("127.0.0.1", port), timeout=5.0)

    target_hs = t_listen + hs_delay_ms / 1000.0
    if target_hs > time.monotonic():
        time.sleep(target_hs - time.monotonic())

    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    raw.settimeout(5.0)
    try:
        ss = ctx.wrap_socket(raw, server_hostname="127.0.0.1")
        ss.close()
        print(f"hs@{hs_delay_ms}ms: OK")
    except Exception as e:
        print(f"hs@{hs_delay_ms}ms: FAIL {type(e).__name__}: {e}")
    proc.kill(); proc.wait()

run_trial(float(sys.argv[1]), 10)
```

Run repeatedly, varying only `hs_delay_ms` (the point within the SAME 300ms
accept-loop cycle at which the already-queued client starts its handshake):

```
$ python3 client_probe3.py 285
hs@285.0ms: OK
$ python3 client_probe3.py 295
hs@295.0ms: FAIL ConnectionResetError: [Errno 54] Connection reset by peer
$ python3 client_probe3.py 297
hs@297.0ms: FAIL SSLEOFError: [SSL: UNEXPECTED_EOF_WHILE_READING] EOF occurred in violation of protocol (_ssl.c:1081)
$ python3 client_probe3.py 299
hs@299.0ms: FAIL SSLEOFError: [SSL: UNEXPECTED_EOF_WHILE_READING] EOF occurred in violation of protocol (_ssl.c:1081)
```

Server's own log for the `299` run (elapsed times measured by
`datetime::monotonicNanos()` around the same `tls::accept` call):

```
listening
accept#0 FAILED 77050008 after 301ms
```

- Observed: a client whose TCP connection was accepted into the queue at
  ~10ms into the cycle, and which starts its (otherwise completely normal,
  sub-5ms-on-loopback) TLS handshake at 295-299ms into the SAME cycle, gets
  its connection closed out from under it — server-side `ErrTimeout`
  (`77050008`), client-side connection reset / truncated handshake — instead
  of completing normally.
- Expected: the handshake completes normally regardless of when within the
  accept cycle it starts, exactly as it does in the contrast case below.

**Contrast case (works correctly today): the same client, same connection
timing, starting its handshake earlier in the same cycle:**

```
$ python3 client_probe3.py 10
hs@10.0ms: OK
$ python3 client_probe3.py 285
hs@285.0ms: OK
```

285ms succeeds, 295ms fails, with nothing about the client, the network, or
the certificate different between the two runs — only the point within the
300ms cycle at which the (already-queued) connection's handshake starts. That
isolates the deadline math, not general TLS brokenness or a slow/hostile
peer, as the cause.

| Environment | timeoutMs | hs_delay_ms | Result |
| --- | --- | --- | --- |
| macOS (Network.framework), this repro | 300 | 10 | OK ✓ |
| macOS (Network.framework), this repro | 300 | 285 | OK ✓ |
| macOS (Network.framework), this repro | 300 | 295 | FAIL ✗ (`ConnectionResetError`) |
| macOS (Network.framework), this repro | 300 | 297, 299 | FAIL ✗ (`SSLEOFError` / truncated handshake) |
| Linux (OpenSSL) | — | — | NOT reproduced — see Root Cause; each phase gets a fresh, full `timeoutMs`, not a shared/decrementing one |
| Windows (Schannel) | — | — | NOT reproduced — same reasoning as Linux, confirmed by code reading only (no Windows box exercised for this bug) |

## Root Cause

**macOS — the reported mechanism is real, cited:**
`src/codegen/builtins/tls/gen_macos/server.rs:lower_tls_accept_macos`. The
comment at line 1477 states the design outright: *"The one absolute deadline
bounds both the wait for a connection and the handshake."* Concretely:

- Lines 1478-1517 compute `DEADLINE` **once**, at the top of the call, as an
  absolute `dispatch_time(DISPATCH_TIME_NOW, timeoutMs * 1e6)` (or the
  `DISPATCH_TIME_FOREVER`/`DISPATCH_TIME_NOW` sentinels for the omitted/`0`
  cases) — this is wall-clock time, not a remaining duration.
- Line 1548 (`wait_loop`, waiting for a connection to be enqueued by the
  listener's new-connection handler) passes that same `DEADLINE` to
  `dispatch_semaphore_wait`.
- Line 1750 (`hs_loop`, waiting for the popped connection's
  `nw_connection_start` to reach `NW_STATE_READY`, i.e. handshake complete)
  passes the **identical, unmodified** `DEADLINE` value again — no
  recomputation, no "time remaining" arithmetic between the two waits.

So a connection popped at `t0 + 295ms` against a 300ms `DEADLINE` set at `t0`
enters its handshake wait with ~5ms left, not a fresh 300ms — exactly what
the repro measures.

**Linux (OpenSSL) does NOT share this mechanism — measured by code reading.**
`src/codegen/builtins/tls/gen_openssl.rs:lower_tls_accept_openssl`: the
connection-wait `poll()` (line 1776, `TIMEOUT_OFFSET`) and the handshake's
`SO_RCVTIMEO`/`SO_SNDTIMEO` (line 1877, reloading the **same**
`TIMEOUT_OFFSET` slot) both read the *raw, unmodified* `timeoutMs` value the
caller passed — there is no elapsed-time tracking between them at all. A
connection-wait that consumes 295 of a 300ms budget is followed by a
*fresh, independent* 300ms `SO_RCVTIMEO` for the handshake, not a 5ms sliver.
This means Linux never starves a handshake this way, but it also means the
combined worst-case latency of one `tls::accept(l, 300)` call is up to
~600ms (poll + handshake, each up to 300ms), not bounded by `timeoutMs` the
way the man page's "bounds... both" wording implies. That is a real,
separate deviation from the documented contract — looser than promised, not
tighter — and is explicitly NOT the bug being fixed here (see Non-goals);
flagged so a fix does not accidentally "fix" Linux into introducing the
macOS hazard while trying to tighten its bound.

**Windows (Schannel): same shape as Linux, same caveat.**
`src/codegen/builtins/tls/gen_schannel_server.rs:lower_tls_accept`, line 757's
own comment: *"WSAPoll and the handshake SO_*TIMEO both reload TIMEOUT"* —
the connection-wait `WSAPoll` (line 771) and the handshake's socket
timeout (lines 799-821, reloading the same `TIMEOUT` slot at line 799) both
use the raw, unmodified value. Confirmed by reading only; no Windows
environment was exercised for this bug (no box in `.ai/remote_systems.md` was
used — flagging that this row is unmeasured at runtime, per the "measured or
a guess" rule).

**Why the three backends disagree:** `poll`/`WSAPoll` + a socket-level
`SO_*TIMEO` are two independent OS primitives, each taking a *duration*, so
"reload the same duration for phase two" was the path of least resistance on
Linux/Windows. Network.framework's `dispatch_semaphore_wait` takes an
*absolute* `dispatch_time_t`, and the accept function computed that absolute
value once, up front, for convenience — then reused it a second time without
noticing that "absolute" is exactly what makes reuse wrong.

## Goal

- A connection's TLS handshake, once it has been popped from the accept
  queue, must get a budget that does not shrink because of time already
  spent waiting for a connection to arrive — on every backend, with a single
  documented contract that all three implement identically.

### Non-goals (must NOT change)

- `tcp::accept`'s semantics — confirmed unaffected (see Blast Radius): it has
  no handshake phase, so there is nothing for a shared deadline to interact
  with, and its own doc already commits to "a bounded accept never overruns
  its `timeoutMs`" (`src/codegen/builtins/tcp/func_accept.rs`) — a guarantee
  this bug's mechanism would violate if `tcp::accept` had a second phase, but
  it does not.
- The API decision of exactly how `tls::accept`'s contract should be
  reworded/re-parameterized — that is the owner's call (see Open Decisions);
  this doc presents options and a recommendation, not an implementation.
- Turning the Linux/Windows "each phase gets a full independent budget"
  behavior into "yes, shared and decrementing, to match macOS" without a
  decision — that would import THIS bug's hazard onto two backends that
  currently do not have it. Whichever fix design is chosen must define ONE
  contract, then bring EVERY backend to it — not just patch the one that
  currently matches the (arguably wrong) documented wording.
- `examples/network-server`'s `acceptHandshakeTimeoutMs() = 2200` /
  `minHandshakeMs() = 250` constants and their resonance-avoidance comments —
  untouched by this doc, but flagged as a required follow-up once a fix
  lands (see Fix Design's last paragraph): they exist only because this bug
  was worked around instead of fixed, and should be revisited (probably
  simplified back toward "any reasonable constant" and the resonance
  rationale deleted) once accept no longer starves a late handshake.
- Writing the fix. Per the task assignment: `tls::accept`'s semantics are an
  API-owner decision; this document stops at Open Decisions + Fix Design.

## Blast Radius

`grep -rn "tls::accept" --include="*.mfb"` across the tree, filtered to
call sites that pass a **bounded, non-huge** `timeoutMs` inside something
that actually loops (the hazard needs repeated cycles under real traffic to
matter; a single `tls::accept` in a one-shot test fixture cannot exhibit
starvation no matter what the deadline does):

- `examples/network-server/src/main.mfb:264` (`serveTls`, `budgetMs` derived
  from the tick remainder) — **affected on macOS**, already documented as a
  known limitation in the surrounding comment (quoted under References); not
  yet fixed. `minHandshakeMs()=250` in `wire.mfb` is the existing, undirected
  mitigation (skip an attempt too close to the tick boundary to finish) —
  it reduces exposure but does not close it, since a connection popped with,
  say, 251ms left can still be squeezed by a slow-arriving handshake.
- `examples/network-server/src/main.mfb:519` (`serveTlsThreaded`,
  `acceptHandshakeTimeoutMs()=2200`) — **affected on macOS**; this is
  precisely bug-704's landing site, phase-shifted rather than fixed (see
  References).
- `tools/thread-package-sources/xfer_tls_listener_worker/src/lib.mfb:18` —
  `tls::accept(server, 90000)`. A 90-second budget makes the *proportional*
  vulnerable window (a few ms out of 90000ms) negligible in practice, but
  the mechanism is present; **latent, out of scope** — no evidence of
  observed failures, and the fix (once designed) covers it for free since
  it is the same code path.
- `tests/syntax/tls/{accept_valid,accept_invalid,listen_valid,close_valid}`,
  `tests/rt-behavior/tls/tls-write-*-rt`, `tests/byte-identity/tls` — all
  call `tls::accept` at most twice per fixture, against a same-process
  client dialing in immediately (no real network latency, no loop, no
  resonance). **Unaffected**: nothing in these fixtures times a handshake
  against a deadline that a connection-wait has already spent down.
- `bugs/completed/bug-564-halfclose-probes/*.mfb` — one-shot repro probes
  for an unrelated, already-fixed bug; same reasoning, **unaffected**.
- `tcp::accept` (`src/codegen/builtins/tcp/func_accept.rs`, and every
  backend under `src/codegen/os/socket/`) — **not affected, by design**: TCP
  has no handshake step. `accept()` either has a connection ready or it does
  not; there is no second phase for a deadline to be shared with. The
  member's own doc states the guarantee this bug's mechanism would break if
  it existed here ("A bounded accept never overruns its timeoutMs"),
  confirming this was already a deliberately-kept invariant, not an
  oversight that happened to be safe.
- `udp` — has no `accept`/listener concept at all; not applicable.

## Fix Design

Three shapes were considered for what "the fix" should even mean, since this
is explicitly an API-owner decision (per the task assignment) rather than
something to implement here:

**Option A — the handshake gets its own full budget once a connection is
taken from the backlog.** Concretely: `tls::accept(l, timeoutMs)` still
bounds the *wait for a connection* by `timeoutMs`, but the moment a
connection is popped, a **fresh** `timeoutMs`-sized window starts for the
handshake (recompute the deadline rather than reusing it). This is a
one-line change on macOS (recompute `DEADLINE` right after the `pop` label,
before `hs_loop`) and requires NO change on Linux/Windows, since they
already behave this way. **Recommended.** It is the simplest change, it
makes all three backends agree without touching two of them, and it matches
what a caller almost certainly wants: "give me a bounded wait for a client,
then a bounded handshake" reads naturally as two separate bounds of the same
size, not one shrinking bound. The cost: the *worst-case total* latency of
one `tls::accept` call becomes 2×`timeoutMs` on every backend (already true
today on Linux/Windows; new on macOS) — callers computing a tick budget
(`serveTls`'s `budgetMs`) need to know this so they do not assume
`tls::accept(l, budgetMs)` returns within `budgetMs` of being called.

**Option B — a separate handshake-timeout parameter**
(`tls::accept(listener, acceptTimeoutMs, handshakeTimeoutMs)`). Gives the
caller independent control of the two phases instead of forcing them equal.
More flexible, but a bigger API surface change (new overload, new default,
new doc, new `mfb man` text, and a decision about what the 2-arg form means
once a 3-arg form exists) for a benefit most callers do not need — the
`network-server` example, the only in-tree caller with a real reason to
tune this, would likely just pass the same value twice.

**Option C — accept waits with the deadline, then hands the caller a
"pending handshake" the caller completes with its own fresh budget** (i.e.
split `accept` into two library calls, mirroring how `tcp::accept` +
something-like-`tls::wrap` might have looked, if `tls::wrap` existed — see
`.ai/net-tls.md`'s "There is no `tls::wrap`" section). **Rejected outright**:
that section explains in detail why an in-place TLS upgrade cannot exist on
macOS at all (Network.framework fixes TLS in `nw_parameters` at connection
creation), so this option is not implementable on the very backend that has
the bug.

Recommendation: **Option A.** It fixes the macOS hazard, requires no Linux/
Windows change, needs no new parameter or overload, and only shifts the
*documented* worst-case latency of the call (already the real Linux/Windows
behavior; only a `mfb man tls accept` wording update on macOS's side, not a
behavior change callers have to opt into).

Rejected middle ground: leaving macOS as-is and just widening
`acceptHandshakeTimeoutMs()`/`minHandshakeMs()` further in the example —
rejected because it is exactly the "phase-shifting, not a fix" pattern
bug-704 already tried and this task was assigned specifically to stop
repeating (a bigger constant still has a boundary; it just moves where the
resonance/edge case would need to line up, which is unfalsifiable to fully
close by tuning the example alone).

**Required follow-up once Option A (or any option) lands:** revert
`examples/network-server/src/wire.mfb`'s `acceptHandshakeTimeoutMs()` /
`minHandshakeMs()` resonance-avoidance rationale — once the handshake gets
its own fresh budget, the 3000-mod-2200 reasoning is no longer protecting
against anything, and leaving it in the example would misdocument a
non-hazard as a live one for anyone reading the code as a reference pattern.

## Open Decisions

**Resolved 2026-09-26 by the owner: Option A ("follow Windows and Linux").** The
man-page wording is treated as a clarification, not a breaking change, as
recommended. The original entries follow for the record.

- **Which fix shape** — Option A (fresh budget per phase, recommended) vs.
  Option B (separate `handshakeTimeoutMs` parameter) vs. Option C (rejected,
  not implementable on macOS). This is the API-owner's call per the task
  assignment; this doc does not implement any of them. (§Fix Design)
- **Whether the man-page wording change is itself a breaking change worth a
  version note** — Option A changes `tls::accept`'s *documented* worst-case
  latency (from "≤ timeoutMs" in the words on the page today, to "≤
  2×timeoutMs, each phase independently bounded", which is already what
  happens on Linux/Windows) even though it changes ZERO observable behavior
  on those two backends and only removes a starvation hazard on macOS.
  Recommend documenting it as a clarification/bugfix, not a breaking change,
  since the "≤ timeoutMs" wording was never actually true on 2 of 3 backends.

## Phases

### Phase 1 — failing test

- [x] `tests/net/rt_tls_accept_handshake_budget.rs`: RED against the base
      compiler (`MFB_TEST_EXE=<base>` gives `late=FAILED 77050008 4007`).

Commit: aec8f9c95

### Phase 2 — the fix

- [x] `gen_macos/server.rs`: `emit_accept_deadline`, called before the
      connection wait and again after the pop. Codegen pin
      `accept_handshake_gets_a_fresh_deadline_after_the_pop` is RED against the
      base `server.rs` and GREEN after.
- [x] `mfb man tls accept` (descriptor prose) and `mfb spec stdlib transports`.

Commit: 2143bc28d (fix), 563d38d4b (docs)

### Phase 3 — example follow-up, goldens, full validation

- [x] `examples/network-server` constants and comments, and the bug-704
      corrections.
- [x] Regenerated the two goldens the change moves. `scripts/artifact-gate.sh
      <debug mfb> all` on the tip with main merged in checked 2160 goldens and
      found 2 diffs, both `macos-aarch64.ncodesum`: `byte-identity/tls`
      (calls `tls::accept`) and `byte-identity/http` (`http::handleRequest`'s
      TLS body calls `tls::accept`). No other target moved.
- [x] Full `cargo test --no-fail-fast` under the heavy lock, on the tip with
      main merged in: `EXIT=0`, 245 `test result: ok`, none failed.
- [x] `tools/network-starvation-probe/probe.sh`. Fixed compiler: 5/5 on the
      merged tip; before the merge, 25/25 plus 5/5 under 8 CPU hogs (one
      earlier failure in about 31 runs did not recur in 55 more, and was not
      localized). Unfixed compiler with the new 1000 ms value: 3/3 OVERALL
      FAIL. Unchanged tree at base (2200 ms): 25/25.

Commit: 91da4c402 (example), d5cae981d (goldens)

## Summary

The reported mechanism is real and reproduces deterministically on macOS: an
already-queued, perfectly well-behaved TLS client can be dropped mid-
handshake because `tls::accept` (`gen_macos/server.rs:lower_tls_accept_macos`,
line 1477's own comment) computes one absolute deadline and reuses it,
unmodified, for both the connection wait and the handshake wait. Linux and
Windows do NOT share this mechanism — both reload the raw configured
`timeoutMs` fresh for the handshake phase (confirmed by code reading;
Windows unmeasured at runtime) — which means today's three backends already
disagree with each other about what the documented "bounds... both" contract
means, independent of this bug. `tcp::accept` has no handshake phase and is
unaffected, confirmed both by design (its own doc's guarantee) and by there
being no code path that could share a deadline with anything. The real risk
in a fix is not the mechanism (well understood, cited, single-line-scale on
macOS) but making the three backends agree on ONE contract without silently
tightening Linux/Windows in a way that reintroduces this exact hazard there,
or silently loosening macOS's promise in a way nothing calls out.
