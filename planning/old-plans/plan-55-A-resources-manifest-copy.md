# plan-55-A: `resources` manifest section + build-time copy

Last updated: 2026-07-17
Overall Effort: large (3h–1d)
Effort: medium (1h–2h)
Depends on: nothing

A `project.json` `"resources"` array declares source files to ship alongside the
built program. Each entry is a glob `src` (project-relative, `**` supported) and a
directory `dst`. At build time every matched file is copied into the output tree at
a location that depends on the build shape:

- **console / non-`--app`** → `build/<dst>/…`
- **macOS `--app`** → `build/<name>.app/Contents/Resources/<dst>/…`
- **Linux `--app`** (plan-51 AppDir) → `build/<name>.AppDir/usr/share/<name>/<dst>/…`

The behavioral outcome: `mfb build` on `examples/audio` (whose manifest maps
`data/**/*.ogg` → `music/`) leaves `build/music/Mozart1.ogg` and
`build/music/Mozart2.ogg` on disk; the same build with `--app` on macOS leaves them
at `build/audio.app/Contents/Resources/music/…`. Stale copies from a previous build
never survive, because the `build/` directory is cleared at the start of every
build. This sub-plan handles the manifest schema and the copy; the runtime locator
(`os::resourcePath`) is plan-55-B, which resolves paths to exactly the directories
this sub-plan writes.

References (read first):

- `src/cli/build.rs:1484` — `vendor_output_dirs`, and `:1521` `copy_vendor_libraries`:
  the exact precedent this mirrors (build-mode → destination dir, then a copy loop
  run after `write_executable`).
- `src/manifest/mod.rs:195` — `validate_sources` (array-of-objects validation shape),
  `:355` `validate_mode`, `:761` `icon_path` (optional-field accessor shape).
- `src/ast/manifest.rs:511` — `glob_matches` / `glob_match_segments`: the in-tree
  `**`-aware glob engine to reuse for `src` expansion (no new crate).
- `src/os/mod.rs:12-35` — `BUILD_DIR`, `VENDOR_DIR`, `MACOS_APP_FRAMEWORKS_DIR`; add
  `MACOS_APP_RESOURCES_DIR` beside them.
- `src/os/macos/link/mod.rs:88` — `resources_dir` = `Contents/Resources`, where
  `AppIcon.icns` already lands; resources join it.
- `planning/plan-51-A-linux-appdir.md` §4.1 — the AppDir layout; §4.4 shows the
  `LinuxApp` arm pattern in `vendor_output_dirs` this sub-plan copies.
- `src/docs/spec/tooling/01_project-manifest.md`, `.../02_source-selection.md`,
  `src/docs/spec/architecture/08_artifacts.md` — the spec pages this obligates.

## 1. Goal

- A `project.json` with `"resources": [ { "src": "data/**/*.ogg", "dst": "music/" } ]`
  is accepted by manifest validation; a malformed entry (missing/non-string `src` or
  `dst`, non-array `resources`, non-object entry) is rejected with a source-spanned
  diagnostic mirroring `validate_sources`.
- After `mfb build` (console), every file matching each `src` glob exists under
  `build/<dst>/…`, with subdirectory structure below the glob's fixed prefix
  preserved (so `data/**/*.ogg` matching `data/sub/a.ogg` lands at
  `build/<dst>/sub/a.ogg`).
- After `mfb build --app`, the same files land in the bundle's resource directory
  (`Contents/Resources/<dst>/…` on macOS; `usr/share/<name>/<dst>/…` on Linux).
- The `build/` directory is cleared at the start of every build, so a file removed
  from a `src` match set no longer appears in `build/` after a rebuild.

### Non-goals (explicit constraints)

- **No runtime API.** `os::resourcePath` is plan-55-B. This sub-plan only puts bytes
  on disk; nothing reads them yet.
- **No `.mfp` / NIR / codegen change.** Resources are a build-orchestration concern,
  invisible to the compiler proper and to the binary format. No golden executable
  bytes change.
- **No manifest schema change beyond the additive `resources` key.** `sources`,
  `mode`, `icon`, `entry`, `targets`, `libraries` are untouched. `resources` is
  optional; a manifest without it behaves exactly as today.
- **No packaging into the binary.** Resources are copied *beside* the executable (or
  into the bundle), never embedded in the ELF/Mach-O. Embedding is out of scope.
- **No `dst` outside the output tree.** An absolute `dst`, or one containing `..`, is
  a validation error — a resource copy must never escape `build/`.

## 2. Current State

### 2.1 The `resources` key is parsed but ignored

`examples/audio/project.json` already carries a `"resources"` array (the user added
it, keyed `resources`). Nothing in `src/manifest/mod.rs` validates it and nothing in
`src/cli/build.rs` consumes it: it is valid JSON that the manifest loader silently
drops. Both halves — validation and copy — are new.

### 2.2 Vendored libraries are the exact precedent

plan-46-D already solved "copy author-declared files into a build-mode-dependent
output directory after linking":

- `vendor_output_dirs(output_dir, name, build_mode)` (`src/cli/build.rs:1484`) maps
  `MacApp` → `build/<name>.app/Contents/Frameworks/`, everything else →
  `build/vendor/`.
- `copy_vendor_libraries(...)` (`src/cli/build.rs:1521`) is called right after
  `target::write_executable` (`src/cli/build.rs:521`) and does the copy.

`resources` is the same shape with a different destination sub-path and a glob
expansion in front of the copy. plan-51-A §4.4 will add the `LinuxApp` arm to
`vendor_output_dirs`; the resource equivalent gets the same arm.

### 2.3 Manifest validation has a reusable shape

`validate_sources` (`src/manifest/mod.rs:195`) is the model: fetch the array, error
if the field is present-but-wrong-type, iterate entries erroring on non-objects,
validate each required string field with a source span from `field_position`.
`validate_mode` (`:355`) shows optional-field validation, and `icon_path` (`:761`)
shows the read-side accessor. A `resource_entries` accessor and a
`validate_resources` validator slot in beside them.

### 2.4 The glob engine already exists

`src/ast/manifest.rs:511` `glob_matches(pattern, path)` matches a `**`-capable glob
against a path, used today for `sources` `include`/`exclude`. It matches; it does not
*enumerate*. Resource copy needs enumeration (walk the tree, keep matches), so this
sub-plan adds a small directory walk that calls `glob_matches` per candidate — no new
matching logic and no new crate.

### 2.5 `build/` is not cleared today

Each build writes into `build/` over whatever is there. Vendored libraries are only
ever added, so a stale `vendor/x.so` could linger; the same would be true of
resources. The user's directive is to clear `build/` at the start of a build.

## 3. Design Overview

Three pieces, layered lowest-risk first:

1. **Manifest validation + accessor** (§4.1) — `validate_resources` +
   `resource_entries`, pure functions mirroring `validate_sources`/`icon_path`. No
   I/O, unit-testable.
2. **`build/` clearing** (§4.2) — remove `build/` once at build start, before
   `write_executable`. One `remove_dir_all` + recreate.
3. **Resource copy** (§4.3) — `resource_output_dir(output_dir, name, build_mode)` +
   `copy_resources(...)`, called next to `copy_vendor_libraries`. Glob-expand each
   `src`, preserve the sub-prefix tree under `dst`, copy.

The correctness risk is small and concentrates in **§4.3's glob-prefix mapping**:
the rule that turns a matched path into its destination-relative path (strip the
fixed prefix before the first wildcard) must be the *same* rule plan-55-B assumes
when it documents "call `os::resourcePath("<dst>/<relpath>")`". If the two disagree,
files land where nothing looks for them. §4.3 pins the rule with a table and tests.

**Rejected — clear only the resource output dir, not all of `build/`.** Narrower and
less destructive, but it leaves stale vendored libraries and stale prior-mode
outputs (an app build after a console build), which is the same class of bug. The
user asked for the whole folder; a single clear point is also simpler to reason
about than one-clear-per-artifact-kind. Recorded as an Open Decision since it is a
visible behavior change.

**Rejected — embed resources in the executable.** Self-contained single-file output,
but it needs a virtual-filesystem read path in the runtime, bloats every binary, and
breaks the "edit an asset without recompiling" workflow. Copy-beside matches macOS
`.app` and Linux AppDir conventions, which is what `os::resourcePath` (plan-55-B)
resolves against.

## 4. Detailed Design

### 4.1 Manifest validation and accessor

In `src/manifest/mod.rs`, beside `validate_sources`:

```rust
/// Validate the optional `resources` array (plan-55-A §4.1). Each entry is an
/// object with a string `src` (a project-relative glob, `**` allowed) and a
/// string `dst` (a destination directory under the build output). Absent is
/// valid — resources are opt-in. Mirrors `validate_sources`' diagnostics.
fn validate_resources(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool
```

Rules, each a spanned error via `field_position`:

- `resources` present but not an array → error.
- an entry that is not an object → error.
- `src` missing / not a string / empty → error (reuse `validate_required_string`).
- `dst` missing / not a string → error. Additionally reject a `dst` that is absolute
  (`starts_with('/')`) or contains a `..` component — a resource must not escape the
  output tree (§1 non-goal). This check is new relative to `validate_sources`, which
  has no destination field.

Call it from the top-level validator next to `validate_mode`
(`src/manifest/mod.rs:107`).

Accessor, beside `icon_path`:

```rust
/// The declared `resources` entries (plan-55-A §4.1), `src`/`dst` pairs. Empty
/// when the manifest declares none. Assumes the manifest already passed
/// `validate_resources`.
pub(crate) struct ResourceEntry { pub(crate) src: String, pub(crate) dst: String }
pub(crate) fn resource_entries(manifest: &HashMap<String, JsonValue>) -> Vec<ResourceEntry>
```

### 4.2 Clearing `build/`

At the start of the native output path in `build_project` (`src/cli/build.rs`),
before `write_executable`, remove and recreate the build directory:

- Target: `output_dir.join(crate::os::BUILD_DIR)` — the same `output_dir`
  `write_executable`/`vendor_output_dirs` use (`src/cli/build.rs:478`,`:524`).
- `std::fs::remove_dir_all` (ignore `NotFound`), then let the existing writers
  recreate it (`copy_vendor_libraries` and the backends already `create_dir_all`).
- **Only for the real build path, not `mfb test`'s host run**, which links into a
  private temp dir (`make_temp_output_dir`, `src/cli/build.rs:474`) and must not
  touch the project's `build/`. A cross-`-target` test build writes to the project
  dir like a normal build and clears normally.

This runs once per `mfb build` invocation, so the two Linux console flavors
(`-glibc.out` + `-musl.out`, written in one invocation) survive each other.

### 4.3 Resource output directory and copy

Destination selector, beside `vendor_output_dirs`:

```rust
/// The directory resources are copied into for a given build shape (plan-55-A
/// §4.3). `<dst>` from each entry is joined *under* this. Kept in lockstep with
/// plan-55-B's `os::resourcePath` base: the runtime locator resolves to exactly
/// this directory, so a change here without the matching change there makes
/// resources unfindable at runtime.
///
/// | build            | resource dir                                   |
/// | ---              | ---                                            |
/// | console          | `build/`                                       |
/// | macos `--app`    | `build/<name>.app/Contents/Resources/`         |
/// | linux `--app`    | `build/<name>.AppDir/usr/share/<name>/`        |
fn resource_output_dir(output_dir: &Path, name: &str, build_mode: NativeBuildMode) -> PathBuf
```

The `LinuxApp` arm depends on plan-51-A's AppDir existing; until 51-A lands, a Linux
`--app` build does not reach this path (Linux app mode is unimplemented pre-51). The
arm is written now so 51-A needs no change here — see the plan-51-A update in §Phases.

Copy, beside `copy_vendor_libraries`:

```rust
/// Copy every file matching each resource entry's `src` glob into
/// `<resource_dir>/<dst>/…` (plan-55-A §4.3), preserving structure below the
/// glob's fixed prefix. Runs after `write_executable`, next to the vendor copy.
fn copy_resources(
    project_root: &Path,
    entries: &[ResourceEntry],
    resource_dir: &Path,
) -> Result<(), String>
```

**Glob expansion + prefix mapping** (the pinned rule):

1. Split `src` into a *fixed prefix* (leading path components with no glob
   metacharacter — `*`, `?`, `[`, `]`) and a *pattern tail*. For `data/**/*.ogg` the
   prefix is `data/`; for `data/*.ogg` it is also `data/`; for `assets/logo.png`
   (no metacharacters) the prefix is `assets/` and the tail is `logo.png`.
2. Walk the prefix directory recursively (a small `read_dir` recursion; no
   `walkdir` crate). For each regular file, form its path relative to `project_root`
   and test `glob_matches(src, rel_path)` (`src/ast/manifest.rs:511`).
3. For each match, the **destination-relative path** is the match minus the fixed
   prefix: `data/sub/a.ogg` with prefix `data/` → `sub/a.ogg`. The file is copied to
   `resource_dir.join(dst).join(dest_relative)`, creating parent dirs.

Worked examples (these are the contract plan-55-B relies on):

| `src` | `dst` | matched file | lands at (under resource dir) |
| --- | --- | --- | --- |
| `data/*.ogg` | `music/` | `data/Mozart1.ogg` | `music/Mozart1.ogg` |
| `data/**/*.ogg` | `music/` | `data/loops/kick.ogg` | `music/loops/kick.ogg` |
| `assets/logo.png` | `img/` | `assets/logo.png` | `img/logo.png` |

Empty match set for a `src` is **not** an error (a glob may legitimately match
nothing on a given checkout); it is silently skipped. A `src` whose fixed-prefix
directory does not exist is likewise a no-op, not an error.

Call site, right after `copy_vendor_libraries` (`src/cli/build.rs:521-528`):

```rust
if let Err(err) = copy_resources(
    &options.location,
    &crate::manifest::resource_entries(&manifest),
    &resource_output_dir(output_dir, &ir.name, build_mode),
) {
    eprintln!("error: {err}");
    return Err(());
}
```

## Compatibility / Format Impact

**Changes:**

- `project.json` gains an optional `"resources"` array. A manifest using it now
  validates; a malformed one now errors where before it was ignored.
- `mfb build` now clears `build/` at the start of every build. A file a previous
  build left in `build/` (including a stale console binary before an `--app` build)
  is gone after the next build. This is the intended stale-file fix and is visible.
- `mfb build` and `mfb build --app` now leave declared resources in the output tree.

**Unchanged:**

- Every executable's bytes, plan, and goldens — resources are copied files, not
  compiled input. `scripts/artifact-gate.sh` sees no diff.
- The `.mfp` format, NIR, the manifest schema for every existing key.
- `mfb test`'s temp-dir isolation: it never writes to or clears the project `build/`.

## Phases

### Phase 1 — Manifest validation + accessor

Pure, no I/O; lands alone and rejects malformed manifests immediately.

- [ ] Add `validate_resources` (§4.1) to `src/manifest/mod.rs` and call it from the
      top-level validator beside `validate_mode` (`:107`).
- [ ] Add `ResourceEntry` + `resource_entries` accessor beside `icon_path` (`:761`).
- [ ] Tests (`src/manifest/mod.rs` `#[cfg(test)]`): valid `resources` accepted;
      non-array rejected; non-object entry rejected; missing/empty `src` rejected;
      missing/non-string `dst` rejected; absolute `dst` and `..`-containing `dst`
      rejected; absent `resources` accepted; `resource_entries` round-trips
      `examples/audio`'s entry.

Acceptance: `cargo test -p mfb manifest` green, including the new negative cases;
`validate_project_manifest` on `examples/audio/project.json` succeeds.
Commit: —

### Phase 2 — Clear `build/` at build start

Isolated so the behavior change is reviewable and its acceptance impact measured
alone.

- [ ] In `build_project` (`src/cli/build.rs`), before `write_executable`, remove
      `output_dir/build` (ignoring `NotFound`) on the real build path only — not the
      `mfb test` host temp path (§4.2).
- [ ] Tests: a build into a tempdir with a pre-seeded `build/stale.txt` no longer
      has `stale.txt` afterward; a `mfb test` run leaves an existing project `build/`
      untouched.

Acceptance: the two tests pass; `scripts/test-accept.sh` shows no golden churn (the
harness builds into temp/`-o` outputs; confirm it does not depend on `build/`
persisting between invocations — if any test does, fix the test, not the behavior).
Commit: —

### Phase 3 — Resource copy (console + macOS app)

The user-visible copy; console + macOS land here, the Linux arm activates with
plan-51-A.

- [ ] Add `MACOS_APP_RESOURCES_DIR = "Resources"` to `src/os/mod.rs` and use it in
      `src/os/macos/link/mod.rs:88` in place of the `"Resources"` literal.
- [ ] Add `resource_output_dir` (§4.3) beside `vendor_output_dirs`
      (`src/cli/build.rs:1484`), including the `LinuxApp` arm (dormant until 51-A).
- [ ] Add `copy_resources` (§4.3) with the fixed-prefix split, `read_dir` recursion,
      `glob_matches` filter, and prefix-stripped destination mapping.
- [ ] Wire the call after `copy_vendor_libraries` (`src/cli/build.rs:521`).
- [ ] Tests: `resource_output_dir` per mode (console/`MacApp`/`LinuxApp`); a
      `copy_resources` unit test over a tempdir fixture covering the three §4.3
      worked examples, including `**` subtree preservation and the empty-match no-op.

Acceptance: `mfb build` on `examples/audio` leaves `build/music/Mozart1.ogg` and
`build/music/Mozart2.ogg`; `mfb build --app` on macOS leaves them at
`build/audio.app/Contents/Resources/music/…`; a rebuild after deleting
`data/Mozart2.ogg` leaves no `build/music/Mozart2.ogg` (Phase 2 clearing).
Commit: —

### Phase 4 — plan-51-A coordination (doc-only here)

- [ ] Confirm plan-51-A §4.1 lists `usr/share/<name>/` as the resource root and that
      its `write_appdir` does not delete it (the copy runs after the AppDir writer).
      The plan-51-A edit itself is made as part of this task (see §Summary).

Acceptance: `resource_output_dir`'s `LinuxApp` arm and plan-51-A §4.1 name the same
directory. No code lands in this phase; it is the lockstep checkpoint.
Commit: —

## Validation Plan

- Tests: unit — `validate_resources` (positive + every negative), `resource_entries`,
  `resource_output_dir` per mode, `copy_resources` prefix mapping + `**` + empty
  match, `build/` clearing + `mfb test` isolation.
- Runtime proof: `mfb build examples/audio` then `ls build/music/` shows both `.ogg`
  files; `mfb build --app examples/audio` on macOS then
  `ls build/audio.app/Contents/Resources/music/` shows them; delete one source, rebuild,
  confirm it is gone from `build/`.
- Doc sync: `src/docs/spec/tooling/01_project-manifest.md` (document `resources`:
  `src` glob, `dst` dir, per-mode destinations), `.../02_source-selection.md`
  (cross-reference the shared glob semantics), `src/docs/spec/architecture/08_artifacts.md`
  (resources in the artifact tree for each mode).
- Acceptance: `scripts/test-accept.sh` (expect no golden churn), `cargo test`,
  `cargo fmt` (plus the second pass in `repository/`, not a workspace member).

## Open Decisions

- **Clear all of `build/`** — *decided (user, 2026-07-17)*: clear the whole `build/`
  directory at build start. The stale-vendored-lib / stale-prior-mode-output cleanup is
  the point, and the visible behavior change is accepted. Not a fork anymore; recorded
  here so the §4.2 behavior is not re-litigated. (§4.2)
- **Empty glob match: silent no-op vs. warning** — recommend silent. A glob matching
  nothing on one checkout is normal (platform-specific assets), and a warning on
  every such build is noise. Revisit if authors report silent-typo confusion. (§4.3)

## Summary

The engineering is deliberately boring: `resources` is `vendor` with a glob in front
and a different destination sub-path, and every hard part — build-mode → directory,
copy-after-link, `**` matching — already has a working precedent in the tree
(`copy_vendor_libraries`, `glob_matches`). The one rule that must not drift is §4.3's
prefix-strip mapping, because plan-55-B's `os::resourcePath` resolves against exactly
the directories written here; the worked-example table is the shared contract.

The one visible behavior change is clearing `build/` at build start (§4.2) — intended,
faithful to the stale-file directive, and isolated in its own phase so its acceptance
impact is measured alone. Left untouched: the compiler, the binary format, every
golden, and `mfb test`'s temp-dir isolation.

As part of landing this sub-plan, `planning/plan-51-A-linux-appdir.md` §4.1 is updated
to name `usr/share/<name>/` as the resource root so `os::resourcePath` (plan-55-B)
resolves correctly inside an AppImage.
