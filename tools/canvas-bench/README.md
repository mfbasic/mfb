# tools/canvas-bench

bug-686's benchmark instrument for macOS Metal canvas rendering: does an
ordinary scene draw on the GPU, and does it hold 60 fps. Promotes the
one-off spike probes at `/tmp/bug686-spike/` (see the bug doc's
"Reproducing the numbers") into a repeatable tool, without changing any
rendering code. bug-688 adds a Linux/Vulkan twin of the same instrument
(same subcommands, same output format), described in "Linux mode" below.

- **bench.sh** — one runner, several subcommands. Everything is headless
  (`MFB_HEADLESS=1`),
  builds temporary MFBASIC projects under `$TMPDIR`, and cleans them up on
  exit.
- **gen/*.sh** — the MFBASIC scene templates `bench.sh` instantiates:
  `stress.sh` (polygons, tiled, moving or static, or one big polygon via
  `R=`/`CX=`/`CY=`), `poly.sh` (one N-point wavy ring plus a box), `pics.sh`
  (a tilemap of two alternating images plus an optional full-screen
  background), `text.sh` (a screen of text rows; takes an optional trailing
  font-path argument, used by Linux mode), `worker.sh` (the
  build/walk/present microbenchmark, no graphics thread involved), `hit.sh`
  (what `canvas::pollMouse` costs to answer `overID`, with and without ids).
- **cmp.py** — pixel-compares two `MFB_CANVAS_DUMP` RGBA files: differing
  pixel count and max per-channel delta. What every oracle check reads.

    bash tools/canvas-bench/bench.sh <path-to-mfb> <subcommand> [args...]

## Windows mode (bug-688 V5)

    bash tools/canvas-bench/bench.sh <path-to-mfb> --target windows-x86_64 --host test@192.168.1.206 sweep

`--target windows-x86_64 --host <user@host>` runs every subcommand against a
Windows box over key-auth ssh: the scene builds as a `.exe` (`mfb build --app
--target windows-x86_64`), is copied into a fresh `canvas-bench-*` directory
under the user's home, and runs headless (`MFB_HEADLESS=1`) through a
CRLF `.bat` that sets the `MFB_CANVAS_*` variables (cmd's `set X=1 && prog`
is unreliable over ssh). Stats and dump files come back to the same local paths
the other modes use, so every parser and `cmp.py` call is unchanged, and the
renderer column reads `Vulkan`. `text` uses `C:\Windows\Fonts\arial.ttf`.

Box 206 (`.ai/remote_systems.md`) has a **real GPU** (Intel UHD 630), so its fps
numbers are GPU numbers — the only Vulkan box where that is true.

## Linux mode (bug-688)

Add `--target linux-aarch64|linux-x86_64 --box <ssh-port>` (and optionally
`--libc glibc|musl`, default `glibc`) to run the exact same subcommands
against a remote Linux/Vulkan box instead of building and running a macOS
`.app` locally:

    bash tools/canvas-bench/bench.sh <path-to-mfb> --target linux-aarch64 --box 2226 poly 1000

These three flags may appear **anywhere** in the argument list — before the
subcommand, after it, or interleaved with its args — `bench.sh` strips them
out before parsing `<mfb>`/`<subcommand>`/its positional args, so
`bench.sh <mfb> poly --target linux-aarch64 --box 2226 1000` works exactly
the same as the example above. `--box` takes the local port an SSH tunnel to
the box listens on; the box is reached as `ssh -p <port> test@127.0.0.1`.
`--target` and `--box` must be given together (one without the other is an
error); `--libc` requires `--target`.

In Linux mode, `bench.sh`:

1. builds with `mfb build --app [--debug] --target <target> <dir>`, which
   writes one AppImage per libc flavor to `<dir>/build/<name>-<libc>.AppImage`;
2. ships the `<libc>` flavor's AppImage to the box with
   `scp -P <port> ... test@127.0.0.1:...`, into a fresh `/tmp/canvas-bench-*`
   directory (never touching any other directory already on the box — e.g. a
   concurrent job's `/tmp/mfb-rows-*` or `/tmp/mfb-probe-*`), and extracts it
   there with `./app.AppImage --appimage-extract`;
3. runs `./squashfs-root/usr/bin/<name>` over
   `ssh -o BatchMode=yes -o ConnectTimeout=10 -p <port> test@127.0.0.1`,
   under `timeout`, with `MFB_HEADLESS=1` plus the same
   `MFB_CANVAS_*` env vars the macOS path uses;
4. `scp`s any `MFB_CANVAS_STATS`/`MFB_CANVAS_DUMP` file the run wrote back to
   the same local path the macOS path would have written, so every stats
   parser and `cmp.py` call downstream of a subcommand reads it exactly as
   it reads a local macOS run;
5. cleans up both its local `$TMPDIR` directory and its remote
   `/tmp/canvas-bench-*` directory on exit.

The `renderer` column (and the `poly`/`compare`/`text`/`pics` oracle status
line) reports **`Vulkan`** instead of `Metal` in Linux mode: readiness comes
from the stats line's `vulkanReady=` field (macOS reads `metalReady=`), and
"every rendered frame counted as a `gpuFrames` frame" is still what makes a
row `Vulkan` vs. `software` vs. `mixed`.

`text` needs a font that exists on the box — the macOS default
(`/System/Library/Fonts/Supplemental/Arial.ttf`) does not. Linux mode passes
`gen/text.sh` an optional trailing font-path argument pointing at
`/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf` (a
metric-compatible Arial substitute already present on the Debian box this
was verified against, box 2226 — no font needs shipping). If a different box
lacks that path, probe it first (`ssh -p <port> test@127.0.0.1 'find
/usr/share/fonts -iname "*.ttf"'`) and update `LINUX_FONT` in `bench.sh`.

**Without `--target`/`--box`, `bench.sh` behaves exactly as it always
did** — every macOS command it runs (the `mfb build -app` invocations, the
`env MFB_HEADLESS=1 ...` runs, the `bin_path`/`grep`/`awk` parsing) is
untouched; Linux mode is purely additive.

**Warning: fps numbers from Linux mode are not GPU numbers.** The Vulkan
boxes reachable from here (e.g. box 2226) run Mesa's `lavapipe` — a
software (CPU) Vulkan implementation — not a GPU. The `renderer` column and
the oracle pixel-compare are still meaningful (they tell you whether the
Vulkan code path drew the frame and drew it correctly), but the `fps`
numbers measure a CPU rasteriser's throughput, not a GPU's, and are **not**
comparable to the macOS/Metal fps numbers or to a real GPU's Vulkan
performance.

## Subcommands

### `sweep [--release]`

Runs the bug doc's section B table: items × points × moving/static
(95×8, 1000×8, 1000×8 static, 2000×8, 2500×8, 5000×3, 10000×4, 10×300,
100×300, 10×4000), 5 s each. Default mode builds `--debug` (needed for
`MFB_CANVAS_STATS`) and reports:

- **fps** — rendered frames over 5 s.
- **renderer** — `Metal` if every rendered frame counted as a `gpuFrames`
  frame, `software` if none did, `mixed` otherwise.
- **builds/frame** — the `generations` counter's delta over the run,
  divided by rendered frames: how many full geometry rebuilds happen per
  rendered frame. At HEAD this is close to 2 per item on every row (section
  C/D): both graphics-thread walks miss the 256-entry cache above 256 items.
- **phase: ...** — per-frame phase-timer ms fields (`spike*Ms=` /
  `phase*Ms=`), if the stats line carries them; `-` if it does not. HEAD's
  `MFB_CANVAS_STATS` line has none yet (`helper_surface.rs`); Phase 0 of the
  bug doc adds them.

`--release` builds without `--debug` and counts presents instead of reading
stats (a release build has no `MFB_CANVAS_STATS` at all). It sets
`MFB_CANVAS_SYNC=1`, which every build reads, so each `present` waits for its frame
and the count of presents is the count of rendered frames. There are no
renderer/geometry columns (release has nothing to read them from).

**Why two build modes at all.** Measured on the 1,000×8 moving scene with
`MFB_CANVAS_SYNC=1` (every present waits for its frame): 116 frames in 5 s
as `--debug`, 157 frames as a release build — **a `--debug` build is about
35% slower on this path.** The fps numbers `sweep`'s default mode prints are
debug-build numbers; read them relative to each other and to the bug doc's
table (which used the same build), not as an absolute release-build number.
Use `--release` when the absolute fps matters and the phase/renderer columns
don't.

**Healthy**: every row on `Metal`, `builds/frame` near 0 on a static scene
and proportional to changed items on a moving one, fps within reach of 60
(see targets below). At HEAD (bug-686 open): rows past the caps show
`software` at 1-2 fps; rows under the caps stay on `Metal` but slide from
~137 fps at 95 items to ~12 fps at 2,000 (the "slope", section B).

### `poly N [tag]`

One N-point wavy ring plus a box, rendered once on software and once on
Metal (`MFB_CANVAS_SYNC=1`, `MFB_CANVAS_DUMP`), pixel-compared. Prints
`gpuSelected`/`metalReady`/`gpuFrames` from the Metal run's stats so a
decline (drawn on software because Metal declined it) is visible. **Healthy**:
0 differing pixels for N under the per-polygon cap (256 at HEAD), a few tens
of pixels at max delta 1-2 once accepted (antialiasing noise, not a bug —
see the bug doc's Non-goals). At HEAD, N above 256 is declined and both
frames draw in software (0 px differ, because there's nothing to disagree
about).

### `pics TILES TILE_PX BG_W BG_H`

A tilemap of TILES pictures (two alternating TILE_PX-square images) plus an
optional BG_W×BG_H background, shifting every frame. 5 s moving run for fps,
then a one-frame software-vs-Metal compare. Section H's texel-cap probe.
**Healthy**: `Metal` at the reported texel count, 0 differing pixels in the
oracle compare. At HEAD, more than 1,024 32-pixel tiles or a ~1 Mpx
background sends the frame to software.

### `text LINES COLS SIZE`

LINES rows of COLS characters at SIZE px, plus a moving cursor rect. Same
5 s + one-frame-oracle shape as `pics`. **Healthy**: `Metal`, and the oracle
compare shows only antialiasing noise (max delta ≤ a couple of levels). At
HEAD, a terminal-sized screen (say 120×36 at 16 px, 4,320 glyphs) exceeds the
4,096-quad frame cap and falls to software.

### `weather [window_secs]`

Builds `examples/weather` unmodified into a temp copy, runs it headless, waits
(up to 150 s) for its second stats line — the first frame after the map
appears, since the fetch screen can take ~30 s on a cold cache — then counts
frames over the next `window_secs` (default 20). Prints `gpuFrames`/`blocks`
at the start and end of the window. This is bug-686's **G3** acceptance
scene, unmodified.

**Healthy**: `gpuFrames` delta equals the frame count (every frame on
Metal), fps ≈ 60. At HEAD: about 1 frame in 20 s, on software.

### `worker`

Builds and runs the release-build worker microbenchmark (`gen/worker.sh`):
build a `List OF DrawItem` of 10,000 rectangles and of 10,000 shared-point
polygons, walk it with `FOR EACH` + `MATCH`, and `canvas::present` each list,
three repetitions. `MFB_CANVAS_GPU=0` so the graphics thread never runs —
this isolates the worker's own cost (section F). Prints the program's own
`io::print` timing lines directly.

**Healthy** (bug doc's **G2**): `canvas::present` at ≤0.5 µs per item. At
HEAD it costs about 2.9 µs/item for rectangles and 3.6 µs/item for polygons.

### `hit`

Builds and runs the release-build hit-test microbenchmark (`gen/hit.sh`) four
times — 1,000 and 10,000 rectangles, each with every item given an `id` and
with none — and prints one row per run:

    items   ids  present(med/min)   poll-first(med/min)  poll-again(med/min)  overID    events

All times are nanoseconds, the median and the minimum over 40 frames. Each
frame moves one item and presents, then polls sixty injected presses:
`poll-first` is the first `canvas::pollMouse` after a present that changed the
scene, which rebuilds the worker's draw list and walks it; `poll-again` is each
of the other fifty-nine against the same scene, the walk alone, timed together
and divided because the clock ticks in microseconds. The presses are at pixel
(444, 54), inside item 484, so the walk — last item first — covers nearly the
whole list.

**Healthy**: the two `ids = 0` rows poll in tens of nanoseconds at either size —
a scene with no `id` builds nothing — and their `overID` is `[]`; the `ids = 1`
rows report `[i484]`; `events` is 60 on every row. An event not collected
within a tenth of a second is dropped, so `events` under 60 means the box was
too slow for that size and its `poll-again` is understated.

The presses are delivered by `MFB_MOUSE_INJECT`, which is POSIX-only, so `hit`
runs locally and on a Linux box and refuses `--target windows-x86_64`.

### `compare stress|poly|pics|text ARGS...`

The general oracle check: render any of the four scene kinds once on
software and once on Metal, report differing pixels and max channel delta.

- `compare stress ITEMS POINTS [R CX CY]` — the stress scene's static bulk,
  one frame. `R`/`CX`/`CY` draw ITEMS as one big polygon of that radius and
  centre when ITEMS=1 (section E's large-polygon probe).
- `compare poly N`
- `compare pics TILES TILE_PX BG_W BG_H`
- `compare text LINES COLS SIZE`

**Healthy**: 0 differing pixels for a scene the backend fully supports (a
picture or clean rectangle scene), or a few tens of pixels at max delta 1-2
for an antialiased polygon or glyph edge — that is the same noise floor as
HEAD's accepted scenes, not a regression. A large, unexplained pixel count
or a max delta near 255 means one renderer drew something the other didn't
(a decline, a missing item, or a real bug) — cross-check with the matching
`poly`/`pics`/`text` subcommand's status fields to see which.

## 60 fps target rows (bug-686's Goal)

These are what bug-686 is judged on. None hold at HEAD; they're listed here
so a later `sweep`/`weather` run can be checked against them directly.

| target | row | status at HEAD |
| --- | --- | --- |
| **G3** | `weather` unmodified, `gpuFrames = frames`, 60 fps | not met: ~0.05 fps, software (section A) |
| **G5** | `sweep`, ~10,000 moving items, 60 fps on Metal | not met: 10,000×4 is declined to software (section B) |
| **G1/G5** | `compare stress 1 8 R=300 CX=450 CY=320` (or `poly` at 64,000 edges), Metal render ≤16 ms | not met: a 64k-edge polygon costs ~252 ms of GPU time alone (section E); `bench.sh` reports pixel agreement, not GPU time — cross-reference section E/G's numbers in the bug doc for the ms figure until a GPU-time field lands in the stats line |
| **G1b** | `pics`, thousands of tiles from a handful of images, 60 fps | not met: >1,024 tiles or a ~1 Mpx background is declined (section H) |
| **G2** | `worker`, `canvas::present` ≤0.5 µs/item | not met: 2.9-3.6 µs/item |

`bench.sh` does not itself measure Metal's `gpuEndTime - gpuStartTime` (the
GPU-time-only numbers in bug doc sections E/G came from a standalone Metal
compute-shader spike, `/tmp/bug686-spike/band/band.swift`, not from the
renderer). Once Phase 0's phase timers land on the stats line, `sweep`'s
`phase: ...` column will report the render-phase ms figure directly instead
of "-".
