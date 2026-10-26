# Baseline results

The committed reference run of the canvas benchmark: the output of one full
`./benchmark/canvas/run.sh` (87 rows, 120 frames each, display sync on). Regenerate by
re-running that command and moving its `canvas-<ts>.log` / `.sums` pair here as
`canvas.log` / `canvas.sums`.

`canvas.log` holds each row's per-frame times, and `canvas.sums` each row's per-frame
counts and the hash of its last frame's pixels (see the parent README). Compare a run
with `./benchmark/canvas/compare.py canvas-<ts>.log`.

Provenance:

- date: 2026-10-25 (run timestamp `20261025-223721`)
- commit: `a9edb077b` (plan-175 A–D merged with main at `ba92007cd`; the run used this
  tree's `reduce.py`, which drops a window's software start-up frames since bug-1087)
- host: Apple M2 Max, macOS 15.7.7 (arm64), built-in display at 120 Hz, 3024 x 1964
  Retina; other sessions were running (load average 70.04 at `uptime` twenty minutes
  before)
- command: `MFB=target/release/mfb ./benchmark/canvas/run.sh`, attended, every window
  unobstructed
- build: `--debug` (the frame profile exists only there)

**Against main's tip in the same session** (`ba92007cd`, built in a detached worktree,
run windowed five minutes later; `compare.py <this run> --base <main's>`): 435
checksums compared, **2 changed**, both intended — `group_fill_10000_draws` 10,001 → 2
and `group_fill_100_draws` 101 → 2 (plan-175-C: adjacent group nodes are one draw).
Every pixel hash and every buffer-byte count is equal. `work` median geomean 1.075:
`group fill 10000` 5,260 → 2,719 µs (0.52×; its encode 2,153 → 60 µs), `tile fill
10000` 3,178 → 2,208 (0.69×), `text fill 10000` 6,807 → 5,258 (0.77×); and 21 rows
slower by 1.25× or more, the flat 10,000-item ones in `gpu` (`rrect fill 10000` 996 →
2,572 µs). That last is a **power state, not GPU work**: bisected to plan-175-D
(`17b7b529e`, the worker's present compares before it copies), which changes nothing the
GPU receives, and headless — frames back to back, the clocks held up — D and its
base draw `circle`/`ellipse fill 10000` in 1,243 to 2,892 µs of GPU time each way with
the CPU fill identical (241 to 248 µs). With the copy gone from every unchanged present,
the frame does less work, the system clocks down, and the GPU's share of a frame that
still makes its vsync grows. Read the flat rows' `gpu` against that.

Against the run it replaces (`20261014-105712` at `840fa4e18`): pixel hashes and buffer
bytes differ on many rows (`rect_fill_1_bytes` 736 → 608, `polyline_stroke_1_pixels`,
…). Main's tip shows the same values as this run, so those changes are main's since
`840fa4e18`, not this baseline's commit — which is why the comparison above is against
main's tip.

`particles fill 10000`'s `gpu` median read 0.76× (2,202 → 1,673 µs): its GPU time is
**bimodal from run to run with the same compiler** (three runs each with two compilers
gave 1,389 to 2,183 µs, with identical draw counts, buffer bytes and passes), and this run
caught the lower band. Read that row's `gpu` and `work` against a 1.5× band, not the 1.25×
of the parent README.

How much a row moves by itself, from the replaced baseline's own check: two runs of one
commit five minutes apart gave `work` median geomean 1.006 and `gpu` 1.008 over 83 rows,
no row past 1.25×, all checksums equal; their `min` column moved up to 1.74× (`work`) and
2.19× (`gpu`) on a single row, which is why the comparison reads the median.

Timings are host- and display-specific: compare a new run against this baseline only on
the same machine, on the same display, with the same refresh rate.
