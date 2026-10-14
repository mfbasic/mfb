# Baseline results

The committed reference run of the canvas benchmark: the output of one full
`./benchmark/canvas/run.sh` (87 rows, 120 frames each, display sync on). Regenerate by
re-running that command and moving its `canvas-<ts>.log` / `.sums` pair here as
`canvas.log` / `canvas.sums`.

`canvas.log` holds each row's per-frame times, and `canvas.sums` each row's per-frame
counts and the hash of its last frame's pixels (see the parent README). Compare a run
with `./benchmark/canvas/compare.py canvas-<ts>.log`.

Provenance:

- date: 2026-10-14 (run timestamp `20261014-044940`)
- commit: `c4eaebebb` (main after plan-173's `canvas::Tile` and the `AutoPaint` walk fix,
  `9a0723cc6`)
- host: Apple M2 Max, macOS 15.7.7 (arm64), built-in display at 120 Hz, 3024 x 1964
  Retina; load average ~2.6
- command: `./benchmark/canvas/run.sh`, attended, every window unobstructed
- build: `--debug` (the frame profile exists only there)

Against the run it replaces (`20261013-190605` at `8b43850b0`), by `compare.py`: `work`
median geomean 1.011 and `gpu` 1.010 over the 83 rows both have, and all 415 shared
checksums equal. Four rows are new: `tile fill 1`, `100`, `10000` and `big`, plan-173's
`canvas::Tile` (`9e9031548`). One row moved for a known reason:

- `autopaint fill 10000`: walk 277.2 → 46.7 ms a frame, work 9.1 → 6.7 ms. The walk had
  grown with the square of the instance count (`9a0723cc6`).

One row moved for no reason in the code:

- `particles fill 10000`: `gpu` median 1,520 → 2,202 µs (1.45×), `work` 1.25×. Its GPU
  time is **bimodal from run to run with the same compiler**: three runs each with the
  `8b43850b0` compiler gave 1,459 / 2,078 / 1,532 µs and with `c4eaebebb` 1,389 / 2,183 /
  1,596 µs (the suite as of `09eaa48f3`, so both compilers drew the same program). The
  draw count, buffer bytes and passes are identical. The replaced baseline caught the
  lower band and this one the upper. Read this row's `gpu` and `work` against a 1.5×
  band, not the 1.25× of the parent README.

How much a row moves by itself, from the replaced baseline's own check: two runs of one
commit five minutes apart gave `work` median geomean 1.006 and `gpu` 1.008 over 83 rows,
no row past 1.25×, all checksums equal; their `min` column moved up to 1.74× (`work`) and
2.19× (`gpu`) on a single row, which is why the comparison reads the median.

Timings are host- and display-specific: compare a new run against this baseline only on
the same machine, on the same display, with the same refresh rate.
