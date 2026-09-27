# plan-158-D: libsnd on Windows + examples/audio

Last updated: 2026-09-26
Effort: medium (1h–2h)
Depends on: plan-158-C

Ship the libsnd package's Windows DLL and make `examples/audio` build and play on Windows.
**Outcome:** `mfb build --target windows-x86_64 examples/audio` succeeds, and on box
192.168.1.206 all four menu items play audibly (user-confirmed).

References: `packages/libsnd/project.json`, `packages/libsnd/vendor/sndfile.dll`,
`examples/audio/{project.json,README.md,src/main.mfb}`,
`src/cli/build/source_packages.rs`, `src/cli/build/native_libs.rs`
(`vendor_source_path`), `src/manifest/libraries.rs` (`imported_vendor_path`),
`.ai/resources-packages.md` §"A committed `.mfp` dependency goes stale silently".

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-158-C complete (archived) | `ls planning/completed/plan-158-C-*` → one file | MET (2026-09-27: `planning/completed/plan-158-C-windows-audio-exclusive-fixes.md`) |

If plan-158-C is not complete, this plan cannot start, full stop.

## 1. Goal

- `packages/libsnd` declares its Windows DLL.
- A source-directory dependency that vendors native libraries builds (bug below).
- `examples/audio` uses the current libsnd and plays on Windows.

### Non-goals

- libsndfile itself is not rebuilt; `sndfile.dll` is used as committed.
- The `audio::` API is unchanged.

## 2. Current State

- `packages/libsnd/vendor/sndfile.dll` is tracked (`git ls-files packages/libsnd`) and is
  `PE32+ executable (DLL) … x86-64` (`file`), but `packages/libsnd/project.json`
  `libraries.libsnd` has 7 entries (macOS aarch64 + six Linux) and none for Windows.
- `examples/audio/project.json` pins registry `jzaun#libsnd` 1.5.0. The installed copy
  (`examples/audio/packages/`, `mfb.lock`) is gitignored, and its vendor tree has no
  `.dll` (README §Dependency says so).
- **Bug (found 2026-09-26, reproduced):** with `"source": "file:../../packages/libsnd"`,
  `mfb build examples/audio` fails with `2-203-0120 NATIVE_LIBRARY_FILE_MISSING … examples/audio/packages/libsnd.vendor/libsndfile.1.0.37.dylib`.
  `build_source_dependencies` compiles the `.mfp` into `build/packages/` but never
  makes the dependency's `vendor/` files available. `vendor_source_path` only looks in
  `<project>/packages/<unit>.vendor/`. So a source dependency that vendors a library
  can never build. A fix was prototyped and reverted: copy `<dir>/vendor/*` to
  `<cache>/<name>.vendor/`, and resolve the vendor dir next to whichever `.mfp`
  `resolved_package_file` returns. With it, the example built for macOS and
  `windows-x86_64`.
- `examples/audio/src/main.mfb` opens `out` (48000/1) once for the whole session, and
  plan-158-C Bug 2 shows another stream open can invalidate it.

## 3. Design Overview

Three small, independent pieces. Byte-identity is not the gate (manifest/CLI change and
example source). Expected diffs: none in `tests/byte-identity`.

## Phases

> **NOTE — keep the checkboxes current as you go** (same rules as plan-158-A).

### Phase 1 — Source-dependency vendor bug

- [x] RED test in `src/cli/build/mod.rs` tests (next to
      `vendor_source_path_distinguishes_own_and_imported_units`): a project with a
      `file:` source dependency that declares a `vendor` library resolves the library's
      path under `build/packages/<name>.vendor/`. It fails at HEAD. —
      `vendor_source_path_finds_a_source_dependency_s_staged_vendor` failed before the
      fix (`left: …/packages/dep.vendor/libdep.so`, `right: …/build/packages/dep.vendor/
      libdep.so`); plus `stage_source_dependency_vendor_copies_the_flat_vendor_files`.
- [x] `src/cli/build/source_packages.rs`: after `build_source_dependency`, copy the
      dependency's flat `vendor/` files to `<cache>/<name>.vendor/`. —
      `stage_source_dependency_vendor`, called from `build_source_dependency` (before
      the nested build; the cache already exists, and staging first means a failed
      copy stops the build before any compile work).
- [x] `src/cli/build/native_libs.rs` `vendor_source_path`: for an imported unit, use
      the `.vendor` dir beside `resolved_package_file(project_root, unit)`, falling back
      to `imported_vendor_path`.
- [x] Spec: `src/docs/spec/tooling/07_cli-reference.md` / the architecture packages
      section states where a source dependency's vendor files are staged; cite
      `stage_source_dependency_vendor`. — in `language/13_modules-and-packages.md`, the
      paragraph that documents source dependencies (C1); `spec-census.sh --citations`
      → 0 misses.

Acceptance: the test passes; the example builds against the source package.
  Check: `cargo test --bin mfb vendor_source` → pass (est. 3 min). — `2 passed`; a
  copy of `examples/audio` with `"source": "file:…/packages/libsnd"` →
  `Wrote executable to /tmp/p158/audiofile/build/audio.out`, and
  `build/packages/libsnd.vendor/` holds the vendored libraries.
Commit: —

### Phase 2 — libsnd Windows locator + release

- [ ] `packages/libsnd/project.json`: add `{ "os": "windows", "arch": "x86_64", "type":
      "vendor", "source": "sndfile.dll" }`; bump `version` per Open Decision 1; update
      `description` (it lists the vendored targets).
- [ ] Check the DLL's imports are system-only:
      `llvm-objdump -p packages/libsnd/vendor/sndfile.dll | grep 'DLL Name'`. Record the
      result here. A codec DLL dependency is a blocker to report.
- [ ] Per Open Decision 1: publish the new version as user `jzaun` (the user authorized
      this on 2026-09-26), or keep it local.

Acceptance: the Windows build of the example links the DLL.
  Check: `mfb build --target windows-x86_64 examples/audio` → `Wrote executable …audio.exe`,
  and `examples/audio/build/vendor/libsnd-sndfile.dll` exists (est. 2 min).
Commit: —

### Phase 3 — examples/audio

- [ ] `examples/audio/project.json`: point at the chosen source (Open Decision 1).
- [ ] `examples/audio/src/main.mfb`: open the 48 kHz MML stream only around each
      `audio::play` (a `SUB playTune`) instead of holding `out` for the session, so a
      file stream cannot invalidate it (plan-158-C Bug 2).
- [ ] `examples/audio/README.md`: Windows build/run line; update §Dependency (the DLL
      now exists; version).
- [ ] Box proof: ship `audio.exe`, `music/`, `vendor/` (tar with `COPYFILE_DISABLE=1`),
      and have the user play items 1–4 on "Speakers (2- Synaptics HD Audio)".

Acceptance: user confirms all four items are audible on the box; no error lines.
  Check: `ssh test@192.168.1.206 "cd audio-test && audio.exe < in.txt"` with input
  `1\n1\n2\n3\n4\nQ\n` → ends with `Done` (est. 20 min: Mozart1 alone is 23 288 808
  frames at 44.1 kHz, about 8.8 min, measured from the `Playing …` line; nothing shorter
  exercises the full file path. Use a shorter excerpt if the user prefers).
Commit: —

## Validation Plan

- Final gate: `cargo test` (the Phase 1 test and existing CLI tests) (est. 30 min).
- Doc sync: README, `07_cli-reference.md`, and the libsnd `description`.

## Open Decisions

1. **Where the example gets libsnd.** Publish libsnd 1.6.0 (a new target is a minor
   bump) to the registry as `jzaun`, and pin the example to it — recommended: keeps the
   example a signed-registry demo, and the user offered to publish — vs.
   `"source": "file:../../packages/libsnd"` (always current, unsigned, needs Phase 1).
   Phase 1 lands either way: it is a bug.

## Corrections

- **C1 (spec location).** The source-dependency mechanism is documented in
  `mfb spec language modules-and-packages` (the `build_source_dependencies`
  paragraph), not in the CLI reference, so the staging sentence went there.

## Summary

Low risk. The real work is plan-158-C; this letter packages and proves it end to end.
