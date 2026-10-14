# tools/particle-bench

plan-150-B's instrument: what one `canvas::ParticleSystem` costs a frame. A benchmark,
so it lives here and not under `scripts/`; nothing in the test suite runs it.

- **src/main.mfb** — one system, every particle alive for the whole run, presented
  `FRAMES` times with `time` advanced each present. It prints the least and the mean
  time a `present` took and what `canvas::getFrameStats` says drew the last frame. Its
  knobs are environment variables, so one build serves every row:

  | Variable | Values | Default |
  |---|---|---|
  | `COUNT` | particles | `1000` |
  | `FRAMES` | presents | `120` |
  | `TEMPLATE` | `circle`, `rect`, `text` | `circle` |
  | `EMITTER` | `disc`, `polygon` (a star: about half its box, so the rejection loop works) | `disc` |
  | `MOTION` | `still`, `moving`, `spinning` | `moving` |
  | `TINT` | `white`, `ramp` | `ramp` |
  | `POLL` | `1` to call `canvas::pollMouse` every frame | `0` |
  | `FONT` | the font a `text` template loads | set by `bench.sh` |

- **bench.sh** — builds the program twice (normally and with `--debug`) under
  `$TMPDIR`, runs rows, cleans up. macOS, local, headless.

      bash tools/particle-bench/bench.sh <path-to-mfb> sweep [FRAMES]
      bash tools/particle-bench/bench.sh <path-to-mfb> row COUNT [NAME=value ...]

## Reading a row

    particles/frame=1000 … | gpu: min=…us mean=…us renderer=Metal | software: min=…us mean=…us renderer=Software | debug: frames=… skipped=… gpuFrames=… blocks=… phaseMs/frame: offsets=… draws=… render=…

Every run sets `MFB_CANVAS_SYNC=1`, under which a `present` returns when its frame has
been drawn. The time around it is therefore a whole frame: the worker's copy and hash
of the scene, and the graphics thread's expansion of the system, its geometry and the
draw. A program that does not wait on its frames pays the worker's part per present and
the rest per frame drawn.

- **gpu** and **software** are the normal build, with and without `MFB_CANVAS_GPU=1`.
  These are the times. Rank on `min` (`benchmark/language/RANKING.md` §1.1): the machine can
  only make a run slower, so the least is the estimate and the mean carries everything
  else that was running.
- **debug** is the `--debug` build under `MFB_CANVAS_GPU=1` with `MFB_CANVAS_STATS`.
  It is slower, so nothing here is a time to quote; it is where the counts come from.
  `gpuFrames` below `frames` means frames were declined to software; `blocks` is the
  item blocks of the last frame, one per particle for every template but text, which
  is one per glyph. The phase timers are the graphics thread's, per frame: `offsets`
  is the scene walk — for a particle scene, the expansion and every particle's
  geometry — `draws` the draw list, `render` the draw itself.
- The first frame of a run is left out of both times: it builds the pipeline and
  fills every cache.

A headless GPU frame is read back to memory, which a frame in a window is not, so the
`gpu` column is an upper bound on what a window pays for the same scene.

## Splitting the expansion

`offsets` is one number for three things: the model (where each particle is), the item
built and hashed for it, and the geometry record built from that item. They are split
by patching the expansion in a scratch checkout — never landed — and re-running a row:

1. the model alone: `__canvas_particleAppend` runs `__canvas_particleState` for every
   slot and appends nothing;
2. the model, the item and its hash: it also builds each particle's item and calls
   `__canvas_hashItem`, and appends nothing;
3. the whole path, unpatched.

Each difference is one part's cost. The patches and the numbers they gave are in
`planning/completed/plan-150-B-*.md`.
