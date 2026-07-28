# plan-22-A: project.json `mode` and `icon` fields

Last updated: 2026-07-12 (tree-drift refresh; design unchanged, no plan-22 work landed)
Overall Effort: large (3h–1d)   (whole plan-22 feature)
Effort: medium (1h–2h)

> **Tree-drift note (2026-07-12).** None of plan-22 has landed (no `mode`/`icon`
> fields, no `src/os/macos/icon.rs`, no `image`/`icns`/`tiny-skia` deps). The
> design is unchanged and still applies, but anchors below have shifted — verify
> before editing. Current anchors:
> - `src/manifest/mod.rs`: `validate_project_manifest` :24, `validate_optional_string`
>   :156, `validate_kind` :293, `project_kind` :338, `entry_point` :346,
>   `field_position` **:364** (plan says :366).
> - `src/cli/build.rs` grew ~100 lines (plan-23 signing + plan-29 Money):
>   manifest load **:149** (was :145); the `-app` flag sets `app_mode` **:114**;
>   app-mode gate `if options.app_mode` **:162**, `target_supports_app_mode`
>   **:167** (were :158/:163); `NativeBuildMode` selection **:177** (was :173);
>   the `target::write_executable` call **:346** (was :246).
> - `src/target.rs`: `enum NativeBuildMode` **:32** (was :31); public
>   `write_executable` **:179** (was :170), trait method :102. Its signature now
>   carries a `signing_metadata: Option<&[u8]>` param (plan-23) — add `app_icon`
>   alongside it; the count/order below assumed the pre-signing signature.
> - macOS backend: `NativeBuildMode::MacApp` match now at
>   `src/target/macos_aarch64/mod.rs:270` (was :228), calling
>   `os::macos::write_linked_app_bundle` (defined at `src/os/macos/mod.rs:37`),
>   which calls `write_app_bundle` (`src/os/macos/link/mod.rs:44`, unchanged).
>   `app_info_plist` :154, `plist_escape` :180 — unchanged.

Extend the `project.json` manifest with two new optional fields and make the
first one build-affecting:

- `"mode"` — selects the native build mode from the manifest, so a project can
  build as a macOS/Linux **app** with a plain `mfb build` and no `-app` flag.
- `"icon"` — a project-relative path to a 1024×1024 source image used to generate
  the macOS app icon. This sub-plan only **validates and resolves** the field and
  threads the resolved path to the backend; the icon is actually *consumed*
  (rendered to `.icns`) in plan-22-B. image must be 1024×1024 source and a PNG, else its a compiler error.

The single behavioral outcome: a project whose `project.json` contains
`"mode": "app"` produces the same app bundle that `mfb build -app` produces
today, and an `"icon"` path is validated at build time (exists, is a readable
image) and made available to the macOS backend.

It complements:

- `./mfb spec tooling project-manifest` (this plan adds two rows to the manifest
  schema and new `PROJECT_JSON_*` diagnostics; canonical source
  `src/docs/spec/tooling/01_project-manifest.md`)
- `./mfb spec app macos-runtime` (app-mode build; canonical source under
  `src/docs/spec/app/**`)
- `./mfb spec diagnostics rule-codes` (`src/docs/spec/diagnostics/01_rule-codes.md`
  + `src/rules/table.rs` — new diagnostic codes)

## 1. Goal

- `project.json` accepts an optional `"mode"` string field with values
  `"console"` (default) and `"app"`. `"app"` is exactly equivalent to passing
  `-app` on the command line.
- `project.json` accepts an optional `"icon"` string field: a path (relative to
  the project directory) to a source image. When present it is validated at build
  time and the resolved absolute path is threaded to the native backend for
  plan-22-B to consume. When absent the backend falls back to the built-in
  default icon (plan-22-B).
- The CLI `-app` flag and `"mode": "app"` compose: app mode is requested if
  **either** is set. `-app` on a project already `"mode": "app"` is not an error.

### Non-goals (explicit constraints)

- No language surface change: no new builtins, types, or syntax. This is a
  build-configuration/manifest change only, so no `func_<pkg>_<fn>` tests are
  required (precedent: app mode is an alternate native lowering, not a language
  feature — plan-04 Phase 7).
- No change to `kind` semantics: `kind` stays `executable`/`package`. `mode` is
  orthogonal — app mode still requires `kind: "executable"` (an app is an
  executable with an AppKit/GTK front-end).
- No change to the console build path, Mach-O layout, or the bytes of the inner
  executable. `mode` only selects the existing `NativeBuildMode`.
- No `.icns` generation, image decoding, or new crate dependency in this
  sub-plan — that is plan-22-B. `icon` is validated and resolved only.

## 2. Current State

- The manifest is validated in `src/manifest/mod.rs::validate_project_manifest`
  (`src/manifest/mod.rs:24`). Required fields `name`/`version`/`mfb`/`sources`/
  `kind`; optional strings `entry`/`author`/`url` via `validate_optional_string`
  (`src/manifest/mod.rs:156`). `kind` is validated by `validate_kind`
  (`src/manifest/mod.rs:293`). Accessors like `project_kind`
  (`src/manifest/mod.rs:338`) and `entry_point` (`src/manifest/mod.rs:346`) read
  fields back out of the returned `HashMap<String, JsonValue>`.
- Build mode is decided in `src/cli/build.rs::build_project`. `options.app_mode`
  (set from the `-app` flag in `src/cli/build.rs:107`) drives the
  executable-only / app-capable-target validation (`src/cli/build.rs:158`) and
  the `NativeBuildMode` selection (`src/cli/build.rs:173`):
  `Console` / `MacApp` / `LinuxApp` (`src/target.rs:31`).
- `build_mode` is passed to `target::write_executable`
  (`src/cli/build.rs:246`), which routes to the per-arch backend; the macOS
  backend `src/target/macos_aarch64/mod.rs:228` matches on it and calls
  `os::macos::write_linked_app_bundle` for `MacApp`.
- `PROJECT_JSON_*` diagnostic codes live in `src/rules/table.rs` and are
  documented in `src/docs/spec/diagnostics/01_rule-codes.md`; the manifest schema
  table lives in `src/docs/spec/tooling/01_project-manifest.md`.
- Precedent for an optional validated string: `validate_optional_string`
  already handles `entry`/`author`/`url`. Precedent for an enumerated string with
  a "continuing validation" soft-warn: `validate_kind` (`src/manifest/mod.rs:293`).

## 3. Design Overview

Two independent pieces:

1. **`mode` field** (build-affecting). Validate as an enumerated optional string
   in `src/manifest/mod.rs`, add an accessor `build_mode_is_app(manifest)`, and
   OR it into the app-mode decision in `src/cli/build.rs::build_project` so the
   existing executable-only / app-capable-target checks and `NativeBuildMode`
   selection run identically whether the request came from the flag or the
   manifest.

2. **`icon` field** (resolved, consumed later). Validate as an optional string,
   and — only when app mode is active on a macOS target — resolve it against the
   project directory and confirm the file exists and is readable, emitting a
   `PROJECT_JSON_*` diagnostic on failure. Thread the resolved
   `Option<PathBuf>` down the `write_executable` call chain to the macOS backend.
   In this sub-plan the backend receives the path but does nothing with it beyond
   accepting the new parameter (plan-22-B renders it).

The correctness risk is small and concentrated in the `build_project`
control-flow edit (making sure `-app` and `"mode": "app"` compose without
double-erroring and that the target/kind validation still gates both).

## 4. Detailed Design

### 4.1 `mode` validation and accessor (`src/manifest/mod.rs`)

- Add `validate_mode(manifest, project_path, contents)` mirroring
  `validate_kind` (`src/manifest/mod.rs:293`): if `mode` is absent → ok; if
  present and not a string → `PROJECT_JSON_FIELD_TYPE`; if a string but not
  `console`/`app` → new soft diagnostic `PROJECT_JSON_UNKNOWN_MODE`
  ("Expected `console` or `app`; continuing validation.") that does not fail the
  build (matches `validate_kind`'s unknown-kind behavior).
- Call it from `validate_project_manifest` alongside the other optional-field
  validators (`src/manifest/mod.rs:86`–`100`).
- Add accessor `pub(crate) fn build_mode_is_app(manifest: &HashMap<String,
  JsonValue>) -> bool` returning `manifest.get("mode") == Some("app")`.

### 4.2 App-mode decision (`src/cli/build.rs`)

- After loading the manifest (`src/cli/build.rs:145`), compute
  `let app_mode = options.app_mode || build_mode_is_app(&manifest);`
- Replace the two later reads of `options.app_mode`
  (`src/cli/build.rs:158`, `:173`) with this combined `app_mode`. The existing
  `kind != "executable"` and `!target_supports_app_mode` checks
  (`src/cli/build.rs:159`, `:163`) then gate both the flag and the manifest
  identically. Rationale for erroring (not silently falling back) when the target
  doesn't support app mode: consistency with the flag, and an
  app-mode project built for a non-app target is a genuine misconfiguration
  (Open Decision 1).

### 4.3 `icon` validation and resolution (`src/manifest/mod.rs`, `src/cli/build.rs`)

- Add `icon` to the optional-string validators in `validate_project_manifest`
  (reuse `validate_optional_string`, `src/manifest/mod.rs:156`).
- Add accessor `pub(crate) fn icon_path(manifest) -> Option<&str>`.
- In `build_project`, after the app-mode decision and only when `app_mode` is
  true, resolve the icon:
  - `let app_icon: Option<PathBuf> = icon_path(&manifest).map(|rel|
    options.location.join(rel));`
  - If `Some(path)` and `!path.exists()` (or not a file), emit a new
    `PROJECT_JSON_ICON_MISSING` diagnostic anchored at the `icon` field position
    (`field_position`, `src/manifest/mod.rs:366`) and return `Err(())`.
  - Deep image validation (decodes, dimensions) is deferred to plan-22-B where
    the image crate is available; here we only check existence/readability so a
    typo path fails fast without pulling in a decoder.

### 4.4 Threading the icon to the backend

- Extend `target::write_executable` (`src/target.rs:170`) with a new parameter
  `app_icon: Option<&Path>`, passed through to the per-arch backend
  `write_executable`. Only the macOS backend (`src/target/macos_aarch64/mod.rs:228`)
  reads it; other backends accept and ignore it.
- macOS backend forwards it to `os::macos::write_linked_app_bundle` →
  `write_app_bundle` (`src/os/macos/link/mod.rs:44`), which gains an
  `app_icon: Option<&Path>` parameter. In this sub-plan `write_app_bundle`
  accepts the parameter but the icon body is unused (a `let _ = app_icon;` with a
  `// consumed in plan-22-B` note) so the chain compiles and A lands independently.

## Layout / ABI Impact

None. The inner Mach-O bytes, `mfb spec memory`, and value/copy/transfer
semantics are unchanged. The only on-disk change is deferred to plan-22-B
(`Contents/Resources/AppIcon.icns` + a plist key). This sub-plan changes only
manifest validation and internal function signatures.

## Phases

### Phase 1 — `mode` field (app mode without `-app`)

Lowest-risk, separately valuable: builds an app from the manifest alone.

- [ ] Add `validate_mode` + `build_mode_is_app` accessor and wire the validator
      into `validate_project_manifest` (`src/manifest/mod.rs`).
- [ ] Add `PROJECT_JSON_UNKNOWN_MODE` to `src/rules/table.rs` and
      `src/docs/spec/diagnostics/01_rule-codes.md`.
- [ ] Combine `options.app_mode || build_mode_is_app(&manifest)` in
      `build_project` and use it for both the validation gate and the
      `NativeBuildMode` selection (`src/cli/build.rs`).
- [ ] Add the `mode` row to the manifest schema table in
      `src/docs/spec/tooling/01_project-manifest.md`.
- [ ] Tests: a fixture project with `"mode": "app"` and no `-app` flag whose
      build produces a `.app` bundle (assert bundle exists on macOS host);
      an invalid-manifest fixture with a non-string `mode` asserting
      `PROJECT_JSON_FIELD_TYPE`.

Acceptance: `mfb build` (no `-app`) on a `"mode": "app"` executable project
produces the same `<name>.app` bundle layout as `mfb build -app` on the same
project (diff the two bundles' `Contents/MacOS/<name>` bytes — identical);
`scripts/test-accept.sh` green.
Commit: —

### Phase 2 — `icon` field validation + backend threading

Resolves and forwards the icon path; no rendering yet.

- [ ] Add `icon` optional-string validation + `icon_path` accessor
      (`src/manifest/mod.rs`).
- [ ] Add `PROJECT_JSON_ICON_MISSING` to `src/rules/table.rs` +
      diagnostics spec, and the existence/readability check in `build_project`
      (`src/cli/build.rs`).
- [ ] Add the `app_icon: Option<&Path>` parameter to `target::write_executable`
      (`src/target.rs`), the per-arch backend `write_executable`s, and
      `os::macos::{write_linked_app_bundle, write_app_bundle}` (unused for now).
- [ ] Add the `icon` row to the manifest schema table
      (`src/docs/spec/tooling/01_project-manifest.md`).
- [ ] Tests: invalid-manifest fixture with `"icon": "does/not/exist.png"`
      asserting `PROJECT_JSON_ICON_MISSING`; a valid fixture with an `icon` path
      that exists builds successfully (icon still ignored at this phase).

Acceptance: a build with a bad `icon` path fails with `PROJECT_JSON_ICON_MISSING`
before lowering; a build with a good `icon` path succeeds and the path reaches
`write_app_bundle` (verify with a temporary debug assert or a unit test on the
threaded value); `scripts/test-accept.sh` green.
Commit: —

## Validation Plan

- Function tests: N/A — no package function added (build-config/manifest change).
  Coverage is via manifest-validation fixtures under `tests/` (valid + invalid),
  the standing requirement for manifest changes.
- Runtime proof: on a macOS host, `mfb build` (no flag) on a `"mode": "app"`
  project yields a launchable `.app` identical to the `-app` build.
- Doc sync: `src/docs/spec/tooling/01_project-manifest.md` (two new schema rows),
  `src/docs/spec/diagnostics/01_rule-codes.md` + `src/rules/table.rs`
  (`PROJECT_JSON_UNKNOWN_MODE`, `PROJECT_JSON_ICON_MISSING`).
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

1. `mode: "app"` on a target that doesn't support app mode — **error like the
   `-app` flag** (recommended, consistency + fail-fast) vs. silently fall back to
   console. (§4.2)
2. Field name: **`mode`** (recommended; the user asked for "a mode") vs. a boolean
   `"app": true`. `mode` leaves room for future modes (`console`/`app`/…). (§4.1)
3. `-app` flag vs. `mode` disagreement (flag says app, manifest says console):
   **flag OR manifest ⇒ app** (recommended — `-app` is additive, never
   subtractive; there is no `-no-app`). (§4.2)

## Non-Goals

- Linux/GTK app icon handling (icon is macOS-only for plan-22). A `"mode": "app"`
  Linux build ignores `icon`.
- `.icns` generation, image decoding, squircle masking — plan-22-B / plan-22-C.

## Summary

The engineering risk is entirely in one `build_project` control-flow edit
(composing flag + manifest without double-erroring). Everything else is
additive manifest validation mirroring existing `validate_kind`/
`validate_optional_string` precedent, plus a mechanical parameter added to the
`write_executable` chain that only macOS reads. The inner executable and all
value/layout semantics are untouched.
