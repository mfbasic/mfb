# canvas — particle systems

Four small programs, one effect each, every one a single `canvas::ParticleSystem`
(three, for the fireworks). Each is about ninety lines and is meant to be copied.

| Example | The emitter | What to read it for |
|---|---|---|
| [`fire`](fire/src/main.mfb) | a `canvas::Line` | a continuous stream, a colour that fades to transparent, additive blending |
| [`fog`](fog/src/main.mfb) | a `canvas::Rectangle` | a few large, faint particles whose overlap is the effect; a gradient-filled particle, which a tint does not reach |
| [`fireworks`](fireworks/src/main.mfb) | a point (`canvas::Circle` of radius 0) | `Burst` mode, a repeating burst, several systems staggered in time |
| [`rain`](rain/src/main.mfb) | a strip above the window | a `canvas::Line` as the particle, turned to lean with the wind |

A particle system holds no state and nothing advances it. Each program's loop is the
same five lines: add the time since the last frame to a clock, build the scene with
that clock as the system's `time`, present it, wait for the frame. `mfb man canvas
types` documents every field.

## Build and run

Canvas needs a window, so these are `--app` builds:

```sh
mfb build examples/canvas/fire
```

- macOS — `open examples/canvas/fire/build/fire.app`
- Linux — `./examples/canvas/fire/build/fire.AppImage`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/canvas/fire`,
  then run `examples/canvas/fire/build/fire.exe`

Replace `fire` with `fog`, `fireworks` or `rain`. **Esc** or **Q** quits.
