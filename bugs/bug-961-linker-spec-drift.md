# bug-961: `mfb spec linker` drift — wrong macOS vendor rpath, wrong ELF stub registers, stale object-plan relocation kinds, dangling citations

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Open
Regression Test: none (spec text); item 6 needs a test if the owner rules it a code bug

Spec-vs-code mismatches found while checking every linker topic against emitted binaries
(Mach-O via `otool`/python parser, ELF via `readelf`/`objdump` in a Debian container, PE via a
python parser + `llvm-readobj-14`). Items 1–5 are SPEC bugs (code is right); item 6 is UNCLEAR.

References: `src/docs/spec/linker/*.md`

## Items

1. **macOS console vendor rpath is `@loader_path/vendor`, not `@executable_path/vendor`.**
   `src/docs/spec/linker/06_macos-aarch64.md` ("Bundle generation contract"): "Vendored libraries
   also select different `rpaths`: `@executable_path/vendor` for console output".
   Measured (`vend` project with a vendored `libfoo.dylib`, `mfb build vend`):
   `otool -l build/vend.out` → `cmd LC_RPATH … path @loader_path/vendor`; program prints `add=1005`
   from `/tmp` cwd. Code: `src/cli/build/native_libs.rs` table (`macos console | @loader_path/vendor`);
   `src/docs/spec/language/17_native-libraries.md` already says `@loader_path/vendor`.
   Fix: spec line → `@loader_path/vendor`.

2. **Linux AArch64 import stub uses `x17`, not `x16`, for the load/branch.**
   `src/docs/spec/linker/04_symbols-and-relocations.md` ("Import stubs and the GOT"): "On AArch64 a
   stub contains: `adrp x16, <GOT page>` / `ldr x16, [x16, <GOT offset>]` / `br x16`".
   Measured (`objdump -D -b binary -m aarch64 hello-glibc.out` at the stub area):
   `adrp x16, 0x5000` / `ldr x17, [x16, #488]` / `br x17`. Mach-O stubs are `ldr x16`/`br x16`
   (`otool -v -s __TEXT __stubs`). Code: `src/os/linux/link/mod.rs:emit_import_stub` emits
   `0xf940_0211` (ldr x17) and `0xd61f_0220` (br x17) — its own comment
   (`// aarch64: adrp x16, GOT_page; ldr x16, [x16, GOT_off]; br x16.`) is stale too.
   Fix: spec says Mach-O uses x16/x16, ELF uses x16/x17; fix the code comment.

3. **Object-plan relocation kinds do not match what is emitted.**
   `src/docs/spec/linker/02_object-plan.md` ("Shape"): "`Local` and `Runtime` become
   `internalCall`, and `Import` becomes `packageCall` (native `LINK` thunk calls)… Data
   references and the `_main` error-message references are recorded as `dataReference`".
   Measured (`mfb build --nobj` of a project with a local `LINK` block plus two imported LINK
   binding packages, macOS / linux-riscv64 / windows): every LINK thunk call is `internalCall`
   (consistent with `03_import-selection.md`: "A call to a `LINK` function is a
   `CallKind::Local`"), no `packageCall` appears; the macOS plan additionally carries one
   `externalCall` relocation per platform import (`_main → _exit`, `_mfb_linker_init → _dlopen`,
   …) — a kind the spec never names — while the Linux and Windows plans carry none.
   Code: `src/os/macos/object.rs:relocations` (the `plan.platform_imports` loop pushes
   `externalCall`); `src/os/linux/object.rs:relocations` has no such loop.
   Also in the same Shape block: the JSON keys are camelCase (`imageBase`, `loadCommands`,
   `codeUnits`, `dataUnits`, `definedSymbols`, `importedSymbols`, `externalSymbols`,
   `symbolTable`, `stringTable`), not the snake_case names listed; and `load_commands` is
   annotated "(macOS)" but the Linux plan emits `loadCommands: [{"kind":"PT_LOAD","name":"load-rx"}]`.

4. **Info.plist key list is incomplete.** `06_macos-aarch64.md` ("App-mode bundle") lists the
   keys `Info.plist` sets; measured (`plutil -p build/app1.app/Contents/Info.plist`) it also
   sets `NSDocumentsFolderUsageDescription` = "app1 reads and writes files in your Documents folder."

5. **Dangling citations** (symbol not in the cited file; checked by grepping the file):
   - `07_linux-aarch64.md`: `[[src/os/linux/link/elf.rs:PT_GNU_RELRO]]` — the constant is
     `src/os/linux/link/mod.rs:PT_GNU_RELRO` (elf.rs only mentions it in a comment).
   - `09_linux-riscv64.md`: `[[src/target/linux_riscv64/plan.rs:libpthread]]` — that file only
     has the `glibc_libpthread` field; `fn libpthread` is `src/target/linux_common/plan.rs:libpthread`.

6. **UNCLEAR — a musl flavor gets `DT_RUNPATH` when only the glibc flavor vendors.**
   `07_linux-aarch64.md`: "`DT_RUNPATH` (only when the build vendors a native library)";
   `language native-libraries`: "each [flavor is] its own codegen pass with its own data image".
   Measured: libraries `foo = [{linux, aarch64, glibc, vendor, libfoo.so}, {linux, system, libfoo.so}]`,
   `mfb build --target linux-aarch64` → `vend-glibc.out` dlopens `vend-libfoo.so` with
   `DT_RUNPATH $ORIGIN/vendor` (runs: `add=1005`), and `vend-musl.out` — which dlopens the
   system `libfoo.so` and vendors nothing — **also** carries `DT_RUNPATH $ORIGIN/vendor`
   (tag 29 present in its `.dynamic`). Code: `src/target/linux_aarch64/mod.rs:write_executable`
   sets `image.rpaths` from the build-wide `vendors_native_libraries`, not per flavor.
   Harmless today (vendored names are `<unit>-<source>`, so they cannot shadow a system soname)
   but it makes the musl binary search a directory it has no business searching. Owner decides:
   per-flavor RUNPATH (code fix) or reword the spec to "when any flavor of the build vendors".

   **Owner decision (2026-10-05): per flavor (CODE change).** Each flavor's image gets
   `DT_RUNPATH $ORIGIN/vendor` only if that flavor vendors a native library. `write_executable` reads
   the flavor's own vendor set, not the build-wide `vendors_native_libraries` (all Linux targets).

## Verified (no drift)

See the B15 summary for the full list of claims measured against binaries.
