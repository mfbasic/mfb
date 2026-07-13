# Symbols And Relocations

## Symbol naming

Internal symbols are generated during native lowering. The conventions:
[[src/target/shared/nir.rs]]

```text
_mfb_fn_<name>               user (and merged package) functions
_mfb_ifn_<name>              compiler-internal functions (sigil-stripped names)
_mfb_global_<project>_<name> program globals
__mfb_init_globals_<project> the global initializer routine
_mfb_rt_<helper>_<call>      runtime helpers (e.g. _mfb_rt_io_io_print)
_mfb_linker_init             the native LINK load-time initializer
_mfb_linker_<alias>_<name>   native LINK marshaling thunks
_main                        the program entry symbol
```
[[src/target/shared/nir/mod.rs:LINK_INIT_SYMBOL]]

`symbol_fragment` rewrites any character that is not ASCII alphanumeric or `_`
to `_`, so symbol names are always valid identifiers.
[[src/target/shared/nir/symbols.rs:symbol_fragment]] Merged package exports use
the same `_mfb_fn_…` namespace as ordinary functions (their per-package identity
is folded into the name during the merge), *not* a distinct package-symbol
namespace.

Symbols fall into three classes for the linker:

- Internal text symbols — functions, runtime helpers, `LINK` thunks.
- Internal data symbols — string constants, runtime data, the main arena global,
  `LINK` library/symbol C strings.
- External import symbols — platform-library functions resolved at load time.

## Relocation kinds and bindings

A relocation carries an `offset`, a `target` symbol, a `kind`, a `binding`, and
an optional `library`. The aarch64 backends support three kinds:

```text
branch26    26-bit B/BL immediate, for calls
page21      ADRP page address (PC-relative, 4 KiB granule)
pageoff12   12-bit offset within a page, for ADD/LDR/STR
```
[[src/arch/aarch64/encode/mod.rs:EncodedRelocation]]

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
appends a 12-byte (3-instruction) stub per imported function:

```text
adrp x16, <GOT page>
ldr  x16, [x16, <GOT offset>]
br   x16
```

Each distinct imported symbol gets one 8-byte GOT slot, zero-filled in the file
and bound by the dynamic loader at load time. `branch26` external relocations are
patched to reach the stub; `page21`/`pageoff12` external relocations are patched
to reach the GOT slot directly.

## Encoded import and initializer capabilities

`EncodedImport` carries, besides `library` and
`symbol`:

- `kind`: `ImportKind::Function` (called through a stub) or `ImportKind::Data`
  (addressed only through the GOT). The kind lets the linker lay out stubs and
  GOT slots deterministically without scanning relocations.
- `version`: an optional symbol version (e.g. `GLIBC_2.17`). `None` emits an
  unversioned reference. Ignored on Mach-O, which binds by dylib ordinal.

[[src/arch/aarch64/encode/mod.rs:EncodedImport]] [[src/arch/aarch64/encode/mod.rs:ImportKind]]

`EncodedImage` also carries `initializers`: internal text symbols that must run,
in order, before the program entry — materialized as ELF `DT_INIT_ARRAY` /
Mach-O `__mod_init_func` (`S_MOD_INIT_FUNC_POINTERS`).

The linkers fully implement `ImportKind::Data`, symbol versioning, and the
initializer array, but the encode path emits every import as
`Function`/unversioned and leaves `initializers` empty: the built-in surface is
function-only, and the native `LINK` initializer is called from the entry
bootstrap rather than through the initializer array. The `Data`/versioning/
initializer capabilities are exercised by the linker tests — symbol
versioning is intended for versioned exports such as OpenSSL 3's
`OPENSSL_3.0.0` (see `linux-aarch64`), and `ImportKind::Data` for data globals
(`tls`, app-mode).

## See Also

* ./mfb spec architecture internal-naming — the internal symbol-naming conventions behind these prefixes
* ./mfb spec linker object-plan — the structural validation over these symbols and relocations
* ./mfb spec linker import-selection — how imported symbols are chosen and resolved
