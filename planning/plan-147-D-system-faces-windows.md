# plan-147-D: System faces on Windows (DirectWrite)

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-147-C

Prerequisites: see plan-147-A. Additionally plan-147-C is archived
(`ls planning/completed/plan-147-C-*` → one file), and the Windows box answers:
`ssh -p 2230 test@127.0.0.1 'ver'` → a Windows version line. If either fails, this
letter cannot start, full stop.

Implements `canvas::systemFaces()` on windows-x86_64 with DirectWrite's system font
collection through COM. Checkable outcome: on box 2230 a headless `--app` program prints
a positive face count and `canvas::loadSystemFont` succeeds for every listed name.

References: plan-147-A/B/C; `src/codegen/builtins/audio/gen_windows.rs` (`com_call`,
`ole_call`), `src/codegen/builtins/audio/gen_windows_devices.rs` (COM enumeration →
record list), `src/target/win_x86_64/plan.rs`, `src/target/win_x86_64/code.rs`
(`emit_wide_to_utf8`), `.ai/arch-abi.md` (Win64), mingw-w64 `dwrite.h`.

## 1. Goal

- `canvas::systemFaces()` on windows-x86_64 answers one `SystemFace` per font in
  `IDWriteFactory::GetSystemFontCollection` that: has no simulations
  (`IDWriteFont::GetSimulations = DWRITE_FONT_SIMULATIONS_NONE`), whose face has a
  `glyf` table (`IDWriteFontFace::TryGetFontTable('glyf')` exists), has no variations
  (`IDWriteFontFace5::HasVariations` is FALSE when that interface is available), and
  is backed by a local file (`IDWriteLocalFontFileLoader`). `name` = informational
  string FULL_NAME, `postScript` = POSTSCRIPT_NAME (each: `en-us` locale if present,
  else index 0), `path` = the local file path.
- Names are converted UTF-16 → UTF-8 losslessly (not the ASCII-only
  `emit_string_from_wstr` in `gen_windows_devices.rs`, which drops the high byte).
- Any COM failure before enumeration → empty list; a failure on one font skips it.

### Non-goals

- No custom font collections, no font-set (`IDWriteFactory3`) API.
- No change to the WASAPI audio code beyond lifting a shared COM-call helper, if lifted.

## 2. Current State

- COM is already used: `ole32` imports `CoInitializeEx`/`CoCreateInstance`
  (`win_x86_64/plan.rs`, audio arms); vtable calls via `com_call(slot, n_args, …)` in
  `audio/gen_windows.rs`, private and bound to audio's frame (`OBJ_OFF`).
- No DirectWrite reference (`grep -rn "DWrite\|IDWrite" src | wc -l` → 0).
- UTF-16 → UTF-8: `emit_wide_to_utf8` and friends in `win_x86_64/code.rs` are private,
  frame-bound. The target is therefore expected to grow a platform-seam method (like
  `emit_opendir`) that converts a wide buffer to an MFBASIC String, or the conversion is
  done in the emitter via an imported `WideCharToMultiByte` (already imported for
  `fs.listDirectory`).
- `DWriteCreateFactory` is the one flat export of `dwrite.dll`; every other call is a
  vtable slot.

### Verified properties (to be filled from `dwrite.h` in Phase 1)

UNVERIFIED — each is a Phase 1 task, recorded here with the header line it came from:
vtable slot numbers for `IDWriteFactory::GetSystemFontCollection`,
`IDWriteFontCollection::{GetFontFamilyCount, GetFontFamily}`,
`IDWriteFontList::{GetFontCount, GetFont}` (IDWriteFontFamily inherits IDWriteFontList),
`IDWriteFont::{GetSimulations, GetInformationalStrings, CreateFontFace}`,
`IDWriteFontFace::{GetFiles, TryGetFontTable, ReleaseFontTable}`,
`IDWriteFontFile::{GetReferenceKey, GetLoader}`,
`IDWriteLocalFontFileLoader::{GetFilePathLengthFromKey, GetFilePathFromKey}`,
`IDWriteLocalizedStrings::{FindLocaleName, GetStringLength, GetString}`,
`IDWriteFontFace5::HasVariations`; the IIDs of `IDWriteFactory`,
`IDWriteLocalFontFileLoader`, `IDWriteFontFace5`; and the enum values
`DWRITE_INFORMATIONAL_STRING_FULL_NAME` / `_POSTSCRIPT_NAME`.

## 3. Design

- Lift `com_call`/`ole_call` into a frame-parameterised helper in
  `src/codegen/os/ffi/` (object slot passed in), point audio at it, and use it here — one
  COM call helper, not two.
- `gen_system_faces_windows.rs`: `DWriteCreateFactory(SHARED, &IID_IDWriteFactory,
  &factory)` → collection → families → fonts, applying §1's filters, collecting
  element records, then `emit_build_record_list`. Every interface obtained is
  `Release`d (slot 2) on every path.
- `win_x86_64/plan.rs:runtime_imports`: `"canvas.systemFaces"` arm with
  `DWriteCreateFactory` from `dwrite.dll` (new DLL constant beside `OLE32`),
  `WideCharToMultiByte` from kernel32, and the arena imports.
- `win_x86_64/mod.rs:RUNTIME_CALLS`: add the call. Delete B's internal
  `Err("… has no backend")` arm — every family now has one.

**Correctness risk**: vtable slot numbers (a wrong slot calls the wrong method) and
Win64 shadow space / stack arguments for >4-arg methods (`TryGetFontTable` has 6).
Mitigation: every slot constant cites its `dwrite.h` line; the runtime test loads every
listed face. **Expected golden diffs**: audio fixtures' Windows `.ncodesum` only if the
COM helper lift changes emitted bytes — it must not; a diff there is a bug to fix.

**Rejected**: registry scan of `HKLM\…\Fonts` (misses per-user fonts and gives no
PostScript name); GDI `EnumFontFamiliesEx` (no file paths).

## Phases

> **NOTE — keep the checkboxes current as you go. An unticked box means NOT DONE.**

### Phase 1 — Constants and the shared COM helper

- [ ] Record every slot/IID/enum of §2 as named constants with `dwrite.h` citations.
- [ ] Lift the COM call helper; audio uses it.
- [ ] Check: audio Windows goldens unchanged — `cargo test --test acceptance audio`
      (name via `rg -l 'IMPORT audio' tests/syntax`) → pass (est. 5 min).

### Phase 2 — Emitter, plan, capability, runtime proof

- [ ] `gen_system_faces_windows.rs`, plan arm, capability entry; delete the
      no-backend arm.
- [ ] Tests: `rt_canvas_system_fonts.rs` cross-build for `windows-x86_64` succeeds.
- [ ] Runtime proof on 2230 via `scripts/test-winapp.sh`-style shipping
      (`win_ship` in `scripts/remote-common.sh`), `MFB_WINAPP_HEADLESS=1`: list then
      load every face; record count in Corrections.

Acceptance: positive count on Win11; every listed face loads; `Arial` present.
  Check: `ssh -p 2230 … probe.exe` → `faces=N loaded=N`, N > 0, and `Arial` in the
  list (est. 5 min).
Commit: —

## Validation Plan

- Tests: cross-build test; remote run on 2230; audio goldens unchanged after the lift.
- Final gate: plan-147-E.

## Open Decisions

- Locale for names — `en-us` then index 0 (recommended; matches the English names
  macOS/fontconfig report) vs. the user's locale (list would differ by machine language).

## Corrections

## Summary

The only new Windows surface is one COM enumeration; the risk is slot numbers, pinned to
header citations and exercised by loading every listed face.
