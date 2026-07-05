# Failure Rules

The linker must fail rather than generate a broken executable. It is never valid
to satisfy missing linker support with a placeholder helper, an empty dynamic
dependency, a zero address, or a runtime "unsupported" fallback. Every failure
surfaces as a `Result::Err(String)` that aborts the build.

## Object-plan gate

The structural gate (see `object-plan`) fails the build before any bytes are
encoded when:

- the planned target/container/status is wrong, the entry symbol `_main` is
  missing from the defined symbols, or a defined symbol (or, on macOS, a dylib)
  is duplicated;
- a relocation's source is not a defined symbol, or its target is neither
  defined, imported, nor external;
- (macOS) the `__TEXT,__text` section is absent, two non-empty sections overlap
  in their file ranges, or a code unit has no operations.

## Encode and link

The encoder fails when an image has no entry symbol. The platform linkers fail
when:

- an internal symbol (the entry, an initializer, or a relocation target) does not
  resolve to the expected section — e.g. `entry symbol '…' does not resolve to
  text`, `initializer '…' does not resolve to a text symbol`, or
  `symbol '…' does not resolve`;
- a `(binding, kind)` relocation pair is unsupported — e.g.
  `macOS linker does not support relocation … …` /
  `linux linker does not support relocation … …`;
- an external function relocation has no generated stub —
  `… linker cannot bind external symbol '…' from …`;
- an external data relocation has no GOT slot —
  `… linker cannot bind external data symbol '…' from …`;
- (macOS) an import names a library with no known install path —
  `macOS linker has no dylib path for library '…'` — or a referenced library has
  no dylib ordinal;
- a file write or permission change fails.

[[src/os/macos/link/mod.rs:patch_relocations]] [[src/os/linux/link/mod.rs:patch_relocations]]

A required package export or ABI dependency that cannot be resolved fails earlier,
during the package merge into IR (see `package-linking`), not in the linker —
since packages are merged, not externally linked, an unresolved package symbol
becomes an unresolved internal symbol and is caught by the object-plan gate or
the symbol-resolution checks above.
