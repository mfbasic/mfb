# canvas-demo — The Enchanted Grove

Ten scenes drawn with the MFBASIC `canvas` package: a title screen offers a
card for each, a short panel before each scene says what it shows (first on the
canvas, then in the language), and **Esc** returns to the title screen from
anywhere. Nine of the ten are interactive, and the tenth can be scrubbed
through its day. Everything is vector-drawn by the program except the
pixel-art sprites of scene VI and the SVG artwork of scene VIII, which is read
and drawn by an SVG reader written in MFBASIC.

| | Scene | Shows off | You can |
|---|---|---|---|
| I | Dawn Awakening | multi-stop gradients, Multiply colour grading, additive light shafts, EvenOdd contours, clipped mortar | drag or scroll to scrub the time of day |
| II | The Guardian of the Spring | parallax from installed groups, ripples clipped to the pool, fading PolyLines | lead the guardian with the mouse; click to dive |
| III | Twilight Flight | a 150-bird boids flock, mirrored reflections by transform, a clipped crescent, Repeat-spread portal | scare the flock, hold to lure it, Space to scatter |
| IV | The Alchemist's Sanctuary | liquid clipped to five bottle shapes, focal glass gradients with FillAndStroke, Screen-blended steam | click a potion (or 1–5) to pour it |
| V | Echoes of the Canopy | hundreds of tumbling leaves, a recursive tree bending in the wind, streamlines | drag to blow, wheel for a breeze |
| VI | The Wanderer's Path | sprite sheets cut with getBytes/createImage, a level installed as one group, Picture tint, night as Multiply | walk and jump (arrows/A D, Space), N for night |
| VII | The Runestone Scribe | caps, joins, miter limits and dashes; the dash-reveal carving trick | carve your own runes in any stroke style |
| VIII | The Living Bestiary | real SVG files (paths, fill rules, gradients, transforms) drawn through canvas; a page turn by transform | turn pages, zoom and pan, W for the wireframe |
| IX | The Cathedral of Light | glass from Repeat and focal gradients, the window's light projected onto the floor by transform | move the sun, click a pane to change its glass |
| X | Lanternlight | a whole second world revealed through `Paint.clip`; darkness from one radial gradient | carry the lantern, find and free the seven spirits |

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
the title screen; `kit.mfb` and `scenery.mfb` the shapes the scenes share;
`svg.mfb` the SVG reader; `audio.mfb` the soundtrack.

Shapes drawn many at a time (a flock, a fall of leaves, the panes of the rose
window) are placed by mapping their points rather than by `Paint.transform`:
an item's antialiasing margin is currently added in its own units, so a polygon
magnified many times by its transform costs the GPU several times what the
same polygon costs given in pixels (bug-991, found while building this demo).

## Assets

See [CREDITS.md](CREDITS.md): the fonts (OFL), the pixel-art sprites
(Kenney, CC0) and the SVG creatures (Google Noto Emoji, Apache 2.0) are
third-party; the sigil SVG, every vector scene and the soundtrack were made for
this demo.

No dependencies beyond the built-in packages: `app`, `audio`, `canvas`,
`collections`, `color`, `errorCode`, `fs`, `io`, `math`, `os`, `strings` and
`thread`.
