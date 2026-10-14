# benchmark/canvas

The standing benchmark for the canvas renderer: what each kind of `canvas::DrawItem`
costs a frame, split into where the time goes, on macOS's Metal renderer in a real
window. It is the canvas twin of `benchmark/language`. Bug-specific canvas instruments
live under `tools/` (`tools/canvas-bench`, `tools/particle-bench`).

    ./benchmark/canvas/run.sh                        # every row, 120 frames each
    ./benchmark/canvas/run.sh --only 'text|picture'  # rows whose name matches an ERE
    ./benchmark/canvas/compare.py benchmark/canvas/canvas-<ts>.log

- **mfb/** — one canvas program; the row is chosen by environment variables (`KIND`,
  `COUNT`, `PAINT`, `SIZE`, `FRAMES`, `FONT`; its header lists them).
- **run.sh** — builds the program once with `--debug`, runs every row in a window, and
  writes `canvas-<ts>.log` and `canvas-<ts>.sums` here (both git-ignored).
- **reduce.py** — turns one row's `MFB_CANVAS_STATS` file into its log block and
  checksum lines; `run.sh` calls it.
- **compare.py** — a run against `baseline/` (or any other run with `--base`).
- **baseline/** — the committed reference run and its provenance.

## What a row is

A row is a scene of one item kind, presented `FRAMES` times under `MFB_CANVAS_SYNC=1`
(each `present` returns once its frame is drawn), with the frame profile on
(`MFB_CANVAS_STATS`). The scene is built once. Each frame appends one small marker
rectangle at a new place, because an unchanged scene is never drawn again. So every
frame is a full redraw, and every item except the marker keeps its cached geometry. The
marker is in every row, so it costs every row alike.

| Rows | What they are |
|---|---|
| `KIND fill N`, N = 1, 100, 10000 | every kind, 16-pixel items scattered over the surface |
| `KIND stroke N`, `KIND fillStroke N`, N = 100, 10000 | the kinds with an inside, outlined, and filled and outlined |
| `KIND fill big` / `KIND stroke big` | one item over most of the surface: the fragment cost |

The kinds are `rect`, `rrect` (rounded corners), `line`, `polygon` (a ten-point star),
`circle`, `arc`, `ellipse`, `text` (`"Ag"`), `polyline` (eight points, a colour each),
`polycurve` (one cubic), `curvepolygon` (four cubics), `autopaint` (a circle with a
hover paint), `picture` (a 32-by-32 image in a circle), `group` (N `canvas::Group` nodes
naming one installed rectangle), `particles` (one `canvas::ParticleSystem` of N
particles) and `tile` (a `canvas::Tile` drawing the middle 16-by-16 cell of the same
image; it has no paint, so it has only `fill` rows). `line`, `arc`, `polyline` and `polycurve` have no inside, so they are always
stroked.

## Reading the log

The log has the language suite's shape: one block per row, one line per column, and the
median, average, min and max over the frames, in **microseconds per frame**. Frame 1 is
dropped: it compiles the pipeline and fills every cache.

| Column | Field (`mfb spec tooling debug-report`, "The canvas frame profile") | What it is |
|---|---|---|
| `vsync` | `profAcquireUs` | waiting for the window's next drawable: the display's pacing |
| `fill` | `profFillUs` | CPU writing every item's data into the frame buffer |
| `encode` | `profEncodeUs` | recording the render pass, draws and the copy into the drawable |
| `wait` | `profWaitUs` | the graphics thread blocked until the GPU finished |
| `gpu` | `profGpuUs` | the GPU's own execution time |
| `present` | `profPresentUs` | presenting the drawable |
| `work` | fill + encode + wait + present | the renderer's own cost of the frame |
| `walk` | `phaseOffsetsMs` + `phaseDrawsMs` | the graphics thread's scene walk and draw list, before the renderer |

**`work` is the number to compare, on its median.** Not on `min`, which is the language
suite's estimator (`benchmark/language/RANKING.md` §1.1): measured across two identical
full runs, a row's `work` median moved 1.09× at p90 and 1.22× at most, and its `min`
1.40× and 1.74× (`gpu`: 1.04× and 1.24× against 1.73× and 2.19×). A row is about 119
frames, and its least catches the odd frame the GPU ran at a higher clock. So a row
that moves by 1.25× or more (`compare.py`'s default) is outside the noise those two runs
showed, and a smaller move is not evidence. `vsync` is reported but kept out of `work`, because it is the display's pacing rather
than the renderer's. With display sync on (the default) the wait for the next refresh
is all in `vsync`, and the other times are the frame's own: measured on one moving
rectangle at 120 Hz, `vsync` was 7.4 ms of an 8 ms frame and the GPU 0.23 ms.

`walk` is on the stats line in whole milliseconds, so one frame's walk is resolved only
to a millisecond; read its average.

The `.sums` file holds each row's per-frame counts (`draws`, `bytes`, `texels`,
`passes`) and a hash of its last frame's pixels, from a second, two-frame run of the row
with `MFB_CANVAS_DUMP` (a dumped frame is read back rather than presented, so it cannot
be the timed run). Dumps are byte-identical from run to run, so a changed hash is a
change in what was drawn.

`run.sh` stops at the first row a frame of which was not drawn by Metal straight to the
window (`gpuFrames`, `directFrames`): a frame declined to software, or read back, is
not the thing being measured.

## Display sync

`--vsync off` sets `MFB_CANVAS_VSYNC=0`, which turns the window layer's display sync
off. It does not take the refresh out of a frame: in a window the compositor still
paces the drawables, and the wait moves from `vsync` into `gpu` and `wait`. On the
rectangle above the GPU went from 0.23 ms to 8.0 ms a frame and the frame from 8 ms to
16 ms. Use it to look at that behaviour, not to measure the renderer; the baseline is
taken with display sync on.

## Running it

- **macOS only.** On Linux and Windows every `prof…` field is 0.
- **A `--debug` build.** The frame profile exists only there, so every number here is
  a `--debug` build's. Compare runs with runs, never with a release build's frame rate.
- **In a window, attended.** Every row opens a window. Keep the display awake and the
  windows unobstructed: a hidden, minimised or occluded window changes how the
  compositor paces its drawables. Close other heavy work; the baseline README records
  the load average.
- About 160 launches (two per row), and a few minutes in all.

## Updating the baseline

Run `./benchmark/canvas/run.sh` on the baseline host, move the `canvas-<ts>.log` and
`.sums` pair to `baseline/canvas.log` and `baseline/canvas.sums`, and update
`baseline/README.md`'s provenance and its comparison with the run it replaces
(`compare.py` gives the numbers).
