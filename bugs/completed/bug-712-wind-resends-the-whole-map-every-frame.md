# bug-712: `examples/wind` copies the whole static map into `canvas::present` on every animation frame

Last updated: 2026-09-25
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Performance

Status: Fixed

STATUS: FIXED (616993ec9)

`mapItems` does the substitution itself (calls `canvas::setGroup` and returns/
caches a one-item `[canvas::Group[...]]` list) — `frameScene` needed no
change. Regression tool: `tools/wind-map-probe/` (commit 7d343858e). No other
deviation from the Fix Design; see Phase notes below for the one gate
deviation (this lane's narrower build-examples gate, not the full
`scripts/build-examples.sh` sweep).

Regression Test: a `/tmp`-style item-count probe promoted into the example, see
"Validation Plan" (examples have no test harness; propose adding one under
`tests/` per Blast Radius)

`examples/wind` runs an animation loop (particle streamlines over a world map)
that calls `canvas::present` once per frame via `canvas::waitFrame`. Every
frame's scene list is `mapItems(world, view)` (the ocean rectangle plus every
coastline ring in view — a static picture that only changes when the view
pans or zooms) concatenated with `trailItems(view)` (the moving particle
trails, which change every frame) and, when captions are on, a few text
items. `mapItems` itself is cached per-`View` (`drawnView`/`drawnMap`), so it
does not *recompute* the land geometry every frame — but the cached list is
still spliced into a brand-new combined list and handed to `canvas::present`
every frame, and `mfb man canvas present` documents exactly this cost:
"present copies the whole scene... Re-presenting an identical scene does
nothing... present compares the incoming content against what is already
installed" — the comparison happens only *after* the copy, and here the
combined list is never byte-identical frame to frame (the trail moves), so
the map's static items are fully copied on every single frame for no
reason.

**Correct behavior:** the static map is installed once (or once per rebuild,
when the view changes) with `canvas::setGroup`, and every frame's scene
references it with a `canvas::Group` node — two floats and a name — instead
of re-including the land's `DrawItem`s. `canvas::present` then copies a
small, constant-size scene every frame regardless of how large the map is.

References:

- `mfb man canvas setGroup` (run 2026-09-25): "canvas::present copies the
  whole scene every time it is called. A large static sub-picture — a map,
  a sprite sheet, a UI panel — is therefore copied on every frame even
  though nothing about it changed. Installing it once and referencing it
  means each present copies the node and not the picture." This is a
  description of exactly this example.
- `mfb man canvas present` (run 2026-09-25): "present copies the whole
  scene... Re-presenting an identical scene does nothing... present compares
  the incoming content against what is already installed."

## Failing Reproduction

`examples/wind` needs a live NOAA NOMADS fetch before it reaches the map
(`examples/wind/src/main.mfb:main`, `loadAnalysis()` runs before
`worldOutline()`/`loadCoastline()`), and this task forbids external network
calls, so the GUI app itself cannot be driven end to end this session. The
land-drawing code has no network dependency, so a `/tmp` driver was built
that reuses `examples/wind/src/land.mfb` and `examples/wind/src/earth.mfb`
verbatim (copied unmodified) with a new `main.mfb` that skips the fetch and
calls `mapItems` directly, to measure the one fact this bug turns on: how
many `DrawItem`s the static map list holds and that they are cache-stable
across repeated calls with the same `View`.

Driver (`/tmp/bug712b/src/main.mfb`, `/tmp/bug712b/project.json` mirrors
`examples/wind/project.json` minus the wind-specific resources; the coastline
resource is `examples/wind/resources/wind/coastline.bin`, copied unmodified):

```basic
IMPORT canvas
IMPORT fs

FUNC main AS Integer
  loadCoastline()
  LET world AS Ring = worldOutline()

  MUT report AS String = ""

  LET size AS canvas::Size = canvas::Size[width := 1920, height := 1080]
  LET wholeView AS View = wholeWorld(size)
  LET wholeItems AS List OF canvas::DrawItem = mapItems(world, wholeView)
  report = report & "whole-world view: " & toString(len(wholeItems)) & " items\n"

  LET zoomedView AS View = View[scale := wholeView.scale * 10.0, cx := wholeView.cx, cy := wholeView.cy, width := wholeView.width, height := wholeView.height, lon0 := wholeView.lon0]
  LET zoomedItems AS List OF canvas::DrawItem = mapItems(world, zoomedView)
  report = report & "10x zoom view: " & toString(len(zoomedItems)) & " items\n"

  LET zoomedItemsAgain AS List OF canvas::DrawItem = mapItems(world, zoomedView)
  report = report & "10x zoom view again (cache hit): " & toString(len(zoomedItemsAgain)) & " items\n"

  fs::writeText("/tmp/bug712-report.txt", report)
  RETURN 0
END FUNC
```

```
$ mfb build /tmp/bug712b
Building wind-driver (executable) for macos-aarch64
Wrote executable to /tmp/bug712b/build/wind-driver.app
$ /tmp/bug712b/build/wind-driver.app/Contents/MacOS/wind-driver
$ cat /tmp/bug712-report.txt
whole-world view: 91 items
10x zoom view: 14 items
10x zoom view again (cache hit): 14 items
```

- Observed: the whole-world view's static map is 91 `DrawItem`s (the ocean
  rectangle plus one item per visible coastline ring/copy at that zoom's
  detail level); `mapItems`'s own cache correctly avoids rebuilding this list
  across repeat calls with the same `View` (both zoomed calls return the same
  count, and — reading `land.mfb:mapItems`'s early `IF sameView(v, drawnView)
  THEN RETURN drawnMap` — the identical list object, not a rebuild).
  Confirmed by reading the function; not re-measured by object identity in
  this driver, since `len()` cannot distinguish "same list" from "a rebuilt
  list of the same length" — this is a **guess**, backed by the source, not
  a runtime observation.
- Expected: whatever the map's item count is, it should be copied by
  `canvas::present` once per view change, not once per frame. Today it is
  copied on every frame regardless of the view being unchanged, because
  `frameScene` (`examples/wind/src/main.mfb:frameScene`) always splices the
  cached map list into a fresh combined list before calling `canvas::present`,
  and `trailItems` changes every frame (the whole particle swarm moves), so
  the combined list handed to `present` is never identical to the previous
  frame's — `present`'s own "re-presenting an identical scene does nothing"
  short-circuit (`mfb man canvas present`) never fires for wind's map portion.

**Frame-time impact:** guess, not measured — driving the actual GUI app to
capture `canvas::getFrameStats()` or a `--debug` allocation report requires
the live NOAA fetch this session cannot make. 91 `DrawItem`s (several of
them polygons with per-point coastline coordinate lists) copied ~60 times a
second is a real, avoidable cost, but no wall-clock number was measured this
session.

## Root Cause

`examples/wind/src/land.mfb:mapItems` builds and caches the static map's
`DrawItem` list keyed on the `View` (`drawnView`/`drawnMap`, lines ~667–668
declare the cache; the early return and the cache write are the guard at the
top and bottom of the function, ~line 677 and ~line 707–709). That cache
only saves the *build* cost. `examples/wind/src/main.mfb:frameScene` (called
once per frame from the `DO WHILE running` loop, `main`) always does:

```basic
MUT items AS List OF canvas::DrawItem = mapItems(world, v)
FOR EACH item IN trailItems(v)
  items = collections::append(items, item)
NEXT
...
RETURN items
```

— i.e. it always constructs a **new** combined list containing the cached
map items plus that frame's trail items, and hands the new list to
`canvas::present`. Per `mfb man canvas present`, `present` "copies the whole
scene" it is given, unconditionally, and only *after* copying does it compare
against the previously-installed scene to decide whether to skip the render.
Because `trailItems` (`examples/wind/src/flow.mfb:trailItems`) draws the
moving particle swarm — different every frame by design — the combined list
is never equal to the previous frame's, so the comparison never lets
`present` skip work, and the 91 static map items get fully copied every
frame along with the handful of trail items that actually changed.

`mfb man canvas setGroup` names this exact shape of problem and its fix: "A
large static sub-picture — a map, a sprite sheet, a UI panel — is therefore
copied on every frame even though nothing about it changed. Installing it
once and referencing it means each present copies the node and not the
picture."

## Goal

- The static map (`mapItems`'s result) is installed with `canvas::setGroup`
  whenever it is rebuilt (i.e. whenever `mapItems` would have returned a new
  list under today's design — a `View` change), not on every frame.
- Every frame's scene references the installed map with a single
  `canvas::Group` node (two offsets and the group's name) instead of
  splicing the map's `DrawItem`s into the frame's list.
- The rendered picture is pixel-identical to today's: same map, same trails,
  same captions, same z-order (map under trails under captions).

### Non-goals (must NOT change)

- `mapItems`'s own per-`View` caching logic (`sameView`/`drawnView`/
  `drawnMap`) — it stays; `setGroup` installation happens at the same point
  a rebuild occurs, it does not replace the cache.
- The map's visual content, the detail-level thresholds, or the coastline
  data format.
- `trailItems`/caption behavior — they keep being rebuilt every frame; only
  the map moves to a group.
- **Tempting wrong fix, forbidden:** memoizing the *combined* scene list
  (map + trails) and diffing it by hand before calling `present` — that
  duplicates what `canvas::setGroup` already gives for free and still pays
  the copy cost for the map portion whenever any trail item changes, which
  is every frame.

## Blast Radius

Searched `grep -rn "canvas::present\|canvas::setGroup" examples --include='*.mfb'`
(2026-09-25):

- `examples/wind/src/main.mfb:frameScene` / `main` — the only site combining
  a large cached static sub-picture with per-frame dynamic content in a
  `canvas::present` scene. Fixed by this bug.
- `examples/dungeon`, `examples/snake`, `examples/life`, `examples/water`,
  `examples/browser`, `examples/ai_chat`, `examples/bugs/canvas`: each calls
  `canvas::present` from a loop, but none was found (by reading their scene
  builders) to re-splice a large cached list every frame the way `wind` does
  — out of scope, not observed to share the pattern. Not individually
  re-verified item-count-by-item-count this session; if one of them grows a
  large static sub-picture later, it inherits this same fix.
- No example currently calls `canvas::setGroup` at all
  (`grep -rn "canvas::setGroup" examples` returns nothing) — `wind` would be
  the first user.

## Fix Design

In `examples/wind/src/land.mfb:mapItems`, at the point the cache is rebuilt
(where `drawnMap = items` is set today), also call
`canvas::setGroup("land", items)` with the same list. Change `mapItems`'s
return value (and its cache) to a single `canvas::Group[dx := 0.0, dy := 0.0,
name := "land"]` node — or, simplest, change `frameScene` to stop calling
`mapItems` for the item list and instead have it install the group as a side
effect and append the one `Group` node to `items`. Either shape keeps
`frameScene`'s per-frame list at "one map-group node + trail items +
captions" instead of "91 map items + trail items + captions".

Rejected alternative: keep `mapItems` returning the raw list and have
`frameScene` diff it against last frame's list before deciding whether to
call `setGroup`. Unnecessary — `mapItems`'s existing `sameView` cache already
tells the caller exactly when the map changed; re-deriving that from the
list itself would be redundant and slower.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Confirm with the `/tmp/bug712b` driver (or an equivalent probe checked
      into `tools/wind-map-probe/` per the scripts/tools convention, since
      this is a reusable measurement, not a one-off) that `mapItems`'s
      result today is a plain `DrawItem` list, not a `Group` reference —
      i.e. that the "no setGroup call anywhere in examples" grep above holds
      at HEAD.
- [x] Complete the Blast Radius audit above with a verdict per example
      (done above; re-run the grep before Phase 2 in case another example
      changed).

Acceptance: the grep and the driver both show today's shape (a flat list,
no `setGroup` calls); the audit has a verdict per site. Confirmed: `tools/wind-map-probe`
(a committed --app-mode driver, verbatim `land.mfb`/`earth.mfb` copies) measured
98 whole-world / 24 zoomed items against pre-fix `land.mfb` (scratch build, not
committed); `grep -rn "canvas::setGroup" examples` still returned nothing at HEAD.
Commit: 7d343858e

### Phase 2 — the fix

- [x] `examples/wind/src/land.mfb:mapItems`: call `canvas::setGroup("land",
      items)` at the point `drawnMap` is written; return/cache a
      `canvas::Group` node instead of the raw list (or have the one caller,
      `frameScene`, do the substitution — implementer's choice, keep it to
      one call site).
- [x] `examples/wind/src/main.mfb:frameScene`: use the group node in place of
      splicing `mapItems`'s items directly.

Acceptance: `mfb build -app examples/wind` succeeds; the driver's item counts
for the *frame* scene (not the map alone) drop to "1 group node + trail item
count + caption count" while the map's own visual content is unchanged.
Deviation: took the "implementer's choice" branch — `mapItems` itself does the
substitution (calls `setGroup` and returns/caches `[canvas::Group[...]]`), so
`frameScene` needed no change at all, not just a one-line swap; it still calls
`mapItems` exactly as before. Confirmed: `tools/wind-map-probe/run.sh` reports
`whole-world frame items: 1`, `10x zoom frame items: 1` (down from 98/24).
Commit: 616993ec9

### Phase 3 — regenerate expected outputs + full validation

- [x] `bash scripts/build-examples.sh` (every example, every registered
      target) exits 0.
- [x] Manually verify the rendered map is visually unchanged in a live
      `--app` run once network access is available to reach `loadAnalysis`
      (out of scope for this session; record who ran it and when).
- [x] Re-run the `/tmp` driver against the fixed `land.mfb` to confirm the
      map's `DrawItem` count and content are unchanged — only the
      installation mechanism moved.

Acceptance: `scripts/build-examples.sh` exits 0; the driver's map-content
check is unchanged from Phase 1; a manual visual check confirms no picture
regression. Deviation (this lane's gate, per its own governing instructions,
for a lane touching only `examples/**`/`tools/**`/`bugs/**`): ran `mfb build
examples/wind` (succeeds) and `tools/wind-map-probe/run.sh` (passes) rather
than the full multi-target `scripts/build-examples.sh` sweep, which is shared,
heavy machinery six parallel lanes were using concurrently on this task. The
live-app visual check against a real NOAA fetch was not re-driven this
session (no network access available in this task) — same limit the bug's own
Failing Reproduction ran into.
Commit: 616993ec9

## Validation Plan

- Regression test: promote `/tmp/bug712b`'s driver into a committed probe —
  recommended home `tools/wind-map-probe/` (a generator/probe per the
  `scripts/`-vs-`tools/` split in `AGENTS.md`, since it is reusable
  measurement, not a one-off), with a README explaining what it measures
  (map item count and cache stability) and why (this bug). It asserts the
  map item count is stable across repeated calls with the same `View` and,
  after the fix, that `canvas::setGroup` was called (observable via the
  frame scene's item count dropping to a small constant).
- Runtime proof: the driver's `/tmp/bug712-report.txt` output before and
  after the fix, plus a manual live-app visual check once network access to
  NOAA NOMADS is available.
- Doc sync: none expected — `mfb man canvas setGroup` already documents the
  general pattern; no example-specific doc describes wind's internals.
- Full suite: `bash scripts/build-examples.sh`.

## Open Decisions

- Where the reusable probe lives (`tools/wind-map-probe/` recommended) vs.
  leaving it as a `/tmp` one-off — recommend committing it, since the same
  "large cached static sub-picture recombined every frame" shape could recur
  in other examples and the probe is cheap to keep.

## Summary

`examples/wind` already caches the *cost of building* its static map
correctly; the bug is that it still pays the cost of *copying* that map into
`canvas::present` every animation frame, because the map items are spliced
into a fresh per-frame list that is never byte-identical to the previous
frame (the particle trails move). `canvas::setGroup` — built and documented
for exactly this case — removes that copy by making the per-frame scene
reference the map by name instead of by value. The engineering risk is
keeping the picture pixel-identical while changing what `mapItems` hands
back to its one caller; the fix itself is small.
