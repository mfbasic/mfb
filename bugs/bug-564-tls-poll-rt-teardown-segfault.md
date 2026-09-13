# bug-564: `tls` acceptance fixtures flake under a loaded full run — two sightings, neither reproducible

Last updated: 2026-09-12
Effort: small
Severity: **MEDIUM** — use-after-munmap in the `tls::close` path on macOS, reachable
by any program that closes a TLS socket shortly before exiting.
Class: Runtime / teardown

Status: **Sighting 1 FIXED. Sighting 2 FIXED** (branch
`bug-564-s2-store-release`: `fd8ba0620`, `d474a4830`, `c60873a7e`,
`c39e7177a`, `9ab9a10ae`). The encoder gained store-release and load-acquire. The handlers
publish the domain before the gate with `stlr`, and `tls::write` loads the gate
with `ldar`. See "Sighting 2: FIXED". **One finding stays OPEN**, and it is not
the ordering race: on macOS a `tls::write` after the peer's clean
`close_notify` never raises. It fails the same way on the pre-fix compiler.
See "OPEN: a write after a clean close_notify never raises".
Regression Test: `codegen::builtins::tls::gen_macos::tests::close_drains_to_cancelled`
(sighting 1). For sighting 2:
`codegen::builtins::tls::gen_macos::tests::{trampolines_publish_the_error_domain_before_the_gate,
write_loads_its_gates_by_load_acquire}` and
`arch::aarch64::encode::tests::encodes_load_acquire_store_release`, with the
positive runtime pin `rt_macos_tls_write_after_clean_close`.

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

## Sighting 2: REPRODUCED and diagnosed (history; it was blocked on a release store until "Sighting 2: FIXED" below)

The doc records sighting 2 as a "behavioural flip … whether it raises". It is not.
The fixture prints

    RETURN "raised=" & toString(err.code = errorCode::ErrConnectionClosed)

so `raised=FALSE` means the write **did** raise — with the **wrong error code**. It
is a *classification* failure, not a missing raise, and the doc's conclusion
("a test that asserts a consequence of a peer's exit should wait for the exit")
does not apply: the fixture already `process::signal(Kill)`s the peer and
`process::waitFor`s it, and already uses bug-467's deterministic 64 KiB × 200
shape. It is not racy in the way the doc says.

### The mechanism: a publication-order race. REPRODUCED (2026-09-12)

`tls::write` reads two ctx slots that a handler on another thread writes, and it
does so without synchronisation. It reads a gate first, then the payload
(`gen_macos/client.rs:lower_tls_write_macos`; the only readers of `CTX_EDOM` and
`CTX_ERROR`, from `grep -n 'load_u.*CTX_EDOM\b\|load_u.*CTX_ERROR\b'`). There
are two paths:

    writer (main thread)         STATE_INVOKE (mfb.tls queue)
    ────────────────────         ────────────────────────────
                                 1. store CTX_STATE  <- 4 (failed)
    a. load CTX_STATE ; >= 4 ?   2. store CTX_ERROR
    b. load CTX_EDOM  ; == 1 ?   3. call nw_error_get_error_domain(err)
       1 -> ErrConnectionClosed  4. store CTX_EDOM   <- domain
       else -> ErrTlsFailed

The gate (`CTX_STATE`) is published in step 1, **before** the payload it implies
(`CTX_EDOM`) in step 4, and an indirect call into Network.framework sits between
them. A writer that reads `CTX_STATE` inside that window sees `4` with
`CTX_EDOM == 0` (zeroed at ctx setup) and reports `ErrTlsFailed`, which prints
`raised=FALSE`.

The second path runs through the send wait: the writer reads `CTX_ERROR` and then
`CTX_EDOM`. **An earlier version of this doc called `SEND_INVOKE` safe; it is
not.** Its reader waits on `CTX_SEM`, but `STATE_INVOKE` signals that same
semaphore, so the writer can wake on the state handler's signal and read a
`CTX_ERROR` that either trampoline has stored and not yet classified.

#### The instrument: `dlsym` interposition, no compiler change

`tls` resolves every Network.framework symbol through `dlsym` at run time. That
includes `nw_error_get_error_domain`, which is parked in `CTX_EDOMFN` and called
from both trampolines. A `DYLD_INSERT_LIBRARIES` dylib that interposes `dlsym` can
therefore wrap the exact call that sits between the gate store and the payload
store, in the real fixture binary, with no instrumented compiler. The wrapper logs
the domain and code, plus its return address, which identifies the calling
trampoline: `str x2,[x19,#0x20]` precedes one call and `str x1,[x19,#0x20]` the
other (`otool -tv`). It can also `usleep` there, and a second wrapper on
`dispatch_data_create` slows the writer before its guard. SIP strips `DYLD_*`
from `/bin/bash` and `xargs`, so a driver has to inject the variable at the
fixture's own exec. The instrument lived in `/tmp` and was never committed.

#### What it measured

All runs were `tls-write-peer-closed-raises-rt`, built by this tree's release
compiler, on the macOS host (12 cores) while peer sessions loaded it; load
averages 27–49. Tallies come from each run's stdout. "STATE-first" means the
first `nw_error` classified in the run came from `STATE_INVOKE`.

| set | build | runs × concurrency | `write raised=FALSE` | STATE-first runs (FALSE among them) | exit≠0 |
|---|---|---|---|---|---|
| uninstrumented | pre-fix | 240 × 8 | 0 | n/a | 0 |
| uninstrumented | pre-fix | 600 × 12 | 0 | n/a | 0 |
| interposed, log only | pre-fix | 240 × 8 | **5** | 19 (**5**) | 0 |
| interposed, 50 ms in the domain call | pre-fix | 40 × 4 | 0 | 0 | 0 |
| interposed, 50 ms domain + 150 ms writer | pre-fix | 4 serial + 40 × 4 | **1** + 0 | 1 (1) + 2 (0) | 0 |
| **matched pair, log only, run side by side** | **pre-fix** | **600 × 8** | **6** | **21 (6)** | 0 |
| **matched pair, log only, run side by side** | **gate-last reorder** | **600 × 8** | **0** | **23 (0)** | 0 |

Taken together:

* **Every `raised=FALSE`, 12 of 12, is a STATE-first run.** Of 1,400+ SEND-first
  runs, none failed. Every failure recorded POSIX domain 1 (code 54, ECONNRESET),
  so `ErrConnectionClosed` was the right answer and the writer read it too early.
* **It is the window, not the peer.** Across the matched pair, 6 of 21
  opportunities failed on the pre-fix build, and 0 of 23 failed once the
  trampolines publish the domain before the gate, under the same load at the same
  time. If the rate among opportunities were unchanged (~29%), 0 of 23 would
  happen by chance with probability about 0.0004.
* **A long sleep in the domain call HIDES the race.** When the handler is slowed,
  the send completion almost always classifies first. That is the "a publication
  race needs both sides slowed" trap in its purest form. The widening that worked
  was the logging wrapper's own `write(2)` syscall, which lengthens the window
  without reordering the handlers.
* **The uninstrumented binary did not fail in 840 runs.** The unwidened window is
  one indirect call, so a full acceptance run reaching it once is consistent
  with a rate this low. Sighting 2 is this bug; it was never a fixture wait.
* No crash report was written by any run (`ls -lt
  ~/Library/Logs/DiagnosticReports`, newest file 08:50, before this work began).

### The contract it breaks

`mfb spec stdlib transports` (`src/docs/spec/stdlib/17_transports.md`): "A write
to a peer that has gone away raises `ErrConnectionClosed` … on every target …
`ErrTlsFailed` stays what it has always meant on `tls` — a handshake, certificate
or protocol failure". `mfb man tls write` says the same. The failing runs raise
`ErrTlsFailed` for a peer that has gone away, so the code is wrong and the docs
are right.

### Why this is BLOCKED rather than fixed

Reordering the stores so that each trampoline saves its arguments in the frame,
classifies, and stores `CTX_ERROR`/`CTX_STATE` last closes the window as it was
observed (matched pair above). **It is still not a correct fix on AArch64, and it
is not landed.** The earlier version of this doc was right about that. Here is the
evidence it lacked:

* **ARMv8's memory model allows plain `STR`s to be observed out of program
  order.** Chong, Sorensen & Wickerson, *The Semantics of Transactions and Weak
  Memory in x86, Power, ARM, and C++*, PLDI'18, §6 (first author at Arm Ltd.):
  "The ARMv8 memory model … like Power, it permits several relaxations to the
  program order. Unwanted relaxations can be inhibited either using barriers
  (DMB, DMB LD, DMB ST, ISB) or using release/acquire instructions (LDAR, STLR)
  that act like one-way fences." The A64 instruction reference entry for `STLR`
  says it "also has memory ordering semantics as described in Load-Acquire,
  Store-Release".
* **The toolchain agrees.** `clang -O2` compiles C11 `atomic_store`/`atomic_load`
  on this host to `stlr`/`ldar`, not `str`/`ldr`: a disassembled
  `publish()/classify()` pair doing exactly this gate/payload publication, checked
  with `otool -tv`.
* **This ABI layer can emit none of the fence instructions.** `grep -rniE
  'stlr|ldar|dmb|store.release|load.acquire' src/arch src/target/shared
  src/codegen/engine` returns no matches.

So a program-order reorder built from plain `str` depends on a guarantee the
architecture does not give. It would pass every test here and could still misreport
on a core that exercises the relaxation. That is the half-fix this investigation
was told not to land.

### Options and their costs

1. **Teach the encoder a release store, then reorder.** Add a store-release op
   (`STLR`, a base-register-only addressing form, so it is `add` + `stlr`), store
   the gate with it, and apply the reorder that has already been measured. The
   cost is a new `CodeOp`, emitted as `STLR` on aarch64 and as a plain store on
   x86_64 (TSO) and riscv64. `StrU64`'s current handling spans
   `src/arch/{ops.rs, aarch64,x86_64,riscv64}/{encode/emitter.rs, regmodel.rs,
   select.rs}`, `src/codegen/engine/{mir/mir.rs, regalloc/linear_scan.rs,
   builder/code_impl.rs}` (`grep -rln StrU64 src/arch src/codegen/engine`), and
   every one of those needs encoder tests. Only the macOS tls goldens move. This
   is the correct fix.
2. **Emit `DMB` between the payload and the gate.** A single fixed-encoding
   instruction with no operands, so it is cheaper in the encoder than (1). It is a
   heavier instruction at run time, but that doesn't matter on an
   error-classification path. It still needs a new `CodeOp` plumbed through every
   backend's selector and regalloc.
3. **Make the reader tolerate a torn view (no new instruction).** When the writer
   sees the gate with `CTX_EDOM == 0`, it waits for the handler to finish before
   classifying, for example by waiting on `CTX_SEM`, which the handler signals
   after publishing. That forces a program-order dependency back through a
   kernel-mediated wakeup (an exception entry commits pending writes). But the
   guard path has no outstanding operation to wait for. It would also need a
   bounded wait so that a state change with a null error (domain stays 0, the
   legitimate `ErrTlsFailed` case) cannot hang `write`. More moving parts than
   (1), and the bounded-wait timeout is a new tuning constant.
4. **Publish gate and payload as ONE word.** Pack the domain into the store the
   reader already treats as the gate. A single store cannot be torn by
   reordering. The costs: `CTX_EDOM`'s "sticky" rule (a later null-error state must
   not erase it) and the send completion's separate `CTX_ERROR` gate both need
   redesigning, and every `CTX_STATE` reader has to mask the new bits. That makes
   it the most invasive option for the shared listener/connection ctx layout
   (`.ai/net-tls.md`, "`STATE_INVOKE` is shared by connection AND listener
   contexts").

Recommendation: (1), filed as its own encoder bug, with this reproduction as its
RED. The reorder, its unit pin and the interposer driver are ready to replay onto
it. See "Parked work" below.

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

## Sighting 2: FIXED (2026-09-12)

Branch `bug-564-s2-store-release`. It carries option (1) from "Options and their
costs", with the reorder ported from the parked `fc60ae253`.

### The ordering argument, from the ARMv8 model

Sullivan, *Compiling a Calculus for Relaxed Memory* (arXiv 1904.05389), §5.4,
states the fences exactly: "Store-Release writes become visible after all
program-order prior stores and all stores observed by program-order prior loads
… Load-Acquire reads, on the other hand, execute before all program-order
successors." Arm ARM DDI0487 section B2.6.11, "Load-Acquire, Load-AcquirePC, and
Store-Release", is the normative source. Its page was located by search but could
not be retrieved as text here.

The writer publishes `CTX_EDOM` (a plain store), then the gate. The reader loads
the gate, then `CTX_EDOM`. The bad view needs the reader's `CTX_EDOM` load to
see memory older than its gate load saw.

* **Writer side: `stlr` on each gate.** The gate becomes visible only after
  every program-order-prior store, `CTX_EDOM` included. So whenever a gate is
  visible, the domain already is. The payload store can stay a plain `str`.
* **Reader side: `ldar` on each gate load, which is needed too.** A plain load
  may execute early, so the `CTX_EDOM` load could run before the gate load,
  against memory from before either store. A load-acquire on the gate executes
  before every program-order-later load. So `t(gate load) <= t(CTX_EDOM load)`,
  and a visible gate implies a visible domain at that time.

The fix therefore touches both sides:

* `fd8ba0620` adds the `CodeOp`s `StlrU64`, `LdarU64` and `LdarU32`, which are
  AArch64-only and base-register-only (A64 has no offset form). It also adds
  the `abi::store_release_u64` / `load_acquire_u64` / `load_acquire_u32`
  builders. These are new instructions, not lowering variants.
* `d474a4830` changes `state_invoke_function` / `send_invoke_function` to park
  their arguments, classify, then `add` + `stlr` `CTX_ERROR` (and
  `CTX_STATE`). It also changes `lower_tls_write_macos` to `add` + `ldar` its
  two gate loads (`CTX_STATE` u32 at the terminal-state guard, `CTX_ERROR` u64
  after the send wait).

The disassembled fixture built by this compiler has 3 `stlr` (two in
`STATE_INVOKE`, one in `SEND_INVOKE`) and 4 `ldar` (two gate loads in each of
`tls_write` and `tls_writeText`). The pre-fix build has 0 of either
(`otool -tv … | grep -E 'stlr|ldar'`).

### Encoding oracle

`encodes_load_acquire_store_release` pins eight words assembled by Apple clang 17
on this host (`clang -c -arch arm64` then `otool -t`):

| instruction | word |
|---|---|
| `stlr x9, [x10]` | `c89ffd49` |
| `stlr x1, [x19]` | `c89ffe61` |
| `stlr x30, [x0]` | `c89ffc1e` |
| `stlr xzr, [sp]` | `c89fffff` |
| `ldar x9, [x10]` | `c8dffd49` |
| `ldar x0, [sp]` | `c8dfffe0` |
| `ldar w9, [x10]` | `88dffd49` |
| `ldar w10, [x9]` | `88dffd2a` |

(A from-memory guess of `LDAR` as `…7c00` was wrong. `o0` is 1 for both
instructions, and the oracle is what caught it.)

### RED, then GREEN

Both REDs were run in throwaway `git worktree add --detach` trees at
`d474a4830`, with only the behaviour files swapped. Command:
`cargo test --no-fail-fast --bin mfb -- gen_macos::tests`.

* **RED A**: `tls.rs` and `client.rs` from main `a76bcb8e6`. Exit 101, 2
  failed. The messages were "`_mfb_tls_nw_state_invoke` must store CTX_EDOM
  (index 15) BEFORE the gate at offset 16 (stores [(6, StrU64)])" and
  "tls::write … must load the gate at offset 16 with LdarU32 (loads [(158,
  LdrU32)])".
* **RED B**: the parked plain-`str` reorder (`tls.rs` from `fc60ae253`). Exit
  101, 2 failed. The order check passes, and the new kind check fails:
  "`_mfb_tls_nw_state_invoke` must publish the gate at offset 16 with a
  store-release (stores [(20, StrU64)])".
* **GREEN** on the branch: `-- gen_macos::tests encode::tests ops::tests
  mir::tests` gave 256 passed, 0 failed, exit 0.

### Matched pair on the final build

This was the fixture `tls-write-peer-closed-raises-rt`, built once by main's
release compiler and once by this branch's. Command:
`B564_DYLIB=interpose3.dylib B564_LOG=1 loop.sh <bin> 600 8 <out>`. Both
ran side by side on the macOS host while the full artifact gate also ran,
with load averages 54–65. Each run was classified by the return address of
its first `domain=` line (pre-fix: STATE `…76b8`, SEND `…770c`; fix: STATE
`…76c8`, SEND `…7734`).

| build | runs × concurrency | `write raised=FALSE` | STATE-first (FALSE among them) | SEND-first | exit≠0 |
|---|---|---|---|---|---|
| pre-fix (main `a76bcb8e6`) | 600 × 8 | **19** | 98 (**19**) | 502 | 0 |
| fix (`d474a4830`) | 600 × 8 | **0** | 120 (**0**) | 480 | 0 |

Every pre-fix failure was a STATE-first run, as before. The other three
fixture lines (`cert`, `empty`, `deadline`) held in all 1,200 runs.

### Containment: 3 goldens, 4 functions each

* **Baseline.** `artifact-gate.sh <main mfb> all` from a detached `a76bcb8e6`
  worktree: 1431 tests, 2009 goldens, **0 diffs**. Every diff below is
  therefore this branch's.
* **Branch, before regen.** **3 diffs**. All are `macos-aarch64`, and all are
  fixtures that `IMPORT tls`. It is the same set sighting 1's fix moved:

      byte-identity/http/golden/http_codegen_cover_rt.macos-aarch64.ncodesum
      byte-identity/resource-xfer-slots/golden/resource_xfer_slots_cover_rt.macos-aarch64.ncodesum
      byte-identity/tls/golden/tls_codegen_cover_rt.macos-aarch64.ncodesum

  Each was localized by building `-ncode` with both compilers and diffing per
  function. In all three, exactly four functions changed, and none appeared or
  vanished (`functions=120/162/99`). `_mfb_tls_nw_state_invoke` and
  `_mfb_tls_nw_send_invoke` changed because of the `stlr` publication order.
  `_mfb_rt_tls_tls_write` and `_mfb_rt_tls_tls_writeText` changed because of
  the `ldar` gate loads. `linux-*` and `windows-x86_64` did not move: those
  backends do not emit these functions.
* **Regen.** `bash scripts/regen-ncodesum.sh target/release/mfb` refreshed 144
  goldens; `git status` changed only those 3. No `.run` golden moved, and no
  `.ir`/`.ast`/`build.log` moved.
* **Branch, after regen.** Same command: 2009 goldens, **0 diffs**, exit 0.
* **Full suite** at `9ab9a10ae`: `cargo test --no-fail-fast` to a file, cargo's
  exit code **0**. 162 test targets: 5364 passed, 0 failed, 6 ignored. That
  includes `artifact_gate_all`, the three new pins, the positive runtime pin and
  `rt_macos_tls_write_capacity`.

### OPEN: a write after `tls::read` has already reported the close never raises

This is a separate defect, and it is not the ordering race. It turned up while
building the positive pin and was measured on BOTH compilers.

* **Shape that fails.** An `openssl s_client -msg` peer sends close_notify (its
  trace shows `>>> TLS 1.3, Alert … warning close_notify`) and exits. The
  server calls `tls::read` until it raises `ErrConnectionClosed`, and THEN writes.
  With this branch's compiler, 2000 writes of 64 KiB printed
  `after=COMPLETED writes=2000`, and a later read printed `77070004`
  (`ErrConnectionClosed`). With 20000 writes (1.28 GiB) the process had already
  exited within a second. `netstat -an -p tcp` showed only the peer's side in
  `TIME_WAIT` and no server-side row, so nothing was transmitted. No
  `nw_error` ever reached a trampoline (the interposer logged no `domain=`
  call). Main's release compiler gives the same `after=COMPLETED`.
* **Shape that holds.** This is the parked probe's shape: no read before the
  write, and the write released only after s_client has exited. On BOTH
  compilers the first write raises: `after=TRUE code=77070004 writes=0`. A
  later read gives `77070004`.

So Network.framework completes `nw_connection_send` with a null error once the
receive side has delivered the peer's close, and the macOS `tls::write` never
learns the peer is gone. That breaks `mfb spec stdlib transports` §17 for this
sequence. A fix needs a decision this bug does not own: whether a received
close_notify alone should fail later writes, given that TLS 1.3 permits
half-close. It also needs a probe of what Network.framework exposes. Commit
`c39e7177a`'s message overstates this as "never raises" after close_notify in
general. The shape above is the correct statement, and `rt_macos_tls_write_after_clean_close`
pins the holding shape.

## Parked work (sighting 2) — superseded by "Sighting 2: FIXED"

Branch `bug-564-s2-program-order-wip` holds three things. It must not merge on its
own, for the reason in "Why this is BLOCKED rather than fixed".

* **The reorder** (`src/target/macos_aarch64/tls.rs`: `state_invoke_function`,
  `send_invoke_function`). Each trampoline parks its arguments at sp+16/24, calls
  `record_error_domain`, then stores `CTX_ERROR` (and `CTX_STATE`). The
  disassembled fixture shows `str w0,[x19,#0xc8]` before `str x10,[x19,#0x20]` and
  `str x10,[x19,#0x10]`.
* **Its unit pin**,
  `codegen::builtins::tls::gen_macos::tests::trampolines_publish_the_error_domain_before_the_gate`.
  It was RED on the unfixed tree (`cargo test --release -p mfb --bin mfb
  --no-fail-fast -- gen_macos::tests`, exit 101: "`_mfb_tls_nw_state_invoke` must
  store CTX_EDOM (index 15) BEFORE the gate at offset 16 (indices [6])"). It is
  GREEN with the reorder (`-- tls::`, 32 passed, exit 0). When option (1) lands,
  the test should also assert that the gate store is the release-store op.
* **The instrument**, `bugs/bug-564-s2-instrument/{interpose3.c,loop.sh}`. To
  replay the matched pair:
  `clang -dynamiclib -O1 -o interpose3.dylib interpose3.c`, then
  `B564_DYLIB=$PWD/interpose3.dylib B564_LOG=1 ./loop.sh <fixture.out> 600 8 <outdir>`
  for each build, run side by side. Classify each run by the return address of its
  first `domain=` line.

Positive pins measured on the reorder (a standalone probe, not committed; 3 runs
each on the pre-fix and the reordered build, identical output):

* A write to a live `openssl s_client` peer (`"hello"` plus 4 KiB) prints
  `live ok=TRUE`.
* A write loop after the peer closes **in an orderly way** (s_client without
  `-quiet`, stdin `/dev/null`, so it sends close_notify and exits by itself, then
  gets reaped) prints `orderly closed=TRUE code=77070004`, which is
  `ErrConnectionClosed`.
* The fixture's own `cert tlsFailed=TRUE`, `empty emptyWritesSucceeded=TRUE` and
  `deadline timedOut=TRUE` lines held in all 600 reordered runs.
* A write after our OWN `tls::close` cannot be written: the compiler refuses it
  with `TYPE_USE_AFTER_MOVE` (2-203-0055), so there is no runtime case to pin.

Not run on the reorder: `scripts/test-accept.sh` over `*tls*`, the `rt_*tls*`
integration tests, and `.ncodesum` regeneration. The reorder moves the
macos-aarch64 `tls`/`http`/`resource-xfer-slots` sums, and those belong to
whichever branch lands option (1).
