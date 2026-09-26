# bug-721: Windows `--app` — the arena free list corrupts while a `thread::start` worker runs beside a canvas frame loop

Last updated: 2026-09-25
Effort: x-large (1d–3d) — Windows-only, reproduces only on a real PC so far, needs a minidump
Severity: HIGH
Class: Memory-safety (Windows runtime, arena allocator under threads)

Status: Open
Regression Test: none yet — see Phase 1

`examples/wind`, built with `mfb build --target windows-x86_64` and run on a real
Windows 11 PC, shows a white window and then exits with `0xC0000005` (access
violation), every time, within a few seconds. The fault is inside the arena
allocator's free-bin walk: it follows a free-list `next` pointer that is garbage,
so the free list was corrupted before the walk. It happens only while the
program's `ISOLATED FUNC fetchForecast` worker (started with `thread::start`) is
running beside the canvas frame loop. With the worker made to return at once, the
same program runs cleanly. The same source runs cleanly on macOS, forecast
included.

**The correct behaviour a fix produces:** `wind.exe` runs on Windows exactly as it
does on macOS. The map appears, the worker fetches and decodes all 40 forecast
hours, and the forecast plays with no fault. More generally, a `thread::start`
worker that allocates heavily never corrupts another thread's arena on
`windows-x86_64`.

The crash is silent from the user's side: there is no MFB error and no message.
The window closes. Windows logs no Application-log event for it (the
`Application Error` provider has no entry), and only the WER report archive
records it.

References:

- `.ai/canvas-threading.md` §2 — arena state is per-thread, and the bug-498
  corollary: "never allocate from another thread's arena". This bug has the same
  symptom as bug-498: a quick-bin pop faulting under two threads.
- `.ai/remote_systems.md` "Box 2230 (Windows)" — the minidump procedure this
  needs (`LocalDumps`, streams 3/4/6, `CONTEXT` offsets).
- bug-479 (completed) — the last Windows canvas fault that needed a minidump to
  localise.
- Found while shipping `examples/wind` (precipitation mode, `6384505e8`) to a
  Windows PC.

## Failing Reproduction

Needs a real Windows x86_64 machine. The fault was seen on a physical PC at
192.168.1.206, Windows 11 build 10.0.26200.9457, as the non-admin account `test`.

```
mfb build --target windows-x86_64 examples/wind
# copy build/wind.exe and build/wind/coastline.bin (keep the wind\ folder beside the exe)
# on the Windows machine:
wind.exe
```

- Observed: a white window for a moment, then the process exits with
  `-1073741819` (`0xC0000005`). WER report `Sig[6]=c0000005`,
  `Sig[7]=00000000007…` (the fault offset in `wind.exe`).
- Expected: the map appears and animates, and the forecast downloads and plays.

The binary built from `6384505e8` (sha1 of `wind.exe`
`286828850e9ccc05b2e0ef13f5aeac69c845dc2c`) faulted at the same offset,
`wind.exe+0x796d1c`, on both of its runs. Disassembly of that site
(`llvm-objdump -d --start-address=0x140796c80`):

```
140796cdb: movq  0x60(%rsp), %r11        ; requested size
140796ce6: shrq  $0x4, %r10              ; size / 16
140796cea: movabsq $0x3f, %r11
140796cf4: andq  %r11, %r10              ; & 63  -> bin index
140796cf7: shlq  $0x3, %r10
140796cfb: addq  %r15, %r10              ; arena state + index*8
140796d01: addq  $0x490, %r11            ; + bins offset
140796d08: movq  (%r11), %r10            ; head of the bin
140796d0f: cmpq  $0x0, %r10
140796d16: je    ...
140796d1c: movq  0x8(%r10), %r12         ; <- FAULT: node->size through a bad node
140796d23: movq  0x60(%rsp), %r14
140796d2b: cmpq  %r14, %r12
140796d2e: je    ...                     ; exact-size match
140796d34: movq  %r10, %r11
140796d37: movq  (%r10), %r10            ; node = node->next
140796d3e: jmp   140796d0f
```

This is a walk of one arena bin's singly linked free list (`next` at +0,
`size` at +8), based off `%r15`, the per-thread arena state. The node it reads
through is not a valid block.

### Where in the program

A probe copy of `examples/wind` appended a marker line to
`%TEMP%\wind-probe.log` (`fs::appendText`) at each step. It showed:

- `main` completes all of its setup: font, the first `present`,
  `loadAnalysis`, `thread::start`, `loadCoastline`, `seedSwarm`,
  `canvas::createImage`, `enableMouse`. It then presents frames normally.
- The worker completes `pruneCache`, then downloads, decodes and
  `thread::send`s forecast hours one after another (hours 3–15 in one run).
- The fault comes on a later frame: 2, 15, 20 and 21 frames in across four
  runs. It lands between the `f scene` marker (before
  `frameScene`/`timelineItems`/`canvas::present`) and the next `frame presented`.
  In both runs where the worker was traced, the worker's last marker was
  `worker fetched …`, so it was inside `windsFrom` → `decodeFile` (the GRIB
  decode) at the moment of the fault.

The frame the fault lands on varies from run to run, while the wall time to the
fault is similar: 12 s with software rendering gave 2 frames and 12 s with GPU
rendering gave 15. That fits a race, not a deterministic bad path.

### Matrix

| Program | Environment | Result |
| --- | --- | --- |
| `examples/wind` | Windows 11 PC, GPU renderer (default for a window) | fails ✗ (`0xC0000005`) |
| `examples/wind` | same PC, `MFB_CANVAS_GPU=0` (software renderer) | fails ✗ — not the GPU path |
| `examples/wind` with `fetchForecast` returning at once (worker starts and ends immediately) | same PC | works ✓ (30 s, no fault) |
| `examples/wind` | macOS aarch64, Metal | works ✓ (full forecast fetched and played) |
| canvas-only loop, one `Rectangle`, 300 frames, no worker | same PC | works ✓ (exit 0) |
| console: worker `append`s 30 × 260,000-Float lists and `send`s them, main appends strings | same PC | works ✓ |
| canvas: the same worker (30 and 400 rounds) beside a 1,000-`Line` frame loop | same PC | works ✓ |
| canvas: worker doing `http::read` over HTTPS 40× and `send`ing the body, beside the frame loop | same PC | works ✓ |

The reduced programs did not reproduce the fault. So the trigger is narrower
than "a worker allocates beside canvas". Whatever `wind` adds is what needs
finding: the GRIB decode's large `collections::set`/`collections::mid` workload,
`fs::writeBytesAtomic`, `Text` items, `canvas::setBytes`, or simply more time
overlapped.

The WER archive for this machine is
`C:\ProgramData\Microsoft\Windows\WER\ReportArchive\AppCrash_wind.exe_*\Report.wer`.
It is readable without admin, and `Sig[7]` is the fault offset. The probe builds
faulted at `0x7af5d4` (four times), `0x7b0698` and `0x7cc286`. Those offsets
differ because the probe code shifted the layout, so they cannot be compared
with the unmodified build.

## Root Cause

Not yet known. The faulting instruction proves the free list of one arena bin
was corrupt when a thread walked it. It does not say which thread faulted,
whose arena that was, or who wrote the bad pointer. The hypotheses, most likely
first:

1. **A cross-thread allocation into another thread's arena (the bug-498
   class), on a path only Windows takes.** `.ai/canvas-threading.md` §2: an
   arena's quick-bin pop is a plain load/store, so two threads popping the same
   arena corrupt it exactly like this. Suspects are any Windows-only code that
   reaches `_mfb_arena_alloc` with an arena state that is not the calling
   thread's own:
   - The Windows app shell. `src/target/win_x86_64/app/mod.rs:arena_alloc` calls
     `ARENA_ALLOC_SYMBOL` directly from the Win64 app layer. Its current callers
     are the term/io marshalling paths (`mod.rs` around lines 1916, 2262, 4403,
     5113). Each one must be checked for the thread it runs on and the `%r15` it
     sees.
   - The graphics thread on Windows. It must never allocate from the language
     thread's arena (§2), for instance through `MAIN_ARENA_GLOBAL_SYMBOL`, which
     names the last entry to run.
   - `thread::send` / `receive` boundary copies on Win64
     (`src/codegen/runtime/thread/runtime_helpers.rs`). The copy must be made in
     the sender's arena and adopted by the receiver.
2. **`%r15` is not the calling thread's own arena at some Windows callback or
   trampoline.** On Win64 `%r15` is callee-saved, so the OS preserves it. But a
   callback the OS invokes on a thread (a window procedure, a thread start
   routine) that runs MFB code must set `%r15` itself. If one inherits a stale
   or shared value, that thread allocates into someone else's arena. Check
   `src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_start_helper` and
   the Windows thread trampoline for the worker's child arena block, and the
   Windows app's window-procedure entry.
3. **A use-after-free or overrun of an arena block**, where some code writes
   into a freed or undersized block and smashes a free-list node. This would
   not need two threads, but it matches the evidence less well: the fault never
   happens without the worker.

How to tell them apart: capture a minidump of the unmodified `wind.exe`
fault. This needs admin on the machine:

```powershell
$k = 'HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps'
New-Item -Force $k | Out-Null
Set-ItemProperty $k DumpFolder C:\Users\test\dumps
Set-ItemProperty $k DumpCount 5
Set-ItemProperty $k DumpType 1
```

From the dump (stream 3 threads, stream 6 exception; `CONTEXT` `Rip` at
`0xF8`, `Rsp` at `0x98`, and `R15` read from the same `CONTEXT`):

- **The faulting thread.** Is it the UI thread, the language thread, the
  graphics thread or the `thread::start` worker?
- **Its `R15` against every thread's `R15`.** Two threads with the same `R15`,
  or a faulting `R15` that is not that thread's own arena state, confirms
  hypothesis 1 or 2 and names the thread.
- **The bad node value** (`R10` at the fault). Is it a small integer, a pointer
  into another arena's chunk, or freed memory?

## Goal

- `examples/wind` built for `windows-x86_64` runs on a real Windows PC for at
  least the full forecast download (all 40 hours) and several minutes of
  playback, with no fault.
- A reduced regression program that faults today and passes after the fix runs
  in `scripts/test-winapp.sh` (or its sibling), so the next Windows threading
  regression is caught by a gate rather than by an example.

### Non-goals (must NOT change)

- The per-thread arena design (`.ai/canvas-threading.md` §2) and the bug-498
  rule: never allocate into another thread's arena. The fix must restore that
  invariant on the offending Windows path, not weaken it.
- **No lock around `_mfb_arena_alloc`.** Serialising every allocation would
  hide the race and cost every program. It is the tempting wrong fix, and it is
  forbidden.
- **No change to `examples/wind` to avoid the crash**, for example dropping the
  worker, fetching on the main thread, or pre-decoding. The example is the
  reproduction. Changing it masks a runtime bug that any threaded Windows canvas
  program can hit.
- macOS and Linux behaviour, the arena layout (bins at `+0x490`, node layout),
  and the `thread::` API surface.

## Blast Radius

Not yet audited. The audit is Phase 1 and depends on the dump naming the path.
The search set:

- `src/target/win_x86_64/app/mod.rs:arena_alloc` and each of its callers — which
  thread each runs on, and whose `%r15` it uses.
- Every Windows-target site that loads `MAIN_ARENA_GLOBAL_SYMBOL` or otherwise
  repoints `%r15` (`git grep -n MAIN_ARENA_GLOBAL_SYMBOL -- src/target/win_x86_64`).
- `src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_start_helper` and
  the Win64 thread trampoline: how the child arena block is carved and when.
- The Windows canvas graphics thread: every call it makes that could reach the
  arena.
- The Linux and macOS counterparts of whichever site is guilty, classified as
  unaffected, with the reason (for example a different trampoline).

## Fix Design

To be written once the root cause is confirmed. The constraint is known already:
the fix makes the offending thread allocate only from its own arena, or hands a
block across by adoption as §2 describes. It does not add locking to the
allocator.

## Phases

### Phase 1 — localise, reduce, audit (no behaviour change)

- [ ] Capture a minidump of the unmodified `wind.exe` fault (admin needed for
      `LocalDumps`). Record the faulting thread, its `R15`, every thread's
      `R15`, and the bad node value here.
- [ ] Reduce to a small program that faults the same way, guided by the dump,
      and add it to the Windows app gate (`scripts/test-winapp.sh` or a
      sibling). Confirm it fails today.
- [ ] Complete the blast-radius audit above with a verdict for each site.

Acceptance: the root cause is cited to `file:symbol` with the mechanism; a
reduced reproduction fails in a gate for the documented reason; every audit
site has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] Make the guilty path allocate only from its own thread's arena (or adopt
      a sender-made block), and apply the same fix to any in-scope sibling from
      the audit.

Acceptance: the Phase 1 reproduction passes; the contrast cases in the matrix
still pass; nothing in Non-goals changed.
Commit: —

### Phase 3 — full validation

- [ ] Run the project's full suite and the Windows app gate.
- [ ] Rebuild `examples/wind` for `windows-x86_64` and run it on the real PC
      through the full forecast download and several minutes of playback.
- [ ] If the fix shifts generated output (`.ncode` goldens for Windows
      fixtures), confirm the delta is only the fix.

Acceptance: full suite green; `wind.exe` runs clean on the PC; any golden
delta is exactly the intended change.
Commit: —

## Validation Plan

- Regression test: the reduced program from Phase 1, in the Windows app gate.
- Runtime proof: `examples/wind` on the real Windows PC, all 40 forecast hours
  decoded and played with no WER `AppCrash_wind.exe` report.
- Doc sync: `.ai/canvas-threading.md` §2 if the guilty path is a new instance
  of the cross-thread-allocation rule; `mfb spec` only if a documented thread
  guarantee changes.
- Full suite: the project's acceptance suite plus `scripts/test-winapp.sh`.

## Open Decisions

- Where the reduced reproduction runs — the emulated box 2230 or a real PC.
  Recommended: try 2230 first, since it is always reachable for a gate. Every
  reduced program so far passed on the real PC, though, so the gate may need
  the real machine until the trigger is known.

## Summary

The risk is in localising, not in the fix. The fault site (an arena bin walk
through a corrupt node) matches the known cross-thread allocation hazard, but
the thread and path that break the per-thread arena rule on Windows are still
unknown, and simple worker-beside-canvas programs do not reproduce it. A
minidump is the next step. The per-thread arena design, the allocator's
lock-free fast path and `examples/wind` itself stay untouched.
