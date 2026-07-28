# plan-22-B: macOS `.icns` generation + embedded default icon

Last updated: 2026-07-12 (tree-drift refresh; design unchanged, no plan-22 work landed)
Effort: medium (1h–2h)

> **Tree-drift note (2026-07-12).** Not landed. Anchors still valid:
> `write_app_bundle` `src/os/macos/link/mod.rs:44`, `app_info_plist` :154,
> `plist_escape` :180. `Cargo.toml` still has only `sha2`/`tinyjson`/`unicode-*`
> (edition 2021) — `image`/`icns` still need adding. `src/os/macos/mod.rs`
> currently declares `mod link; mod object;` — add `mod icon;` there. Note: after
> plan-22-A, `write_app_bundle` must gain the `app_icon: Option<&Path>` param it
> does not yet have (its current signature is `(project_dir, project_name,
> image)`).

Generate a real macOS icon set (`Contents/Resources/AppIcon.icns`) for every
`mfb build -app` on a macOS target, wired into the bundle writer and referenced
from `Info.plist`. This sub-plan produces a **correct, complete `.icns`** at all
required resolutions from either the project's `icon` image (resolved in
plan-22-A) or a built-in 1024×1024 default embedded in the compiler. The source
image is used as-is (square, no masking) here; the squircle mask and margin
polish are plan-22-C.

The single behavioral outcome: after `mfb build -app` (or a `"mode": "app"`
build), the produced `<name>.app` contains a valid multi-resolution
`Contents/Resources/AppIcon.icns` and Finder/Dock display the app's icon.

Depends on: **plan-22-A** (the `icon` field is validated/resolved and the
`app_icon: Option<&Path>` parameter already threaded to `write_app_bundle`).

It complements:

- `./mfb spec app macos-runtime` (app bundle layout; canonical source
  `src/docs/spec/app/01_macos-runtime.md`)
- `./mfb spec tooling project-manifest` (the `icon` field, added in plan-22-A)

## 1. Goal

- `write_app_bundle` writes `Contents/Resources/AppIcon.icns` containing PNG
  icon entries at the standard macOS sizes: 16, 32, 128, 256, 512 each at @1x
  and @2x (icns types `icp4 icp5 ic07 ic11 ic12 ic13 ic14 ic08 ic09 ic10`).
- `Info.plist` gains `<key>CFBundleIconFile</key><string>AppIcon</string>`.
- Icon source resolution: use the project `icon` path (plan-22-A) when present;
  otherwise the compiler's embedded 1024×1024 default. A **provided** `icon`
  must be exactly 1024×1024 — any other size (including non-square) is a
  hard error (resolved Open Decision 4).
- The build fails with a clear error if a **provided** `icon` file is not a
  decodable image, or is not exactly 1024×1024 (the embedded default is always
  valid).

### Non-goals (explicit constraints)

- No language surface change; no `func_<pkg>_<fn>` tests (native-lowering /
  bundle-packaging change only).
- No change to the inner Mach-O bytes or any layout/ABI (`mfb spec memory`
  untouched). Only bundle *sidecar* files change.
- No squircle/rounded-rect masking, margin inset, or shadow — the icon is
  packaged as the plain (square) source scaled to each size. Masking is
  plan-22-C. (So this sub-plan is landable and useful on its own — a square icon
  is a valid icon.)
- No Linux/GTK icon. macOS only.

## 2. Current State

- `write_app_bundle` (`src/os/macos/link/mod.rs:44`) creates
  `<name>.app/Contents/MacOS/<name>` + `Contents/Info.plist` only. It now (after
  plan-22-A) receives `app_icon: Option<&Path>` but ignores it.
- `app_info_plist` (`src/os/macos/link/mod.rs:154`) emits the plist keys
  (`CFBundleName`/`CFBundleExecutable`/`CFBundleIdentifier`/`CFBundlePackageType`/
  `NSPrincipalClass`). `plist_escape` (`src/os/macos/link/mod.rs:180`) is
  available for any string value.
- No image/PNG/icns crate is present today (`Cargo.toml` has only
  `sha2`/`tinyjson`/`unicode-*`; edition 2021). The project otherwise hand-rolls
  binary formats (Mach-O linker), but the user has explicitly opted into image
  crates for this feature (Open Decision 1).
- Embedded-asset precedent: `include_str!` for package sources
  (`src/builtins/encoding.rs` etc.) and `include_str!("../third_party/…")` for
  the utf8proc table. Binary embedding via `include_bytes!` is the analogous
  pattern for the default icon.

## 3. Design Overview

Four pieces, layered:

1. **Dependencies** — add `image` (decode/resize/encode PNG) and `icns`
   (`.icns` container) to `Cargo.toml`. These are **build-time only** (the `mfb`
   compiler); they add nothing to generated programs. `tiny-skia` is added in
   plan-22-C (masking), not here.
2. **Embedded default icon** — a committed `src/os/macos/assets/default_icon.png`
   (1024×1024 RGBA) pulled in with `include_bytes!`.
3. **Icon module** — new `src/os/macos/icon.rs` with a single entry point
   `build_icns(source: Option<&Path>) -> Result<Vec<u8>, String>` that decodes
   the source (or the embedded default), normalizes it to a 1024×1024 RGBA
   canvas, downsamples to each required size, and encodes the `.icns` bytes.
4. **Bundle wiring** — `write_app_bundle` creates `Contents/Resources/`, writes
   `AppIcon.icns`, and `app_info_plist` gains `CFBundleIconFile`.

Correctness risk concentrates in the RGBA/premultiplied-alpha handling across
crate boundaries and in emitting an `.icns` that macOS actually accepts (correct
OSTypes and PNG payloads at exact pixel sizes).

## 4. Detailed Design

### 4.1 Dependencies (`Cargo.toml`)

```toml
image = { version = "0.25", default-features = false, features = ["png"] }
icns  = "0.3"   # latest maintained (mdsteele); the "0.1" in the request is stale
```

- `default-features = false` + `features = ["png"]` keeps the transitive tree
  minimal (only PNG codec, which is all we need). Note in the commit message that
  these are compiler build-time deps only.
- Open Decision 1 records the alternative (hand-roll icns + reuse an existing PNG
  path) if the maintainers reject new crates.

### 4.2 Embedded default icon (`src/os/macos/assets/default_icon.png`)

- Commit a 1024×1024 PNG "standard" mfb icon under
  `src/os/macos/assets/default_icon.png`.
- In `icon.rs`: `const DEFAULT_ICON_PNG: &[u8] =
  include_bytes!("assets/default_icon.png");`
- The default is authored as the raw glyph/artwork; plan-22-C applies the
  squircle at render time, so the committed default need not be pre-masked
  (keeps one source of truth for the shape).

### 4.3 `build_icns` (`src/os/macos/icon.rs`)

```
pub(crate) fn build_icns(source: Option<&Path>) -> Result<Vec<u8>, String>
```

1. Decode: if `Some(path)`, `image::open(path)` → on error return
   `"icon '<path>' is not a decodable image: <err>"`. If `None`, decode
   `DEFAULT_ICON_PNG` (unwrap/expect — a corrupt embedded asset is a compiler
   bug).
2. Validate dimensions → RGBA canvas (`image::RgbaImage`):
   - A **provided** `icon` must be exactly 1024×1024. Otherwise return
     `"icon '<path>' must be 1024×1024, got WxH"` (hard error, resolved Open
     Decision 4). This check belongs here (the decoder is available) rather than
     in plan-22-A's existence check.
   - The embedded default is 1024×1024 by construction; use directly.
3. `let hook = ImageRenderHook::identity();` — a seam (a function taking the
   1024 RGBA and returning 1024 RGBA) that this sub-plan sets to identity and
   plan-22-C replaces with the squircle mask. Keeps 22-C a localized change.
4. For each target size, downsample the (masked-in-22-C) 1024 canvas with
   Lanczos3 and add to the `icns::IconFamily`:
   | size | icns `OSType` |
   | --- | --- |
   | 16   | `icp4` |
   | 32   | `icp5` |
   | 32   | `ic11` (16@2x) |
   | 64   | `ic12` (32@2x) |
   | 128  | `ic07` |
   | 256  | `ic08` |
   | 256  | `ic13` (128@2x) |
   | 512  | `ic09` |
   | 512  | `ic14` (256@2x) |
   | 1024 | `ic10` (512@2x) |

   Use `icns::Image::from_data(PixelFormat::RGBA, w, h, rgba_bytes)` then
   `IconFamily::add_icon_with_type(&image, IconType::…)`. The `icns` crate PNG-
   encodes the retina types itself.
5. `let mut buf = Vec::new(); family.write(&mut buf)?;` → return `buf`.
   Map any `icns`/io error to a `String`.

Determinism: the output depends only on the (fixed) input image + fixed resize
filter + the `icns` crate's encoder. For a given `image`/`icns` version the bytes
are reproducible, but the PNG payloads are **not** guaranteed byte-stable across
crate upgrades — tests assert structure (magic, entry OSTypes, per-entry decoded
dimensions), not a byte golden (§Validation).

### 4.4 Bundle wiring (`src/os/macos/link/mod.rs`)

- In `write_app_bundle` (`src/os/macos/link/mod.rs:44`), after writing the
  executable and before/after the plist:
  ```
  let resources_dir = contents_dir.join("Resources");
  fs::create_dir_all(&resources_dir)?;
  let icns = crate::os::macos::icon::build_icns(app_icon)?;
  fs::write(resources_dir.join("AppIcon.icns"), icns)?;
  ```
  (Wrap each fs op with the existing `format!("failed to …")` error style.)
- In `app_info_plist` (`src/os/macos/link/mod.rs:154`) add
  `  <key>CFBundleIconFile</key>\n  <string>AppIcon</string>\n` to the dict.
  (`CFBundleIconFile` may omit the `.icns` extension — macOS resolves it.)
- Register the new `mod icon;` in `src/os/macos/mod.rs`.

## Layout / ABI Impact

None to the executable or `mfb spec memory`. The bundle gains a
`Contents/Resources/AppIcon.icns` sidecar and one plist key. `mfb spec app`
macos-runtime bundle-layout diagram is updated to include `Resources/AppIcon.icns`.

## Phases

### Phase 1 — icns pipeline + embedded default (no bundle wiring)

Land the generator behind a unit test before touching the build path.

- [ ] Add `image` + `icns` deps to `Cargo.toml` (§4.1).
- [ ] Commit `src/os/macos/assets/default_icon.png` (1024×1024).
- [ ] Implement `src/os/macos/icon.rs::build_icns` with the identity render hook
      (§4.3) and register `mod icon;` in `src/os/macos/mod.rs`.
- [ ] Unit test (`src/os/macos/icon.rs` `#[cfg(test)]`): `build_icns(None)`
      returns bytes starting with `icns`, decodes via `icns::IconFamily::read`
      to the 10 expected `OSType`s, and each entry decodes to its exact expected
      pixel dimensions.
- [ ] Unit test: `build_icns(Some(<non-1024 png fixture>))` returns the
      "must be 1024×1024" error; `build_icns(Some(<non-image fixture>))` returns
      the "not a decodable image" error.

Acceptance: `cargo test` passes the icns structure test; `build_icns` produces a
valid `.icns` from the embedded default and from a provided PNG fixture.
Commit: —

### Phase 2 — bundle wiring + plist

- [ ] `write_app_bundle` creates `Contents/Resources/` and writes
      `AppIcon.icns` from `build_icns(app_icon)` (§4.4).
- [ ] `app_info_plist` emits `CFBundleIconFile`.
- [ ] Update `src/docs/spec/app/01_macos-runtime.md` bundle-layout to include
      `Resources/AppIcon.icns` and the plist key.
- [ ] Extend the app-mode regression fixture `tests/macos-app-mode-io` (or a new
      `tests/macos-app-icon` fixture) so its `-app` build is asserted to contain
      a valid `AppIcon.icns` and a `CFBundleIconFile` plist key. Prefer a
      script/structure assertion over a binary golden (the icns PNG payloads are
      not byte-stable across crate versions).
- [ ] Extend `scripts/test-macapp.sh` with a headless assertion: after a `-app`
      build, `Contents/Resources/AppIcon.icns` exists, begins with `icns`, and
      `Info.plist` contains `CFBundleIconFile`.

Acceptance: `mfb build -app` on a fixture produces `<name>.app/Contents/
Resources/AppIcon.icns` that `iconutil`/`sips`/the `icns` crate reads as a valid
multi-resolution family, and the app shows an icon in Finder/Dock on a macOS
host; `scripts/test-accept.sh` green.
Commit: —

## Validation Plan

- Function tests: N/A (no package function). Coverage via the icns structure
  unit test + the app-mode bundle fixture.
- Runtime proof: on a macOS host, build an `-app` fixture and confirm the icon
  renders in Finder/Dock and `sips -g pixelWidth Contents/Resources/AppIcon.icns`
  reports the expected sizes.
- Doc sync: `src/docs/spec/app/01_macos-runtime.md` (bundle layout + plist key).
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

1. **RESOLVED — use the crates.** The hand-rolled binary posture applies to the
   *generated program* (mfb's own Mach-O/ELF linker), not to the *compiler*,
   which already depends on `sha2`/`tinyjson`/`unicode-*`. `image`/`icns` are
   compiler build-time deps and add nothing to emitted programs. Pin
   `default-features = false` to bound the transitive tree. (§4.1)
2. `icns` crate version — **`0.3`** (recommended; the request's `0.1` is stale
   and lacks the retina PNG icon types) vs. `0.1`. (§4.1)
3. Golden strategy — **structure assertions** (magic + OSTypes + decoded sizes;
   recommended) vs. byte-golden the `.icns` (rejected: PNG payload bytes aren't
   stable across `image`/`icns` upgrades). (§4.3)
4. **RESOLVED — hard error.** A provided `icon` must be exactly 1024×1024; any
   other size (including non-square) fails the build. (§4.3 step 2)

## Non-Goals

- Squircle mask, margin inset, shadow — plan-22-C.
- Linux/GTK icon.
- Regenerating the icon on incremental builds only when the source changed
  (always regenerate; it is fast and the build is not incremental today).

## Summary

The risk is in producing an `.icns` macOS actually accepts (correct OSTypes,
exact per-entry pixel sizes, valid RGBA→PNG) and in keeping the crate footprint
bounded. The identity render hook in §4.3 is the single seam plan-22-C swaps for
the squircle, so this sub-plan ships a complete, valid (square) icon on its own
and the masking work is isolated. No executable bytes or language semantics
change.
