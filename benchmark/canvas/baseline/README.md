# Baseline results

The committed reference run of the canvas benchmark: the output of one full
`./benchmark/canvas/run.sh` (87 rows, 120 frames each, display sync on). Regenerate by
re-running that command and moving its `canvas-<ts>.log` / `.sums` pair here as
`canvas.log` / `canvas.sums`.

`canvas.log` holds each row's per-frame times, and `canvas.sums` each row's per-frame
counts and the hash of its last frame's pixels (see the parent README). Compare a run
with `./benchmark/canvas/compare.py canvas-<ts>.log`.

Provenance:

- date: 2026-10-14 (run timestamp `20261014-105712`)
- commit: `840fa4e18` (main with the arena zero fill, `4bb55ea8c`)
- host: Apple M2 Max, macOS 15.7.7 (arm64), built-in display at 120 Hz, 3024 x 1964
  Retina; load average ~3.2
- command: `./benchmark/canvas/run.sh`, attended, every window unobstructed
- build: `--debug` (the frame profile exists only there)

Against the run it replaces (`20261014-044940` at `c4eaebebb`), by `compare.py`: `work`
median geomean 0.984 and `gpu` 0.996 over all 87 rows, no row's `work` moved by 1.25×
or more, and all 435 checksums equal (the `.sums` file is byte-identical). The arena's
zero fill (`4bb55ea8c`) shows in the `walk` column, the graphics thread's scene walk,
which frees per frame what it builds:

| Row | walk ms a frame |
|---|---|
| `autopaint fill 10000` | 46.7 → 22.8 (0.49×) |
| `group fill 10000` | 16.2 → 12.5 (0.77×) |
| `picture fill 10000` | 12.1 → 9.6 (0.79×) |
| `particles fill 10000` | 21.0 → 18.1 (0.86×) |
| `tile fill 10000` | 5.6 → 4.9 (0.88×) |

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
