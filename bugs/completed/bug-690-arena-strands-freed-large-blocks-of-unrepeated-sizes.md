# bug-690: the arena never reuses a freed large block unless the next request is the exact same size, so an animation whose scene size varies maps memory without bound

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: HIGH
Class: Other (memory: unbounded address-space growth with no leaked live data)

Status: Fixed (worktree-B-690; see STATUS block)
Regression Test: `tests/runtime/rt_arena_large_block_reuse.rs` (console varying-vs-fixed, climbing length, canvas worker + graphics arenas)

## STATUS: FIXED (ef4a8f0db)

Reproduced exactly as documented at main `4ef538bb9` (console: 507 maps /
11,202,560 B varying vs 9 / 159,744 B fixed; canvas doc program: worker
2,676,719,616 B mapped for 47.6 MB peak live, graphics 183,549,952 B for
30.3 MB). Mechanism confirmed: `arena.rs:lower_arena_alloc`
`arena_alloc_large_bin` scanned only for an exact-size node, and
`lower_arena_free` `arena_free_large_bin` hashed by exact size; no large-bin
drain existed anywhere (the Phase 2 bullet about "`lower_arena_flush_coalesce`'s
large-bin drain" was a false premise — the flush never gathered the large bins).

What landed (all in `src/codegen/memory/arena/arena.rs` +
`error_constants.rs`), deviating from the doc's design 1 in three measured ways:

1. **Design 1 as written** — 64 log2 size classes (4 per power of two,
   `emit_large_bin_slot`), best fit among 16 nodes of the own class (exact wins),
   else the head of the next non-empty higher class, split + re-park. Alone it left
   the console repro at 2,154,496 B (15× fixed): splits only shrink chunks.
2. **Design 2 was needed**, gated differently than proposed. The flush now also
   gathers the large bins and re-parks large runs by class. A big miss flushes only
   while a new arena-state word `ARENA_LARGE_FLUSH_CREDIT_OFFSET` (appended at
   +3768; state 3768 → 3776 B; no existing offset moved) is positive: large frees
   add their bytes, each flush resets it to −256 B per chunk it sorted. The
   doc's "parked > mapped/2" / bytes-left-parked gate was measured too
   conservative (console stuck at 466,944 B, 3.4×).
3. **Climb slack** (not in the doc): mapped blocks never merge, so a request
   climbing a little past what it freed maps afresh every step. A big grow with a
   chunk parked in its own class or the one below maps `size/4` extra. Without it
   the canvas test's worker arena ended at 157 MB for 26 MB live (6×).

Also: a small request that finds nothing else takes at most 64 KiB
(`ARENA_LARGE_DV_MAX`) of the smallest parked large chunk as its designated
victim, re-parking the rest.

Results (macos-aarch64, `--debug` counters):

| case | before | after |
| --- | --- | --- |
| console varying / fixed | 11,202,560 / 159,744 B | 139,264 / 139,264 B |
| climbing copy, 1.36 MB peak | 678,772,736 B | 3,506,176 B |
| doc canvas program, worker | 2,676,719,616 B (47.6 MB peak) | 107,753,472 B (57.9 MB peak) |
| doc canvas program, graphics | 183,549,952 B (30.3 MB peak) | 41,480,192 B (30.2 MB peak) |
| doc canvas RSS 10 s → 20 s (release) | 1,085 → 2,135 MB | 144 → 144 MB |

Cross-target runtime: console + climb programs give identical counters on
windows-x86_64 (box 2230) and linux-aarch64 glibc (box 2226). linux-x86_64 and
linux-riscv64 were not run (boxes 2227/2228/2229/2232 refused ssh; docker could
not start an emulated container) — covered by the byte gate only.

Timing: whole-benchmark total unchanged (3,658 vs 3,663 ms, sum of per-row min
medians over 3 base / 2 fixed `--run 5` runs); `arena` group unchanged; the
mixed-churn repro flat at ~76 ms/repeat on both. `list (Fixed) mid` measured
+20–25% and `string slice` +10%, `reduce`/`reduceRight` −19%. Isolated: `take`
with the identical 4,048-byte alloc/free pair and identical path counters was
unchanged; the `mid` loop's machine code and addresses were identical; an
unrelated later edit to `arena_alloc` moved `mid` from ~800 to 707 µs (base
673). Recorded as a layout-sensitive microbenchmark shift, not an allocator
cost — see `.ai/codegen-invariants.md`.

`examples/wind`: 10 minutes at a flat 286 MB RSS with the fix — but the base
compiler's wind was also flat (311 → 320 MB over 90 s) on this host, so the run
did not reproduce the growth and is not evidence either way.

Goldens: `scripts/regen-native-goldens.sh` rewrote 230, 166 changed (157
`.ncodesum`, 7 `.ncode`, 2 `.mir`); per-function diff of every raw `.ncode`:
only the entry (`_main` / `_mfb_macapp_program`) and the four arena helpers.
`artifact-gate.sh all`: 0 diffs at main and on the branch (2,158 goldens).

A program that frees and reallocates blocks larger than `ARENA_QUICK_BIN_MAX`
(2048 bytes) keeps mapping new memory whenever the sizes don't repeat exactly.
Nothing is leaked: every allocation is freed, and live bytes stay flat. But the
freed blocks are never handed out again, so the arena's mapped size grows with
the number of *distinct* sizes the program has ever freed.

`canvas::present` makes this dramatic. It deep-copies the scene into a block
sized exactly to that scene, and builds hash and signature lists sized by the
item count (`src/codegen/builtins/canvas/func_present.rs:__canvas_present`).
An animation whose item count changes from frame to frame (particles being born
and dying, trails of varying length) frees a differently sized multi-megabyte
block every frame. None of them is ever reused.
`examples/wind` at `732eed460` grows about 20 MB/s (1,182 → 1,391 MB RSS in
10 s) and reached 13.4 GB in one run with the forecast player added.

**Correct behavior after the fix.** Memory a program frees can be handed out
again for any request it fits, not only for a request of exactly the same
size. A loop whose live data is bounded keeps a mapped size bounded by a small
multiple of its peak live bytes, whatever sequence of sizes it allocates. The
cost of that reuse must stay amortized O(1) per allocation. plan-25-A and
plan-64 exist because the naive fix went quadratic.

References:

- `mfb man canvas` (package description): "a program presenting items with new
  coordinates every frame reaches a steady memory size — set by how much is on
  screen, not by how long it has been running — and can animate indefinitely at
  that size." This bug breaks that promise for any scene whose size changes.
- `.ai/codegen-invariants.md` § "Arena free-list goes quadratic on mixed-size
  transient churn". This is the same mixed-size problem, showing up as memory
  growth rather than time.
- `bugs/completed/bug-175-codegen-robustness-nits.md` item H: already recorded
  that "large bins [are] reclaimed only at `arena_destroy`" and corrected a
  comment claiming otherwise. It did not treat that as a bug.
- `planning/completed/plan-25-A-arena-large-block.md` (introduced the large
  bins), `planning/completed/plan-64-benchmark-perf.md` (A1: flush-before-grow
  gated to small requests).
- Found while adding the forecast player to `examples/wind` (commit
  `295520211`).

## Failing Reproduction

Two programs. Both are built with `--debug` so the arena counters print on exit
(`mfb spec tooling debug-report`).

**Console (no canvas).** Take an exact-length copy of a list 20,000 times,
dropping each one. The length cycles through 1,000 values, or is fixed with an
argument.

```basic
IMPORT collections
IMPORT io
IMPORT os

FUNC main AS Integer
  LET fixed AS Boolean = len(os::args()) > 0
  MUT big AS List OF Integer = []
  FOR k = 1 TO 4000
    big = collections::append(big, k)
  NEXT
  MUT total AS Integer = 0
  FOR i = 1 TO 20000
    MUT n AS Integer = 2000 + ((i * 37) MOD 1000)
    IF fixed THEN n = 2500
    LET xs AS List OF Integer = collections::mid(big, 0, n)
    total = total + len(xs)
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
```

```
mfb build --debug <dir> && ./build/<name>.out 2>&1 | grep -E 'arena.0.(maps|mapped_bytes|peak_live_bytes) '
./build/<name>.out fixed 2>&1 | grep ...
```

| run | peak live bytes | maps | mapped bytes |
| --- | --- | --- | --- |
| lengths vary | 69,216 | **507** | **11,202,560** |
| length fixed | 69,216 | 9 | 159,744 |

- Observed: with the same live data, varying lengths map 70× more memory, 162×
  the peak live bytes. The growth is bounded here only because there are just
  1,000 distinct sizes.
- Expected: both rows map about the same amount.

**Canvas (`--app`).** Present 10,000–10,999 moving lines a frame for 600
frames. Only the count varies; the contrast run uses a fixed 11,000.

```basic
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT os

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  FOR frame = 1 TO 600
    MUT items AS List OF canvas::DrawItem = []
    FOR i = 0 TO 9999 + ((frame * 37) MOD 1000)
      LET x AS Float = toFloat((i * 37 + frame * 3) MOD 800) + toFloat(frame) * 0.013
      LET y AS Float = toFloat((i * 13) MOD 600) + toFloat(i) * 0.0007
      LET p AS canvas::Paint = canvas::stroke(color::withAlpha(color::rgb(120 + (i MOD 100), 170, 215), 40 + ((i + frame) MOD 195)), 1.35)
      LET item AS canvas::DrawItem = canvas::Line[x1 := x, y1 := y, x2 := x + 5.0, y2 := y + 3.0, cap := canvas::CapStyle.Round, paint := p]
      items = collections::append(items, item)
    NEXT
    canvas::present(items)
    os::sleep(16)
  NEXT
  RETURN 0
END FUNC
```

Debug report at exit (macOS aarch64, `mfb build --debug`):

| arena | count | alloc = free calls | peak live bytes | maps | mapped bytes |
| --- | --- | --- | --- | --- | --- |
| main (worker) | varies | 88,251,290 / 88,251,247 | 51,758,784 | **1,571** | **2,678,161,408** |
| main (worker) | fixed | 92,421,890 / 92,421,847 | 63,962,608 | 58 | 79,892,480 |
| graphics | varies | 11,385 / 11,349 | 29,640,208 | **1,891** | **208,732,160** |
| graphics | fixed | 11,340 / 11,304 | 19,718,080 | 37 | 29,470,720 |

RSS sampled 10 s → 20 s after launch: fixed count 111 → 111 MB, varying count
939 → 1,794 MB (**85 MB/s**). `flushes` is 0 in every run.

Isolation on `examples/wind` at `732eed460` (RSS growth 10 s → 20 s), each a
one-line change to the frame loop:

| variant | growth |
| --- | --- |
| unchanged | ~20 MB/s |
| trails never move (`stepSwarm` removed; same item count, same items every frame) | 0 |
| map only (`present(mapItems(...))`) | 0 |
| trails only (`present(trailItems(view))`; count varies) | 56 MB/s |
| map + trails, no text | 48 MB/s |
| scene built but never presented | 2 MB/s |

Contrast cases, which work today and must keep working:

- A fixed item count with changing coordinates stays flat: 11,000 moving lines
  with per-item paints, fractional coordinates, and zero-length or sub-pixel
  segments were all measured at 0 MB/s.
- A list grown by `collections::append` stays flat (94 KB mapped for 20,000
  lists of 2,000–3,000 elements), because append capacities round up and the
  same few sizes recur.

**plan-154-D makes wind a faster repro (2026-09-25).** With each trail drawn as
one `canvas::PolyLine` holding its own point and colour lists, the scene's size
changes on almost every frame instead of cycling through a bounded set. A
release, headless, `MFB_CANVAS_GPU=1` wind grows linearly: 1,423 → 11,819 MB RSS
over 150 s (about 70 MB/s). The `Line` version it replaced levels off near 4.8 GB
over the same 150 s. `vmmap` on the PolyLine wind after 25 s shows the growth in
`VM_ALLOCATE`. It holds 520 regions of 5,296–5,360 KB, about one per frame and
about a scene copy's size. Malloc and IOAccelerator memory stay flat. Software
rendering (`MFB_CANVAS_GPU=0`) levels off at about 1.1 GB. Constant-size probes
(1,000 moving PolyLines a frame, with or without colours) stay flat on both
renderers. So it is this bug, not the new item kind.

| Environment | Result |
| --- | --- |
| macos-aarch64, console and `--app` | fails ✗ |
| other targets | not measured. The allocator is emitted per target from the same `lower_arena_alloc`/`lower_arena_free`, so expected to fail (guess) |

## Root Cause

`src/codegen/memory/arena/arena.rs:lower_arena_free`, label
`arena_free_large_bin`: a freed chunk larger than `ARENA_QUICK_BIN_MAX`
(`src/codegen/error/constants/error_constants.rs`, 2048) is pushed onto one of
`ARENA_LARGE_BIN_COUNT` (64) singly linked bins, indexed by
`(size >> 4) & 63`. It never goes onto the address-ordered coalescing list.

`src/codegen/memory/arena/arena.rs:lower_arena_alloc`, label
`arena_alloc_large_bin`: a large request scans *its own* bin for a node of
**exactly** its size. A node of any other size is skipped. The in-code comment
says it "stays parked in its bin and is only reclaimed at `arena_destroy` —
there is no flush-before-grow drain for large bins". On a miss the request
falls to the first-fit walk of the address-ordered list, which holds no large
frees (they all went to bins), and then to `arena_alloc_grow`. There, the
flush-before-grow drain that could merge parked chunks is gated off for big
requests ("A BIG request (> QUICK_BIN_MAX, or align > 16) grows directly") and
the arena maps a new block.

So a freed large block is reused only if a later request has exactly the same
size. A block whose size never recurs stays parked until the arena is
destroyed. The arena's mapped size grows with the sum of the sizes freed, not
with live data.

Why the contrast cases are immune: a fixed scene size, and append-grown lists
with rounded-up capacities, request the same sizes over and over, so each
request pops the chunk freed before it. `canvas::present` is the worst case
because every block it allocates is sized exactly to the scene. Those are the
deep copy (`canvas::publishScene`), the carried and computed hash lists
(`canvas::carriedHashes`, `__canvas_hashScene`), and the group signature
(`__canvas_groupSignature`). Each is freed a frame later through the
retirement list (`.ai/canvas-threading.md` §3), so every frame parks several
blocks of new sizes.

The graphics arena grows the same way (209 MB vs 29 MB above) through its own
per-frame allocations of item-count-sized blocks. It uses the same allocator.

Why it was built this way: routing large frees through the address-ordered
list made both the insert and every later walk quadratic under the
benchmark's large-list churn (plan-25-A). Flushing before a big grow drained
huge parked inventories through an O(list) insert: "hundreds of millions of
insert steps" (plan-64 A1). Both decisions traded memory reuse for time, and
the trade is only safe when sizes repeat.

## Goal

- The console reproduction maps about the same memory with varying and with
  fixed lengths: `mapped_bytes(varying) ≤ 2 × mapped_bytes(fixed)` (threshold to
  be confirmed in Phase 1 against the fixed baseline).
- The canvas reproduction's worker and graphics arenas each end with
  `mapped_bytes ≤ 4 × peak_live_bytes`, and RSS growth 10 s → 20 s is 0 MB/s ±
  noise.
- `examples/wind` animates for 10 minutes without RSS growth after the forecast
  has loaded.
- No benchmark row that plan-25-A / plan-64 fixed regresses in time.

### Non-goals (must NOT change)

- Allocation semantics, the arena block layout, the `FreeNode {next, size}`
  overlay, and the arena-state word offsets. Code in every build reads those
  offsets.
- The canvas scene-ring retirement rules (`.ai/canvas-threading.md` §3). The
  worker still frees only its own blocks, and only after a frame completes.
- The debug-report key set (`mfb spec tooling debug-report`).
- **Forbidden wrong fixes:**
  - Pooling or padding scene blocks to fixed sizes inside `canvas::present`.
    It hides the canvas symptom and leaves every other varying-size workload
    (the console reproduction) broken.
  - Changing `examples/wind` to present a constant number of items.
  - Re-routing all large frees through `arena_insert_free` unconditionally, or
    flushing on every big grow. plan-25-A and plan-64 A1 measured both as
    quadratic.

## Blast Radius

Found by reading `lower_arena_alloc`/`lower_arena_free` (every arena
allocation goes through them). The call sites below are the ones measured or
read.

- `canvas::present` / `presentLayers` (`func_present.rs`,
  `func_present_layers.rs`): the scene deep copy, hashes, and signature. Fixed
  by this bug; this is the measured case.
- The graphics-thread arena's per-frame allocations: fixed by this bug, since
  it uses the same allocator (measured: 209 MB vs 29 MB).
- Any exact-size copy of a varying-length list, String, or record
  (`collections::mid`/`take`/`drop`/…, string building, JSON/CSV parse
  output): fixed by this bug (console repro).
- Thread message copies (`thread::send` of a large varying-size value, copied
  into the receiver's arena): fixed by this bug. Same allocator. Not separately
  measured.
- Small requests (≤ 2048): unaffected. Exact quick bins plus the gated
  flush-before-grow already reuse them.
- `.ai/codegen-invariants.md` "quadratic on mixed-size transient churn": a
  related time hazard in the small path. Latent and out of scope, but the fix
  must not make it worse (Phase 3 re-measures it).

## Fix Design

The large-bin index must be able to satisfy a request from a *larger* parked
chunk, and parked chunks must eventually merge. Candidates, to be decided in
Phase 1 with measurements:

1. **Size-class large bins with split (recommended).** Index large bins by
   log2 size class (e.g. 4–8 sub-classes per power of two) instead of an exact
   hash. A request scans its class and then the higher classes for the first
   chunk that is large enough, and splits off the remainder (re-binned; a
   remainder ≤ 2048 goes to a quick bin). This is O(classes) per request, which
   is constant, and needs no address-ordered walk. It fixes reuse for a
   varying size without any flush.
2. **Bounded flush-before-grow for big requests.** Before mapping for a large
   miss, drain the large bins into the coalescing list, but only when parked
   large bytes exceed a threshold proportional to live or mapped bytes (e.g.
   parked > mapped/2). That amortizes the O(list) sort over the memory it
   recovers. This recovers adjacency-merged runs that (1) alone cannot, so it
   is a complement to (1), not an alternative.
3. **Return fully free mapped blocks to the OS** (`munmap`/`VirtualFree`) when
   coalescing produces a whole block. This bounds RSS, not reuse. Deferred.

Rejected:
- An unconditional flush on big grows: measured quadratic (plan-64 A1).
- Fixing only canvas: see Non-goals.

Expected output shift: none in `.ir`/`.ast`. The allocator is emitted native
code, so `tests/byte-identity/**/*.ncodesum` for every target shifts, and so
may any golden that embeds allocator instructions. Regenerate through
`scripts/artifact-gate.sh` only (`.ai/testing-gates.md`).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/runtime/rt_arena_large_block_reuse.rs`: build the console
      reproduction with `tests/common/debug_report.rs:build_debug`, run the
      varying and fixed variants, and assert
      `mapped_bytes(varying) ≤ 2 × mapped_bytes(fixed)`. Confirm it fails today
      (11.2 MB vs 0.16 MB).
- [x] Add an `--app` canvas case (`common::build_app_debug`, headless) that
      presents a varying-count scene for N frames and asserts
      `mapped_bytes ≤ 4 × peak_live_bytes` for both arenas. Confirm it fails.
- [x] Record baseline timings for the plan-25-A / plan-64 benchmark rows and
      the mixed-churn repro in `.ai/codegen-invariants.md`, for Phase 3 to
      compare against.

Acceptance: both new tests fail for the documented reason; baselines are
written into this file (STATUS block).
Commit: cc428d877 (also a climbing-length case; all three RED at 4ef538bb9)

### Phase 2 — the fix

- [x] Implement design 1 in `lower_arena_alloc` / `lower_arena_free`
      (`src/codegen/memory/arena/arena.rs`) and the bin constants
      (`error_constants.rs`). Keep the arena-state size and offsets if the
      class count fits in the existing 64 slots; otherwise append, never
      reorder (see the `ARENA_LARGE_BIN_BASE_OFFSET` comment).
- [x] Decide, from the Phase 1 measurements, whether design 2 is needed as
      well.
- [x] Update `lower_arena_flush_coalesce`'s large-bin drain to match the new
      bin index.
- [x] Correct the comments that describe exact-size-only reuse.

Acceptance: the Phase 1 tests pass; the contrast cases still behave as
documented; nothing in Non-goals changed (one word appended to the arena
state, no offset moved).
Commit: ef4a8f0db (design 1 + a credit-gated design 2 + climb slack), 29bcf52ff (fmt)

### Phase 3 — regenerate expected outputs + full validation

- [x] Regenerate `.ncodesum` goldens through `scripts/artifact-gate.sh`; the
      delta must be only the allocator's code.
- [x] Re-run the plan-25-A / plan-64 benchmark rows and the mixed-churn repro
      against the Phase 1 baselines; no time regression.
- [x] Full suite — per coordinator instruction only the touched area was run on this branch (artifact-gate all + arena/runtime/canvas test binaries); the coordinator runs the full suite after merge.
- [x] `examples/wind` for 10 minutes (flat, but not discriminating here — see STATUS) with RSS sampled: flat after the forecast
      loads.
- [x] Update `mfb spec memory arenas` to describe the new large-bin policy.
      Update `.ai/codegen-invariants.md`: record the fix, and note whether the
      mixed-churn entry changed.

Acceptance: full suite green; the golden delta is exactly the allocator; the
reproductions pass; no benchmark regression.
Commit: 04383c0ee (goldens)

## Validation Plan

- Regression tests: `tests/runtime/rt_arena_large_block_reuse.rs` (console and
  canvas cases).
- Runtime proof: `examples/wind` and the canvas reproduction, with flat RSS.
- Doc sync: `mfb spec memory arenas` (large-bin policy),
  `.ai/codegen-invariants.md`. No `mfb man` change: the canvas page already
  promises the behavior the fix delivers.
- Full suite: `cargo test`, `scripts/artifact-gate.sh target/release/mfb all`.

## Open Decisions

- Large-bin indexing: log2 size classes with split (recommended) vs keeping
  the exact hash plus a bounded drain. Decide on the Phase 1 measurements.
- Whether to return whole free blocks to the OS (design 3): defer unless RSS,
  as opposed to mapped reuse, still grows after design 1.

## Summary

The engineering risk is time, not correctness. The large bins exist to keep
large-list churn O(1), and two earlier attempts at reuse went quadratic. The
fix has to restore reuse across sizes while keeping allocation amortized
constant, and Phase 3's benchmark comparison is the gate that proves it. Canvas
semantics, the scene ring, the arena layout and every public surface stay
untouched.
