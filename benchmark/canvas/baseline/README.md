# Baseline results

The committed reference run of the canvas benchmark: the output of one full
`./benchmark/canvas/run.sh` (83 rows, 120 frames each, display sync on). Regenerate by
re-running that command and moving its `canvas-<ts>.log` / `.sums` pair here as
`canvas.log` / `canvas.sums`.

`canvas.log` holds each row's per-frame times, and `canvas.sums` each row's per-frame
counts and the hash of its last frame's pixels (see the parent README). Compare a run
with `./benchmark/canvas/compare.py canvas-<ts>.log`.

Provenance:

- date: 2026-10-13 (run timestamp `20261013-190605`)
- commit: `8b43850b0` (main plus the benchmark/language move, its re-baseline, and
  `MFB_CANVAS_VSYNC`)
- host: Apple M2 Max, macOS 15.7.7 (arm64), built-in display at 120 Hz, 3024 x 1964
  Retina; load average ~3.3
- command: `./benchmark/canvas/run.sh`, attended, every window unobstructed
- build: `--debug` (the frame profile exists only there)

How much a row moves by itself: the run five minutes earlier (`20261013-190108`, same
commit, same command) against this one, by `compare.py`: `work` median geomean 1.006
and `gpu` 1.008 over 83 rows, no row moved by 1.25× or more, and all 415 checksums
equal. On the `min` column the same two runs moved up to 1.74× (`work`) and 2.19×
(`gpu`) on a single row, which is why the comparison reads the median.

Timings are host- and display-specific: compare a new run against this baseline only on
the same machine, on the same display, with the same refresh rate.
