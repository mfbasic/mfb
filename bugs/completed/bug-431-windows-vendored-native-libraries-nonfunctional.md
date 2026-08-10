# bug-431: Windows drops `rpaths` — vendored native libraries are non-functional on Windows

Last updated: 2026-08-08
Effort: x-large (1d–3d)
Severity: HIGH
Class: Correctness

Status: FIXED (673f149da)
Regression Test: src/target/shared/code/link_thunk.rs (new Windows-init codegen test), src/target/win_x86_64/plan.rs (link_imports test), plus a Wine/Windows runtime proof

STATUS: FIXED (673f149da) — Windows vendored native `LINK` libraries now load and
call successfully. Three documented loader layers landed (Win32 loader in
`_mfb_linker_init`, kernel32/shlwapi `link_imports`, exe-relative `vendor/`
resolution) plus a fourth defect the runtime proof surfaced: the x86-64 LINK thunk
captured the native call result from the wrong register (aligned MFB bank instead
of the C-ABI `rax`), which broke *every* x86-64 native return (Linux and Windows),
latent because x86-64 LINK execution is not in the acceptance oracle. Proven on
box 2230 (Win11) and box 2227 (Alpine musl x86_64): a program that vendors
`sndfile.dll`/`.so` loads it from `vendor/` and `sf_version_string` returns
`libsndfile-1.2.2`. POSIX aarch64/riscv/macOS and non-vendoring Windows builds are
byte-identical. Deviation from Fix Design: used `PathRemoveFileSpecA` (shlwapi) +
`lstrcatA` to build the path (no hand-rolled backslash scan), and drove the
vendored-vs-system choice off the resolved locator's `LibType` rather than
threading `vendors_native_libraries`.

The presenting symptom — the one that prompted this bug — is that the Windows PE
linker **silently ignores `image.rpaths`**: `write_executable`'s
`vendors_native_libraries` parameter is `_`-prefixed and unused
(`src/target/win_x86_64/mod.rs:263`), and `image.rpaths` is never set for a Windows
build. But the ignore is only the visible tip: **native `LINK` bindings that vendor
a native library do not work on Windows at all.** A program that `LINK`s against a
vendored DLL (the user's two bindings, and libsndfile) will not resolve the library
at runtime on Windows, even though the manifest layer accepts Windows locators
(`src/manifest/libraries.rs:615`) and the build copies the DLLs into `build/vendor/`
(`src/cli/build/native_libs.rs:206`).

There are three linked defects: (1) the load-time initializer emits `dlopen`/`dlsym`
on every platform, but Windows has no `dlopen`; (2) Windows declares none of the
loader imports; (3) even with a loader, nothing tells the Windows DLL search order
to look in the exe-relative `vendor/` directory — which is what `rpaths`
(`$ORIGIN/vendor` / `@loader_path/vendor`) accomplishes on ELF/Mach-O.

The single correct behavior a fix produces: a Windows build that vendors a native
library loads it at runtime from the exe-relative `vendor/` directory and its
`LINK` symbols resolve and call successfully — the same end-to-end behavior Linux
and macOS already deliver. A build that vendors nothing stays byte-identical to
today.

References:

- `rpaths` data model: `src/arch/image.rs:52` — its own doc comment says it is
  "materialized as ELF `DT_RUNPATH` / Mach-O `LC_RPATH`" and is "loader-relative
  and per output shape" (`$ORIGIN/vendor`, `@loader_path/vendor`,
  `@executable_path/../Frameworks`) — **Windows has no representation here.**
- Linux emits `DT_RUNPATH` (`src/os/linux/link/elf.rs:824`, `runpath_string`
  `elf.rs:881`); macOS emits `LC_RPATH` (`src/os/macos/link/macho.rs:151`,
  `load_rpath` `commands.rs:362`).
- Vendor plumbing: `src/os/mod.rs:30` (`VENDOR_DIR`), `:34`/`:44` (the ELF/Mach-O
  vendor rpath tokens); `src/cli/build/native_libs.rs:206` (`vendor_output_dirs`,
  which already routes Windows console/app vendored DLLs to `build/vendor/`).
- LINK loader: `src/target/shared/code/link_thunk.rs:312` (`_mfb_linker_init`),
  `:340` (`dlopen(filename, RTLD_NOW)`), `:365` (`dlsym`).
- Spec: `src/docs/spec/language/17_native-libraries.md:544` (the rpath-per-shape
  table — **Windows row absent**); `src/docs/spec/linker/03_import-selection.md:88`
  (LINK resolved via `dlopen`/`dlsym`, not the import table).
- plan-46-{C,D} (the native-library vendoring / rpath design). Found alongside
  bug-432 (the parallel `signing_metadata` drop) during the Windows spec work.

## Failing Reproduction

End-to-end (needs a Windows or Wine runner):

```
# A binding package that vendors a DLL (e.g. libsndfile), imported by a program:
mfb build --target windows-x86_64 <project-using-a-vendored-LINK-binding>
# then run build/<name>.exe on Windows/Wine
```

- Observed: the program using a vendored `LINK` symbol fails — the vendored DLL is
  not found/loaded. At the codegen level the linker cannot even satisfy the loader:
  `_mfb_linker_init` emits `dlopen`/`dlsym`, which Windows does not import
  (`src/target/win_x86_64/plan.rs:570` `link_imports` → `Vec::new()`;
  `native_call_imports` → `Vec::new()`), so there is no `dlopen` symbol to bind.
- Expected: the DLL loads from `build/vendor/` and the `LINK` symbols resolve, as on
  Linux/macOS.

Codegen-level reproduction (no runner needed — the Phase-1 RED test):

- A `LINK`-using module lowered for `windows-x86_64` emits `_mfb_linker_init` calling
  `dlopen`/`dlsym` (via `link_thunk.rs:340`) rather than `LoadLibraryEx`/
  `GetProcAddress`, and `win_x86_64::link_imports` returns no loader imports.

Contrast (works today): `linux-x86_64` / `macos-aarch64` builds set `image.rpaths`
when `vendors_native_libraries`, emit `DT_RUNPATH`/`LC_RPATH`, import `dlopen`/`dlsym`
from libc/libSystem, and load the vendored library from `build/vendor/`.

## Root Cause

Three layers, all cited:

1. **The loader is POSIX-only.** `link_thunk::emit_linker_init`
   (`src/target/shared/code/link_thunk.rs:312`) unconditionally emits
   `dlopen(filename, RTLD_NOW)` (`:340`) then `dlsym(handle, symbol)` (`:365`) via
   `platform.emit_libc_call`. There is no platform branch for Windows, which has no
   `dlopen`.
2. **Windows declares no loader imports.** `src/target/win_x86_64/plan.rs:570`
   (`link_imports`) and `:566` (`native_call_imports`) both return `Vec::new()`, and
   grep finds **no** `LoadLibrary`/`GetProcAddress` anywhere under
   `src/target/win_x86_64/**` or `src/os/windows/**`. So even the `dlopen` symbol the
   initializer references is unresolvable.
3. **No DLL-search vehicle.** `image.rpaths` is the ELF/Mach-O mechanism for pointing
   the loader at the exe-relative `vendor/` dir; Windows has no rpath concept and the
   PE linker never reads `image.rpaths`. `src/target/win_x86_64/mod.rs:263` takes
   `_vendors_native_libraries` and ignores it, so nothing conveys "search `vendor/`"
   to the runtime. Meanwhile `vendor_output_dirs` (`native_libs.rs:249`) *does* copy
   the DLLs to `build/vendor/`, so the files are present but unreachable — the exe
   sits in `build/` and the default DLL search never looks in `vendor/`.

Linux/macOS are immune because all three layers exist there: libc/libSystem
`dlopen`, the per-target `link_imports`, and the `DT_RUNPATH`/`LC_RPATH` rpath.

## Goal

- A `windows-x86_64` build that vendors a native library resolves and calls its
  `LINK` symbols at runtime, loading the DLL from the exe-relative `vendor/`
  directory (matching `build/vendor/` where `vendor_output_dirs` places it).
- `_mfb_linker_init` on Windows uses a real Win32 loader (`LoadLibraryEx*` +
  `GetProcAddress`) instead of `dlopen`/`dlsym`.
- A build that vendors nothing (no `LINK`, or `system`-only locators) is
  byte-identical to today on Windows.
- The spec's native-libraries rpath table gains a Windows row describing the
  DLL-search mechanism.

### Non-goals (must NOT change)

- **Linux/macOS behavior** — the `dlopen`/`DT_RUNPATH`/`LC_RPATH` paths stay exactly
  as they are; this is additive for Windows only.
- **Do NOT route vendored `LINK` libraries through the PE `.idata` import table.**
  That would make the vendored DLL a hard load-time dependency (the process fails to
  start if it's missing), whereas the vendoring model is a *runtime* `dlopen`-style
  load whose failure is a catchable runtime error. Keep it a runtime
  `LoadLibraryEx`, not a static import. (`03_import-selection.md:88` documents that
  LINK is deliberately not an ordinary import.)
- **Non-vendoring builds stay byte-identical** — the loader init and any new imports
  appear only when `link_count > 0` / the build vendors something.
- Do NOT "fix" this by copying the DLLs beside the `.exe` and calling it done unless
  that is the chosen design (see Open Decisions) — the `vendor/` subdirectory model
  is the cross-platform convention and diverging needs a deliberate decision.
- Do NOT mask the codegen RED test by stubbing `dlopen` on Windows; the initializer
  must emit a real Win32 loader.

## Blast Radius

Found by grep for `rpaths`, `dlopen`/`dlsym`, `link_imports`, `vendor`:

- `src/target/shared/code/link_thunk.rs:312-402` (`emit_linker_init`) — **fixed by
  this bug**: add a Windows branch emitting `LoadLibraryEx*`/`GetProcAddress`. Affects
  every `LINK`-using program on Windows.
- `src/target/win_x86_64/plan.rs:570` (`link_imports`) and `:566`
  (`native_call_imports`) — **fixed**: declare the kernel32 loader imports
  (`LoadLibraryExA`/`W`, `GetProcAddress`, and whatever the vendor-dir resolution
  needs, e.g. `GetModuleFileNameW`).
- `src/target/win_x86_64/mod.rs:263` (`_vendors_native_libraries`) — **fixed**: wire
  it so the initializer includes the vendor-dir search.
- `image.rpaths` on Windows — **decision** (Open Decisions): keep empty and resolve
  the exe-relative `vendor/` at runtime, or repurpose the field. Whichever, the PE
  linker does not gain an rpath tag.
- `src/cli/build/native_libs.rs:206` (`vendor_output_dirs`) — **verify only**: it
  already copies Windows vendored DLLs to `build/vendor/`; confirm no change needed
  once the loader searches there.
- `src/docs/spec/language/17_native-libraries.md:544` (rpath table) and
  `src/docs/spec/linker/10_windows-x86_64.md` — **updated**: add the Windows
  DLL-search story.
- `src/manifest/libraries.rs:615` (windows locator slots) — **unaffected**: the
  manifest already accepts Windows locators; this bug makes the backend honor them.
- System-symbol imports (kernel32/ole32/etc. via `.idata`) — **unaffected**: those
  are ordinary PE imports, a separate mechanism from `LINK`.

## Fix Design

Give Windows the three missing layers:

1. **Win32 loader in `_mfb_linker_init`.** Branch on the platform in
   `emit_linker_init`: on Windows emit `LoadLibraryExA(source, NULL, flags)` →
   handle, then `GetProcAddress(handle, symbolName)` → slot, replacing the
   `dlopen`/`dlsym` pair. `source`/symbol names are ASCII bare filenames, so the
   `-A` variants avoid UTF-16 conversion. Declare these in
   `win_x86_64::link_imports` (kernel32).

2. **Point the loader at `vendor/`.** The Windows analog of `$ORIGIN/vendor` is not
   a link-time tag but a runtime search decision. Recommended: build the absolute
   path `<exe_dir>\vendor\<source>` at load time (`GetModuleFileNameW` → strip the
   filename → append `vendor\<source>`) and `LoadLibraryExA(abs, NULL,
   LOAD_WITH_ALTERED_SEARCH_PATH)`, so the DLL and its own dependencies resolve from
   `vendor/`. This needs no global search-order mutation and mirrors the
   loader-relative token model. (Alternatives in Open Decisions.)

3. **Wire `vendors_native_libraries`** through the Windows `write_executable` so the
   initializer only does the vendor-dir work when the build actually vendors.

Where the risk concentrates: the `link_thunk` platform branch (it is shared codegen
touched by all targets — the Windows arm must not perturb the Linux/macOS emit) and
the runtime correctness of the exe-relative path construction (a wrong path silently
fails to load). Rejected alternative: a `dlopen`/`dlsym` shim so `link_thunk` stays
uniform — rejected because `dlopen`'s `RTLD_*` flags and error/`dlerror` semantics
don't map cleanly onto `LoadLibraryEx`, and an explicit branch is clearer than a
faux-POSIX shim.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add a codegen test: a `LINK`-using module lowered for `windows-x86_64` emits a
      Win32-loader `_mfb_linker_init` (asserts `LoadLibraryEx*`/`GetProcAddress`, not
      `dlopen`); today it fails (emits `dlopen`). Add a `win_x86_64::link_imports`
      test expecting the loader imports (fails: currently empty).
      — `windows_link_initializer_uses_win32_loader_not_dlopen` (RED: `got calls:
      {dlsym, dlopen}`) + `link_imports_declare_the_win32_loader`.
- [x] Confirm the blast-radius verdicts above by grep; record any additional LINK
      call site. — audit surfaced a **fourth** defect (see below): the x86-64 LINK
      thunk captures the native call result from the wrong register.

Acceptance: both tests fail for the documented reason; audit complete.
Commit: 673f149da

### Phase 1b — fourth defect found by the audit (x86-64 native-call return)

The three documented layers are the *loader*. Runtime proof surfaced a fourth,
**pre-existing** defect that also blocked a working Windows call: the native LINK
marshaling thunk (`lower_link_thunk`) captured the C function's return value from
`abi::return_register()` — the aligned MFB result bank, which realizes to `rdi`
(SysV) / `rcx` (Win64) — instead of `abi::c_return(0)` (`rax`). On AArch64/RISC-V
and macOS those coincide (`x0`/`a0`), so those platforms worked; **every x86-64
native return** (Linux *and* Windows) surfaced a clobbered caller-saved register,
i.e. garbage/0. Latent because x86-64 LINK *execution* is not in the acceptance
oracle (only macOS aarch64 is). Verified pre-existing on box 2227 with the base
commit binary (`ptr=0, ver=[]`).

- [x] RED test `native_call_result_is_captured_from_c_return_register` (neutral
      operand-level, backend-independent).
- [x] Fix: capture `c_return(0)`. Byte-identical on aarch64/riscv/macOS (`x0`/`a0`
      == C return); corrects x86-64 (SysV + Win64).

Commit: 673f149da

### Phase 2 — the fix

- [x] `link_thunk.rs`: two new `CodegenPlatform` hooks `emit_link_dlopen` /
      `emit_link_dlsym` (POSIX defaults byte-identical to today); Windows override
      in `win_x86_64/code.rs` emits `LoadLibraryExA` + `GetProcAddress` with the
      exe-relative `vendor/` path resolution (`GetModuleFileNameA` +
      `PathRemoveFileSpecA` + `lstrcatA`, then
      `LOAD_WITH_ALTERED_SEARCH_PATH`). Per-library vendored flag from the resolved
      locator's `LibType`; `system` locators load by bare name.
- [x] `win_x86_64/plan.rs`: declare the kernel32/shlwapi loader imports in
      `link_imports` (`LoadLibraryExA`, `GetProcAddress`, `GetModuleFileNameA`,
      `lstrcatA`, `PathRemoveFileSpecA`). No `native_call_imports` change needed.
- [x] `vendors_native_libraries` threading was unnecessary: the per-library
      `LibType` (vendor vs system) is the precise signal, threaded through
      `emit_link_support`. The scratch/`\vendor\` data objects are emitted only
      when a Windows build actually vendors.
- [x] Verified `vendor_output_dirs` places the DLL at `build/vendor/<unit>-<name>`,
      which is exactly `<exe_dir>\vendor\` where the loader now looks.

Acceptance: Phase 1 tests pass; POSIX aarch64/riscv/macOS `.ncode` byte-identical
(proven via `-ncode` diff of a system-LINK fixture across all four POSIX targets
and a non-LINK Windows build); a non-vendoring Windows build is byte-identical.
Commit: 673f149da

### Phase 3 — runtime proof + docs + full validation

- [x] On box 2230 (Win11): a project vendoring `sndfile.dll` (self-contained, imports
      only kernel32) loads it from the exe-relative `vendor/` and `sf_version_string`
      returns `libsndfile-1.2.2`. Removing the DLL makes init fail with the
      "could not be loaded at startup" error — proving the loader really depends on
      the vendored copy. The same program on box 2227 (Alpine musl x86_64) confirms
      the return-capture fix (was `ver=[]`, now `libsndfile-1.2.2`).
- [x] Updated `17_native-libraries.md` (Windows row + DLL-search paragraph) and
      `10_windows-x86_64.md` (native `LINK`/vendored-DLL section); spec drift-guards
      (`spec_links_resolve`, `every_rule_is_documented_in_the_spec`) green.
- [x] `cargo test --bin mfb` (3789 ok) + `scripts/artifact-gate.sh all` — no golden
      shifts (no builtin byte-identity fixture uses LINK; native LINK fixtures carry
      only frontend + macOS-runtime goldens, both unchanged).

Acceptance: the vendored `LINK` program runs on Windows; full suite green;
non-vendoring goldens unchanged.
Commit: 673f149da

## Validation Plan

- Regression test(s): the `windows-x86_64` `_mfb_linker_init` codegen test + the
  `link_imports` test (both in the compiler crate).
- Runtime proof: a Windows/Wine run of a project that `LINK`s a vendored DLL from
  `build/vendor/` and calls into it (the user's two bindings / libsndfile are the
  real target).
- Doc sync: `17_native-libraries.md` rpath table (add Windows) + `10_windows-x86_64.md`.
- Full suite: `cargo test --bin mfb`, `scripts/artifact-gate.sh` (non-vendoring
  builds must stay byte-identical).

## Open Decisions

- **DLL-search mechanism** (§Fix Design step 2):
  - *Recommended:* absolute-path `LoadLibraryExA(<exe_dir>\vendor\<source>, NULL,
    LOAD_WITH_ALTERED_SEARCH_PATH)` — no global state, resolves the DLL's own deps
    from `vendor/`, keeps the `vendor/` subdirectory model.
  - *Alt A:* `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)` +
    `AddDllDirectory(<exe_dir>\vendor)` once in the initializer, then bare-name
    `LoadLibraryExA` — closest structural mirror of `$ORIGIN/vendor`, but mutates
    process-wide search order.
  - *Alt B:* copy vendored DLLs *beside* the `.exe` in `build/` (default search
    finds them, zero runtime code) — simplest, but diverges from the shared
    `vendor/` layout `vendor_output_dirs` produces and risks name collisions.
- **`image.rpaths` on Windows** — keep empty (runtime exe-relative resolution) vs.
  populate with a Windows token the initializer parses. Recommend keeping it empty;
  the vendor path is a runtime construction, not a link-time string.

## Summary

The engineering risk is in the shared `link_thunk` platform branch (must add a
Windows loader without perturbing the Linux/macOS emit) and the runtime correctness
of the exe-relative `vendor/` path. The `rpaths` "ignore" the user reported is real
but is the surface of a deeper gap: Windows has no `LINK` loader at all, so vendored
native libraries — the user's two bindings and libsndfile — cannot work until all
three layers land. Non-vendoring builds and the Linux/macOS backends are untouched.
