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

**Overall: NOT complete.** §12 (user `LINK` bindings) is complete and
runtime-verified, including the boundary validations. The static-linker
generalization (Phases 1–5) is **not started** — it has no runtime consumer in
the codebase yet (no `tls` built-in, no app mode exists).

Implemented by commits `f4d4098f` ("Implement native LINK binding codegen
(plan-linker.md §12)") and the follow-up closing the §12.3/§12.4 boundary
validations.

---

## Half 1 — Static-linker generalization (§4–§11 / Phases 1–5): NOT STARTED

| Plan item | Phase | Status | Evidence |
|---|---|---|---|
| §5.1 `ImportKind` on plan/code/encoded import records | 1 | ❌ Not added | no `ImportKind` in `plan.rs`/`code/mod.rs`/`encode.rs` |
| §5.2 optional per-import `version` string | 1 | ❌ Not added | no `version` field on imports |
| §5.3 `EncodedImage.initializers` + error-if-unhonored | 1 | ❌ Not added | no `initializers` field |
| §6.3 / §7.1 macOS multiple `LC_LOAD_DYLIB` (library→path table) | 2 | ❌ Not added | `src/os/macos/link.rs` still gated on `imports_libsystem` |
| §7.2 per-symbol dylib ordinal in `bind_info` | 2 | ❌ Not added | bind opcodes still pin ordinal 1 |
| §7.3 framework path mapping (`Network`, AppKit…) | 2 | ❌ Not added | — |
| §6.2 Linux `verneed`/`versym` symbol versioning | 3 | ❌ Not added | no `.gnu.version*` emission |
| §6.1 Linux `GLOB_DAT` for imported data globals | 4 | ❌ Not added | only `JUMP_SLOT` emitted |
| §6.4 Linux `DT_INIT_ARRAY` / `.init_array` | 4 | ❌ Not added | — |
| §7.5 macOS `S_MOD_INIT_FUNC_POINTERS` | 4 | ❌ Not added | — |
| §7.4 imported data globals from multiple dylibs | 4 | ❌ Not added | — |
| §5 Phase 5 hardening (GOT/stub layout at GUI scale) | 5 | ❌ Not started | — |
| §9 goldens for new `kind`/`version`/`initializers` fields | — | N/A | fields not added |

**Driver/blocker.** Per plan §10 the driving consumer is `tls` (Phases 2–3) and
GUI app modes (Phase 4). Neither `tls` nor any app mode exists in the codebase, so
these phases currently have **no runtime consumer**; they can only be validated
against synthetic targets or after `tls` is built. Phase 4 (data globals +
initializers) is explicitly "gated on the GUI app plans and exercised by no
current built-in package" (plan §3.1, §10).

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

§12 is complete. The only remaining work is the static-linker generalization:

1. **Implement Phases 1–5 (large, no runtime consumer yet):**
   - Phase 1: `ImportKind` + `version` + `initializers` contract extensions and goldens.
   - Phase 2: macOS multi-dylib + per-symbol dylib ordinals + framework paths.
   - Phase 3: Linux `verneed`/`versym`.
   - Phase 4: `GLOB_DAT` + `init_array`/`S_MOD_INIT_FUNC_POINTERS`.
   - Phase 5: GOT/stub layout hardening at GUI-scale import counts.
   - Validation for Phases 2–4 realistically requires first landing the `tls`
     built-in (OpenSSL 3 on Linux, `Network.framework` on macOS) as the concrete
     driver named in the plan.
