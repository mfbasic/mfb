# plan-147-B: System faces on macOS (CoreText)

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-147-A

Prerequisites: see plan-147-A. Additionally: plan-147-A is archived
(`ls planning/plan-147-A-* 2>/dev/null` → nothing; `ls planning/completed/plan-147-A-*`
→ one file). If it is not, this letter cannot start, full stop.

This letter adds the internal runtime call **`canvas::systemFaces() AS List OF
canvas::SystemFace`** and implements it on macOS with CoreText. `SystemFace` is an
internal record `{name AS String, postScript AS String, path AS String}`: the face's
full name, its PostScript name, and the file that holds it. Checkable outcome: on this
Mac, a headless `--app` program printing `len(canvas::systemFaces())` prints 413 (the
probe's count, §2), and every entry's `canvas::loadFontFace(path, postScript, name)`
succeeds.

References: plan-147-A; `src/codegen/builtins/audio/gen_macos_shared.rs`
(`emit_cfstring_field`), `src/codegen/memory/marshal/record_list.rs`
(`emit_build_record_list`), `.ai/arch-abi.md` (macOS AArch64), `.ai/compiler.md`.

## 1. Goal

- `canvas::systemFaces()` on macos-aarch64 answers one `SystemFace` per installed face
  that (a) has a `glyf` table and (b) is not a variable font, in CoreText's order.
- Absent/odd entries (no URL, no name) are skipped, never fatal.

### Non-goals

- No public member (E adds them). `systemFaces` is `internal_only`.
- Linux and Windows are C and D. On those targets `canvas.systemFaces` is **not** in
  `RUNTIME_CALLS` yet, so a program calling it fails the capability check — no stub.
- No sorting or de-duplication here; E does that in MFBASIC.

## 2. Current State

- No CoreText anywhere (`grep -rn "CoreText\|CTFont" src | wc -l` → 0).
- A framework is linked by naming it as a `PlatformImport` library; the names are mapped
  to dylib paths in `src/os/macos/link/mod.rs:dylib_path` and
  `src/os/macos/object.rs:dylib_for_library`. `"CoreFoundation"` is already known
  there; `"CoreText"` is not.
- Imports per runtime call live in `src/target/macos_aarch64/plan.rs:runtime_imports`;
  the capability list is `src/target/macos_aarch64/mod.rs` (`"canvas.fontFromBytes"` is
  a sibling entry).
- Calls: `platform.emit_external_call` (`src/codegen/engine/types/types.rs`) →
  `macos_aarch64/code.rs:emit_libsystem_call`, which fails the build if the import is
  missing from the plan.
- CFString → MFBASIC String: `emit_cfstring_field` in
  `audio/gen_macos_shared.rs` does `CFStringGetCString` into a 256-byte buffer at fixed
  audio frame offsets — not reusable as-is. Font full names fit (longest on this machine:
  `awk -F'\t' '{print length($1)}' /tmp/ctprobe/out.tsv | sort -n | tail -1`), but paths
  may not, so this letter writes a length-driven converter
  (`CFStringGetLength` → `CFStringGetMaximumSizeForEncoding` → arena buffer →
  `CFStringGetCString`) and a `CFURLGetFileSystemRepresentation` path read into a
  1024-byte (`PATH_MAX`) buffer.
- List building: `emit_build_record_list` is the audio-devices precedent
  (`audio/gen_alsa_devices.rs`, `gen_macos` devices). A list-returning call must be
  added to `CALLER_ARENA_BLOCK_RESULTS` (`src/codegen/registry/mod.rs`) or the test
  `every_block_returning_runtime_helper_is_classified` fails.

### Measured populations (probe `/tmp/ctprobe/p.c`, 2026-09-21, this Mac)

| What | Count | Command |
|---|---|---|
| Faces CoreText reports | 550 | `/tmp/ctprobe/p` → `urls=550 faces=550` |
| …with a `glyf` table | 440 | same → `glyf=440` |
| …with a variation dictionary | 51 | same → `variation=51` |
| Listed (glyf and not variable) | 413 | `awk -F'\t' '$5=="glyf=1 var=0"' /tmp/ctprobe/out.tsv \| wc -l` |
| Duplicate full names among those | 0 | `… \| cut -f1 \| sort \| uniq -d` → nothing |
| Enumeration wall time | 0.77 s | `time /tmp/ctprobe/p` |

### Verified properties

- CoreText exposes named instances of variable fonts as separate faces with synthetic
  PostScript names (`Skia-Regular_Bold`), which match no nameID 6 in the file and would
  draw the default outlines under the wrong name — hence rule (b). Verified in the probe
  output (`grep 'var=1' /tmp/ctprobe/out.tsv`).
- The API sequence, all functions (no data-symbol imports needed):
  `CTFontManagerCopyAvailableFontURLs` → per URL
  `CTFontManagerCreateFontDescriptorsFromURL` → per descriptor
  `CTFontCreateWithFontDescriptor(d, 0.0, NULL)` → `CTFontCopyFullName`,
  `CTFontCopyPostScriptName`, `CTFontCopyTable(f, 'glyf', 0)`, `CTFontCopyVariation`;
  path from `CFURLGetFileSystemRepresentation`. Verified by the probe compiling and
  running against exactly this list.
- `CTFontCreateWithFontDescriptor` takes a `CGFloat` size in `d0`: the emitter must
  zero `d0` (AArch64 passes the float in a SIMD register, not `x1`). UNVERIFIED in this
  codebase's call seam — Phase 1 task checks how `emit_external_call` treats float
  arguments.

## 3. Design

- Registry: `canvas/mod.rs` gains internal record `SystemFace` (`export: false`) and
  `func_system_faces.rs` registers `systemFaces` (`internal_only: true`,
  `Body::abi_function(lower_system_faces)`), errors `ErrOutOfMemory`.
- `lower_system_faces` dispatches on `platform.family()` (the audio precedent,
  `audio/gen_shared.rs`), to `gen_system_faces_macos.rs` here; C and D add their arms.
  The non-macOS arms are **absent** in this letter (the capability list keeps those
  targets from ever reaching the lowering); the `match` returns an internal
  `Err("canvas.systemFaces has no <family> backend")` for them, which is a compiler
  error path, not a runtime stub, and is deleted by D.
- Two passes like `fs::listDirectory` would cost two CoreText enumerations; instead one
  pass builds element records into an arena pair array sized by the URL count
  (`CFArrayGetCount`), then `emit_build_record_list` with the real count.
- Every CF object obtained by Copy/Create is `CFRelease`d on every path.

**Correctness risk**: CF reference leaks and the float argument. **Gate class**:
behavior-changing (new call). Expected golden diffs: none — nothing existing calls
`systemFaces` and it is not an `always` helper; any golden diff is a bug.

**Rejected**: reading names from the files (675 MB, A §2); `CTFontCollectionCreate…`
with `kCTFont*Attribute` keys (needs data-symbol imports from CoreText for no gain).

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the
> work. **An unticked box means NOT DONE.**

### Phase 1 — Plumbing and the macOS emitter

- [ ] `src/os/macos/link/mod.rs:dylib_path` and `src/os/macos/object.rs:dylib_for_library`:
      add `"CoreText"` → `/System/Library/Frameworks/CoreText.framework/CoreText`
      (same convention as the `CoreFoundation` arm).
- [ ] Check how `emit_external_call` passes a float argument on macOS; if it has no
      float seam, emit `fmov d0, xzr` (via the builder's float move) before the
      `CTFontCreateWithFontDescriptor` call. Record the finding in Corrections.
- [ ] `canvas/mod.rs`: `SystemFace` record; `func_system_faces.rs` registration;
      `gen_system_faces_macos.rs` emitter (§3).
- [ ] `macos_aarch64/plan.rs:runtime_imports`: a `"canvas.systemFaces"` arm with the
      CoreText and CoreFoundation functions of §2 plus the arena/allocation imports the
      record builder needs (copy from the audio devices arm).
- [ ] `macos_aarch64/mod.rs` capability list: add `"canvas.systemFaces"`.
- [ ] `registry/mod.rs:CALLER_ARENA_BLOCK_RESULTS`: add the call.
- [ ] Tests: new `tests/canvas/rt_canvas_system_fonts.rs` (macOS-gated with
      `#[cfg(target_os = "macos")]`), one headless `--app` program that asserts:
      `count > 0`; every entry has non-empty `name`, `postScript` and `path`; for
      every entry `loadFontFace(path, postScript, name)` succeeds (this is what catches
      a named instance or a CFF face slipping through the filters); an entry named
      `Helvetica` with a path ending `Helvetica.ttc` is present. It prints
      `system faces: N` for the acceptance log.

Acceptance: every face `systemFaces` lists on this Mac loads.
  Check: `cargo test --test rt_canvas_system_fonts` → pass; its log line
  `system faces: N` shows N = 413 on this machine (est. 4 min; loading 413 faces is the
  point — a smaller sample would miss a bad face).
Commit: —

## Validation Plan

- Tests: `tests/canvas/rt_canvas_system_fonts.rs` (every listed face loads; known face
  present; no empty fields).
- Runtime proof: the same test runs a real headless `--app` binary.
- Doc sync: none public yet; `06_canvas.md` is updated in E.
- Final gate: plan-147-E.

## Open Decisions

- Variable fonts excluded entirely (recommended: the loader ignores `gvar`, so every
  instance would draw the default outlines) vs. listing only the default instance
  (needs axis-default comparison in CF dictionaries; revisit if variable-font rendering
  is ever added).

## Corrections

## Summary

New native surface limited to one enumeration emitter; risk is CF ownership and the
float argument, both covered by the load-every-face test.
