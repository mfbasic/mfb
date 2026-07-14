# plan-30-B: iOS `.app` bundle + simulator install

Last updated: 2026-07-07
Effort: small (<1h)

This sub-plan makes `mfb build` emit an installable **iOS `.app` bundle** for the
simulator target from 30-A: the flat iOS bundle layout (no `Contents/` subtree), an
iOS-shaped `Info.plist`, and the code-signed executable at the bundle root — so
`xcrun simctl install` accepts it and `xcrun simctl launch` runs it under the
simulator's SpringBoard.

The single behavioral outcome: `mfb build` produces `<name>.app` that
`xcrun simctl install booted <name>.app` accepts without error and
`xcrun simctl launch booted <bundle-id>` launches to a running process.

It complements:

- `mfb spec linker macos-aarch64` (`src/docs/spec/linker/06_macos-aarch64.md` — the macOS bundle writer this parallels)

## 1. Goal

- Emit a flat iOS `.app` bundle: executable + `Info.plist` at the bundle **root**
  (not under `Contents/MacOS`), ad-hoc code-signed.
- The `Info.plist` carries the iOS-required keys (see §4.2) so the simulator will
  install and launch it.
- `xcrun simctl install booted <name>.app` succeeds; `xcrun simctl launch booted
  <bundle-id>` starts the process.

### Non-goals (explicit constraints)

- No change to the macOS `.app` writer or its output — the iOS bundle is a separate
  path selected by target.
- No app icon / asset catalog / launch storyboard beyond the minimal `UILaunchScreen`
  needed to run full-screen (icons are a later polish, cf. plan-22 for macOS).
- No real-device packaging (`.ipa`, provisioning, `embedded.mobileprovision`).
- No UIKit code yet — the bundled executable is still the console process from 30-A;
  30-C makes it a UIKit app.

## 2. Current State

- macOS bundle writer: `write_app_bundle` (`src/os/macos/link/mod.rs` ~36–60) creates
  `<name>.app/Contents/MacOS/`, writes the executable (chmod `0o755`) and
  `Contents/Info.plist`.
- macOS `Info.plist` template (`src/os/macos/link/mod.rs` ~150–168): `CFBundleName`,
  `CFBundleExecutable`, `CFBundleIdentifier = dev.mfbasic.<project>`,
  `CFBundlePackageType = APPL`, `NSPrincipalClass`.
- Bundle selection is driven by `NativeBuildMode::MacApp` (`src/target.rs:31`,
  `is_app()` at :52); the CLI `-app` flag sets it (`src/cli/build.rs:110`).

## 3. Design Overview

Add an **iOS bundle writer** parallel to `write_app_bundle`, selected when the target
is the 30-A iOS-simulator target and app mode is on. Two differences from macOS: a
**flat layout** (executable and `Info.plist` at the bundle root) and an **iOS
`Info.plist`** key set. Reuse the existing ad-hoc signing. Everything else — building
the inner Mach-O — is 30-A's job.

## 4. Detailed Design

### 4.1 Flat bundle layout

`<name>.app/` containing:
- `<name>` — the ad-hoc-signed Mach-O (chmod `0o755`), at the root.
- `Info.plist` — at the root.

No `Contents/`, no `MacOS/`, no `Resources/` (until icons/assets land).

### 4.2 iOS `Info.plist` keys

Minimum for `simctl install`/`launch`:

- `CFBundleName`, `CFBundleExecutable` = project name
- `CFBundleIdentifier` = `dev.mfbasic.<project>`
- `CFBundlePackageType` = `APPL`
- `CFBundleSupportedPlatforms` = `[iPhoneSimulator]`
- `DTPlatformName` = `iphonesimulator`
- `LSRequiresIPhoneOS` = `true`
- `MinimumOSVersion` = the 30-A `min_os_version()` value
- `UIDeviceFamily` = `[1]` (iPhone) — or `[1, 2]` for universal (Open Decision)
- `UILaunchScreen` = `{}` (empty dict → full-screen; without it the app letterboxes)

### 4.3 Signing

Reuse the existing ad-hoc `mfb_sign_segment` path (30-A Phase 2). The bundle
executable must be signed for the simulator to load it.

## Layout / ABI Impact

None at the object/Mach-O level beyond 30-A. This is purely the on-disk bundle
directory shape. No language, runtime-layout, or golden-output change.

## Phases

### Phase 1 — iOS bundle writer + Info.plist

- [ ] Add an iOS bundle writer (flat layout) parallel to `write_app_bundle` (`src/os/macos/link/mod.rs`), selected by the 30-A target + app mode.
- [ ] Add the iOS `Info.plist` template with the §4.2 keys.
- [ ] Ensure the bundle executable is ad-hoc signed (reuse 30-A signing).

Acceptance: `mfb build` yields `<name>.app` with a root-level signed executable and
a valid `Info.plist` (`plutil -lint Info.plist` passes; `codesign -v` on the inner
binary passes).
Commit: —

### Phase 2 — Install & launch proof

- [ ] Document and run the `simctl` install/launch sequence; confirm the process starts.

Acceptance: `xcrun simctl install booted <name>.app` succeeds and
`xcrun simctl launch booted dev.mfbasic.<project>` returns a running pid; the
command sequence is recorded.
Commit: —

## Validation Plan

- Function tests: none (no `mfb`-language surface).
- Runtime proof: Phase 2 — the bundle installs and launches on a booted simulator.
- Doc sync: extend the 30-A linker-spec simulator section with the iOS bundle layout
  and `Info.plist` keys.
- Acceptance: `scripts/test-accept.sh` unaffected.

## Open Decisions

- **`UIDeviceFamily`** — recommend `[1]` (iPhone only, simplest) vs. `[1, 2]`
  (universal). (§4.2)
- **Bundle identifier scheme** — recommend reusing macOS `dev.mfbasic.<project>`
  vs. a configurable id (defer configurability to plan-22-style manifest work). (§4.2)

## Non-Goals

- Icons, asset catalogs, launch storyboards beyond `UILaunchScreen = {}`.
- `.ipa` / device provisioning.

## Summary

A thin, low-risk sibling of the macOS bundle writer: flat layout + iOS `Info.plist`
keys + reuse of existing signing. All the hard object-file work is 30-A's; this is
directory shape and plist keys, proven by a `simctl install`/`launch` round-trip.
