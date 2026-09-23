# bug-683: `canvas::present` leaks a whole scene copy per call, on both the skip and the publish path

Last updated: 2026-09-23
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Open
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
- `gen_group.rs` — `canvas::setGroup` has a structurally identical
  retire/reclaim (`CANVAS_GROUP_RETIRED_FRAME`, `:304`, `:485-521`), with the
  same single retired slot and the same gate. **Latent, same hazard**; a
  program calling `setGroup` per frame should be measured. In scope for the
  audit, and for the fix if it shares the mechanism.
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

- [ ] Add a canvas test presenting a fixed scene N times (`static`) and a
      changing one (`moving`), asserting allocation/RSS flat after warm-up.
      Confirm both fail today.
- [ ] Add the `presentLayers` row.
- [ ] Measure `setGroup`-per-frame and record the verdict in Blast Radius.

Acceptance: both rows fail, for the two documented reasons.
Commit: —

### Phase 2 — the skip-path free (leak 1)

- [ ] Free the fresh copy before returning at `gen_present.rs:232`.

Acceptance: the `static` row passes; `moving` still fails.
Commit: —

### Phase 3 — bounded retirement (leak 2)

- [ ] Replace the single retired slot with a frame-stamped retirement list;
      reclaim every entry the frame counter has passed.
- [ ] Apply to `presentLayers`, and to `gen_group.rs` if the audit says it
      shares the mechanism.

Acceptance: the `moving` row passes; no use-after-free under
`MFB_CANVAS_SYNC` and under a free-running renderer.
Commit: —

### Phase 4 — validation

- [ ] `cargo test --test 'rt_canvas_*'` green, every golden reference image
      unchanged — this fix must not move a pixel.
- [ ] `tests/canvas/rt_canvas_present_deep_copy.rs` still green (it pins the
      copy semantics this must not change).
- [ ] Re-run the reproduction: all three modes flat.
- [ ] `examples/wind` holds steady RSS for five minutes.
Commit: —

## Validation Plan

- Regression test: the present-flatness test from Phase 1.
- Runtime proof: the three-mode table above, all flat.
- Doc sync: `.ai/canvas-threading.md` §3's ordering gains the retirement-list
  step; `mfb man canvas`'s "no-op" sentence becomes true rather than aspirational.
- Full suite: `cargo test`, canvas goldens in particular.

## Open Decisions

- Retirement list vs. blocking present — recommended: **retirement list**, for
  the reasons in Fix Design. (§Fix Design)

## Summary

Leak 1 is small and safe. The risk is entirely in Phase 3: retirement exists to
prevent a use-after-free on a block the renderer is still reading, so a fix that
frees too eagerly trades a leak for a crash. The drain gate must survive
unchanged; only the number of blocks it can hold changes.
