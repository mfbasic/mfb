# plan-147-B: System fonts on macOS (CoreText) and the public members

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-147-A

Prerequisites: see plan-147-A. Additionally: plan-147-A is archived
(`ls planning/plan-147-A-* 2>/dev/null` → nothing; `ls planning/completed/plan-147-A-*`
→ one file). If it is not, this letter cannot start, full stop.

This letter adds the internal runtime call **`canvas::systemFaces() AS List OF
canvas::SystemFace`**, implements it on macOS with CoreText, and adds the two public
members on top of it (moved here from plan-147-E — see Corrections):
`canvas::listSystemFonts() AS List OF String` and
`canvas::loadSystemFont(name AS String) AS canvas::Font`. `SystemFace` is an internal
record `{name AS String, postScript AS String, path AS String}`: the face's full name,
its PostScript name, and the file that holds it. Checkable outcome: on this Mac,
`len(canvas::listSystemFonts())` is 413 (the probe's count, §2), and
`canvas::loadSystemFont(n)` succeeds for every `n` in the list.

References: plan-147-A; `src/codegen/builtins/audio/gen_macos_shared.rs`
(`emit_cfstring_field`), `src/codegen/memory/marshal/record_list.rs`
(`emit_build_record_list`), `.ai/arch-abi.md` (macOS AArch64), `.ai/compiler.md`.

## 1. Goal

- `canvas::systemFaces()` on macos-aarch64 answers one `SystemFace` per installed face
  that (a) has a `glyf` table and (b) is not a variable font, in CoreText's order.
- Absent/odd entries (no URL, no name) are skipped, never fatal.
- `canvas::listSystemFonts()` — the `name` of every `systemFaces` entry, sorted (the
  `collections` string sort), duplicates removed.
- `canvas::loadSystemFont(name)` — among entries whose `name` equals `name` (exact,
  case-sensitive), the first after sorting by `path` then `postScript` (deterministic
  when two files carry one full name) is loaded with
  `canvas::loadFont(path, postScript)` — or `canvas::loadFont(path, name)` when the OS
  gave no PostScript name. None → `FAIL error(77050004, "no system font named: " &
  name)` (`ErrNotFound`). Both are `Body::mfb` in `func_system_fonts.rs`; no
  `Mode.Canvas` check (like `loadFont`).

### Non-goals

- `systemFaces` is `internal_only`; users reach it only through the two public members.
- Linux and Windows are C and D. Until they land, `canvas.systemFaces` is **not** in
  those targets' `RUNTIME_CALLS`, so a program using the public members fails the
  build for them with "native backend does not support runtime call" — a compile-time
  refusal, not a stub. This state lives only on the feature branch: nothing merges to
  main before plan-147-E.
- No fuzzy/family matching, no fallback or default font. Man pages and spec prose are
  E's.

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
- [ ] `func_system_fonts.rs`: public `listSystemFonts` and `loadSystemFont` (§1),
      `Body::mfb`, with a short working `intro`/`desc`/`example` (E polishes the prose
      against `.ai/man-content.md`).
- [ ] Tests: new `tests/canvas/rt_canvas_system_fonts.rs` (macOS-gated with
      `#[cfg(target_os = "macos")]`), headless `--app` programs asserting:
      the list is non-empty, sorted and duplicate-free and contains `Helvetica`;
      `loadSystemFont(n)` succeeds for **every** listed `n` (this is what catches a
      named instance or a CFF face slipping through the filters) and prints
      `system fonts: N loaded: N`; `loadSystemFont("Helvetica")` draws non-empty text;
      `loadSystemFont("No Such Font")` → 77050004.

Acceptance: every face listed on this Mac loads; an unknown name is `ErrNotFound`.
  Check: `cargo test --test rt_canvas_system_fonts` → pass; log line
  `system fonts: N loaded: N` with N = 413 on this machine (est. 4 min; loading every
  face is the point — a smaller sample would miss a bad face).
Commit: —

## Validation Plan

- Tests: `tests/canvas/rt_canvas_system_fonts.rs` (every listed face loads; known face
  present; sorted/unique; unknown name → `ErrNotFound`).
- Runtime proof: the same test runs a real headless `--app` binary.
- Doc sync: `06_canvas.md` and the final man prose are E's.
- Final gate: plan-147-E.

## Open Decisions

- Variable fonts excluded entirely (recommended: the loader ignores `gvar`, so every
  instance would draw the default outlines) vs. listing only the default instance
  (needs axis-default comparison in CF dictionaries; revisit if variable-font rendering
  is ever added).

## Corrections

- **The native result is one `String`, not a `List OF SystemFace`.** Reading
  `audio/gen_macos_devices.rs` showed a record list costs a frame of fixed slots, a
  per-record builder and a pair array per backend — three times over. Instead the
  internal call is **`canvas::systemFontTable() AS String`**: one record per face
  separated by U+001E (record separator), fields `name`, `postScript`, `path`
  separated by U+001F (unit separator) — control characters no font name or font path
  carries. The MFBASIC members split it. On macOS the table is assembled *inside
  CoreFoundation* (`CFStringCreateMutable` + `CFStringAppend`, path via
  `CFURLCopyFileSystemPath`) and converted to an MFBASIC String once
  (`CFStringGetLength` → `CFStringGetMaximumSizeForEncoding` → arena → `CFStringGetCString`
  → `strlen`), so no per-field buffers exist. Every mention of `systemFaces` /
  `SystemFace` / `emit_build_record_list` in B–E means this call now; its
  `CALLER_ARENA_BLOCK_RESULTS` row is `"canvas.systemFontTable", // String`.

- **The public members moved here from plan-147-E.** `systemFaces` is `internal_only`,
  and the resolver refuses internal members from test programs
  (`src/resolver/resolution.rs:1700`), so the plan's "tests call `systemFaces` /
  `loadFontFace` directly" was impossible. The public members are the only way a test
  can reach the backend, so they land with the first backend. `loadFontFace` itself was
  replaced by the public `canvas::loadFont(path, face)` overload (plan-147-A
  Corrections; user-approved).

## Summary

New native surface limited to one enumeration emitter; risk is CF ownership and the
float argument, both covered by the load-every-face test.
