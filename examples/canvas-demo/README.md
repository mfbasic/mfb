# canvas-demo — The Enchanted Grove

Ten scenes drawn with the MFBASIC `canvas` package: a title screen offers a
card for each, a short panel before each scene says what it shows (first on the
canvas, then in the language), and **Esc** returns to the title screen from
anywhere. Nine of the ten are interactive, and the tenth can be scrubbed
through its day. Everything is vector-drawn by the program except the
pixel-art sprites of scene VI and the SVG artwork of scene VIII, which is read
by an SVG reader written in MFBASIC and drawn as curves.

| | Scene | Shows off | You can |
|---|---|---|---|
| I | Dawn Awakening | time-of-day gradients, Multiply grading, an EvenOdd `CurvePolygon` archway installed as a group, songbirds placed by `Group` nodes, motes as particle systems | drag or scroll to scrub the time of day |
| II | The Guardian of the Spring | parallax from installed groups, a curved basin, ripples clipped to the pool, a `Burst` of sparks where the guardian lands | lead the guardian with the mouse; click to dive |
| III | Twilight Flight | a 150-bird boids flock, mountains and fountain mirrored by a flipped `Group` node, a two-arc crescent, `PolyCurve` limbs | scare the flock, hold to lure it, Space to scatter |
| IV | The Alchemist's Sanctuary | five `CurvePolygon` bottles on their own transforms, liquid clipped to each, a `Continuous` spark stream, bubbles, `AutoPaint` bottles | click a potion (or 1–5) to pour it |
| V | Echoes of the Canopy | hundreds of tumbling leaves, a recursive tree bending in the wind, the spray as a particle system | drag to blow, wheel for a breeze |
| VI | The Wanderer's Path | a level of `Tile`s from one sheet in one group moved by the camera and culled, a mirrored hero, `Picture` tint, a `Cover` portrait and a `Contain` map | walk and jump (arrows/A D, Space), N for night |
| VII | The Runestone Scribe | caps, joins, miter limits and dashes; the dash-reveal carving trick; an `AutoPaint` toolbox | carve your own runes in any stroke style |
| VIII | The Living Bestiary | real SVG files as `BezierCurve`s and `CurvePolygon`s, zoomed by each shape's own transform; a page turn by a `Group` node | turn pages, zoom and pan, W for the wireframe |
| IX | The Cathedral of Light | `CurvePolygon` panes glazed with Repeat and focal gradients, the light on the floor as a sheared `Group` node, panes clicked by id | move the sun, click a pane to change its glass |
| X | Lanternlight | a whole second world revealed through `Paint.clip`; a lantern swung by its node; spirits clickable only inside the light | carry the lantern, find and free the seven spirits |

A soundtrack plays throughout: a quiet ambience for each screen synthesised
sample by sample on a thread of its own (`audio.mfb`). With no audio device the
demo runs silently.

## Build and run

Canvas needs a window, so this is an app build (`"mode": "app"` in
`project.json`):

```sh
mfb build examples/canvas-demo
```

- macOS — `open examples/canvas-demo/build/canvas-demo.app`
- Linux — `./examples/canvas-demo/build/canvas-demo-glibc.AppImage`
  (or `-musl.AppImage`); build with `--target linux-x86_64` or
  `--target linux-aarch64`
- Windows — `mfb build --target windows-x86_64 examples/canvas-demo`, then copy
  `examples/canvas-demo/build/` (the exe and its `fonts/`, `sprites/` and
  `svg/` folders) and run `canvas-demo.exe`

## Controls

Everywhere:

- **Esc** — back to the title screen
- **I** — the current scene's panel again
- **F** — full screen on and off
- **M** — sound off and on
- **P** — drawing statistics: frames and presents per second, which renderer,
  how long a frame took, how many items the scene has
- **Q** — quit (from the title screen); **1**–**9** and **0** open a scene

Each scene's own controls are on its panel and along the bottom of the window.

Options, for presenting:

- `--scene N` — open scene N (1–10) straight away
- `--skip-intro` — with `--scene`, skip the scene's panel
- `--stats` — start with the drawing statistics showing

## How it is put together

`main.mfb` runs the frame loop: it gathers the mouse and keyboard into an
`Input` record, steps whichever screen is showing, presents what it draws, and
fades through black between screens. `demos.mfb` holds the ten scenes in one
`UNION Scene`, so stepping and drawing the current scene is a single
exhaustive `MATCH`. Each scene is one file with `<name>New`, `<name>Step` and
`<name>Draw`. `ui.mfb` has the panel, the buttons and text layout; `menu.mfb`
the title screen; `kit.mfb` and `scenery.mfb` the shapes, curves and particle
systems the scenes share; `svg.mfb` the SVG reader; `audio.mfb` the soundtrack.

The demo is written to the canvas's shape model throughout:

- **Shapes are built where they stand.** A shape's points, a gradient's ends
  and a clip's rings are all measured from its `position`, so moving a shape
  is changing its position, and its own `transform` turns and scales it about
  that point. `kit.mfb` has one small builder per shape, each returning one
  item as a full literal.
- **Figures are groups.** Anything drawn the same way frame after frame — the
  menu's card frame, the archway, the songbirds, the mountains, the window's
  glass, the lantern, the level — is installed once with `canvas::setGroup`
  and placed by a `canvas::Group` node, whose transform moves, turns, scales
  or mirrors it. One group is drawn ten times on the title screen, each node
  with its own id.
- **Curves are curves.** SVG paths, arches, bottles, crescents, petals and
  limbs are `canvas::BezierCurve`s in `CurvePolygon`s and `PolyCurve`s, arcs
  as exact rational quadratics. `canvas::flatten` is used only where points
  are what is needed: a liquid's clip, a mortar clip, a gradient's bounding
  box. Where a curve would be re-made every frame for no visible gain — a
  grass blade, a falling leaf, a writhing vine — the demo measured it and
  uses points; the comment there gives the numbers. Likewise a flock's birds
  and the falling leaves have their points turned by the program rather than
  carrying a transform of their own, because the canvas builds an
  untransformed shape on its fast path (`kit.mfb`'s `turned` has the
  measurement).
- **The canvas says what the mouse is on.** Whatever can be clicked carries an
  `id`, and `canvas::MouseEvent.overID` names it; nothing tests the mouse
  against a rectangle. A button's hover and press are a `canvas::AutoPaint`
  laid over it, which the window repaints by itself. Only the topmost item
  under the mouse is reported, so each scene draws its hotspots last, over its
  vignette.
- **Effects that are born, move and fade are particle systems.** Motes,
  sparks, spray and bubbles are `canvas::ParticleSystem`s advanced by their
  clock alone; a mote that brightens and then fades is two systems sharing a
  seed, the second running half a life behind the first. Effects whose
  particles react to something or cause something — falling leaves, the
  fountain's drops that ripple the pool, the guardian's trail, the steam — are
  simulated by hand, and the comment on each says why.
- **The level is tiles.** Scene VI draws every cell as a `canvas::Tile` of its
  sprite sheet, all in one group moved by the camera; the renderer skips the
  tiles outside the window. Images are cut from a sheet only where a tile
  cannot do the job: a tinted sprite and a portrait cut to an oval.

## Known issues

- **Windows: new windows open at 400 by 300** (bug-997), so the demo switches
  to full screen at startup when its window is that small.
- **Linux and Windows: the mouse is reported only to a program that also reads
  the keyboard** (bug-1015). The demo reads the keyboard every frame.

## Assets

See [CREDITS.md](CREDITS.md): the fonts (OFL), the pixel-art sprites
(Kenney, CC0) and the SVG creatures (Google Noto Emoji, Apache 2.0) are
third-party; the sigil SVG, every vector scene and the soundtrack were made for
this demo.

No dependencies beyond the built-in packages: `app`, `audio`, `canvas`,
`collections`, `color`, `errorCode`, `fs`, `io`, `math`, `os`, `strings`,
`thread` and `vector`.
