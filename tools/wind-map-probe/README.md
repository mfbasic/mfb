# wind-map-probe

Measures how many `canvas::DrawItem`s `examples/weather`'s static map contributes
to a single animation frame's scene — the quantity bug-712 was about.

`examples/weather` runs an animation loop that calls `canvas::present` once per
frame. `land.mfb:mapItems` builds the ocean rectangle plus every visible
coastline ring — several dozen items even at a world-sized zoom — and caches
that list per `View` (`sameView`/`drawnView`/`drawnMap`). Before bug-712's
fix, the cached list was still spliced into a fresh per-frame scene by value,
so `canvas::present` (which copies the whole scene it is handed, every call)
copied the whole map on every single frame even though nothing about it had
changed. The fix installs the map once with `canvas::setGroup` and hands
`present` a single `canvas::Group` node in its place.

## What this tool checks, and how

This tool builds and runs the **real** `examples/weather/src/land.mfb` and
`earth.mfb`, and the **real** `examples/weather/resources/weather/coastline.bin` —
nothing here is a copy of them. `run.sh` stages them into a fresh scratch
directory on every invocation (`mktemp -d`, then `cp` from the live example
tree), alongside `probe/main.mfb` — the only file this tool commits, which is
probe-only glue (it calls `mapItems` directly and prints how many items it
returns), not a fork of any example file. If `land.mfb` or `earth.mfb`
regresses, the very next run stages the regressed file and this probe
catches it; a committed copy could not.

It calls `mapItems` directly for a whole-world view, a 10x-zoomed view, and
the zoomed view again (a cache hit), and prints how many items each call's
result holds. It needs `app::Mode.Canvas` active (`canvas::setGroup`
requires it), so it runs headless via `MFB_MACAPP_HEADLESS=1` rather than as
a plain console program — no window is opened.

Run it:

```
$ ./tools/wind-map-probe/run.sh target/release/mfb
whole-world frame items: 1
10x zoom frame items: 1
10x zoom frame items again (cache hit): 1
PASS: wind-map-probe — the static map contributes exactly one item (a canvas::Group node) to a frame's scene, at any zoom
```

`run.sh` builds the staged copy with the `mfb` binary path given on its
command line (no separate `mfb build tools/wind-map-probe` step — there is no
committed `project.json` here, since the project is assembled fresh in the
scratch directory each run), runs it headless with a 15-second watchdog (the
same shape `scripts/test-macapp.sh` uses for every other `--app` runtime
probe), and asserts every reported count is exactly `1`.

Verified RED against the pre-fix `land.mfb` (`git show <pre-fix commit>^:examples/weather/src/land.mfb`,
swapped in temporarily, not committed): `whole-world frame items: 98`,
`10x zoom frame items: 24` — the raw map item count at each zoom, recomputed
by value into every frame's scene. Restored, this tool is GREEN again.

Skips (exit 0) on a non-macOS host, since this is an `--app` build and only
macOS is exercised here.

If a future example grows a large cached static sub-picture spliced into a
per-frame scene the same way `wind` used to, the fix is the same
(`canvas::setGroup` once, a `canvas::Group` node every frame) and this tool's
shape — stage the real files, call the builder function directly, count the
items it hands back — is the template for measuring it there too.
