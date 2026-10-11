# bug-1024: a Metal text frame is dumped with unwritten tiles (seen on a loaded machine)

Last updated: 2026-10-10
Effort: unknown — not yet root-caused
Severity: MEDIUM
Class: Concurrency (suspected)

Status: Open
Regression Test: `cargo test --test rt_canvas_font gpu` (exists; red when this happens)

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
