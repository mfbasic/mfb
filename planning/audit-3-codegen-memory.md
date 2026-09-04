# audit-3 — Surface 3: codegen & runtime memory safety

Part of `planning/goal-08-platform-security-review.md`. Finding prefix `MEM-`
(arena/collections/strings: MEM-11..14; engine/backends: MEM-40..46;
threads/canvas/vector: MEM-70..77). Untrusted party: whoever controls the runtime
inputs to a compiled program (sizes, strings, collection contents, cross-thread
transfers, published scenes) — and, for the backend findings, the program shape.

**Verdict: 4 HIGH memory-corruption bugs + MEDIUM/LOW.** These are the most
serious findings in audit-3 after the Surface-4 CRITICAL: two are reachable from
**ordinary 15–20-line MFBASIC programs** (MEM-11 OOB read, MEM-12 UAF), one is a
reproducible **cross-thread heap-corruption SIGSEGV** (MEM-70), and one is a
Windows-only **arbitrary pointer read/write** (MEM-40). All four were verified by
the lead (three reproduced live, one via emitted-code inspection). The audit-1
dominant class — unchecked size arithmetic into an allocation — is **closed**
(MEM-01/02/05/07 re-verified fixed); the new HIGHs are aliasing / liveness /
concurrency / ABI-layout defects.

## HIGH

### MEM-11 — bounds-check elision on a stale length → OOB heap read → **bug-495**

plan-86-G1 keeps the `end == len(L)−k` fact live across an *enclosing* loop's back
edge that reassigned `L` shorter, so `collections::get`'s check is elided while the
list is short (`builder_control.rs:1615`, `gen_list.rs:77`). Lead-reproduced at
`-O0` and `-O3`: `out=24` from an 8→1 list, 14 heap words leaked; `List OF String`
segfaults. Spike `spikes/audit-3/MEM-11/`.

### MEM-12 — operand-aliasing UAF: `g = <op>(g, f())` where f reassigns global g → **bug-496**

Operand 0 lowers to a pointer into the global's block; evaluating operand 1's call
frees it; the op reads the freed block (`builder/mod.rs:2841` `&`;
`list_mutate.rs:31` `append`). Lead-reproduced: `GS & same()` → `len=4` not `12`.
The general-aliasing half of open bug-487, reached with a plain global. Spike
`spikes/audit-3/MEM-12/`.

### MEM-70 — `thread::send`/`emit` allocates on the peer thread's arena unlocked → **bug-498**

The send lowering repoints `x19` at the destination thread's arena and deep-copies
there before taking the queue mutex, racing that thread's own allocation; the arena
has no synchronization (`builder_thread_cleanup.rs:154,163`, `arena.rs:161`).
Lead-reproduced: SIGSEGV 3/3 on an ordinary parent→worker send loop. Spike
`spikes/audit-3/MEM-70/`.

### MEM-40 — Win64 entry RNG seed scratch aliases `arena[ARENA_STDIN_LOCAL_BUF_OFFSET]` → arbitrary pointer R/W → **bug-512**

`ENTRY_SEED_SCRATCH_OFFSET = ARENA_STATE_SIZE` is not shifted by the Win64 shadow
while the arena base is, so the 8 seed bytes land in the stdin local-buffer slot;
the stdin path reads them as a buffer pointer and writes through it
(`entry.rs:396`, `error_constants.rs:169`). Lead-verified in the emitted
windows-x86_64 ncode (seed write and stdin read both at `arena+3736`); linux
control writes one word past the arena. Spike `spikes/audit-3/MEM-40/`.

## MEDIUM

- **MEM-41** — the shared thread trampoline stores its saved arena/`%thread`/
  `%closure_env` inside the Win64 callee home space (no shadow reserved)
  (`runtime_helpers.rs:996-1005`).
- **MEM-71** — canvas scene ring leaks a whole scene block on every `present` that
  lands between two frames (144 MB vs 36 MB measured) (`gen_present.rs:362`).
- **MEM-72** — `canvas::setBytes` leaks the entire pixel shadow per call
  (~256 KiB/call, 130 MB in 400 calls) (`func_set_bytes.rs:105`).
- **MEM-13** — copy / tight-copy / concat allocation sizes bypass the checked-size
  helpers (successor to audit-2 MEM-10; not demonstrated)
  (`builder_collection_layout.rs:485`).

## LOW / NTH

- **MEM-14** — arena free-node metadata is in-band, unscrubbed and unauthenticated;
  free size re-derived from the block header; no guard page (`arena.rs:1112`).
- **MEM-42/43** — Win64 program entry pops 16 bytes it never pushed (shadow shrinks
  to 16); `emit_arena_start_time` reserves 16 not 32 (`code.rs:760`).
- **MEM-44** — AArch64 encoder's hidden `x15/x16/x17` scratch still reachable via
  `abi::SCRATCH[6..8]` (bug-124.1 half-fixed) (`aarch64/encode/operand.rs:101`).
- **MEM-45** — every frame/regalloc safety guard is `debug_assert`-only and CI runs
  release; the x86 residual-ABI-token path silently miscompiles in release
  (`vreg_frame.rs:127`, `x86_64/select.rs:204`). Cross-ref the memory note
  "CI is RELEASE" — this is why the guards do not fire.
- **MEM-46** — x86-64 `patch_labels` truncates an out-of-range rel32 silently
  (aarch64/riscv64 hard-error) (`x86_64/encode/emitter.rs:194`).
- **MEM-73** — scene read through three unsynchronized loads, no barriers
  (`func_present.rs:88`). **MEM-74** — both GPU emitters index the geometry buffer
  with an unbounded `offsets[i]` the software oracle bounds-checks
  (`metal.rs:1462`, `vulkan.rs:5226`). **MEM-75** — graphics spawn ignores
  `pthread_create`'s result (`runtime/canvas/mod.rs:1467`). **MEM-77** —
  `canvas::loadFont` bytes never reclaimed (`func_destroy_font.rs:63`).
  **MEM-76** (NTH) — fragment shader indexes its storage buffer without a clamp
  (driver-contained).

## Re-verified fixed / clean

MEM-01/02 (string size-overflow), MEM-05, MEM-07 still fixed (runtime-confirmed at
`-O0` and `-O3`); **MEM-09 now fixed** (bug-266 idempotency guards on both bin-park
paths); MEM-03 fixed. The audit-1 unchecked-size-arithmetic class is closed. The
IR verifier holds as the type/shape/resource boundary (Surface 2 FE-50 note).

**Canvas GPU path mostly clean** (records the negative so a later audit does not
re-derive): every `vkCreate*` result is checked, Metal nil-checks its texture and
zeroes the slot before reallocating, `createImage` uses a checked size multiply,
the frame-buffer regions are capped twice, and the resize handshake has no race
(the renderer reads the size once and sizes the buffer from that copy). The
closed-flag texture rule has nothing to verify yet (no texture for a
`canvas::Image`). `vector`/`bits`/`math`/`money` clean.

**Engine/backend hazard sweep clean** (scanner over emitted `-ncode`, not reading
alone): 0 caller-saved-held-across-call, 0 x86 frame-parity deviations, 0 unprobed
Win64 frames, 0 register-valued operand fields outside the allocator allowlist,
full determinism across 25 fixtures × 3 targets × 3 rebuilds including the linked
`.exe`. One doc correction surfaced: `.ai/arch-abi.md:421-436` ("a Windows thread
start routine is entered ALREADY 16-byte aligned") is bug-478's superseded belief
and contradicts bug-479's 2230 measurement — worth fixing.

## Bug docs filed

bug-495 (MEM-11), bug-496 (MEM-12), bug-498 (MEM-70), bug-512 (MEM-40). Spikes:
`spikes/audit-3/{MEM-11,MEM-12,MEM-70,MEM-40}/`.

## Coverage

Read: `codegen/memory/{arena,collection layout,value}`, `codegen/collection/**`
(get/append/mutate/sort), `codegen/string/{repr,util}`, `codegen/cleanup/**`,
`codegen/engine/{control,builder,function/entry}`, `codegen/runtime/thread/**`,
`codegen/runtime/canvas/{metal,vulkan,present,scene}`, `arch/{aarch64,x86_64}/**`
(via sweeps), `target/win_x86_64/**` (entry/trampoline/shadow),
`builtins/{vector,bits,math,money}`.

Gaps: `codegen/string/format/*` (~4400 LOC float/int parse-format kernels) unread;
`regalloc/linear_scan.rs` (1019 lines — the open riscv64 pool-shrink fault lives
there) not read; instruction selection (`select_aarch64`/`select_riscv64`),
`mir.rs`, opt2 beyond `peephole.rs`, and the ~20k lines of `macos_aarch64/app/` +
`linux_gtk/` assembly emitters covered by sweeps only; `helper_glyph_cache.rs`
eviction (would confirm/kill MEM-74) not reached; `helper_png`/`helper_inflate`
audited on Surface 5. All runtime repros macOS-aarch64; MEM-40 codegen-only.
