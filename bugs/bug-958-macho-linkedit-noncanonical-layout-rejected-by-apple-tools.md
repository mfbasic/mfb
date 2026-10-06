# bug-958: Mach-O `__LINKEDIT` layout is non-canonical — `strip`, `install_name_tool`, `dyld_info` refuse every mfb macOS executable

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: src/os/macos/link/tests.rs (new: LINKEDIT ordering + 8-byte alignment assertion), plus a tools gate running `install_name_tool -add_rpath` on a built binary

Every macOS executable the in-tree Mach-O writer emits (console and `--app`, with or without
`--debug`) is rejected by Apple's binary tools:

- `strip` / `install_name_tool`: `fatal error: file not in an order that can be processed (function starts data out of place)`
- `dyld_info`: `mis-aligned LINKEDIT content 'symbol table'` (and prints nothing else)

The binary still loads and runs (dyld, `codesign -v`, `nm`, `otool`, `lldb`, `sample`, `atos`
all accept it), so the defect is silent until a user tries to post-process the executable —
e.g. add an rpath, strip it for distribution, or inspect fixups. The single correct behavior:
`__LINKEDIT` content is laid out in the order ld64 emits and that cctools/`dyld_info` require
(rebase, bind, …, export, function starts, data-in-code, symbol table, indirect symbol table,
string table, code signature), with the `nlist_64` symbol table and indirect symbol table
8-/4-byte aligned, so those tools succeed.

References:

- `mfb spec linker macos-aarch64` ("Load commands", "Debug symbol table": "Everything sits in
  `__LINKEDIT` below `LC_CODE_SIGNATURE`"); the spec states no order/alignment, which is
  why the defect is uncovered.
- Found during B15 linker hardening.

## Failing Reproduction

```
cd /tmp && /tmp/hardening/mfb-head init h && /tmp/hardening/mfb-head build -q h
cp h/build/h.out x.out
install_name_tool -add_rpath /tmp x.out ; echo rc=$?
strip x.out ; echo rc=$?
dyld_info -fixups x.out
otool -l x.out | sed -n '/LC_DYLD_INFO_ONLY/,/LC_DYSYMTAB/p'
```

- Observed: `install_name_tool: fatal error: file not in an order that can be processed (function starts data out of place)` rc=1;
  `strip:` same error rc=1; `dyld_info`: `mis-aligned LINKEDIT content 'symbol table'`.
  Layout measured for hello world: `bind_off=0x10000 bind_size=191`, `symoff=0x100bf`
  (`symoff % 8 == 7`), `indirectsymoff=0x1015f`, `stroff=0x10187`, `LC_FUNCTION_STARTS
  dataoff=0x1020a` (after the string table), `LC_DATA_IN_CODE dataoff=0x1020b`, signature at
  `0x10210`. `--debug` build: `symoff % 8 == 6`, same order.
- Expected: all three tools succeed (rc 0), `dyld_info -fixups` lists the 10 binds.

Contrast: `codesign -f -s -` re-signs it fine and the result runs; `vtool -set-build-version`
works; `nm -m`, `otool -l`, `lldb image dump symtab`, `sample` all read it.

## Root Cause

`src/os/macos/link/commands.rs:linkedit_layout` packs the pieces back to back with no
alignment and in a non-ld64 order: `exports_offset = fixups_offset + fixups_size`;
`symtab_offset = exports_offset` (not aligned to 8 — the bind opcode stream has odd length);
`indirect_symbol_offset = symtab_offset + n*16`; `string_offset = …`;
`function_starts_offset = string_offset + string_size`;
`data_in_code_offset = function_starts_offset + …`. cctools (`strip`/`install_name_tool`)
require function-starts/data-in-code to precede the symbol table, and dyld's
`MachOFile` linkedit validation requires the symbol table to be pointer-aligned.
`src/os/macos/link/macho.rs:encode_unsigned_mach_o` writes the bytes at those offsets.

## Goal

- `install_name_tool -add_rpath`, `strip`, and `dyld_info -fixups` succeed on console,
  `--app`, and `--debug` outputs; the stripped/re-pathed binary still runs after `codesign -f -s -`.
- `symoff % 8 == 0`, `indirectsymoff % 4 == 0`, function starts and data-in-code precede the symtab.

### Non-goals (must NOT change)

- The ad-hoc signature, UUID rules, LC_NOTE payload placement, and the debug symtab contents
  (indices, `LC_DYSYMTAB` ranges) described in the spec.

## Blast Radius

- `linkedit_layout` / `encode_unsigned_mach_o` — fixed by this bug.
- The import-less `LC_DYLD_CHAINED_FIXUPS` path in the same function — same packing; unreachable
  from the CLI today (every console build imports `_exit`), not measured.
