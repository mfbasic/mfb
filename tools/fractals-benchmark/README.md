# Fractals comparison

Run from the repository root on macOS with an active desktop session:

```sh
python3 tools/fractals-benchmark/compare.py
```

This opens fullscreen windows in succession. Avoid resizing windows or running
other graphics workloads during measurement. It requires a built compiler at
`target/release/mfb`; override with `--mfb PATH`.

The harness instruments temporary copies of `examples/fractals` and
`examples/fractals-image`, builds both with the same compiler and default
optimization, and runs the actual AppKit/Metal applications. It waits for the
fullscreen transition before measurement, discards three warmup frames, then
samples each fractal at 0, 6, and 11 seconds into its cycle (including crossfades).
The default is six frames per position and three runs per implementation,
alternating their order. `--frames` and `--rounds` override these defaults.

It checks that the shared sampling/palette functions are identical and that every
measured frame has matching canvas dimensions and a matching position-weighted
RGB checksum. These checks verify the calculated colours, not screenshot equality:
the original uses separated circles while the image version fills the surface.
The same checksum calculation is included in both timings.

The output separates CPU frame construction plus `present` from the complete
paced iteration through `waitFrame`. The latter includes rendering backpressure
and display pacing; it is not an isolated GPU-duration measurement. `waitFrame`
waits for drawing to start, not for the GPU to finish. Startup, compilation,
logging, and the fullscreen transition are excluded from measured iterations.

Raw results are written to `/tmp/fractals-benchmark.json` (override with
`--output PATH`). Each row contains position index, build-and-submit nanoseconds,
paced-iteration nanoseconds, colour checksum, canvas width, and canvas height.

No speedup result is recorded from the initial session: other compiler processes
were saturating CPU cores, and some repeated launches reported different canvas
sizes. The harness rejects dimension or checksum mismatches rather than reporting
incomparable timings. Run it on a quiet desktop and accept results only when the
final matching-checks message appears.
