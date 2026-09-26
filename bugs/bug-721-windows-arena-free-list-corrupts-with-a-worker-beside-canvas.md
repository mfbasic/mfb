# bug-721: Windows `--app` — the arena free list corrupts while a `thread::start` worker runs beside a canvas frame loop

Last updated: 2026-09-26
Effort: x-large (1d–3d) — Windows-only, reproduces only on a real PC so far, needs a minidump
Severity: HIGH
Class: Memory-safety (register aliasing: a hand-picked virtual register named a live minted one)

Status: Fixed
Regression Test: `gen_schannel_tests.rs::connect_keeps_its_host_scratch_pointer_intact_to_the_release`, `scripts/test-wintls.sh`, `tests/guards/architecture_guards.rs` (register guards)

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

**Not a thread race and not a Windows allocator defect: a double free in Schannel
`tls::connect`, caused by a hand-numbered virtual register aliasing a minted one.**

Measured on the real PC (192.168.1.206) with a no-admin Win32 debug-API harness
(`CreateProcess(DEBUG_ONLY_THIS_PROCESS)` + `GetThreadContext` on every thread at the
first access violation), then with probe builds of the allocator:

1. **The faulting thread is the `thread::start` worker, walking its OWN arena.** At the
   fault (`_mfb_arena_alloc+0x502`, the large-bin walk off `%r15+0x490`), `r15` was the
   worker's heap-carved arena state; the UI, language and graphics threads each had a
   distinct `r15`. The corrupt node was always the first chunk of a 0x11000 mapping
   (a ~68 KiB large block), and its first words were live SMALL-object data: once the
   ASCII `"nomads.ncep.noaa"` (the forecast host), three times the three `Float`s of a
   freshly built `grib.mfb:Scaling` record (`2005.76, 1.0, 10.0`).
2. **No second thread ever entered the worker's allocator.** A probe recording each
   arena's first caller's stack and trapping on any other thread's
   `_mfb_arena_alloc`/`_mfb_arena_free` never fired. Every allocator operation is in
   `memory/arena/arena.rs`; none is inlined elsewhere.
3. **The block was freed twice with different sizes.** A probe trapping when a
   quick-bin (small) free hands in a pointer already parked in a large bin fired at
   `_mfb_arena_free`, called from `_mfb_rt_tls_tls_connect+0x230f`, freeing
   `ptr` whose node read `{next, size = 0x10000}` with a request of `0x15` bytes —
   `"nomads.ncep.noaa.gov"` + NUL.

The mechanism, from the generated code of `tls_connect`:
`lower_tls_connect` (`builtins/tls/gen_schannel_impl.rs`) mints its vregs from a
`Vregs` counting from `%v0`; its host C-string `HelperScratch` pointer was the tenth,
`%v9`. `marshal::emit_free_buffer_guarded` staged the pointer it frees in a
**hard-coded `"%v9"`** — the same name, so the same register to the allocator. The
success path freed the 64 KiB wide server-name buffer through it, overwriting the
scratch pointer; the release at `done` then freed that 64 KiB block AGAIN at the host
string's size. The block was linked in large bin 0 and in the 32-byte quick bin at
once; the next 32-byte allocation (a `Scaling`) was carved on top of the large node,
and the next large request hashing to bin 0 (`0x103800` in `unpackComplex`) walked
through `Scaling`'s bytes and faulted.

That is why it looked thread- and canvas-related: only the worker made HTTPS requests
(one per forecast hour) and only its GRIB decode issued the 1 KiB-multiple large
requests that walk bin 0; the reduced programs never combined a `tls::connect` with
that allocation pattern. macOS (Network.framework) and Linux (OpenSSL) use other TLS
backends.

A parallel investigation (bug-564, on box 2230) found the same double free from the
other end — a second `tls::connect` in one process crashing on the 8-byte host
"8.8.8.8" — and landed `97afb6768` (`emit_free_buffer_guarded` names no vreg) and
`753391711` (every marshal helper mints from the caller's `Vregs`), which remove this
instance.

**The class is the real root cause.** A register was TEXT: any emitter could write
`"%v9"`, `"x9"` or `format!("%v{n}")`, and two independent counters (`Vregs::new()`,
each `CodeBuilder`) both started at `%v0`. The source guard banning hand-numbered
vregs scanned only `src/codegen/builtins`, so the builtin reached the banned style
through one call into `src/codegen/memory/marshal`; the physical-register guard only
looked at lines containing `abi::`/`.field(`. The migration found more live instances:
Linux OpenSSL `tls::connect`'s set-nonblocking hook wrote a hard-coded `%v9` that WAS
its SNI scratch pointer (a failed or timed-out connect freed address 2048); four mouse
emitters, `lower_generate` and `emit_mouse_ready_verify` each started a second counter
at `%v0` inside a caller's stream; `emit_link_expr` hand-numbered from `%v64`; the
macOS app shell staged C-call arguments in raw `"x0"`–`"x4"`.

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

Every emitter that names a register. Before the fix: 2,412 hand-numbered `"%vN"`
literals in emission code across 16 files (measured with the guard's own rules over all
of `src/`), 88 functions that started a second `Vregs` counter inside a caller's
instruction stream, ~130 hand-picked physical registers reaching operands through
helpers the physical guard did not recognise, and `format!`-built vreg names. All
targets: the aliasing is target-neutral; only which instance corrupts memory, and where,
differs (Windows Schannel connect: double free; Linux OpenSSL connect: free of address
2048 on failure).

## Fix Design

Registers are fixed **by construction**, not by a lint (owner decision 2026-09-26).

1. **One id source.** `regalloc::mint_vreg` is the only way to obtain a virtual register;
   `CodeBuilder`, `Vregs` and the optimizer's GVN all draw from it, reset once per module
   (`lower_module_for_platform`). Two minters can no longer hand out the same name.
   `VirtualRegister::new` / `Operand::vreg` are engine-private; `Operand::parse` is
   test-only.
2. **A typed register.** `abi::Reg` is the only thing a register-valued `abi::` operand
   accepts. It has a private representation and no conversion from text: a register is
   a minted `VirtualRegister`, an ABI role (`c_arg`, `mfb_return`, …), a role token
   `abi.rs` defines (`ZERO`, `ARENA`, `SCRATCH[i]`, `LOCAL[i]`, …), or a physical register
   the allocator or a backend chose (`Reg::physical`). A hand-picked register is a
   compile error on every target.
3. **Backstops.** `CodeInstruction::field` panics on text in a register-role field;
   `physical_field` / `Reg::physical` are confined to `src/arch`, the allocator and the
   frame finalizer (`physical_registers_stay_in_the_backends`); `no_hand_picked_vreg`
   scans all of `src/`; `no_constructed_vreg_name` bans `format!("%v…")`.
4. **Runtime gate.** `scripts/test-wintls.sh` runs every `rt-behavior/tls` fixture on the
   Windows box.

## Phases

### Phase 1 — localise, reduce, audit (no behaviour change)

- [x] Capture the fault state. `test` is not an admin, so instead of `LocalDumps` a
      Win32 debug-API harness (PowerShell + C#: `CreateProcess(DEBUG_ONLY_THIS_PROCESS)`,
      `GetThreadContext` on every thread at the first access violation) recorded every
      thread's registers: the faulting thread is the `thread::start` worker, `r15` its
      own heap-carved arena state, the bad node's first words the forecast host text
      and (three runs) a live `grib.mfb:Scaling` record.
- [x] Localise with allocator probe builds: no second thread ever entered the
      worker's arena; a cross-bin probe trapped the small free of a block already in a
      large bin at `_mfb_rt_tls_tls_connect+0x230f` (size 0x15 = the host C string).
- [x] Reduce: the reduced reproduction is `tests/rt-behavior/tls/tls-poll-rt` /
      `tls-poll-list-rt` (a second `tls::connect` in one process), run on Windows by
      the new `scripts/test-wintls.sh`. RED with the pre-fix compiler (`4df37a4fe`) on
      box 2230: both exit `-1073741819`.
- [x] Blast-radius audit: 2,412 hand-numbered vreg literals across 16 files, 88
      second counters, ~130 hand-picked physical registers, `format!`-built names
      (see Blast Radius); a second live instance (Linux OpenSSL connect frees
      address 2048 on failure) and several latent ones.

Acceptance: met. Root cause cited (`marshal::emit_free_buffer_guarded`'s literal
`%v9` = `lower_tls_connect`'s minted host-scratch pointer); the reduced reproduction
fails in a gate for that reason.
Commit: fcb634d19, a3775d1ca, 8818c963e

### Phase 2 — the fix

- [x] The instance: landed on `main` by the parallel bug-564 investigation
      (`97afb6768`, `753391711`) and merged here (`9dab53968`).
- [x] The class, by construction (owner decision 2026-09-26): one vreg id source
      per compilation; `abi::Reg` as the only register-valued operand, with no
      conversion from text, on every target; the backends-only doors
      (`Reg::physical`, `Operand::phys`, `physical_field`); `CodeInstruction::field`
      refuses text in a register field. Every emitter migrated — the hand-numbered
      literals minted, the macOS app shell's `"x0"`–`"x4"` staged as `abi::c_arg(n)`,
      the Win64/GTK callee-saved lists taken from `src/arch/x86_64/regmodel.rs`, the
      link thunks' `format!("%v{n}")` minted, ~240 registers that had been turned back
      into text (`Operand::from(r.render())`) passed typed.
- [x] Guards: `no_hand_picked_vreg` (all of `src/`), `no_constructed_vreg_name`,
      `physical_registers_stay_in_the_backends`; unit pins in `abi.rs`.

Acceptance: met. The reduced reproduction passes (see Phase 3); no text-to-register
path remains in production code (rustc + the guards).
Commit: c42ab29e9, fcfe05167, f00fae760, 1cb8525d2, 2741e61dc, 5adb3ce58 (and the
per-area migrations merged at a6a3b3d10, c102bb307, 191d16853)

### Phase 3 — full validation

- [x] `cargo test`: unit suite `4395 passed; 0 failed`; every integration target
      green (see the landing commit for the final `--no-fail-fast` run).
- [x] `scripts/test-wintls.sh target/release/mfb` on box 2230: 6 match, 2 skipped
      (their peer is the `openssl` CLI, absent on the box); RED before (above).
- [x] `examples/wind` for `windows-x86_64` on the real PC: 720 s under the debug
      harness with no access violation; the worker cached all 40 forecast hours plus
      the analysis (`%TEMP%\mfb-wind-gfs`: 41 files, the last `f120`); 0 new
      `AppCrash_wind.exe` reports. The pre-fix build faulted within 5–20 s every run.
- [x] Goldens: main tip 0 diffs over 2160; this branch moved 153, each proven
      allocation/frame-layout only (136 identical skeletons, 13 uniform frame
      shifts, 2 fused `add_pageoff`, 2 `.mir` identical modulo layout); regenerated,
      then 0 diffs.

Acceptance: met.
Commit: 7647bd053, ec5329aba

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
