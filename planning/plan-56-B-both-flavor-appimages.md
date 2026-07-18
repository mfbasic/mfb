# plan-56-B: Both-flavor AppImage output

Last updated: 2026-07-18
Effort: medium (1h–2h)
Depends on: plan-56-A

A Linux console build emits one executable per libc world —
`build/<name>-glibc.out` and `build/<name>-musl.out` — but a `--app` build emits
only `build/<name>.AppImage`, glibc-only, because plan-05 declared GTK a
glibc-world dependency and plan-51 inherited that. Alpine ships `gtk4.0`, so the
restriction is not a fact about the world; it is a decision nobody revisited.

This sub-plan makes `--app` mirror the console path: two AppImages, one per
flavor, from one build.

The behavioral outcome: `mfb build --app -target linux-x86_64` produces
`build/<name>-glibc.AppImage` **and** `build/<name>-musl.AppImage`, each
independently runnable on its own libc world.

References (read first):

- `src/target/linux_x86_64/mod.rs:323-327`, `src/target/linux_aarch64/mod.rs:307-311`
  — the `if app_mode { &[Glibc] }` flavor gates.
- `src/cli/build.rs:1414-1439` — `emitted_link_targets`, the third glibc-only gate
  (vendor resolution).
- `src/cli/build.rs:1585-1605` — `vendor_output_dirs`, already returning a `Vec`.
- `src/cli/build.rs:1633-1650` — `resource_output_dir`, returning a single
  `PathBuf` — the one signature that must widen.
- `src/os/linux/appdir.rs:42-51` — `write_appdir`, which names the AppDir
  `<name>.AppDir` with no flavor component.
- `src/os/linux/appimage/mod.rs:143-152` — `seal`, which reads
  `<name>.AppDir` and writes `<name>.AppImage`.
- `src/target.rs:finalize_app_bundle` — the seam that returns **one** path.
- plan-51-A §4.4 / plan-51-C §3.2 — the RPATH↔directory table and the
  seal-after-vendoring ordering this sub-plan must preserve per flavor.

## 1. Goal

- `mfb build --app` on `linux-x86_64`/`linux-aarch64` emits
  `build/<name>-glibc.AppImage` and `build/<name>-musl.AppImage`, both mode 0755.
- `--app-debug` additionally leaves `build/<name>-glibc.AppDir` and
  `build/<name>-musl.AppDir`.
- A vendoring build puts **each flavor's own** libc-matching library inside that
  flavor's image, reachable via the existing `$ORIGIN/../lib` RUNPATH.
- Declared `resources` land inside **both** images.
- Two builds of one project produce byte-identical outputs, per flavor.

### Non-goals (explicit constraints)

- **No fat/universal AppImage.** Two files, each single-libc. The AppImage
  runtime blob is static musl and runs anywhere, but the *payload* is
  libc-specific and GTK4 is not bundled (plan-51-A §1). A musl AppImage needs a
  musl GTK4 host (Alpine's `gtk4.0`); a glibc one needs a glibc GTK4 host.
- **No riscv64.** No GTK entry ported (bug-117.1) and no upstream AppImage
  runtime (plan-51-A §3.3). `linux_riscv64` keeps rejecting app mode.
- **No console-path change.** `<name>-glibc.out` / `<name>-musl.out` keep their
  names, bytes, and `$ORIGIN/vendor` RUNPATH.
- **No macOS change.** `.app` stays one artifact with no flavor component;
  `finalize_app_bundle` still returns `None` there.
- **No AppDir layout change.** plan-51-A owns the tree; only the directory's
  *name* gains a flavor suffix.
- **No change to the seal.** `[runtime][squashfs]` at `runtime.len()`, unchanged
  (plan-51-C §4.1).

## 2. Current State

### 2.1 Three independent glibc-only gates

App mode's glibc-only rule is asserted in three places that must all flip
together, and each is currently a separate literal:

| site | today |
| --- | --- |
| `linux_x86_64/mod.rs:323` | `if app_mode { &[LinuxFlavor::Glibc] } else { &ALL }` |
| `linux_aarch64/mod.rs:307` | same |
| `cli/build.rs:1419` | `if build_mode.is_app() { &[Libc::Glibc] } else { &[Glibc, Musl] }` |

The third governs **vendor resolution** (`resolved_vendor_libraries`), so leaving
it behind would emit a musl AppImage whose vendored library is the glibc blob.

### 2.2 The artifact names have no flavor component

`src/os/linux/appdir.rs:49-51` builds `build/<name>.AppDir`, and
`appimage::seal` (`:145`) reads that exact path and writes `build/<name>.AppImage`.
Emitting two flavors today would make the second overwrite the first — verified
during the plan-56 feasibility spike, where a both-flavor build produced a single
`mu.AppDir` containing the musl binary (last flavor wins).

### 2.3 `resource_output_dir` returns one path

`src/cli/build.rs:1633` returns a single `PathBuf`, and `copy_resources`
(`:1787`) takes a single `resource_dir`. `vendor_output_dirs` by contrast already
returns `Vec<PathBuf>` and `copy_vendor_libraries` already takes a slice — so
vendoring widens for free and resources do not.

### 2.4 `finalize_app_bundle` returns one path

`src/target.rs:finalize_app_bundle` is `Result<Option<PathBuf>, String>`, and the
CLI (`src/cli/build.rs:585-598`) replaces `executable_paths` with `vec![path]`.
Two AppImages need `Vec<PathBuf>`.

## 3. Design Overview

Four mechanical widenings plus one naming decision:

1. **Flip the three gates** (§4.1) to `LinuxFlavor::ALL` / both `Libc`s.
2. **Flavor the artifact names** (§4.2) — `<name>-<flavor>.AppDir` and
   `<name>-<flavor>.AppImage`.
3. **Widen the routing** (§4.3) — `vendor_output_dirs` gains the second AppDir;
   `resource_output_dir` → `resource_output_dirs` returning a `Vec`.
4. **Seal both** (§4.4) — `finalize_app_bundle` returns `Vec<PathBuf>`.

The correctness risk is concentrated in **§4.3's vendor routing**, and it is
subtler than it looks: `copy_vendor_libraries` copies *every* resolved library
into *every* output directory. With both flavors resolved, that puts the glibc
blob inside the musl image and vice versa — harmless at runtime (each binary
`dlopen`s its own filename) but it doubles the vendored payload and ships a
library that can never load. §4.3 resolves this by routing per flavor rather than
broadcasting.

### 3.1 Why the flavor goes in the artifact name, not inside the AppDir

The AppDir's *contents* stay unflavored: the executable is `usr/bin/<name>`, the
desktop entry `<name>.desktop`, the icon `<name>.png`. Only the containing
directory and the sealed file carry `-glibc` / `-musl`. This mirrors the console
path exactly — `<name>-glibc.out` is a flavored *filename* wrapping an unflavored
program — and it keeps `AppRun -> usr/bin/<name>` identical between the two, so
the `.desktop`, the `StartupWMClass`, and `os::resourcePath`'s
`strip "bin/<name>", append "share/<name>"` derivation are untouched.

Rejected: **flavoring the inner executable** (`usr/bin/<name>-glibc`). It would
break `AppRun`'s fixed relationship, force the `.desktop` `Exec=` to differ per
flavor, and change the resource-path derivation — all to disambiguate files that
live in separate directories and can never collide.

### 3.2 Why not one AppDir sealed twice

Tempting: build one AppDir, swap the executable, seal again. Rejected — it makes
the two seals order-dependent and mutually destructive, and `--app-debug` could
only ever retain the last one. Two directories cost one extra tree write and keep
each artifact independently inspectable, which is the entire argument plan-51-A
§3.1 makes for having an AppDir at all.

## 4. Detailed Design

### 4.1 Flipping the gates

Both backends collapse to the console form:

```rust
// app mode is no longer glibc-only (plan-56): GTK4 exists in the musl world
// (Alpine's `gtk4.0`), and plan-56-A made the import surface flavor-correct.
let flavors: &[LinuxFlavor] = &LinuxFlavor::ALL;
```

and `emitted_link_targets` (`cli/build.rs:1419`) drops its `is_app()` branch
entirely, so vendor resolution covers both libcs for every Linux build.

### 4.2 Flavored artifact names

`write_appdir` gains a flavor suffix used **only** for the directory name:

```rust
/// The AppDir for one libc flavor (plan-56-B §4.2): `build/<name>-<flavor>.AppDir`.
/// The suffix names the *container*, never its contents — the executable stays
/// `usr/bin/<name>` so `AppRun`, the `.desktop`, and `os::resourcePath`'s
/// derivation are identical across flavors.
pub(crate) fn write_appdir(
    project_dir: &Path,
    project_name: &str,
    flavor_suffix: &str,
    …
) -> Result<PathBuf, String>
```

`appimage::seal` takes the same suffix, reading `<name>-<flavor>.AppDir` and
writing `<name>-<flavor>.AppImage`. `remove_appdir` likewise.

A single helper owns the string so the writer and the seal cannot disagree:

```rust
fn appdir_name(project_name: &str, flavor: LinuxFlavor) -> String {
    format!("{project_name}-{}", flavor.suffix())
}
```

### 4.3 Per-flavor vendor and resource routing

⚠️ `copy_vendor_libraries(vendored, …, output_dirs)` copies **every** entry in
`vendored` into **every** directory in `output_dirs`. Returning both AppDirs'
`usr/lib` from `vendor_output_dirs` would therefore put both libc variants of
every vendored library into both images.

The fix is to route by flavor. `vendor_output_dirs` becomes flavor-aware for
`LinuxApp`, and the CLI calls the copy once per flavor with only that flavor's
resolved libraries — which `resolved_vendor_libraries` can already produce, since
it iterates `emitted_link_targets` and each resolved library carries the
`LinkTarget` it came from.

The doc table becomes:

| build | rpath | vendor files |
| --- | --- | --- |
| linux console | `$ORIGIN/vendor` | `build/vendor/` (both flavors share it; source filenames are unique project-wide) |
| linux `--app` glibc | `$ORIGIN/../lib` | `build/<name>-glibc.AppDir/usr/lib/` |
| linux `--app` musl | `$ORIGIN/../lib` | `build/<name>-musl.AppDir/usr/lib/` |
| macos console | `@loader_path/vendor` | `build/vendor/` |
| macos `--app` | `@executable_path/../Frameworks` | `build/<name>.app/Contents/Frameworks/` |

`resource_output_dir` → `resource_output_dirs` returning `Vec<PathBuf>`: one entry
for console and macOS, two for `LinuxApp`. Resources are flavor-independent, so
both AppDirs get the same copy — here broadcasting *is* correct, which is exactly
why the vendor case above needs the opposite treatment and why the two must not
share a helper.

### 4.4 Sealing both

```rust
/// Finalize an app-mode build (plan-51-C §4.5, widened by plan-56-B §4.4).
/// Returns every artifact that replaces what `write_executable` reported —
/// two AppImages on Linux, empty on macOS and console.
fn finalize_app_bundle(&self, …) -> Result<Vec<PathBuf>, String>
```

The Linux override seals once per flavor and removes each AppDir unless
`keep_intermediate`. The CLI replaces `executable_paths` when the returned vec is
non-empty. macOS returns `Vec::new()`.

Ordering is unchanged and still load-bearing: vendoring and the resource copy
both run before *any* seal, because a sealed artifact cannot gain files
(plan-51-C §3.2).

## Compatibility / Format Impact

**Changes:**

- `mfb build --app` on Linux emits two artifacts instead of one, and their names
  gain a flavor component: `<name>.AppImage` → `<name>-glibc.AppImage` +
  `<name>-musl.AppImage`. **This renames the existing glibc artifact** — anything
  scripted against `build/<name>.AppImage` breaks.
- `--app-debug` leaves two AppDirs, similarly renamed.
- Two `Wrote executable to …` lines instead of one.

**Unchanged:** the AppDir layout and every file in it, the `.desktop` contents,
the icons, the RUNPATH string, the GTK app id, the seal format, every console and
macOS artifact, the `.mfp` format, and the manifest schema.

## Phases

### Phase 1 — Flavored names, still glibc-only

Lands the renaming with one flavor still emitted, so the naming change is
reviewable in isolation and `tests/linux_app_mode.rs` moves in one obvious step.

- [ ] Add `appdir_name` and thread a flavor suffix through
      `src/os/linux/appdir.rs:write_appdir`, `src/os/linux/mod.rs:write_linked_appdir`,
      `src/os/linux/link/mod.rs:write_appdir`, and
      `src/os/linux/appimage/mod.rs:{seal,remove_appdir}`.
- [ ] Update `vendor_output_dirs` / `resource_output_dir` LinuxApp arms to the
      flavored AppDir path (`src/cli/build.rs`).
- [ ] Tests: update `tests/linux_app_mode.rs` and the `os::linux` layout test for
      `<name>-glibc.AppImage`; `vendor_output_dirs` / `resource_output_dir` cases.

Acceptance: `mfb build --app -target linux-x86_64` emits exactly
`build/<name>-glibc.AppImage`, byte-identical to the `<name>.AppImage` the same
source produced before the rename.
Commit: —

### Phase 2 — Emit both flavors

- [ ] Flip the three gates (§4.1): both backends' `flavors`, and
      `emitted_link_targets`.
- [ ] Widen `finalize_app_bundle` to `Vec<PathBuf>` (`src/target.rs` + both Linux
      backends + `src/cli/build.rs`); seal once per flavor.
- [ ] Per-flavor vendor routing and `resource_output_dirs` (§4.3).
- [ ] Tests: a `--app` build emits both AppImages and no AppDir; `--app-debug`
      leaves both AppDirs; the musl image's inner ELF names the musl interpreter
      and the glibc image's names `/lib64/ld-linux-*`; a vendoring build puts
      **only** the matching libc's blob in each image.

Acceptance: `mfb build --app -target linux-x86_64` emits both AppImages; each
inner ELF carries its own flavor's interpreter; a vendoring fixture's musl image
contains the musl blob and not the glibc one.
Commit: —

## Validation Plan

- **Tests:** integration in `tests/linux_app_mode.rs` (cross-builds and inspects
  without executing; the dev host is macOS) — artifact set per flag, per-flavor
  interpreter, per-flavor vendored blob. Unit in `src/cli/build.rs` for
  `vendor_output_dirs` / `resource_output_dirs` per mode and flavor, and in
  `src/os/linux/appimage` for the flavored seal paths. Negative: a `--app` build
  must leave no unflavored `<name>.AppImage` behind.
- **Runtime proof:** deferred to plan-56-C, which owns the hardware gate. Note
  that running the musl AppImage proves **less than it appears** — musl absorbs
  glibc compat names (plan-56-A §2.4), so a launch cannot distinguish a correct
  build from a wrong one. plan-56-C's `DT_NEEDED` assertion is the real check.
- **Doc sync:** `src/docs/spec/architecture/08_artifacts.md` (two AppImage rows),
  `src/docs/spec/tooling/07_cli-reference.md` (output shape),
  `src/docs/spec/app/02_linux-runtime.md` + `app/spec.md` (app mode is no longer
  glibc-only), `src/docs/spec/language/17_native-libraries.md` (per-flavor vendor
  table). plan-56-C owns the sweep; do not half-edit them here.
- **Acceptance:** `scripts/test-accept.sh`, `scripts/artifact-gate.sh`,
  `cargo test`, `cargo fmt` with the second pass in `repository/`.

## Open Decisions

- **Rename the glibc artifact vs. keep `<name>.AppImage` for glibc** — recommend
  renaming both, matching the console path's symmetry. Keeping the glibc one
  unsuffixed would be backward compatible but makes the two artifacts look
  asymmetric and hides which libc the unsuffixed one is. It is a fresh feature
  (plan-51 shipped this session), so the compatibility cost is near zero. (§4.2)
- **Per-flavor vendor routing vs. broadcasting both blobs** — recommend routing.
  Broadcasting works at runtime but doubles the vendored payload and ships a
  library that can never load in that image. (§4.3)

## Summary

Most of this sub-plan is mechanical widening of signatures that were written for
one artifact and now need two. The one place to be careful is **§4.3's vendor
routing**: `copy_vendor_libraries` broadcasts every library to every directory,
so the naive change silently puts the glibc blob inside the musl image — which
does not fail, it just bloats the artifact and ships dead weight. Resources want
exactly the opposite (broadcast is correct), which is why they get their own
helper rather than sharing one.

The second thing to hold onto is that **this sub-plan's output cannot be
validated by running it** (plan-56-A §2.4). Both AppImages will launch on a
correctly-provisioned host whether or not the flavoring is right.

Left untouched: the AppDir layout, the seal format, the RPATH string, every
console and macOS artifact, and riscv64's rejection.
