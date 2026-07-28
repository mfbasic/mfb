# plan-51-A: Linux AppDir emission

Last updated: 2026-07-17
Overall Effort: x-large (1d–3d)
Effort: medium (1h–2h)
Depends on: nothing

Today a Linux `--app` build emits a bare `build/<name>.out` — an ELF with no icon,
no `.desktop`, no name, and no way for a desktop environment to launch it. The
`app_icon` and `app_version` the CLI already resolves and hands to the backend are
discarded on arrival (`src/target/linux_x86_64/mod.rs:216-218`). This sub-plan makes
the Linux app-mode output an **AppDir**: the standard on-disk tree that is both
directly runnable (`./build/<name>.AppDir/AppRun`) and the exact payload plan-51-C
seals into a single-file `.AppImage`.

The behavioral outcome: `mfb build --app -target linux-x86_64` produces
`build/<name>.AppDir/` containing a launchable `AppRun`, a valid `<name>.desktop`,
the project icon rendered to PNG at every hicolor size, the executable at
`usr/bin/<name>`, and any vendored native libraries at `usr/lib/` reachable through
a `$ORIGIN/../lib` RUNPATH.

References (read first):

- `src/os/macos/link/mod.rs:67-101` — `write_app_bundle`, the precedent this mirrors:
  encode the image once, then lay out a directory around it.
- `src/os/macos/icon.rs:55-128` — `build_icns`/`normalize_source`/`apply_squircle_mask`;
  the decode + validate + resize pipeline whose front half is reused here.
- `src/os/linux/link/mod.rs:112-134` — the `if app_mode` branch this replaces.
- `src/os/mod.rs:12-35` — `BUILD_DIR`, `VENDOR_DIR`, and the RPATH constant block.
- `src/cli/build.rs:1475-1506` — `vendor_output_dirs` and its RPATH↔directory table.
- `src/target/linux_gtk/mod.rs:187-188` — `STR_APP_ID` / `STR_TITLE`, hardcoded today.
- `planning/old-plans/plan-05-linux-app.md:118-146` — §4.1/§4.2/§4.3, which scoped
  exactly this out and named the (now obsolete) reason.
- https://docs.appimage.org/reference/appdir.html — the AppDir reference.

## 1. Goal

- `mfb build --app` for `linux-x86_64` and `linux-aarch64` emits
  `build/<name>.AppDir/` with the layout in §4.1, replacing `build/<name>.out`.
- `./build/<name>.AppDir/AppRun` launches the GTK app on a Linux box with no FUSE,
  no AppImage runtime, and no environment variables set.
- The project `icon` (or the compiler's embedded default) is rendered to PNG at the
  hicolor sizes and referenced by a `.desktop` that `desktop-file-validate` accepts
  and `appimagetool` would accept unmodified.
- Vendored `libraries` (plan-46) land in `usr/lib/` and resolve via a
  `$ORIGIN/../lib` `DT_RUNPATH` — no `LD_LIBRARY_PATH`, no wrapper script.
- The GTK application id becomes `dev.mfbasic.<name>`, matching the macOS
  `CFBundleIdentifier` and the `.desktop` `StartupWMClass`.

### Non-goals (explicit constraints)

- **No `.AppImage` file.** Sealing the AppDir into a single file is plan-51-C. This
  sub-plan's output is a directory, and it is complete and useful as one.
- **No GTK4 bundling.** `libgtk-4.so.1` and friends stay `DT_NEEDED` and resolve
  from the host. Bundling them needs a Linux sysroot to harvest ~40 transitive
  libraries from, which does not exist when cross-building from macOS. The AppDir
  requires GTK4 on the host (Ubuntu 22.04+, Debian 12+, Fedora 36+). Only
  author-declared `vendor` locators are bundled.
- **No console-path change.** `build/<name>-glibc.out` and `build/<name>-musl.out`
  keep their names, contents, and `$ORIGIN/vendor` RUNPATH exactly.
- **No macOS change.** `write_app_bundle`, the `.icns` pipeline, and every
  `macos-aarch64` golden stay byte-identical.
- **No riscv64 app mode.** `linux_riscv64/mod.rs:197` already returns
  `supports_app_mode = false` (bug-117.1); this plan does not change that, and
  §3.3 records why that is now permanent rather than pending.
- **The icon squircle stays macOS-only.** `apply_squircle_mask` encodes a Big Sur
  convention. Linux icons are unmasked.

## 2. Current State

### 2.1 Linux app mode emits a bare ELF

`src/os/linux/link/mod.rs:118-134` is the whole of it:

```rust
let out_dir = project_dir.join(BUILD_DIR);
fs::create_dir_all(&out_dir)?;
let path = if app_mode {
    out_dir.join(format!("{project_name}.out"))
} else {
    out_dir.join(format!("{project_name}-{}.out", flavor.suffix()))
};
fs::write(&path, bytes)?;
```

App mode is glibc-only (`src/target/linux_x86_64/mod.rs:304-335`: `let flavors = if
app_mode { &[LinuxFlavor::Glibc] } else { &LinuxFlavor::ALL }`), so `-app` collapses
the two flavored outputs into one unsuffixed `<name>.out`. That single-artifact
shape is why the AppDir can take the same slot without a second decision.

### 2.2 The icon and version already reach the backend and are dropped

`src/cli/build.rs:274-297` resolves the manifest `icon` against the project
directory **for every app build including Linux**, and emits
`PROJECT_JSON_ICON_MISSING` with a source span when it does not exist. The path then
flows through `target::write_executable` (`src/cli/build.rs:494-517`) into the Linux
backend, which discards it:

```rust
// src/target/linux_x86_64/mod.rs:216-218
let _ = app_icon;   // App icons are macOS-only (plan-22); the Linux/GTK backend ignores it
let _ = app_version; // Bundle version keys are macOS-only (bug-248); Linux has no bundle.
```

**A Linux `--app` build with a broken `icon` already fails today** and a working one
already produces nothing. The plumbing is complete; only the consumer is missing.

### 2.3 The icon pipeline is macOS-shaped but its front half is generic

`src/os/macos/icon.rs` splits cleanly:

- `normalize_source` (`:79-100`) — decode via `image`, hard-error unless exactly
  1024×1024, else fall back to the embedded `APP_ICON_PNG`. **Format-neutral.**
- `apply_squircle_mask` (`:107-128`) — Big Sur shaping. **macOS-only convention.**
- `ICON_ENTRIES` (`:37-48`) + `IconFamily` — ten RGBA entries into an `.icns`.
  **macOS-only container.**

The default PNG asset lives elsewhere again, at
`src/target/macos_aarch64/app/icon.rs`. Both `image` (PNG-only features) and `icns`
are already compiler-only dependencies (`Cargo.toml`), so rendering PNGs adds no new
crate.

### 2.4 Vendoring works on Linux, and plan-05's blocker is obsolete

`planning/old-plans/plan-05-linux-app.md:140-146` §4.3 scoped AppImage out because
self-contained bundles need rpath and vendoring, which the built-in linker did not
have. **plan-46 landed both**: `ELF_VENDOR_RPATH = "$ORIGIN/vendor"`
(`src/os/mod.rs:20`), emitted as `DT_RUNPATH` (tag 29, not `DT_RPATH`) at
`src/os/linux/link/elf.rs:789-795`, with `copy_vendor_libraries`
(`src/cli/build.rs:1521`) copying resolved blobs into `build/vendor/`. Open
Decision 1 at `plan-05-linux-app.md:863-864` — whether app mode should emit a
`.desktop` and icon — is what this sub-plan answers.

`vendor_output_dirs` (`src/cli/build.rs:1484-1506`) currently routes `LinuxApp`
through the `_ =>` catch-all to `build/vendor/`, sharing the console's directory.

### 2.5 The GTK app id is a codegen constant

`src/target/linux_gtk/mod.rs:187-188`:

```rust
const STR_APP_ID: (&str, &str) = ("_mfb_gtkapp_str_app_id", "dev.mfbasic.app");
const STR_TITLE: (&str, &str) = ("_mfb_gtkapp_str_title", "MFBASIC App");
```

Every MFBASIC GTK app on a machine therefore shares one D-Bus name and one window
class. macOS by contrast derives `dev.mfbasic.<name>`
(`src/os/macos/link/mod.rs:208-238`). A `.desktop` file cannot match its window
without a per-project id, so this is a prerequisite, not a nicety.

## 3. Design Overview

Three independent pieces, layered:

1. **An icon renderer** (§4.2) — hoist the format-neutral half of
   `src/os/macos/icon.rs` into `src/os/icon.rs` and add a PNG raster entry point
   beside the existing `.icns` one. Pure, no I/O, unit-testable.
2. **An AppDir writer** (§4.1, §4.3) — `src/os/linux/appdir.rs`, structurally a
   copy of `write_app_bundle`: encode the ELF once via the existing path, then lay
   a tree around it. Reuses `encode_dynamic_elf` verbatim.
3. **A per-project GTK identity** (§4.5) — parameterize `STR_APP_ID`/`STR_TITLE` off
   the module name. The only codegen change in this sub-plan.

The correctness risk concentrates in **§4.5's app-id sanitization**: an id that
fails `g_application_id_is_valid` makes `g_application_new` emit a `g_critical` and
the app dies before its first frame. Nothing at build time would catch it. §4.5
resolves this by sanitizing to a conservative character set rather than trusting
project names.

### 3.1 Why an AppDir rather than going straight to a sealed file

The AppDir is not scaffolding for plan-51-C — it is the artifact that makes the rest
testable. Sealed AppImages cannot be inspected without FUSE, cannot run under
qemu-user at all (plan-51-C §4.6), and turn every layout bug into a mount failure
with no diagnostic. An AppDir is a directory: `find` shows the tree, `readelf -d`
shows the RUNPATH, and `./AppRun` executes on any Linux box with zero privileges.
Landing it first means plan-51-C debugs *one* thing — the squashfs — against a
payload already proven correct.

It is also independently valuable: an AppDir is a legitimate distribution format,
and `appimagetool build/<name>.AppDir` turns it into an AppImage today for anyone
who has the tool.

### 3.2 Why `AppRun` is a symlink, not a wrapper script

The conventional AppRun is a `#!/bin/sh` wrapper exporting `LD_LIBRARY_PATH`. We do
not need one: the RUNPATH is baked into the ELF at link time (§4.4), so the loader
finds `usr/lib/` with no environment help. A symlink `AppRun -> usr/bin/<name>` is
enough — the runtime's `execv` follows it, and the AppDir gains no dependency on a
shell.

Rejected: **making `usr/bin/<name>` the real `AppRun` at the AppDir root.** It
removes the symlink but breaks the `$ORIGIN/../lib` relationship and puts a
project-named binary where every tool expects a fixed name.

Rejected: **a shell wrapper.** It would work, but it adds a `/bin/sh` dependency and
an `LD_LIBRARY_PATH` that leaks into any child process the app spawns — a real
source of "wrong library loaded" bugs in the wild. The RUNPATH is strictly better
and we already emit one.

### 3.3 Why riscv64 stays unsupported

`linux_riscv64/mod.rs:197` returns `supports_app_mode = false` (bug-117.1, the GTK
entry never ported). That was a temporary gap. It is now permanent for AppImage
purposes regardless: **AppImage/type2-runtime publishes no riscv64 runtime** —
`build-runtime.sh` `exit 2`s on anything outside `x86_64`/`aarch64`/`armhf`/`i686`.
Even a ported GTK entry could not be sealed. An rv64 AppDir would be emittable, but
shipping a mode that works on two of three Linux targets and silently produces a
different artifact shape on the third is worse than the current honest rejection.

### 3.4 Rejected: keeping `build/<name>.out` alongside the AppDir

Backward compatible and zero golden churn, but it means one `--app` build emits two
executables that differ only in RUNPATH, and the `.out` is the one nothing points
at. macOS `--app` emits only `build/<name>.app`; Linux should mirror that. The
`.out` name stays free for the console path, where it still means something.

## 4. Detailed Design

### 4.1 The AppDir layout

```text
build/<name>.AppDir/
  AppRun                    -> usr/bin/<name>          (symlink)
  <name>.desktop                                        (0644)
  <name>.png                                            (256×256, 0644)
  .DirIcon                  -> <name>.png              (symlink)
  usr/
    bin/<name>                                          (the ELF, 0755)
    lib/                                                (vendored libs, plan-46; created only when non-empty)
    share/
      applications/<name>.desktop                       (byte-identical copy)
      icons/hicolor/<N>x<N>/apps/<name>.png             (N ∈ 16,32,48,64,128,256,512)
      <name>/<dst>/…                                    (project resources, plan-55; created only when declared)
```

What each file is for, and who requires it:

| path | AppImage runtime | appimagetool | desktop integration |
| --- | --- | --- | --- |
| `AppRun` | **required** (`execv`s it) | ignored | — |
| `<name>.desktop` | ignored | **required** | **required** |
| `<name>.png` (root) | ignored | **required**, must be extension-less in `Icon=` | — |
| `.DirIcon` | ignored | auto-created if absent | thumbnailer |
| `usr/share/icons/hicolor/…` | ignored | never checked | **required** by `appimaged` |
| `usr/share/applications/…` | ignored | ignored | conventional |
| `usr/share/<name>/…` | ignored | ignored | — (read by `os::resourcePath`, plan-55) |

The runtime requires exactly one thing: an executable `/AppRun`. Everything else is
convention — but it is convention that costs nothing and is the difference between
a file that runs and an application the desktop knows about. We satisfy all of it.

The root `<name>.png` and `.DirIcon` are duplicated from
`usr/share/icons/hicolor/256x256/apps/<name>.png` because appimagetool looks only at
the AppDir root and never at `usr/share/icons` (verified: zero references in
`appimagetool.c`).

**`usr/share/<name>/` is the project-resource root (plan-55).** Files declared in the
manifest `resources` section are copied there by `copy_resources`
(plan-55-A §4.3), *not* by this writer — the copy runs after `write_appdir`, so
`write_appdir` must create `usr/share/` and leave it, never wipe it. `os::resourcePath`
(plan-55-B) resolves against exactly this directory: the executable is at
`usr/bin/<name>`, so its base is `../share/<name>` relative to `usr/bin`, i.e.
`strip "bin/<name>", append "share/<name>"`. This holds for both a directly-run AppDir
(`/proc/self/exe` → the real `usr/bin/<name>` behind the `AppRun` symlink) and a
FUSE-mounted `.AppImage` (`/proc/self/exe` → `<mountpoint>/usr/bin/<name>`), so no
`$APPDIR` env var is needed and none is set.

### 4.2 The icon renderer

Hoist `src/os/macos/icon.rs` into a shared `src/os/icon.rs`, and move the embedded
default PNG out of `src/target/macos_aarch64/app/icon.rs` beside it. The split:

| item | lands in | note |
| --- | --- | --- |
| `APP_ICON_PNG` | `src/os/icon.rs` | the embedded 1024×1024 default |
| `normalize_source` | `src/os/icon.rs` | decode + 1024×1024 check + default fallback |
| `render_png(source, size)` | `src/os/icon.rs` | **new**: normalize → Lanczos3 resize → PNG bytes |
| `apply_squircle_mask` | `src/os/macos/icon.rs` | stays; macOS convention |
| `ICON_ENTRIES`, `build_icns` | `src/os/macos/icon.rs` | stays; calls the hoisted `normalize_source` |

`render_png` is the whole Linux surface:

```rust
/// Render the app icon at `size`×`size` as PNG bytes (plan-51-A §4.2).
///
/// Shares `normalize_source` with the macOS `.icns` path, so a project `icon`
/// that is accepted on one platform is accepted on both and an icon rejected on
/// one is rejected on both. The macOS squircle mask is deliberately not applied:
/// it encodes a Big Sur shaping convention, and Linux icon themes shape icons
/// themselves.
pub(crate) fn render_png(source: Option<&Path>, size: u32) -> Result<Vec<u8>, String>
```

The 1024×1024 requirement, the `icon '…' must be 1024×1024, got W×H` error, and the
embedded default all become shared behavior by construction rather than by two
copies agreeing.

`HICOLOR_SIZES: [u32; 7] = [16, 32, 48, 64, 128, 256, 512]`. All seven are
downsamples of the same 1024 source; 512 and below are all genuine size reductions,
so no upscaling ever occurs.

### 4.3 The `.desktop` file

```ini
[Desktop Entry]
Type=Application
Name=<name>
Exec=<name>
Icon=<name>
Categories=Utility;
StartupWMClass=dev.mfbasic.<sanitized-name>
X-AppImage-Version=<manifest version>
```

Generated by `desktop_entry(project_name, app_id, app_version)` in
`src/os/linux/appdir.rs`, structurally the sibling of `app_info_plist`
(`src/os/macos/link/mod.rs:208-238`) — a hand-built format string with an escape
helper, no third-party writer.

Key-by-key, and why:

- **`Type` / `Name`** — the only two keys freedesktop marks `required=TRUE`.
- **`Exec=<name>`** — the bare binary name. Desktop-integration tools rewrite this
  to the absolute AppImage path when they install the entry; the value here only has
  to be present and non-empty.
- **`Icon=<name>`** — **extension-less, mandatory.** appimagetool appends the
  extension (`g_strdup_printf("%s/%s.png", source, icon_name)`), so `Icon=<name>.png`
  makes it search for `<name>.png.png` and fail.
- **`Categories=Utility;`** — freedesktop does *not* require this, but appimagetool
  hard-`die()`s without it. One line to stay tool-compatible. See Open Decisions.
- **`StartupWMClass`** — must equal the GTK application id (§4.5) or the desktop
  cannot associate the window with the launcher, and the app shows a generic icon in
  the dock.
- **`X-AppImage-Version`** — the only `X-AppImage-*` key appimagetool itself writes.
  Sourced from the manifest `version` that `bug-248` already threads to the backend
  for `CFBundleShortVersionString`. This is the second consumer of `app_version`,
  and the reason the parameter stops being `let _ =`.
- **No `Terminal=` key.** `Terminal=true` *disables* desktop integration in
  libappimage. Omitting it defaults to false, which is what a GUI app wants.

Escaping: the freedesktop spec reserves `\` in values and requires `;` in list keys
to be escaped as `\;`. `desktop_escape` handles both. Project names reaching here
have already passed manifest validation, but the escape is not conditional on that.

### 4.4 RUNPATH and the vendor directory

The binary is at `usr/bin/<name>`; its libraries at `usr/lib/`. So:

```rust
// src/os/mod.rs, beside the existing constants
/// ELF `DT_RUNPATH` for a vendored **AppDir** build (plan-51-A §4.4): the
/// executable sits at `usr/bin/<name>` and its libraries at `usr/lib/`, the
/// layout every AppDir-consuming tool expects. `$ORIGIN` is expanded by the
/// loader, not the build — take care that no format string interpolates it.
pub(crate) const ELF_APPDIR_VENDOR_RPATH: &str = "$ORIGIN/../lib";
```

Two call sites change in lockstep — this is the pair `vendor_output_dirs`' doc
comment warns about ("*Must stay in lockstep with the RPATH each backend emits: the
loader looks exactly here and nowhere else*"):

1. **RPATH selection**, `src/target/linux_x86_64/mod.rs:324-326` and the
   `linux_aarch64` equivalent: pick `ELF_APPDIR_VENDOR_RPATH` when
   `build_mode == NativeBuildMode::LinuxApp`, else `ELF_VENDOR_RPATH`.
2. **Vendor destination**, `src/cli/build.rs:1484-1506`: `LinuxApp` gains an explicit
   arm rather than falling through `_ =>`:

```rust
target::NativeBuildMode::LinuxApp => vec![build_dir
    .join(format!("{project_name}.AppDir"))
    .join("usr")
    .join("lib")],
```

The doc table becomes:

| build | rpath | vendor files |
| --- | --- | --- |
| linux console | `$ORIGIN/vendor` | `build/vendor/` |
| linux `--app` | `$ORIGIN/../lib` | `build/<name>.AppDir/usr/lib/` |
| macos console | `@loader_path/vendor` | `build/vendor/` |
| macos `--app` | `@executable_path/../Frameworks` | `build/<name>.app/Contents/Frameworks/` |

This makes Linux exactly mirror macOS: the console and app shapes of a **vendoring**
build differ by precisely one RUNPATH string, and a non-vendoring build emits no
`DT_RUNPATH` at all so the two remain byte-identical. Identical bytes for a
vendoring build would mean one of them is wrong.

⚠️ **The `.dynstr` hazard.** plan-46-D §1 records that `DT_RUNPATH` grows `.dynstr`,
and *two* independent computations derive from its length — the emitter and
`dynamic_prefix_size` (`src/os/linux/link/elf.rs:899`), which fixes the GOT offset
baked into every import stub. Updating one and not the other made every imported
call jump through the wrong GOT slot and segfault, while unit tests passed and
`readelf -d` printed the runpath correctly. `$ORIGIN/../lib` (15 chars) and
`$ORIGIN/vendor` (14 chars) are **different lengths**, so any code path that assumed
a fixed runpath length is now wrong. `runpath_string` (`elf.rs:849-855`) is the
single owner and both consumers already call it — verify that holds rather than
assuming it.

### 4.5 Per-project GTK identity

`STR_APP_ID` and `STR_TITLE` (`src/target/linux_gtk/mod.rs:187-188`) become derived
values threaded from the module name through `app_mode_data_objects()`
(`:790-804`), which is where the read-only string data objects are emitted.

```rust
/// The GTK/GApplication id for `project_name` (plan-51-A §4.5), matching the
/// macOS `CFBundleIdentifier` (`src/os/macos/link/mod.rs:app_info_plist`).
///
/// The name is sanitized to `[A-Za-z0-9_]` with a `_` prefix ahead of a leading
/// digit. `g_application_new` does not tolerate an invalid id: it emits a
/// `g_critical` and the app dies before its first frame, with nothing at build
/// time to catch it. The accepted set here is deliberately narrower than
/// `g_application_id_is_valid` accepts — it is also valid under the stricter
/// `g_dbus_is_name`, so the id works as a bus name too, and a project named
/// `my-app` yields `dev.mfbasic.my_app` rather than a runtime abort.
fn gtk_app_id(project_name: &str) -> String
```

`STR_TITLE` becomes the project name (matching the `.desktop` `Name=`), replacing
the constant `"MFBASIC App"`. `StartupWMClass` in §4.3 uses `gtk_app_id`'s output —
GTK4 sets the window's WM_CLASS from the application id, so these must agree
exactly or the launcher association silently fails.

This is the sub-plan's only codegen change and it moves `ncode` bytes for
`linux-{x86_64,aarch64}` app builds. No `linux-*.app.*` goldens exist yet, so there
is no golden churn — but `tests/gtk_term_utf8_grid.rs` and `tests/linux_app_mode.rs`
assert on the code plan and both need a read.

### 4.6 The writer

`src/os/linux/appdir.rs`, called from `src/os/linux/link/mod.rs`'s `if app_mode`
branch, with `src/os/linux/mod.rs` gaining a `write_linked_appdir` sibling to
`src/os/macos/mod.rs:42`'s `write_linked_app_bundle`:

```rust
/// Write an app-mode AppDir (plan-51-A §4.1) into the project's `build/`
/// directory:
///
/// ```text
/// build/<name>.AppDir/
///   AppRun -> usr/bin/<name>
///   <name>.desktop
///   <name>.png
///   .DirIcon -> <name>.png
///   usr/bin/<name>
///   usr/share/{applications,icons/hicolor/<N>x<N>/apps}/…
/// ```
///
/// The inner ELF is byte-identical to the `<name>-glibc.out` the console path
/// produces from the same image — **unless the build vendors native libraries**
/// (§4.4), where the two carry different `DT_RUNPATH` strings because they load
/// from different places. `usr/lib/` is not created here: `copy_vendor_libraries`
/// creates it iff the build vendors something, so a non-vendoring AppDir has no
/// empty directory in it.
///
/// Returns the path to the `build/<name>.AppDir` directory.
pub(crate) fn write_appdir(
    project_dir: &Path,
    project_name: &str,
    bytes: &[u8],
    app_icon: Option<&Path>,
    app_version: &str,
) -> Result<PathBuf, String>
```

`app_version` stops being `let _ =` and becomes required for `LinuxApp`, mirroring
`macos_aarch64/mod.rs:320-322`'s treatment of `None` as an internal error — the CLI
always supplies it from the manifest, which validates `version` as a required
non-empty string (`src/manifest/mod.rs:77-81`).

Symlinks are written with `std::os::unix::fs::symlink`. The build host is macOS or
Linux, both of which have it; there is no cross-platform concern because the
compiler itself only builds on Unix.

## Compatibility / Format Impact

**Changes:**

- `mfb build --app` on `linux-x86_64`/`linux-aarch64` emits `build/<name>.AppDir/`
  instead of `build/<name>.out`. The `Wrote executable to …` line prints the AppDir
  path, exactly as macOS prints the `.app` directory today.
- A **vendoring** Linux app build's `DT_RUNPATH` changes from `$ORIGIN/vendor` to
  `$ORIGIN/../lib`, and its vendor files move to `build/<name>.AppDir/usr/lib/`.
- GTK app id: `dev.mfbasic.app` → `dev.mfbasic.<name>`. Window title: `MFBASIC App`
  → `<name>`. Both are observable at runtime and in `ncode`.
- A Linux `--app` build now renders the icon, so a `project.json` `icon` that is
  present but not 1024×1024 begins failing a Linux build that previously ignored it.
  This is a behavior change and is intended — the icon was always declared, just
  never honored.

**Unchanged:**

- Every console artifact: names, bytes, RUNPATH, both flavors.
- Every macOS artifact, including all `macos-aarch64` goldens.
- `mfb build --app` remains rejected for riscv64 and for non-executable projects,
  with the same diagnostics.
- The `.mfp` format, the NIR/nplan `"buildMode": "linux-app"` string, and the
  manifest schema.

## Phases

### Phase 1 — Hoist the icon pipeline

Pure refactor with no behavior change; lands alone and keeps macOS byte-identical.

- [ ] Create `src/os/icon.rs`; move `APP_ICON_PNG` there from
      `src/target/macos_aarch64/app/icon.rs` and `normalize_source` from
      `src/os/macos/icon.rs`.
- [ ] Add `render_png(source, size)` per §4.2 (Lanczos3 downsample → PNG bytes).
- [ ] Leave `apply_squircle_mask`, `ICON_ENTRIES`, and `build_icns` in
      `src/os/macos/icon.rs`, now calling the hoisted `normalize_source`.
- [ ] Tests: keep every existing case in `src/os/macos/icon.rs:135-199` passing
      unmoved; add `render_png` cases in `src/os/icon.rs` — each `HICOLOR_SIZES`
      entry decodes to exactly that size, a non-1024 source is rejected with the
      existing message, `None` yields the embedded default.

Acceptance: `cargo test` green, and a `.app` built before and after this phase has a
byte-identical `AppIcon.icns`.
Commit: —

### Phase 2 — Per-project GTK identity

Isolates the only codegen change so its artifact diff is reviewable on its own.

- [ ] Add `gtk_app_id(project_name)` per §4.5 to `src/target/linux_gtk/mod.rs`.
- [ ] Thread the module name into `app_mode_data_objects()` (`:790-804`); make
      `STR_APP_ID` → `dev.mfbasic.<sanitized>` and `STR_TITLE` → the project name.
- [ ] Tests: `gtk_app_id` unit cases — plain name, hyphens (`my-app` →
      `dev.mfbasic.my_app`), leading digit (`3d` → `dev.mfbasic._3d`), dots, unicode.
- [ ] Read `tests/gtk_term_utf8_grid.rs` and `tests/linux_app_mode.rs` for
      assertions on the changed data objects; update if they pin the constants.

Acceptance: a cross-built `linux-x86_64` app plan contains `dev.mfbasic.<name>` and
no occurrence of `dev.mfbasic.app`; `scripts/artifact-gate.sh` green.
Commit: —

### Phase 3 — RUNPATH and vendor routing

The `.dynstr` hazard lives here; lands before the writer so the AppDir is populated
correctly the first time it exists.

- [ ] Add `ELF_APPDIR_VENDOR_RPATH` to `src/os/mod.rs` per §4.4.
- [ ] Make RPATH selection mode-aware in `src/target/linux_x86_64/mod.rs:324-326`
      and the `linux_aarch64` equivalent.
- [ ] Add the `LinuxApp` arm to `vendor_output_dirs` (`src/cli/build.rs:1484-1506`)
      and update its doc table to the four-row form in §4.4.
- [ ] Verify `runpath_string` (`elf.rs:849-855`) is the sole owner and that both
      `DynamicPayload::build` (`:567`) and `dynamic_prefix_size` (`:899`) derive
      from it — the plan-46-D §1 failure mode is a wrong-length runpath, and these
      two strings differ in length.
- [ ] Tests: extend `src/cli/build.rs:2388-2403`'s `vendor_output_dirs` cases with
      `LinuxApp`; assert the emitted `DT_RUNPATH` string per mode.

Acceptance: a vendoring `linux-x86_64` `--app` build's ELF shows
`DT_RUNPATH $ORIGIN/../lib` under `readelf -d`, and a vendoring console build still
shows `$ORIGIN/vendor`; a non-vendoring app build shows no `DT_RUNPATH` and is
byte-identical to its console counterpart.
Commit: —

### Phase 4 — The AppDir writer

The user-visible change; lands last because it depends on all three above.

- [ ] Add `src/os/linux/appdir.rs` with `write_appdir` (§4.6), `desktop_entry`
      (§4.3), and `desktop_escape`.
- [ ] Add `write_linked_appdir` to `src/os/linux/mod.rs`, mirroring
      `src/os/macos/mod.rs:42-50`.
- [ ] Replace the `if app_mode` branch in `src/os/linux/link/mod.rs:118-134` with a
      call to it; hoist the ELF encode so console and AppDir share one `bytes`.
- [ ] Stop discarding `app_icon`/`app_version` in `src/target/linux_x86_64/mod.rs:216-218`
      and `linux_aarch64/mod.rs:207-209`; treat a `None` version for `LinuxApp` as an
      internal error, per `macos_aarch64/mod.rs:320-322`.
- [ ] Tests: an `src/os/linux/mod.rs` layout test mirroring
      `src/os/macos/mod.rs:144-153` — tempdir build asserts every §4.1 path exists,
      `AppRun` resolves to `usr/bin/<name>`, `.DirIcon` resolves to `<name>.png`,
      the ELF is 0755, and no `usr/lib/` exists for a non-vendoring build.
- [ ] Tests: update `tests/linux_app_mode.rs:97-125`, which pins `<name>.out`.
- [ ] Tests: a `desktop_entry` case asserting `Icon=` is extension-less,
      `Categories=` is present, `Terminal=` is absent, and `StartupWMClass` equals
      `gtk_app_id`'s output.

Acceptance: `mfb build --app -target linux-x86_64` on a fixture emits the full §4.1
tree; `desktop-file-validate build/<name>.AppDir/<name>.desktop` exits 0 on a Linux
box; `./build/<name>.AppDir/AppRun` opens a GTK window titled `<name>` on the
Debian 12 GTK box (port 2226) and the Ubuntu x86_64 GTK box (port 2228).
Commit: —

## Validation Plan

- **Tests:** unit — `render_png` sizes + rejection, `gtk_app_id` sanitization,
  `desktop_entry` keys, `vendor_output_dirs` per mode, `runpath_string` per mode.
  Integration — `tests/linux_app_mode.rs` (cross-builds and inspects without
  executing; the dev host is macOS) updated for the AppDir shape, plus a layout test
  mirroring `src/os/macos/mod.rs:144-153`.
- **Runtime proof:** `.ai/compiler.md` requires real hardware for codegen changes,
  and §4.5 is one. On the Debian 12 GTK box (`ssh -p 2226`) and the Ubuntu x86_64
  GTK box (`ssh -p 2228`): `./build/<name>.AppDir/AppRun` opens a window titled
  `<name>`; `readelf -d usr/bin/<name>` shows the expected `DT_RUNPATH`; a vendoring
  fixture loads its library from `usr/lib/` with no `LD_LIBRARY_PATH`;
  `desktop-file-validate` exits 0. The GTK boxes are the only ones that matter —
  app mode is glibc-only, so the Alpine/musl boxes are out of scope by construction.
- **Doc sync:** `.ai/specifications.md` forbids the spec contradicting the compiler.
  Owed: `src/docs/spec/architecture/08_artifacts.md:46-49` (artifact table — add the
  AppDir row; note the table does not list Linux `--app` output *today*, which is
  itself a gap this fixes), `src/docs/spec/app/02_linux-runtime.md` (the app id and
  title are no longer constants), `src/docs/spec/tooling/07_cli-reference.md:147-151`
  (`--app` output shape), `src/docs/spec/linker/07_linux-aarch64.md` +
  `08_linux-x86_64.md` (the app-mode RUNPATH), `src/docs/spec/tooling/01_project-manifest.md`
  (`icon` now applies to Linux).
- **Acceptance:** `scripts/test-accept.sh` (no new goldens expected — no
  `linux-*.app.*` golden files exist), `scripts/artifact-gate.sh` for Phase 2's
  codegen change, `cargo fmt` (remember the second pass in `repository/`, which is
  not a workspace member).

## Open Decisions

- **`Categories=Utility;` hardcoded vs. a manifest `categories` field** — recommend
  hardcoding `Utility;` now. It is required only by appimagetool, not by
  freedesktop, and no MFBASIC project has yet asked to control it. A `categories`
  string array in `project.json` is a clean follow-up if one does; the `.desktop`
  writer takes it as a parameter either way, so the field is additive. (§4.3)
- **Root icon at 256×256** — recommend 256. appimagetool copies the largest icon it
  finds and 256 is the desktop convention; 512 doubles the root PNG for no consumer.
  (§4.1)
- **Whether `--app` should warn when the host GTK4 is absent** — recommend no. The
  build is a cross-compile and the host's GTK has nothing to do with the target's.
  Noted because it will be asked. (§1 non-goals)

## Summary

The real engineering risk is in two places, neither of them the AppDir tree itself.
**First, §4.4's RUNPATH change**: plan-46-D §1 documents that a runpath edit already
silently corrupted every import stub's GOT offset once, passing unit tests and
`readelf` inspection before segfaulting on real hardware, and this change alters the
runpath's *length* — the exact axis that failed. **Second, §4.5's app id**: an id
`g_application_new` rejects kills the app before its first frame with no build-time
signal, which is why the sanitizer is conservative rather than permissive.

Everything else is layout work against a precedent that already exists:
`write_app_bundle` has done this shape for macOS since plan-04, and this sub-plan is
its Linux twin. The icon hoist is a pure refactor. The `.desktop` writer is a format
string with an escape helper.

Left untouched: the console path on every platform, the whole macOS backend, the
`.mfp` format, the manifest schema, the squashfs and AppImage work (plan-51-B/C),
and riscv64 — which §3.3 argues is now permanently out rather than pending, because
no upstream runtime exists to seal it with.
