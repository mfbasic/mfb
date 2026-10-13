# bug-1024: a Metal text frame is dumped with unwritten tiles (seen on a loaded machine)

Last updated: 2026-10-12
Effort: unknown — not yet root-caused
Severity: MEDIUM
Class: Concurrency (suspected)

Status: Open
Regression Test: `cargo test --test rt_canvas_font gpu` (exists; red when this happens)

## Not reproducable (coordinator, 2026-10-13)
The command-buffer status check landed; the failure never reproduced, so the link is unproven.

## Cause (coordinator, 2026-10-12, bug-staging-23)

- UNPROVEN link (the failure has not been reproduced since 2026-10-10); the cause below is a defect found by reading, and it produces this picture.
- Rule: the honesty gate (`mfb spec app canvas`; bug-688 non-goals): a frame the GPU did not draw is declined and drawn in software, never reported as a GPU frame.
- Place: `metal.rs`, the "submit and wait" block: `commit`, `waitUntilCompleted`, then `getBytes:` or `[drawable present]`. Nothing reads the command buffer's `status` (`grep -n 'sel_status' src/target/macos_aarch64/app` finds nothing). A buffer that ends in `MTLCommandBufferStatusError` (a GPU restart another process caused, a timeout) leaves the tile-based target partly written, in whole 32 x 32 tiles, and the frame is read back and counted as drawn.
- Fix: after the wait, a buffer whose status is not Completed makes the submit answer 0 (as `direct_failed` already does), so the caller falls back to the software frame. The bug stays OPEN unless the failure is reproduced and shown gone.

Two GPU text tests fail with a Metal frame in which whole 32 × 32 blocks are
`[0, 0, 0, 0]` — never written, alpha included — where the software oracle has opaque
black. How many blocks are missing changes from run to run, up to the whole frame.

**The single correct behavior a fix produces:** with `MFB_CANVAS_SYNC=1`, a frame dumped
after `canvas::present` returns has every pixel written, on Metal as on the software
renderer, whatever else the machine is doing.

References:

- `tests/canvas/rt_canvas_font.rs:text_on_the_gpu_matches_the_software_oracle`,
  `a_transformed_text_run_reaches_the_gpu_and_matches_the_oracle`.
- `.ai/canvas-threading.md` — the three-thread model and the frame handshake.
- Found while closing plan-172-F.

## Re-verified 2026-10-12: not reproduced, 5 of 5 passes (HEAD `555130a25`)

```
cargo test --no-fail-fast --test rt_canvas_font gpu     # five times
```

`2 passed; 0 failed` each time. The first run finished at a load average of 23.61
(`uptime`, with a `cargo build` beside it; each test ran for over 60 s and the pair took
233 s), the last at 10.00. So it passed again under load heavier than the load it first
failed under. No source change since 2026-10-10 is known to address it; plan-172-G to N
and plan-164 landed in between. It stays open as intermittent with the cause unknown.

**bug-1059 is the same two tests** failing 6 of 6 times on 2026-10-11 with a different
picture: opaque black where the oracle has text (`[0, 0, 0, 255]`), where this bug has
unwritten tiles (`[0, 0, 0, 0]`). Neither document cited the other. Whoever takes one
should take both: whether they are one defect is not known, and 1059's guesses (display
asleep, another session using the GPU) were not tested here. The display was awake for
these five runs (`ioreg -n IODisplayWrangler`: `CurrentPowerState=4`).

## Failing Reproduction

```
cargo test --no-fail-fast --test rt_canvas_font gpu
```

- Observed (2026-10-10, 14:45–14:56, `uptime` load averages `10.49 14.98 21.00`), five
  runs: `text_on_the_gpu…` differed in 37888, 38912, 36864, 37888 and then 576000 of
  576000 pixels (`first: (480, 0, [0, 0, 0, 0], [0, 0, 0, 255])` and the like);
  `a_transformed_text_run…` in 169984 each time. Every count is a multiple of 1024.
- Expected: both pass, as they did earlier the same day on the same tree
  (`rt_canvas_font: 33 passed` at plan-172-E's close and again in plan-172-F's first
  verification round).

- Later the same day (17:25, `uptime` load averages `7.38 10.38 10.08`, after merging
  `main`): the same command passed twice in a row, `2 passed` each. So it is
  intermittent, and it stopped without a source change addressing it.

- 21:10–21:40 the same day, load averages `34.52 50.31 43.28`: `rt_canvas_font` → 33
  passed in plan-172-H's round. Heavier load than when it failed, and it passed — so
  load alone is not the trigger.

## What is and is not known

- **It is not plan-172-F's builder change.** The scene is one `canvas::Text`; the
  native geometry builder plan-172-F edited does not build text. Between the round in
  which the two tests passed and the one in which they failed, the only compiler
  source that changed was `func_geo_build.rs` (offset loads hoisted, shared edge point
  carried). Putting the earlier `func_geo_build.rs` back and rebuilding: still fails
  (37888 pixels). So the source that passed fails now.
- **Shape scenes on Metal are unaffected at the same moment.**
  `cargo test --test rt_canvas_golden the_gpu_draws` → 6 passed, and `rt_canvas_metal`
  → 48 passed in the same round the two text tests failed in.
- **The count varies between runs of one binary**, which is why this is filed as a
  suspected race rather than a wrong pixel: something the dump depends on has not
  finished when the frame is read back.
- **Not known:** whether machine load is the trigger or a coincidence (the machine was
  loaded by another job at the time, and was also loaded earlier when the tests
  passed); whether it reproduces at `main`. Neither was tested: a quiet machine was
  not available.

## Fix Plan

- [ ] Re-run the reproduction on a quiet machine, on this tree and at `main`. If both
      pass, re-run under an artificial load to see whether load is the trigger.
- [ ] Catch it in the act with `mfb dbg` (owner's suggestion): build the test's scene
      with `--debug -O0`, launch it under `mfb dbg <binary>` with the test's
      environment (`MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1
      MFB_CANVAS_DUMP=…`), set a `bp` on the frame dump's function with `bt all` as its
      command, and read where the graphics thread is when the dump runs — a graphics
      thread still inside the frame's encode or readback is the race. `show arena` on
      the graphics thread's arena gives the scene it was drawing.
- [ ] Root-cause in the Metal text path (a glyph's coverage is a `setFragmentBytes:`
      payload; the frame is read back for the dump) against the frame handshake in
      `.ai/canvas-threading.md`.
- [ ] Fix, with a test that holds under load.

## bs23-agent-a, 2026-10-12: the status check landed; the link to this failure is still unproven

This bug stays OPEN.

### Reproduction attempt: not reproduced, 5 of 5 passes

```
cargo test --no-fail-fast --test rt_canvas_font gpu     # five times, one after another
```

`2 passed; 0 failed` each time. `uptime` load averages after each run: 11.13, 10.47,
9.78, 9.16, 8.66 (one-minute); the first run took 78 s and the others 14.5 s. The display
was awake (`ioreg -n IODisplayWrangler`: `CurrentPowerState=4`). Because nothing failed,
the planned instrumentation (read the buffer's `status` in a failing run) could not be
done: whether the frames with unwritten tiles came from a buffer in
`MTLCommandBufferStatusError` is neither proved nor refuted.

### What landed

- `metal.rs:emit_metal_draw`: after `commit` and `waitUntilCompleted` the renderer sends
  `status` to the command buffer and leaves for the pool release unless the answer is
  `MTLCommandBufferStatusCompleted` (4). That is before the readback (`getBytes:`) and
  before `[drawable present]`, and before either stores 1 into `OFF_PRESENTED`, so the
  submit answers 0: the answer and the slot `direct_failed` and "no texture" already use.
- `canvas::metalDrawScene` returns that answer (`AS Boolean`; it was `AS Nothing` and the
  readback path's 0 was ignored), and `__canvas_renderMetal` returns FALSE on it. So a
  frame whose buffer did not complete is drawn in software, on the readback path and on
  the direct present (whose FALSE first tries the readback path, as before).
- No environment variable and no stats field were added.

### Proof of the fallback (the check itself, not the link to this bug)

- Unit tests beside `the_frame_is_committed_then_waited_on`:
  `the_buffer_status_is_read_after_the_wait_and_before_the_frame_is_used` and
  `a_buffer_that_did_not_complete_reports_the_frame_not_drawn`. Both failed before the
  check existed (`_mfb_macapp_sel_status is sent once, got []`).
- One experiment, not committed: with the constant set to 5, so that EVERY buffer counts
  as not completed, a two-shape scene (`/tmp/bs23-agent-a/fb`, `mfb build --app --debug`)
  run with `MFB_CANVAS_GPU=1 MFB_CANVAS_SYNC=1`:
  - headless (readback path): stats `gpuSelected=TRUE gpuFrames=0`, and the
    `MFB_CANVAS_DUMP` frame is byte-identical (`cmp`) to the `MFB_CANVAS_GPU=0` frame;
  - real window (direct present): stats `gpuSelected=TRUE gpuFrames=0 directFrames=0`.

  With the constant back at 4 the same two runs report `gpuFrames=1` (headless) and
  `gpuFrames=1 directFrames=1` (window).

### What remains unproven or open

- That a failed command buffer is what produced the unwritten 32 x 32 tiles. If it was,
  the two tests now see a software frame and pass; if it was not, this change does not
  touch the failure.
- A buffer that failed leaves no trace a test can read: no stats field says "a GPU
  frame was declined after submit", by this round's rule of adding none. `gpuFrames=`
  not rising for a frame is the only sign.
- Vulkan has the same gap on its readback path and was not changed:
  `vulkan.rs:emit_vulkan_draw_scene` calls `vkQueueSubmit` and `vkQueueWaitIdle` and
  tests neither result before it copies the mapped image out. The direct present
  (`vulkan_present.rs`) tests `vkQueueSubmit` and `vkQueuePresentKHR`, and not
  `vkQueueWaitIdle`.
- bug-1059 (opaque black where the oracle has text) is not explained by this.

### Golden impact

`scripts/artifact-gate.sh target/release/mfb all`: 9 diffs. Four are the
`syntax/app/app-mouse-surface/app_mouse_surface.<target>.app.ncode` goldens that bug-968
and bug-970 already move. Five are new with this change:

| golden | why |
|---|---|
| `syntax/app/app-mouse-surface/app_mouse_surface.ir` | `__canvas_renderMetal` tests `canvas::metalDrawScene`'s answer |
| `syntax/app/app-mouse-surface/app_mouse_surface.macos-aarch64.app.nir` | the same body, and `metalDrawScene` returns `Boolean` |
| `syntax/app/app-mouse-surface/app_mouse_surface.macos-aarch64.app.nplan` | the same |
| `syntax/resources/loop-body-resource-reference-valid/loop_body_resource_reference_valid.ir` | it imports `canvas`, so it holds `__canvas_renderMetal` |
| `syntax/app/app-window-surface/app_window_surface.macos-aarch64.app.ncode` | a macOS app holds `_mfb_macapp_metal_draw` and the selector names; both gained `status` (`grep -c _mfb_macapp_sel_status` on its `--ncode`: 1) |

Goldens were not regenerated.

