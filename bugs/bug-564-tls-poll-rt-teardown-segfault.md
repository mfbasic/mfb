# bug-564: `tls` acceptance fixtures flake under a loaded full run — two sightings, neither reproducible

Last updated: 2026-09-12
Effort: small
Severity: **MEDIUM** — use-after-munmap in the `tls::close` path on macOS, reachable
by any program that closes a TLS socket shortly before exiting.
Class: Runtime / teardown

Status: **Sighting 1 DIAGNOSED AND FIXED. Sighting 2 still OPEN and unreproduced.**
Regression Test: `codegen::builtins::tls::gen_macos::tests::close_drains_to_cancelled`
(RED before the fix, GREEN after).

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

---

# Investigation, 2026-09-12

## Sighting 1: DIAGNOSED. The macOS crash report survived.

The bug doc said the evidence was lost ("ephemeral; the file will not survive").
The `build.log` was — but macOS wrote a full crash report for the same process
and it is still on the host:

    ~/Library/Logs/DiagnosticReports/tls_poll_rt.out-2026-09-06-201953.ips

(copied to `/tmp/b564/sighting1.ips` for this investigation). It is the exact
process from the sighting: `procLaunch 2026-09-06 20:19:48.85`, `captureTime
20:19:50.64`, `Segmentation fault: 11`. Four facts from it, together, name the
mechanism with no guessing left:

1. **`exception`**: `EXC_BAD_ACCESS`, `KERN_INVALID_ADDRESS at 0x10492cec0`.
   Not a null deref — a *specific* address that is not mapped.
2. **`vmregioninfo`**: `0x10492cec0 is not in any region. … ---> GAP OF 0x4000
   BYTES` between a `mapped file` region ending at `0x10492c000` and `libnetwork`
   at `0x104930000`. A 16 KiB hole: a page that **was** mapped and has been
   **unmapped**.
3. **Thread 0 (`com.apple.main-thread`)** is inside `exit` → `_fwalk`. `main`
   has returned; `_mfb_shutdown` has already run.
4. **Thread 3 is named `mfb.tls` and is the TRIGGERED thread.** Its stack is
   `start_wqthread` → `_pthread_wqthread` → `_dispatch_workloop_worker_thread` →
   `_dispatch_lane_serial_drain` → `_dispatch_client_callout` →
   `tls_poll_rt.out+0x3f63c`. `mfb.tls` is the name `tls` gives its
   per-connection `dispatch_queue_create`
   (`gen_macos/client.rs:171`, `gen_macos/server.rs:953`), and `+0x3f63c` is in
   the tail of a `0x44000` `__TEXT` — where the five `_mfb_tls_nw_*_invoke`
   trampolines are emitted (functions 87–91 of 99 in the `-ncode` dump).

So: a Network.framework callback ran on the TLS dispatch queue, after `main`
returned, and dereferenced an unmapped page.

### Why the page was unmapped

`_mfb_shutdown` calls `_mfb_arena_destroy`, which **`munmap`s every arena block**
(`mfb spec memory arenas` §"Cleanup and Reclamation";
`src/codegen/memory/arena/arena.rs:lower_arena_destroy`). The free is skipped only
for a program embedding a `thread.` runtime call
(`skip_entry_arena_destroy`, `src/codegen/engine/builder/mod.rs:1340`).
`tls-poll-rt` imports `io strings encoding tls errorCode` — no `thread.` — so the
arena **is** destroyed, and libc `exit`'s `_fwalk` then holds the process open
long enough for the queued callback to run into the hole.

And `lower_tls_close_macos` (`gen_macos/client.rs`) deliberately leaves the ctx
there to be found. Its own comment:

> NB: ctx->sem is intentionally NOT released here. nw_connection_cancel is
> asynchronous; the connection's state-changed handler still fires a "cancelled"
> transition afterwards and does dispatch_semaphore_signal(ctx->sem) … The single
> per-connection semaphore is reclaimed with the arena-allocated ctx block.

"Reclaimed with the arena" is only safe while the arena exists. At process exit it
does not.

### The asymmetry that makes this an oversight, not a design

This backend has **five** cancel sites. Four of them already drain to the terminal
`cancelled` state before returning, each with a comment naming this precise crash:

| site | drains? | pinned by |
|---|---|---|
| `connect` failure exit | yes (bug-380) | `client.rs:854` `_cancel_drain` |
| `accept` conn-fail exit | yes (bug-412) | `accept_failure_exits_drain_to_cancelled` |
| `accept` handshake-timeout exit | yes (bug-412) | same |
| `closeListener` | yes (bug-412) | `close_listener_drains_to_cancelled` |
| **`tls::close` (a Socket)** | **NO** | — |

`server.rs:10`'s doc comment on `emit_cancel_drain` already states the failure
verbatim: "a queued handler [runs] against a freed ctx after the program exits and
the arena is torn down → EXC_BAD_ACCESS on the shared `mfb.tls` serial queue
(intermittent, load-dependent)". The one cancel site left out is the ordinary,
most-executed one.

## Docs vs code: `skip_entry_arena_destroy`'s premise is wrong, and it is written down

`src/codegen/engine/builder/mod.rs:1338` says:

> The gate keeps it surgical: a program with no `thread.` runtime call can have no
> worker outliving it, and still destroys the arena byte-identically.

and `src/docs/spec/memory/04_arenas.md:288` repeats it. **The code is wrong and the
spec repeats the error.** A `tls::` program has no `thread.` symbol and does have
concurrency outliving `main`: a libdispatch serial queue holding a callback over
arena memory. This crash report is the counter-example.

That is left as a *latent* hazard rather than widened here, because the fix below
removes the only `tls` path that relied on it.

### The two sibling shapes HAVE now been audited — both are safe

Audited after the fix landed, by reading the emitters rather than by reasoning
from the predicate. Recorded here because "not checked" is an open question and
this closes it.

**`process::detach`'s reaper pthread — safe by construction.** It never touches
arena memory, and that is deliberate and already documented at
`src/codegen/builtins/process/gen_unix.rs:lower_process_reaper_helper`:

> The child pid arrives **by value** in the C first-argument register, never a
> pointer to the `Process` record: the record's arena block may be reclaimed at
> the detaching scope's exit while this thread is still blocked in `waitpid`.

> **Arena.** Arena state is per-thread and a spawned thread gets its own zeroed
> copy, so a reaper must not allocate, free, or read through `x19`. It does not:
> the whole body is register moves, `waitpid`, the errno accessor, and a return.

So `arena_destroy` cannot fault it — bug-474 had already solved this exact
problem for a different reason (a `SIGCHLD` disposition bug) and got the memory
discipline right on the way past.

**The macOS AudioQueue callbacks — safe by construction.** The callback does
dereference a state block under a mutex (`S_CLOSED`, `S_FREE_TOP`, `S_XRUNS`,
`S_COND`), so it *looks* like the `tls` shape. It is not: that block is its own
`mmap`, not arena memory. `src/codegen/builtins/audio/gen_shared.rs:33` declares
`H_STATE: usize = 64; // -> mmap'd AudioState`, and
`gen_macos_stream.rs` allocates it with a direct anonymous `mmap`
(`fd = -1`, `STATE_PAGE`). The decisive check: **`arena_alloc` does not appear
anywhere under `src/codegen/builtins/audio/`** (`grep -rn arena_alloc
src/codegen/builtins/audio/` -> no matches). `arena_destroy` walks only the
arena's own block chain, so it never unmaps this page.

**Conclusion: `tls` was the only exposed case.** The predicate at
`builder/mod.rs:1338` is still *stated* wrongly and should be corrected in place
— platform-owned callback threads (libdispatch queues, AudioQueue, the reaper)
are governed by a different rule than the `thread.` gate: each must either touch
no arena memory, or be drained before `main` returns. The three known cases now
satisfy that rule. Correcting the prose is a separate, behaviour-free change; the
gate's *condition* needs no widening.

## The fix

Apply the backend's own established `emit_cancel_drain` to `lower_tls_close_macos`,
in the position `closeListener` already uses it: after `nw_connection_cancel`,
**before** the `nw_release` / `dispatch_release` calls, so the connection, its
queue and the semaphore are all still retained for the handler that is about to
run. `cancelled` (connection state 5) is terminal, so once `ctx->state` reaches it
no handler can run afterwards.

`emit_cancel_drain` was private to `server.rs`; it is now `pub(super)`.

### RED first

`codegen::builtins::tls::gen_macos::tests::close_drains_to_cancelled`, written
before the fix and mirroring `close_listener_drains_to_cancelled`:

    test ...::close_drains_to_cancelled ... FAILED          (before)
    test result: ok. 31 passed; 0 failed                    (after, all tls:: tests)

### Positive pins — the fix must not turn `close` into a hang

A `DISPATCH_TIME_FOREVER` wait added to the hot close path is the obvious way to
make this worse, so what was measured is that close still *returns*:

* `scripts/test-accept.sh <fixed-mfb> … "*tls*"` — **30 tests, exit 0**, including
  `rt-behavior/tls/tls-poll-rt`, `tls-write-peer-closed-raises-rt`,
  `rt-behavior/resources/closed-default-tls-drop-rt` (the **scope-drop**-emitted
  close, not a written one) and `rt-behavior/threads/thread-transfer-tls-socket-rt`
  (close on a worker thread).
* `cargo test --no-fail-fast --test rt_double_close_is_refused --test
  rt_tls_listener_local_address --test rt_tls_listener_thread_transfer --test
  rt_tls_connect_allow_self_signed --test rt_macos_d4_union_state_tls` — **12
  passed, 0 failed**. `rt_double_close_is_refused` is the one that matters: the
  first close must still succeed and the second must still raise
  `ErrResourceClosed`.
* 200 executions of three TLS programs, 8 concurrently, on a box at load average
  74–144: **0 non-zero exits, 0 hangs.**

### Only an added check; no lifetime moved

Per the memory-lifetime gate: this adds a *wait* and moves no allocation, no free
and no release. `nw_release(conn)`, `dispatch_release(queue)`, the `CTX_PEND_BUF`
`arena_free` and the `CTX_PCONTENT` release all stay exactly where they were and in
the same order; the ctx and its semaphore are still reclaimed with the arena and
are still not released individually. The contract it realizes is `mfb spec memory
arenas` §"Cleanup and Reclamation" — *"the thread control block must not retain any
bare handle into a worker arena past the point that arena becomes eligible for
reclamation"* — read with a libdispatch queue as the worker: the drain is what makes
"past that point" unreachable.

### Blast radius — measured, 3 goldens

Only the macOS emitter changed, so only `macos-aarch64` moves, and only for
fixtures that `IMPORT tls`. Classified against a **pristine HEAD** binary built in
the same worktree (`git stash` → build → sum → `stash pop`); all three goldens were
byte-identical to HEAD before the change, so all three diffs are this change's:

    tests/byte-identity/tls/golden/tls_codegen_cover_rt.macos-aarch64.ncodesum
    tests/byte-identity/http/golden/http_codegen_cover_rt.macos-aarch64.ncodesum
    tests/byte-identity/resource-xfer-slots/golden/resource_xfer_slots_cover_rt.macos-aarch64.ncodesum

`linux-{aarch64,x86_64,riscv64}` and `windows-x86_64` re-summed SAME for all three.
Controls `byte-identity/strings` and `byte-identity/net` re-summed SAME.

## Sighting 2: still OPEN, and the doc misread it

The doc records sighting 2 as a "behavioural flip … whether it raises". It is not.
The fixture prints

    RETURN "raised=" & toString(err.code = errorCode::ErrConnectionClosed)

so `raised=FALSE` means the write **did** raise — with the **wrong error code**. It
is a *classification* failure, not a missing raise, and the doc's conclusion
("a test that asserts a consequence of a peer's exit should wait for the exit")
does not apply: the fixture already `process::signal(Kill)`s the peer and
`process::waitFor`s it, and already uses bug-467's deterministic 64 KiB × 200
shape. It is not racy in the way the doc says.

### The mechanism it most likely is — an ordering bug, not yet proven

`tls::write`'s terminal-state guard is an **unsynchronised poll** of two ctx slots
written by a handler on another thread (`client.rs:1571`, `client.rs:1668`):

    writer (main thread)         STATE_INVOKE (mfb.tls queue)
    ────────────────────         ────────────────────────────
                                 1. store CTX_STATE  <- 4 (failed)
    a. load CTX_STATE ; >= 4 ?   2. store CTX_ERROR
    b. load CTX_EDOM  ; == 1 ?   3. call nw_error_get_error_domain(err)
       1 -> ErrConnectionClosed  4. store CTX_EDOM   <- domain
       else -> ErrTlsFailed

The gate (`CTX_STATE`) is published in step 1, **before** the payload it implies
(`CTX_EDOM`) in step 4 — with an indirect call into Network.framework in between.
A writer that reads `CTX_STATE` inside that window sees `4` and `CTX_EDOM == 0`
(zeroed at ctx setup) and reports `ErrTlsFailed` → `raised=FALSE`. Exactly one
fixture, exactly once, only under load.

`SEND_INVOKE` has the same store order and is *safe*, because its reader waits on
the semaphore that is signalled after both stores. `STATE_INVOKE`'s reader waits on
nothing.

**Not landed, because it is not proven and the obvious fix is incomplete.**
Swapping the two stores fixes *program* order only; AArch64 is weakly ordered and
two plain stores to different cache lines can be observed out of order by another
core, so a correct fix needs a release barrier — and this ABI layer emits no
memory barrier at all (`grep -ri 'dmb|barrier|fence' src/codegen src/arch
src/target/shared` → no instruction, only prose). Adding one is a real change to
the encoder and wants its own bug with a reproduction behind it.

## Reproduction attempts — what was tried and what it cost

All on the macOS host, with a peer session's `cargo test --release` already
running; extra load from 14 spinning shell hogs. Load average observed 12 → 144.

| what | shape | runs | result |
|---|---|---|---|
| `tls-write-peer-closed-raises-rt` (the sighting-2 binary) | serial, under load | 60 | 60/60 `write raised=TRUE`; `cert`/`empty`/`deadline` all correct |
| a 4-connect/4-close/exit probe (narrowest close→`exit` window) | serial, under load | 200 | 0 non-zero exits |
| `tls-poll-rt` (the sighting-1 binary) | **8 concurrent**, 15 rounds | 120 | 0 non-zero exits |

**Neither sighting reproduced live**, in ~380 executions. That is the honest
result, and it is also why the crash report matters: sighting 1 is diagnosed from
a *post-mortem artifact*, not from a live repro.

### Ruled out

* **Not `timeoutMs=0`** (`.ai/net-tls.md`'s classic `tls` fixture trap):
  `tls-poll-rt` passes `30000`.
* **Not network reachability.** 8.8.8.8:443 answered on every one of the 320 live
  runs; the fixture printed `httpResponse=TRUE` each time.
* **Not the `tls-write-peer-closed-raises-rt` fixture being racy about the peer's
  exit**, as the doc proposed — it kills and reaps the peer and uses the
  deterministic large-write shape.
* **Not `rt_macos_tls_write_capacity`'s known CPU-starvation timeout** — a
  different test, not touched here.

### The instrument gap, and how it was closed

The reason this sat as "unreproduced" is that the harness records only
`[exit 139]`. **`~/Library/Logs/DiagnosticReports/<program>.out-<date>.ips` is
written for every fixture SIGSEGV and survives the harness.** For any future
`[exit 139]`/`[exit 138]`/`[exit 134]` in an acceptance run, read it first:

    ls -lt ~/Library/Logs/DiagnosticReports | head
    python3 -c "import json,sys; raw=open(sys.argv[1]).read(); i=raw.index(chr(10)); \
      b=json.loads(raw[i:]); print(b['exception'], b.get('vmregioninfo','')); \
      [print(t.get('name'), [f.get('symbol') or hex(f['imageOffset']) for f in t['frames'][:8]]) \
       for t in b['threads']]" <report.ips>

`vmregioninfo`'s "GAP OF … BYTES" is the tell that separates a use-after-**munmap**
from an ordinary dangling pointer, and the thread NAME (`mfb.tls`) localises the
subsystem before any symbolisation.

The remaining gap is that nothing in the tree *harvests* those reports: a fixture
SIGSEGV in CI on a Linux box leaves no equivalent, and `test-accept.sh` does not
copy the macOS report next to the failing `build.log`. That is worth doing and is
not done here.
