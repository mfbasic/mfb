# bug-683: `canvas::present` leaks a whole scene copy per call, on both the skip and the publish path

Last updated: 2026-09-23
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: FIXED
Regression Test: `tests/canvas/` — an RSS/allocation-flatness test over repeated presents; see Phases

Every `canvas::present` deep-copies the caller's scene into a fresh block
(`.ai/canvas-threading.md` §3, step 1). On the two paths measured below that
block, or the one it displaces, is never freed — so a program that presents
every frame grows without bound at a rate proportional to its scene's geometry,
regardless of whether the scene changed.

Measured with a scene of 95 polygons of 400 points each, presented at ~60 Hz:
RSS climbed **361 MB → 1441 MB in 15 seconds (~90 MB/s)**, linearly, and at the
same rate whether the scene was identical every frame or moving. The identical
case is the sharper one, because `mfb man canvas` promises the opposite:
"re-presenting an unchanged scene is a no-op."

**The single correct behavior a fix produces:** presenting in a loop — the same
scene or a different one — holds steady-state memory. A program can animate, or
sit re-presenting an unchanged scene, indefinitely at constant RSS.

This is distinct from bug-682 (now fixed), which was the geometry *cache*
arena. This is the scene ring, it is on the cache-**hit** path as much as the
miss path, and it survives that fix.

References:

- `.ai/canvas-threading.md` §3 "The scene ring" — the normative ordering
  (copy, compare, reclaim, retire, publish) this violates, and §3.1 the frame
  skip.
- `src/codegen/builtins/canvas/gen_present.rs:232` the `skip` label;
  `:253` `emit_reclaim_retired` and the retire loop at `:254-275`;
  `:353` `emit_reclaim_retired` itself and its gate at `:368`.
- `mfb man canvas` — "present is therefore not a per-frame call — a static
  picture is presented once and costs nothing thereafter, and re-presenting an
  unchanged scene is a no-op."
- `bugs/completed/bug-682-*` — the geometry arena, fixed; this is the other
  allocation on the present path and was hidden behind it.
- Found while writing `examples/wind`, which is blocked on it: the example
  presents every frame and grows ~20 MB/s from its map alone.

## Failing Reproduction

A canvas program that builds a fresh scene each frame and presents it. Nothing
in the program holds state across frames — the scene is a local, dropped at the
end of the iteration — so all growth is the runtime's:

```basic
FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  MUT frame AS Integer = 0
  DO WHILE frame < 100000
    frame = frame + 1
    LET scene AS List OF canvas::DrawItem = sceneFor(frame, moving, items, points)
    IF present THEN canvas::present(scene)
    os::sleep(16)
  LOOP
  RETURN 0
END FUNC
```

95 polygons × 400 points, RSS sampled every 3 s:

| mode | 3 s | 6 s | 9 s | 12 s | 15 s | rate |
| --- | --- | --- | --- | --- | --- | --- |
| `nopresent` (build the scene, never present) | 70 MB | 75 MB | 75 MB | 70 MB | 70 MB | **flat ✓** |
| `static` (identical scene every frame) | 361 MB | 634 MB | 901 MB | 1168 MB | 1441 MB | ~90 MB/s ✗ |
| `moving` (new coordinates every frame) | 327 MB | 607 MB | 881 MB | 1177 MB | — | ~94 MB/s ✗ |

- Observed: unbounded linear growth on both presenting modes.
- Expected: flat, as `nopresent` is.

`nopresent` being flat is the load-bearing contrast: building and dropping the
identical scene values every frame leaks nothing, so this is not the language's
value semantics — it is `present`.

Growth scales with scene size, confirming it is the scene copy rather than a
fixed per-call cost:

| scene | rate |
| --- | --- |
| 95 items × 400 points | ~90 MB/s |
| 10 items × 400 points | ~17 MB/s |
| 95 items, plain rectangles | ~1.6 MB/s |

| Environment | | Result |
| --- | --- | --- |
| macos-aarch64 | release, software raster | fails ✗ |
| Linux / Windows | not measured | `gen_present.rs` is shared codegen, so expect ✗ everywhere |

## Root Cause

Two independent leaks on the two exits of `canvas::present`
(`src/codegen/builtins/canvas/gen_present.rs`), which is why both modes leak at
the same rate:

**1. The frame skip abandons the fresh copy.** The deep copy into `fresh` is
made *before* the content comparison (§3 step 1 precedes step 2). When
`emit_compare_bytes_branch` finds the content identical it branches to `skip`
(`:232`), which sets the result to 0 and returns:

```rust
builder.emit(abi::label(&skip));
builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
…
builder.emit(abi::return_());
```

Nothing frees `fresh`. So every *unchanged* re-present — exactly the case the
man page calls a no-op — leaks one full scene copy. That is the `static` row.

**2. Retire overwrites an unreclaimed retirement.** On the publish path,
`emit_reclaim_retired` (`:353`) frees the previous retirement only when the
frame counter has advanced past it (`:368`, `branch_ls` to done). The retire
loop that follows (`:254-275`) then overwrites the retired slots
unconditionally:

```rust
emit_reclaim_retired(builder, &scene, &symbol)?;
for (retired, live) in [ … ] {
    builder.emit(abi::load_u64(&displaced, &scene, live));
    builder.emit(abi::store_u64(&displaced, &scene, retired));   // no guard
}
```

There is exactly one retired slot. When the gate does not fire — two presents
inside one rendered frame, which is the normal case for a program presenting at
60 Hz against a renderer that completes fewer frames — the block already in the
retired slot is overwritten and its pointer lost forever. That is the `moving`
row.

`.ai/canvas-threading.md` §3 already anticipates the cost of getting this wrong:
"Without this every publish abandoned its predecessor: a 200-frame animation
grew by ~0.11 MB a frame with nothing ever reclaiming it." The reclaim exists;
it is just not sufficient, because retirement can outpace it and the skip path
bypasses it entirely.

## Goal

- Presenting in a loop holds steady-state RSS, for an unchanged scene and a
  changing one alike.
- A new test presents N times and asserts allocation/RSS flat after warm-up;
  it fails today on both `static` and `moving`.

### Non-goals (must NOT change)

- **The use-after-free that retirement exists to prevent.** A displaced block
  may be the one the renderer is copying right now; it must not be freed at the
  publish. Any fix keeps the "a frame has completed since" gate.
- **Cross-thread frees.** Only the worker frees, and only blocks it allocated
  (§3 "Who frees"). The graphics thread must not be given a free.
- **The frame-skip behavior itself.** An identical re-present must still
  publish nothing and still report FALSE; only the leak goes away.
- **The content comparison** (§3.1 compares content, not bytes) stays as is.
- Do not "fix" this by removing the deep copy, or by making `present` alias the
  caller's scene — `present` copies so the caller may discard what it built,
  which `mfb man canvas` promises.

## Blast Radius

- `gen_present.rs` skip path (`:232`) — **fixed by this bug** (leak 1).
- `gen_present.rs` retire loop (`:254-275`) — **fixed by this bug** (leak 2).
- `canvas::presentLayers` — shares this code path via `SceneShape::Layered`
  (the same retire loop covers `CANVAS_SCENE_RETIRED_LAYERS_OFFSET`), so
  **same hazard, fixed by the same change**. Needs its own test row.
- `gen_group.rs` — `canvas::setGroup` has a structurally similar retire/reclaim
  (`CANVAS_GROUP_RETIRED_FRAME`, `:304`, `:485-521`). **Audited and NOT the same
  mechanism — unchanged by this fix.** It does not overwrite an occupied retired
  slot: `emit_retire_current_items` frees the prior retirement before storing the
  new one, so it holds at most one buffer per slot. Measured on a program calling
  `setGroup` every frame at 60 Hz: `arena.0.live_bytes` 52,992 at 120 frames and
  52,992 at 240 — exactly flat.

  The audit did turn up a **separate** leak, which became `bugs/bug-684-*` and is now
  also fixed: a program that calls `setGroup` per frame *and* presents a scene naming
  that group lost one block per present. It is not in `gen_group.rs` either — it is
  `canvas::publishHashes` dropping the hash block it displaced, which survived only
  because a following `publishScene` used to retire that slot on its behalf. See bug-684
  for the diagnosis; it also corrects two latent faults in the code this bug added.
- Every `Mode.Canvas` program that presents per frame — `examples/wind`
  (blocked), and any animation.
- Static programs that present once — **unaffected**, one leaked copy at most.

## Fix Design

Leak 1 is a one-line class of fix: free `fresh` before returning on the skip
path. It is unambiguously safe — the block was allocated in this call and was
never published, so no renderer can hold it.

Leak 2 is the real design question: one retired slot cannot absorb an
unbounded number of retirements between frames. Options:

- **A retirement list rather than one slot** — retire onto a small list stamped
  with the frame counter, and reclaim every entry the counter has passed. The
  list is bounded in practice by presents-per-frame, and the reclaim loop is
  the one that already exists, run to fixed point.
- **Reclaim-before-retire, and if the gate blocks, free nothing but do not
  retire either** — i.e. keep the older retirement and free the newly displaced
  block instead. Wrong: the newly displaced block is the one that was just
  published, so it is the one the renderer may be reading.
- **Block until a frame completes.** Correct but turns `present` into a
  synchronising call, which §3 deliberately avoids.

Recommended: the retirement list. It preserves the drain gate exactly, adds no
cross-thread free, and the reclaim logic is unchanged apart from looping.

## Phases

### Phase 1 — failing test (no behavior change)

- [x] Add a canvas test presenting a fixed scene N times (`static`) and a
      changing one (`moving`), asserting allocation/RSS flat after warm-up.
      Confirm both fail today.
- [x] Add the `presentLayers` row.
- [x] Measure `setGroup`-per-frame and record the verdict in Blast Radius.

The test is `tests/canvas/rt_canvas_present_leak.rs`, and it reads
`arena.0.live_bytes` from the `--debug` report rather than sampling RSS: the leak is
an exact byte count, so the signal is exact and the assertion needs no smoothing.

Two deviations from the plan as written, both forced by measurement:

- **The programs animate at 60 Hz (`os::sleep(16)`), not in a tight loop.** A present
  cannot free what it displaces until a frame has *completed*, so an unpaced loop
  presents thousands of times per rendered frame and legitimately holds thousands of
  scenes — 905 KB over 200 presents on the *fixed* compiler. That is the design, not a
  leak, and a test that called it one would have been unfixable.
- **The `moving` row presents twice per frame.** At one paced present per frame the
  renderer keeps up with a 40-item scene, the gate fires every time, and leak 2 does
  **not** reproduce — that row went green on the unfixed compiler. The bug report hit
  it at one present per frame only because its scene (95 polygons × 400 points) took
  longer than a frame to draw. Two back-to-back presents are the documented trigger
  ("two presents inside one rendered frame") and need no assumption about how fast
  anything is.

RED on the unfixed compiler, each for its own documented reason: `unchanged`
16,048 B/present, `changing` 21,060 B/present, `layers` 41,657 B/present.

Acceptance: met — all three rows fail, for the documented reasons.
Commit: 58f72a602

### Phase 2 — the skip-path free (leak 1)

- [x] Free the fresh copy before returning at `gen_present.rs:232`.

Acceptance: met — the `unchanged` row passes and the other two still fail.
Commit: 58f72a602

### Phase 3 — bounded retirement (leak 2)

- [x] Replace the single retired slot with a frame-stamped retirement list;
      reclaim every entry the frame counter has passed.
- [x] Apply to `presentLayers` — it shares `emit_publish`, so it shares the fix.
- [x] `gen_group.rs`: audited, does **not** share the mechanism, left alone (see
      Blast Radius).

The four scene words `RETIRED_ITEMS/HASHES/LAYERS/FRAME` become one
`CANVAS_SCENE_RETIRED_HEAD_OFFSET`, pointing at a list of arena-allocated nodes
`{next, items, hashes, layers, frame}`. Nodes are **newest-first**, which is what
keeps the drain a single gate rather than a search: the head carries the largest
stamp, so `frame_now > head.frame` proves a frame has completed since every node
behind it and the whole chain goes at once. That is never more eager than a per-node
gate — no node is freed before its own stamp allows — and it holds an older node at
most one extra frame.

The gate itself is byte-for-byte the one that was there (`branch_ls` away when
`frame_now <= stamped`), which is the non-goal this had to respect.

One addition the plan did not anticipate: the node allocation can fail. That path
frees the fresh copy and raises `ErrOutOfMemory`, and it is emitted **out of line**,
after the publish's `ret` — an inline second `ret` inside the publish body made "what
does the publish return" ambiguous and broke
`the_skip_reports_false_and_the_publish_reports_true`.

Acceptance: met — all three rows pass, under `MFB_CANVAS_SYNC` and free-running;
`rt_canvas_graphics_thread` and `rt_canvas_group_ownership` green.
Commit: 58f72a602, 6e8ca07f0 (the out-of-line OOM path)

### Phase 4 — validation

- [x] `cargo test --test 'rt_canvas_*'` green, every golden reference image
      unchanged — this fix must not move a pixel.
- [x] `tests/canvas/rt_canvas_present_deep_copy.rs` still green (it pins the
      copy semantics this must not change).
- [x] Re-run the reproduction: all three modes flat.
- [x] `examples/wind` holds steady RSS for five minutes: 87,072 KB at 3 s, then
      87,008 KB at every 20-second sample from 1 min to 5 min 08 s. It grew ~20 MB/s
      before.
Commit: 5236e5a82, 7ac650722

Re-run of the reproduction on the fixed compiler, `arena.0.live_bytes`, paced at
60 Hz:

| mode | N=120 | N=240 | N=480 | verdict |
| --- | --- | --- | --- | --- |
| `unchanged` (`MFB_CANVAS_SYNC`) | 37,072 | 37,072 | 37,072 | flat ✓ |
| `changing` (`MFB_CANVAS_SYNC`) | 53,536 | 53,536 | 53,536 | flat ✓ |
| `changing` (free-running) | 53,536 | 70,000 | — | flat ✓ (137 B/frame: one copy held at exit) |

Three tests had to be corrected, each proven wrong by the layout change rather than
re-baselined:

- `retired_scene_blocks_are_freed_only_after_a_frame_completes` — counted 3 frees
  behind the gate and 0 before. Now 4 behind it (three blocks plus the node), 1 on the
  skip path and 1 on the cold OOM path, each asserted **by region** rather than as one
  total. The invariant it protects — nothing on the *publish* path is freed before the
  gate — is now stated directly instead of being implied by a zero.
- `every_publish_retires_the_displaced_blocks_and_stamps_the_frame` — looked for four
  scene offsets that no longer exist; now checks the list head, its ordering against
  the revision, and all five node fields.
- `rt_canvas_present_deep_copy.rs`'s `scene_stores` — filtered retirement bookkeeping
  out **by offset** (48..72). Node fields live at 0/8/16/24/32, numerically identical
  to the scene's revision/count/items, so the filter read four node stores as a second
  out-of-order publish. It now skips the retire span by region.

## Validation Plan

- Regression test: the present-flatness test from Phase 1.
- Runtime proof: the three-mode table above, all flat.
- Doc sync: `.ai/canvas-threading.md` §3's ordering gains the retirement-list
  step; `mfb man canvas`'s "no-op" sentence becomes true rather than aspirational.
- Full suite: `cargo test`, canvas goldens in particular.

## Open Decisions

- Retirement list vs. blocking present — recommended: **retirement list**, for
  the reasons in Fix Design. (§Fix Design) **Settled: the retirement list.**
  Blocking was never reachable: `present` is not rate-limited to one call per
  rendered frame, and the renderer re-reads the installed pointer at arbitrary
  points inside a frame (`__canvas_sceneDraws` and `__canvas_sceneOffsets` each call
  `canvas::installedItems`), so every block displaced since the last frame tick is
  one a render in flight may be holding. The count that has to be held is whatever
  the schedule produced, and only a list can hold it.

## Summary

Leak 1 is small and safe. The risk is entirely in Phase 3: retirement exists to
prevent a use-after-free on a block the renderer is still reading, so a fix that
frees too eagerly trades a leak for a crash. The drain gate must survive
unchanged; only the number of blocks it can hold changes.

## STATUS: FIXED (7ac650722)

Both documented leaks are gone and `canvas::present` is byte-exactly flat on both
its exits. `arena.0.live_bytes` at 120 / 240 / 480 presents, paced at 60 Hz:

| row | 120 | 240 | 480 |
| --- | --- | --- | --- |
| unchanged scene (`MFB_CANVAS_SYNC`) | 37,072 | 37,072 | 37,072 |
| changing scene (`MFB_CANVAS_SYNC`) | 53,536 | 53,536 | 53,536 |

Before the fix the same rows were linear at 16,048 and 21,060 bytes per present.
`examples/wind` — the example this blocked — holds 87 MB for five minutes, against
~20 MB/s of growth.

Landed in 58f72a602, 6e8ca07f0, 5236e5a82, 7ac650722. Full suite green
(`cargo test`), including the artifact gate at 2116 goldens and 0 diffs.

### Deviations from the plan as written

- **The regression tests pace at 60 Hz and the `moving` row presents twice per
  frame.** Both were forced by measurement rather than chosen; see Phase 1. The
  short version is that an unpaced present loop legitimately holds thousands of
  retirements — that is the design, not a leak — and that one paced present per
  frame does not reproduce leak 2 with a scene light enough to test quickly.
- **The test measures `arena.0.live_bytes` from the `--debug` report, not RSS.**
  The plan said "allocation/RSS flat after warm-up". The arena counter is an exact
  byte count of what was allocated and never returned, so the assertion needs no
  warm-up and no smoothing, and the failure message can name the per-present cost.
- **`gen_group.rs` is untouched.** The audit Phase 1 asked for says it does not
  share the mechanism — see Blast Radius. It did surface a *different* leak, written up
  and fixed as `bugs/bug-684-*`: `canvas::publishHashes` drops the hash block it
  displaces. bug-684 also corrects two latent faults in the code *this* bug added — a
  retirement node that only initialised the fields its caller named, and a hashes slot
  retired by `publishScene` as well as by the call that actually overwrites it. Both
  were harmless while `emit_publish` was the only caller and became crashes the moment
  it was not.
- **Three tests were corrected** (listed under Phase 4), each disproved by the layout
  change rather than re-baselined. Two were pinned to the four retirement words that
  no longer exist; the third discriminated scene stores from retirement stores *by
  offset*, which the node layout made ambiguous.
- **One addition the plan did not anticipate:** the retirement node allocation can
  fail. That path frees the fresh copy and raises `ErrOutOfMemory`, emitted out of
  line after the publish exit.

### What to watch

The retirement list is bounded by **presents per rendered frame**, not by a constant.
A program that presents in a tight loop against a slow renderer will hold every scene
it published since the last frame tick — by construction, because each of them may be
the one being read. That is the same bound the single slot claimed to have and did not
honour; it is now real, but it is not a fixed ceiling, and a program that wants a fixed
ceiling has to pace its presents.
