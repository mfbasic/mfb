# weather

NOAA's global 10 m wind field, drawn as streamlines on an Equal Earth map,
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
meaningful. The first run downloads about 0.9 MB (the current analysis,
decoded before the map appears) and then about 44 MB more in the background
for the five-day forecast; later runs of the same GFS cycle read cached files
instead of re-downloading. Only the land-and-lake map resource is bundled.

## Controls

- **Q / E** — zoom in / out
- **A / D** — spin the world west / east (wraps all the way round)
- **W / S** — pan up / down
- **Arrow keys** — same as W A S D
- **F** — toggle fullscreen
- **R** — reset to the whole world
- **H** — hide or show the captions
- **Space** — play or stop the forecast
- **Esc** — quit
- **Mouse** — click the round button (top right) to cycle wind → temperature
  → precipitation → wind; click the tracker's own button to play/stop the
  forecast; click or drag along the tracker to scrub to a moment of the
  forecast

No dependencies beyond `app`, `canvas`, `color`, `collections`, `errorCode`,
`math`, `strings`, `thread`, and the rest of the standard packages the
program imports.
