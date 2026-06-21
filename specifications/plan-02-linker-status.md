# plan-linker.md — Implementation Status

Last verified: 2026-06-21

This document tracks how much of `specifications/plan-linker.md` is implemented
in the compiler. The plan has **two independent halves**:

1. **§4–§11 / Phases 1–5 — static-linker generalization.** The plan's primary
   subject (multi-library Mach-O, ELF symbol versioning, imported data globals,
   load-time initializers). Serves the `tls` built-in and the GUI app modes.
2. **§12 — user `LINK` bindings.** A layered sub-plan that resolves user
   binding packages (e.g. `bindings/sqlite`) at run time through `dlopen`/`dlsym`
   plus generated MFB↔C marshaling thunks. It "consumes none of the static-linker
   machinery" (plan §12 intro).

**Overall: nearly complete.** §12 (user `LINK` bindings) is complete and
runtime-verified. The static-linker generalization (Phases 1–4) is now
implemented and runtime-validated against real loaders, with two documented
caveats (glibc main-exe `init_array`; macOS mod-init). Phase 5 (GUI-scale
hardening) is the only untouched phase and has no consumer.

Validation hosts: macOS arm64 (Mach-O, executed locally) and real aarch64 Linux
(Arch/glibc, Alpine/musl over SSH; also Debian/glibc via podman) for ELF.

---

## Half 1 — Static-linker generalization (§4–§11 / Phases 1–5)

| Plan item | Phase | Status | Evidence |
|---|---|---|---|
| §5.1 `ImportKind` on the encoded contract | 1 | ✅ Done | `EncodedImport.kind` (`encode.rs`) |
| §5.2 optional per-import `version` string | 1 | ✅ Done | `EncodedImport.version` |
| §5.3 `EncodedImage.initializers` | 1 | ✅ Done | `EncodedImage.initializers` |
| §6.3 / §7.1 macOS multiple `LC_LOAD_DYLIB` (library→path table) | 2 | ✅ Done | `import_libraries`/`dylib_path` replace `imports_libsystem` |
| §7.2 per-symbol dylib ordinal in `bind_info` | 2 | ✅ Done | IMM ≤15 / ULEB above; runtime-validated |
| §7.3 framework path mapping (`Network`, AppKit…) | 2 | ✅ Done | library→path table incl. Network.framework |
| §6.2 Linux `verneed`/`versym` symbol versioning | 3 | ✅ Done | `.gnu.version`/`.gnu.version_r` + DT_VERSYM/VERNEED/VERNEEDNUM; runtime-validated |
| §6.1 Linux `GLOB_DAT` for imported data globals | 4 | ✅ Done | GOT slot + GLOB_DAT; runtime-validated (`environ`) |
| §6.4 Linux `DT_INIT_ARRAY` / `.init_array` | 4 | ⚠️ Emitted | `readelf`-verified; glibc runs the **main exe's** init_array from the CRT, not `ld.so`, so a custom-entry binary does not invoke it at load |
| §7.4 imported data globals from dylibs (macOS) | 4 | ✅ Done | already supported via `external` page21/pageoff12 + GOT |
| §7.5 macOS `S_MOD_INIT_FUNC_POINTERS` | 4 | ❌ Deferred | needs a writable `__DATA` segment + REBASE opcode stream added to the byte-fragile Mach-O emitter; high regression risk for a feature with no consumer |
| §5 Phase 5 hardening (GOT/stub layout at GUI scale) | 5 | ❌ Not started | no GUI-scale consumer yet |

### Runtime validation (Phases 2–4)

- **Phase 2 (macOS multi-dylib).** `os::macos::link` test
  `links_and_runs_program_importing_from_two_dylibs` links and *executes* a
  program importing `_exit` (libSystem, ordinal 1) and `nw_path_monitor_create`
  (**Network.framework**, ordinal 2) — a wrong ordinal/missing `LC_LOAD_DYLIB`
  makes dyld fail to bind at launch, so a clean exit proves it.
- **Phase 3 (Linux versioning).** A glibc ELF requiring `_exit@GLIBC_2.17` runs
  on real Arch Linux and Debian glibc; `LD_DEBUG=versions` shows the loader
  checking `GLIBC_2.17 ... required by` the binary.
- **Phase 4 (Linux GLOB_DAT).** A program reads the libc `environ` data global
  through the GOT/GLOB_DAT and exits 0 on real Arch glibc.

**Driver.** Validation uses the real `tls` driver library where applicable
(Network.framework on macOS) and real glibc/musl loaders on Linux, per plan §9's
"not synthetic stubs" requirement. The full `tls` socket *runtime* (async
`nw_connection` ↔ blocking bridge, libdispatch, Objective-C block-literal codegen)
is explicitly "`tls` codegen, not linker" (plan §7.3) and remains separate work.

---

## Half 2 — §12 user `LINK` bindings: COMPLETE

Implemented in `src/target/shared/code/link_thunk.rs` plus threading through
`ir.rs` → `nir.rs` → `plan.rs` → `code/mod.rs`, package serialization in
`binary_repr.rs`/`ir.rs`, and supporting fixes in `typecheck.rs`/`monomorph.rs`.

| Plan item | Status | Notes |
|---|---|---|
| §12.1 `dlopen`/`dlsym` pre-main initializer (`_mfb_linker_init`); startup abort on failure (`ErrNativeBindingUnavailable` 77030007) | ✅ Done | resolves into reserved writable global slots |
| §12.2 marshaling thunk (`_mfb_linker_<alias>_<name>`): in-marshal, `CONST` pins, `OUT` allocation, call-via-pointer, out-marshal, `SUCCESS_ON`, `RESULT`, error propagation (`ErrNativeBindingCallFailed` 77030008) | ✅ Done | |
| §12.3 type mapping `CInt64`/`CPtr`/`CString`(in)/`CBool`/`CByte`/`CInt32`/`CDouble` pass | ✅ Done | |
| §12.3 `CInt32` 64→32 range-check on input | ✅ Done | out-of-range Integer fails with `ErrOverflow` (77050010) |
| §12.3 `CDouble` NaN/Inf rejection on output | ✅ Done | non-finite `double` return fails with `ErrFloatNaN`/`ErrFloatInf` |
| §12.4 `CString` return copy-and-leave + NULL → empty `String` | ✅ Done | |
| §12.4 UTF-8 validation of returned bytes | ✅ Done | invalid bytes fail with `ErrEncoding` (77020004) |

### §12 verification (runtime)

- `tests/native-link-sqlite-rt` — inline `LINK` block, full CRUD against an
  in-memory SQLite db (`create/exec/prepare/bindText/step/columnText/finalize/
  close`) → prints `1=alice`, `2=bob`.
- `tests/native-link-import-sqlite-rt` — imports the `bindings/sqlite` package and
  runs `create/createTable/query(insert+select)/listTables/close` → prints
  `1=alice`, `2=bob`, `table: users`.
- `scripts/test-accept.sh` passes; `cargo test` passes (75 tests).

### Supporting work delivered alongside §12 (not in the plan, but required to make
the imported binding usable)

- Cross-package resource type identity (`typecheck.rs`).
- Native `LINK` resource types treated as scalar `CPtr` handles, not 0-field
  records (`TypeModel::from_module_and_packages`).
- Imported overloaded-function resolution (`monomorph.rs`).
- Pre-existing nested-generic fix: `binary_repr.rs type_id` greedy
  `trim_start_matches("List OF ")` → `strip_prefix` (unblocks `query`'s
  `List OF List OF String` result).

---

## Remaining work to reach 100%

§12 and Phases 1–4 of the static-linker generalization are done and
runtime-validated. Remaining:

1. **macOS `S_MOD_INIT_FUNC_POINTERS` (§7.5).** Add a writable `__DATA` segment
   with a `__mod_init_func` section plus a REBASE opcode stream so dyld slides
   and runs the initializer pointers. Deferred: it modifies the byte-fragile
   Mach-O emitter for a feature no current package consumes (dyld *does* run the
   main exe's mod-init, so it is validatable on macOS when implemented).
2. **glibc main-exe `init_array` invocation (§6.4).** The array is emitted; to
   have it run at load, MFB's custom-entry binaries would either go through the
   CRT or have the generated entry call the listed initializers directly (the
   existing `_mfb_linker_init` entry-call pattern) — the loader-independent,
   cross-platform way to honor `initializers`.
3. **Phase 5 hardening.** Re-validate GOT/stub layout at GUI-scale import counts
   once a GUI app mode exists to drive it.
4. **Compiler-side threading.** `ImportKind`/`version`/`initializers` are present
   on the encoded contract and consumed by the linkers; threading them from a
   real producer (`PlatformImport`→`CodeImport`) is wired when the `tls` built-in
   or an app mode emits versioned/data/init imports.
