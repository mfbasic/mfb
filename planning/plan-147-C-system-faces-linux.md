# plan-147-C: System faces on Linux (fontconfig via dlopen)

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-147-B

Prerequisites: see plan-147-A. Additionally plan-147-B is archived
(`ls planning/completed/plan-147-B-*` → one file), and a GTK Linux box with fontconfig
answers: `ssh -p 2228 test@127.0.0.1 'fc-list | wc -l'` → a positive count (2628 on
2026-09-21). If either fails, this letter cannot start, full stop.

Implements `canvas::systemFaces()` on the Linux targets with fontconfig, loaded at run
time through `dlopen("libfontconfig.so.1")`/`dlsym` — never a `DT_NEEDED`, so a canvas
program still starts on a machine without fontconfig; there the list is **empty**.
Checkable outcome: on box 2228 a headless `--app` program prints a positive face count,
and every listed face loads through `canvas::loadFontFace`.

References: plan-147-A/B; `src/codegen/builtins/audio/gen_alsa_shared.rs`
(`emit_dlopen`, `emit_dlsym`, `emit_call_fnptr`), `src/codegen/builtins/audio/gen_alsa_devices.rs`
(list of records from a dlopened library), `src/target/linux_common/plan.rs`,
`.ai/arch-abi.md` (x86-64 SysV), `.ai/remote_systems.md`.

## 1. Goal

- `canvas::systemFaces()` on linux-x86_64 / linux-aarch64 answers one `SystemFace` per
  fontconfig pattern with `fontformat = "TrueType"`, `variable = false`, and
  `index >> 16 = 0` (not a named instance), having a `fullname` and a `file`.
  `postScript` is `postscriptname` when present, else `""` (A's `loadFontFace` then
  matches by full name).
- Library absent, or any fontconfig call returning NULL → empty list, no error.

### Non-goals

- linux-riscv64 has no app mode (`src/target/linux_riscv64/plan.rs` app entry is
  `unimplemented!`); the shared `linux_common::RUNTIME_CALLS` entry makes the call
  *compile* there, and nothing can run it. No riscv64-specific work.
- No variadic calls: `FcObjectSetBuild` is avoided in favour of `FcObjectSetCreate` +
  `FcObjectSetAdd`.
- Windows is D.

## 2. Current State

- Shared Linux backend: `linux_common::code::Platform<A>` and `LinuxPlan` serve all three
  Linux targets; `linux_common::RUNTIME_CALLS` (`src/target/linux_common/mod.rs`) is the
  one capability list.
- dlopen precedents: ALSA (`audio/gen_alsa_shared.rs`, absent library → error),
  Vulkan (`runtime/canvas/vulkan.rs`, absent → FALSE). The plan imports only `dlopen`/
  `dlsym` from libc (`linux_common/plan.rs`, the Vulkan arm).
- No fontconfig reference exists (`grep -rln "fontconfig\|FcInit" src | wc -l` → 0).
- Box 2228 (Ubuntu x86_64 GTK, emulated) has `/usr/lib/x86_64-linux-gnu/libfontconfig.so.1`
  and `fc-list | wc -l` → 2628 (probe 2026-09-21). Boxes 2224/2225/2226 refused
  connections that day. 2228 is the only app-capable box with fontconfig confirmed;
  it is x86_64 and emulated, so runs there are kept to the one test.

### Fontconfig ABI used

| Call | Signature |
|---|---|
| `FcInitLoadConfigAndFonts` | `FcConfig *(void)` |
| `FcPatternCreate` | `FcPattern *(void)` |
| `FcObjectSetCreate` / `FcObjectSetAdd` | `FcObjectSet *(void)` / `FcBool (FcObjectSet*, const char*)` |
| `FcFontList` | `FcFontSet *(FcConfig*, FcPattern*, FcObjectSet*)` |
| `FcPatternGetString` | `FcResult (FcPattern*, const char*, int, FcChar8**)` |
| `FcPatternGetInteger` / `FcPatternGetBool` | `FcResult (FcPattern*, const char*, int, int*)` |
| `FcFontSetDestroy` / `FcObjectSetDestroy` / `FcPatternDestroy` / `FcConfigDestroy` | `void (…*)` |

`FcFontSet` is `{int nfont; int sfont; FcPattern **fonts;}` — `fonts` at offset 8 on
both LP64 targets. `FcResultMatch = 0`. Object names: `"fullname"`, `"postscriptname"`,
`"file"`, `"fontformat"`, `"variable"`, `"index"`. Strings returned by
`FcPatternGetString` are owned by the pattern (copy before `FcFontSetDestroy`).
UNVERIFIED: that `FcConfigDestroy` on the config from `FcInitLoadConfigAndFonts` is safe
while GTK holds its own config — Phase 1 checks fontconfig's docs/source and records it.

## 3. Design

- `gen_system_faces_linux.rs`, an arm of `lower_system_faces`'s `platform.family()`
  match. dlopen/dlsym helpers reused from `audio/gen_alsa_shared.rs` if they are
  frame-independent; otherwise lifted to a shared `src/codegen/os/ffi/` helper and both
  callers pointed at it (no copy-paste of a third dlopen).
- Two passes over the `FcFontSet` (count qualifying, then fill) — the set is in memory,
  so the second pass is cheap; then `emit_build_record_list`.
- `linux_common/plan.rs:runtime_imports`: `"canvas.systemFaces"` arm importing `dlopen`,
  `dlsym`, `strlen` (and the arena imports the record builder needs) from libc.

**Correctness risk**: x86-64 SysV call alignment/`al` rules through function pointers,
and copying pattern-owned strings before destroy. **Expected golden diffs**: none.

**Rejected**: `DT_NEEDED` on libfontconfig (a canvas binary would fail to start without
it — the Vulkan rule); parsing `fonts.conf` and scanning directories (re-implements
fontconfig).

## Phases

> **NOTE — keep the checkboxes current as you go. An unticked box means NOT DONE.**

### Phase 1 — Emitter, plan, capability

- [ ] Resolve the `FcConfigDestroy` question (§2) and record it in Corrections.
- [ ] `gen_system_faces_linux.rs` (§3); the linux arm in `func_system_faces.rs`.
- [ ] `linux_common/plan.rs` imports; `linux_common/mod.rs:RUNTIME_CALLS` entry.
- [ ] Tests: `tests/canvas/rt_canvas_system_fonts.rs` gains a cross-build assertion
      that `mfb build --app --target linux-x86_64` and `linux-aarch64` of the probe
      program succeed and the binary has no `DT_NEEDED` on libfontconfig
      (`readelf -d`/object inspection helper already used by Vulkan tests —
      `rg -n 'libvulkan' tests/` names it).

Acceptance: builds for both app-capable Linux targets, no fontconfig `DT_NEEDED`.
  Check: `cargo test --test rt_canvas_system_fonts linux` → pass (est. 3 min).
Commit: —

### Phase 2 — Runtime proof on 2228

- [ ] Ship the x86_64 build of the probe program (list, then load every face) to 2228
      with `scripts/remote-common.sh` helpers; run headless (`MFB_GTKAPP_HEADLESS=1`).
- [ ] Record count and result in Corrections.

Acceptance: positive count; every listed face loads.
  Check: `ssh -p 2228 … ./probe` → `faces=N loaded=N`, N > 0 (est. 5 min; emulated
  box — this is the only fontconfig-bearing app box available).
Commit: —

## Validation Plan

- Tests: cross-build + no-DT_NEEDED test; remote run on 2228.
- Final gate: plan-147-E.

## Open Decisions

- Absent fontconfig → empty list (recommended, agreed with the user) vs. an error.

## Corrections

## Summary

Risk is ABI detail of calling fontconfig through pointers; covered by loading every
listed face on a real glibc x86_64 GTK box.
