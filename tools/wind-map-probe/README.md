# wind-map-probe

Measures how many `canvas::DrawItem`s `examples/wind`'s static map contributes
to a single animation frame's scene — the quantity bug-712 was about.

`examples/wind` runs an animation loop that calls `canvas::present` once per
frame. `land.mfb:mapItems` builds the ocean rectangle plus every visible
coastline ring — several dozen items even at a world-sized zoom — and caches
that list per `View` (`sameView`/`drawnView`/`drawnMap`). Before bug-712's
fix, the cached list was still spliced into a fresh per-frame scene by value,
so `canvas::present` (which copies the whole scene it is handed, every call)
copied the whole map on every single frame even though nothing about it had
changed. The fix installs the map once with `canvas::setGroup` and hands
`present` a single `canvas::Group` node in its place.

This tool is a standalone `--app`-mode MFBASIC program (verbatim copies of
`examples/wind/src/land.mfb` and `earth.mfb`, plus their `coastline.bin`
resource) that calls `mapItems` directly for a whole-world view, a 10x-zoomed
view, and the zoomed view again (a cache hit), and prints how many items each
call's result holds. It needs `app::Mode.Canvas` active (`canvas::setGroup`
requires it), so it runs headless via `MFB_MACAPP_HEADLESS=1` rather than as a
plain console program — no window is opened.

Run it:

```
$ target/release/mfb build tools/wind-map-probe   # once, or whenever mfb itself changes
$ ./tools/wind-map-probe/run.sh target/release/mfb
whole-world frame items: 1
10x zoom frame items: 1
10x zoom frame items again (cache hit): 1
PASS: wind-map-probe — the static map contributes exactly one item (a canvas::Group node) to a frame's scene, at any zoom
```

Before the fix (measured against the pre-fix `land.mfb`, not committed here):
`whole-world frame items: 98`, `10x zoom frame items: 24` — the raw map item
count at each zoom, recomputed by value into every frame's scene.

`run.sh` builds the tool with the `mfb` binary path given on its command
line, runs it headless with a 15-second watchdog (the same shape
`scripts/test-macapp.sh` uses for every other `--app` runtime probe), and
asserts every reported count is exactly `1` — the map now costs one
`canvas::Group` node a frame, not one item per coastline ring. Skips (exit 0)
on a non-macOS host, since this is an `--app` build and only macOS is
exercised here.

If a future example grows a large cached static sub-picture spliced into a
per-frame scene the same way `wind` used to, the fix is the same
(`canvas::setGroup` once, a `canvas::Group` node every frame) and this tool's
shape — call the builder function directly, count the items it hands back —
is the template for measuring it there too.
