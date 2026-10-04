# weather

NOAA's global wind at 10 m above the ground, at 850 mb (about 1.5 km up) and
at 250 mb (about 10 km up, the jet streams), drawn as streamlines on an Equal
Earth map (each height in its own colours, the higher ones fainter and
underneath),
plus a five-day forecast that plays hour by hour once it has downloaded. A
button and a bottom tracker switch between the wind tracers, a temperature
colour wash, and a precipitation colour wash. See the header comment in
[`src/main.mfb`](src/main.mfb) for the full file layout (eleven source files).

## Build and run

Canvas needs a window, so this is a `--app` build:

```sh
mfb build examples/weather
```

- macOS — `open examples/weather/build/weather.app`
- Linux — `./examples/weather/build/weather.AppImage`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/weather`,
  then run `examples/weather/build/weather.exe`

## Network dependency

The program downloads live GFS analysis and forecast data from NOAA NOMADS
(`nomads.ncep.noaa.gov`) at startup and needs network access to show anything
meaningful. The first run downloads about 1.7 MB (the current analysis,
decoded before the map appears) and then about 75 MB more in the background
for the five-day forecast; later runs of the same GFS cycle read cached files
instead of re-downloading. Only the land-and-lake map resource is bundled.

## Controls

- **Q / E** — zoom in / out
- **A / D** — spin the world west / east (wraps all the way round)
- **W / S** — pan up / down
- **Shift+W / Shift+S** — tilt the world north / south, as far as putting
  a pole in the middle of the map
- **Arrow keys** — same as W A S D
- **F** — toggle fullscreen
- **R** — reset to the whole world
- **H** — hide or show the captions
- **Space** — play or stop the forecast
- **1 / 2 / 3** — turn the wind at the ground (10 m), at mid range
  (1.5 km) or in the jet stream (10 km) on or off; only the ground is on at
  start
- **Esc** — quit
- **Mouse** — click the round button (top right) to cycle wind → temperature
  → precipitation → wind; click the tracker's own button to play/stop the
  forecast; click or drag along the tracker to scrub to a moment of the
  forecast

No dependencies beyond `app`, `canvas`, `color`, `collections`, `errorCode`,
`math`, `strings`, `thread`, and the rest of the standard packages the
program imports.
