# Symbols And Relocations

## Symbol naming

Internal symbols are generated during native lowering. The conventions:
[[src/target/shared/nir/mod.rs]]

```text
_mfb_fn_<name>               user (and merged package) functions
_mfb_ifn_<name>              compiler-internal functions (sigil-stripped names)
_mfb_global_<project>_<name> program globals
__mfb_init_globals_<project> global initializer NIR name (encoded via function_symbol)
_mfb_rt_<helper>_<call>      runtime helpers (e.g. _mfb_rt_io_io_print)
_mfb_linker_init             the native LINK load-time initializer
_mfb_linker_<alias>_<name>   native LINK marshaling thunks
_main                        the program entry symbol
```
[[src/target/shared/nir/mod.rs:LINK_INIT_SYMBOL]]
[[src/target/shared/nir/symbols.rs:function_symbol]]
[[src/target/shared/nir/symbols.rs:global_initializer_name]]

`symbol_fragment` preserves ASCII alphanumeric bytes and escapes every other
UTF-8 byte, including `_`, as `_XX` with uppercase hexadecimal digits. For
example, `a_b` becomes `a_5Fb`; separators and literal underscores stay distinct.
[[src/target/shared/nir/symbols.rs:symbol_fragment]] Merged package exports use
the same `_mfb_fn_…` namespace as ordinary functions (their per-package identity
is folded into the name during the merge), *not* a distinct package-symbol
namespace.

Symbols fall into three classes for the linker:

- Internal text symbols — functions, runtime helpers, `LINK` thunks.
- Internal data symbols — string constants, runtime data, the main arena global,
  `LINK` library/symbol C strings.
- External import symbols — platform-library functions or data resolved at load time.

## Relocation kinds and bindings

A relocation carries an `offset`, a `target` symbol, a `kind`, a `binding`, and
an optional `library`. The aarch64 backends support three kinds:

```text
branch26    26-bit B/BL immediate, for calls
page21      ADRP page address (PC-relative, 4 KiB granule)
pageoff12   page offset: ADD for internal data, scaled LDR for an external GOT slot
```
[[src/arch/image.rs:EncodedRelocation]]

with three bindings:

- `internal` — a direct call to a defined symbol in this image (`branch26`). The
  linker computes the final delta and patches the branch directly.
- `data` — data addressing to a defined symbol in this image
  (`page21`/`pageoff12`). The linker patches the ADRP/ADD page pair directly.
- `external` — the target is an imported symbol. The linker resolves it through
  the import machinery: `branch26` is redirected to the symbol's import stub;
  `page21`/`pageoff12` are redirected to the symbol's GOT slot (data-style
  external addressing).

An unsupported `(binding, kind)` pair, an external relocation with no stub, or an
external data relocation with no GOT slot is a hard linker error (see
`failure-rules`).

Indirect calls (function values, `CallKind::Indirect`) are an indirect branch
through a register and carry no relocation.

## Import stubs and the GOT

External function calls do not branch directly to the dynamic symbol. The linker
appends a 12-byte stub per encoded import on ELF and Mach-O, including
data-kind imports whose stubs are unused. PE emits thunks only for function-kind
imports. On AArch64 a stub contains:

```text
adrp x16, <GOT page>
ldr  x16, [x16, <GOT offset>]
br   x16
```

Each encoded import record gets an 8-byte GOT slot, zero-filled in the file
and bound by the dynamic loader at load time. `branch26` external relocations are
patched to reach the stub; `page21`/`pageoff12` external relocations are patched
to reach the GOT slot directly. x86-64 uses a RIP-relative jump stub and RISC-V
an `auipc`/`ld`/`jalr` stub; Windows uses an IAT slot instead of an ELF/Mach-O GOT.
[[src/os/linux/link/mod.rs:emit_import_stub]]
[[src/os/macos/link/mod.rs:append_import_stubs]]
[[src/os/windows/link/mod.rs:append_thunks]]

## Encoded import and initializer capabilities

`EncodedImport` carries, besides `library` and
`symbol`:

- `kind`: `ImportKind::Function` or `ImportKind::Data`. ELF uses it for the
  dynamic symbol type and loader relocation kind; PE uses it to omit thunks
  for data imports. ELF/Mach-O still reserve a stub for every import record.
- `version`: an optional symbol version (e.g. `GLIBC_2.17`). `None` emits an
  unversioned reference. Ignored on Mach-O, which binds by dylib ordinal.

[[src/arch/image.rs:EncodedImport]] [[src/arch/image.rs:ImportKind]]

`EncodedImage::initializers` lists internal text symbols for initialization.
Mach-O emits a dyld-run `__mod_init_func` array. The dynamic ELF writer emits
`DT_INIT_ARRAY`/`DT_INIT_ARRAYSZ` and relative relocations for its pointers;
invocation depends on C-runtime startup, so a raw custom entry does not run this
array merely because it is present. The static ELF and PE writers do not consume
`initializers`. PE also does not consume ELF symbol versions.
[[src/os/macos/link/mod.rs:encode_executable_bytes]]
[[src/os/linux/link/elf/dynamic.rs:build]]
[[src/os/linux/link/tests/elf_writing.rs:writes_init_array_glibc_elf]]
[[src/os/linux/link/mod.rs:encode_executable_bytes]]
[[src/os/windows/link/mod.rs:write_executable]]

The shared encoder emits every import as `Function` with `version: None`,
including macOS app globals addressed through external data relocations, and
leaves `initializers` empty. Relocation binding controls whether a reference
addresses a stub or a slot. Native `LINK`
initialization is an explicit bootstrap call. Data-kind and versioned imports
are supported linker inputs, not classifications produced by this encoder.
[[src/arch/encode_plan.rs:encode_plan]]
[[src/os/link_encode.rs:patch_aarch64_reloc]]

## Debug symbol tables

A `--debug` build writes the encoder's symbols into the executable's native
symbol table (`EncodedImage::debug_symbols` set by the target backend; see
`./mfb spec tooling debug-report`). Every format writer takes its entries from one
format-neutral list, so the formats list the same symbols in the same order:
[[src/os/debug_symbols.rs:debug_symbols]]

- **Source.** Exactly `EncodedImage::symbols`: every function and runtime helper
  (text) and every data object (data). Linker-appended import stubs and thunks
  have no entry. Text symbols are functions; data symbols are objects.
- **Names.** The native names verbatim, as `symbol_fragment` escaped them
  (`_mfb_fn_a_5Fb`), never unescaped to source spelling. They are unique by
  construction, match `--ncode`, and are what a developer types into a debugger.
- **Order.** By section (text, then data), then offset, then name.
- **Size.** Each symbol runs to the next *distinct* offset in its section, so
  aliases at one offset all get the full size; the last symbol in a section runs
  to the section end (`text.len()` / `data.len()`, excluding appended stubs). A
  data object's size includes any alignment padding before the next object.
- **Address.** The writer maps `(section, offset)` with the same rule relocations
  use, so an entry and a relocation to the same name always agree.
  [[src/os/link_encode.rs:section_vmaddr]]
- **Refusals.** A duplicate name, or an offset past its section end, fails the
  link rather than writing an ambiguous or dangling entry.

The entries are local symbols, never exports: no export trie, dynamic symbol
table or import table changes. A build without `--debug` writes none of these
names.

## See Also

* ./mfb spec architecture internal-naming — the internal symbol-naming conventions behind these prefixes
* ./mfb spec linker object-plan — the structural validation over these symbols and relocations
* ./mfb spec linker import-selection — how imported symbols are chosen and resolved
