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
and `canvas::loadSystemFont` succeeds for every name `canvas::listSystemFonts` returns.

References: plan-147-A/B; `src/codegen/builtins/audio/gen_alsa_shared.rs`
(`emit_dlopen`, `emit_dlsym`, `emit_call_fnptr`), `src/codegen/builtins/audio/gen_alsa_devices.rs`
(list of records from a dlopened library), `src/target/linux_common/plan.rs`,
`.ai/arch-abi.md` (x86-64 SysV), `.ai/remote_systems.md`.

## 1. Goal

- `canvas::systemFaces()` on linux-x86_64 / linux-aarch64 answers one `SystemFace` per
  fontconfig pattern with `fontformat = "TrueType"`, `variable = false`, and
  `index >> 16 = 0` (not a named instance), having a `fullname` and a `file`.
  `postScript` is `postscriptname` when present, else `""` (`loadSystemFont` then
  passes the full name to `canvas::loadFont(path, face)`).
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

- [x] ~~Resolve the `FcConfigDestroy` question (§2)~~ — moot: the emitter calls
      `FcFontList(NULL, …)`, which lists against fontconfig's *current* configuration
      (loading it on first use), so no config is created and none is destroyed. That
      also reuses the configuration a GTK app has already loaded.
- [x] `gen_system_fonts_linux.rs` (§3) and the `PlatformFamily::Linux` arm in
      `func_system_fonts.rs`; the call helpers shared with macOS moved to
      `gen_system_fonts_shared.rs` (macOS `.app.ncodesum` unchanged after the move —
      `/tmp/p147_regen_app.sh` → `same macos-aarch64 ncodesum`).
- [x] `linux_common/plan.rs` imports (`dlopen`, `dlsym`, `strlen`, `strcmp`, `memcpy`
      from libc); `linux_common/mod.rs:RUNTIME_CALLS` entry.
- [x] Tests: `tests/canvas/rt_canvas_system_fonts.rs`
      `linux_reaches_fontconfig_through_dlopen_not_a_link` — `mfb build -app -target
      linux-x86_64|linux-aarch64 -nplan` succeeds, the plan imports `dlopen`/`dlsym` and
      names no fontconfig library (the nplan's import table *is* what becomes
      `DT_NEEDED`; see Corrections).
- [x] Restore the Linux canvas-app build broken since plan-147-B (every canvas app
      reaches `canvas.systemFontTable`): `mfb build -ncode -target linux-x86_64 --app`
      and `-target linux-aarch64` of `tests/syntax/app/app-mouse-surface` succeed, and
      their `.app.ncodesum` goldens are regenerated (the target-shared `.ir` already
      shows the diff is only the plan-147 members). Re-check → `same` for both.

Acceptance: builds for both app-capable Linux targets, no fontconfig `DT_NEEDED`.
  Check: `cargo test --test rt_canvas_system_fonts linux` → pass (est. 3 min).
  **Observed: `test result: ok. 1 passed` (135.79 s).**
Commit: c73794420

### Phase 2 — Runtime proof on 2228

- [x] Ship the x86_64 build of the probe program (list, then load every face) to 2228;
      run headless (`MFB_GTKAPP_HEADLESS=1`). Shipped with `scp` plus a runner script
      (`/tmp/p147-run.sh`), not `scripts/remote-common.sh` — one program, one run.
- [x] Record count and result in Corrections.

Acceptance: positive count; every listed face loads.
  Check: `ssh -p 2228 … ./probe` → `faces=N loaded=N`, N > 0 (est. 5 min; emulated
  box — this is the only fontconfig-bearing app box available).
  **Observed: `exit=0 seconds=281`, `faces=2429 loaded=2429`, `unknown: 77050004`.**
Commit: c73794420

## Validation Plan

- Tests: cross-build + no-DT_NEEDED test; remote run on 2228.
- Final gate: plan-147-E.

## Open Decisions

- Absent fontconfig → empty list (recommended, agreed with the user) vs. an error.

## Corrections

- **Runtime proof (2228, Ubuntu x86_64 GTK, fontconfig 2.15.0, 2026-09-21):** the
  glibc AppImage of the smoke program listed **2429** faces and loaded **all 2429**;
  `loadSystemFont("No Such Font")` → `77050004`; 281 s wall on the emulated box.
- **No aarch64 runtime proof.** The app-capable aarch64 boxes (2224 musl GTK, 2226
  glibc GTK) refused connections on 2026-09-21; 2223 has no GTK. linux-aarch64 is
  covered by the cross-build plan test and the `.app.ncodesum` golden; its emitter is
  the same shared Linux code as x86_64, which differs only in ABI lowering already
  exercised by every other dlopen'd call (ALSA, Vulkan).
- **The DT_NEEDED check reads the nplan, not `readelf`.** There is no Vulkan test that
  inspects `DT_NEEDED` (`rg -n libvulkan tests/` → nothing). The plan's import table is
  the source of the ELF's dynamic section, so "no fontconfig library in the nplan"
  is the same fact one step earlier and needs no Linux tooling on the macOS host.

## Summary

Risk is ABI detail of calling fontconfig through pointers; covered by loading every
listed face on a real glibc x86_64 GTK box.
