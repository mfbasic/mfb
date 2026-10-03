# plan-166-A: Debug symbol model, `--debug` plumbing, and Mach-O symbol table

Last updated: 2026-10-03
Overall Effort: x-large (estimate: 1d–2d across A, B, C)
Effort: large (estimate: 3h–1d)
Depends on: nothing

A `--debug` build today carries no names for its own code. Every linker has the
names in hand at link time (`EncodedImage.symbols`) and drops them when it writes
the file. Profilers (`sample`, Instruments, `perf`), debuggers (lldb, gdb) and
`nm`/`atos` therefore see only raw addresses. This has blocked profiling and crash
analysis in at least five sessions; see the history under References.

plan-166 writes those names into the native symbol table of each output format
when the build is `--debug`: the Mach-O `LC_SYMTAB` (A), the ELF `.symtab` (B), and
the PE COFF symbol table (C). It covers **names only**: no DWARF, no PDB, no line
tables. A build without `--debug` stays byte-identical to today's.

This sub-plan builds the format-neutral symbol list and the `--debug` → linker
flag, and does Mach-O. The outcome: `nm build/<name>.out` on a `--debug` build
lists every function and data object with its address, `lldb` can set a breakpoint
by function name, and a build without `--debug` is byte-identical to the
pre-plan compiler on all five targets.

References:

- `src/arch/image.rs:EncodedImage`, `EncodedSymbol`: the symbol list each
  linker already receives.
- `src/arch/encode_plan.rs:encode_plan`: where text symbol offsets are produced,
  one per function, in emission order.
- `src/os/link_encode.rs:symbol_vmaddr`: the shared text/rodata/data address
  rule every linker uses.
- `src/os/macos/link/macho.rs:encode_unsigned_mach_o`,
  `src/os/macos/link/commands.rs:linkedit_layout`, `symtab`, `dysymtab`,
  `symbol_table`, `string_table`.
- `src/codegen/debug/mod.rs:DebugOptions`: the `--debug` value.
- `planning/completed/audit-1-linker-hardening.md` ("No internal symbol-name
  leakage"): the release-build property this plan must keep.
- `mfb spec tooling debug-report`, `mfb spec linker symbols-and-relocations`,
  `mfb spec linker macos-aarch64`.
- History: the T3 thread "Assess PE Debug Symbol Map" (2026-09-26) scoped the PE
  side and stopped at a confirmation question. "No symbols" blocked work in the
  Brogue port (2026-09-21), bug-686 (2026-09-24), bug-721 (2026-09-26), the calls
  workspace (2026-09-28) and benchmark measurement (2026-10-03).
  `planning/completed/plan-59-A-universal-resource-record.md` records "lldb cannot
  break on mfb binaries."

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| The host `nm` reads Mach-O, ELF and COFF (it is llvm-nm) | `nm --version \| head -1` → `llvm-nm, compatible with GNU nm` | MET (2026-10-03) |
| A release compiler built from the pre-plan commit is saved for the byte-identity check | `ls /tmp/mfb-166-base` → exists | NOT MET (Phase 0 task) |

Everything below assumes these hold.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all*
> prerequisites.

## 1. Goal

- `mfb build --debug` (and `mfb test --debug`) on `macos-aarch64` produces a
  `.out` / `.app` executable whose symbol table lists every `EncodedImage.symbols`
  entry as a defined local symbol at its runtime address, and whose
  `LC_FUNCTION_STARTS` lists every function start.
- The same build without `--debug` is byte-identical to the pre-plan compiler's
  output on all five targets.

### Non-goals (explicit constraints)

- **No change to non-`--debug` output**, on any target or build mode. The
  audit-1 property "internal MFBASIC function names … are never written to the
  file" stays true for every build without `--debug`.
- No DWARF, no line tables, no variable locations, no dSYM, no PDB.
- No new CLI flag. `--debug` gains a second meaning (symbols); the help text and
  spec say so.
- No change to codegen, `.ncode`, or any artifact-gate golden. The symbol list
  comes from `EncodedImage` after encoding.
- No names for linker-generated import stubs (Mach-O `__stubs`). lldb already
  synthesizes stub names from the indirect symbol table.
- No change to exported symbols: the symbols are **local** (`N_SECT` without
  `N_EXT`), and the export trie is unchanged.

## 2. Current State

**The names exist and are dropped.** `encode_plan` pushes one `EncodedSymbol {
name, section: Text, offset }` per function, offsets accumulated from instruction
sizes, plus one `Data` symbol per data object from `layout_data_objects`. Every
linker uses them only to resolve relocations (`symbol_vmaddr`).

**Mach-O already writes a symbol table, with imports only.**
`encode_unsigned_mach_o` always emits `LC_SYMTAB` and `LC_DYSYMTAB`.
`linkedit_layout` sizes the table as `image.imports.len()` entries (zero on the
import-less chained-fixups path). `symbol_table` writes one undefined
`N_EXT` entry per import. `string_table` writes a leading NUL and then each import
name. `dysymtab` sets `ilocalsym=0, nlocalsym=0, iextdefsym=0, nextdefsym=0,
iundefsym=0, nundefsym=count`. The indirect symbol table is
`for index in 0..imports.len() { put_u32(index) }`: **it indexes into the symbol
table, so it assumes the imports start at index 0.**

**`LC_FUNCTION_STARTS` is present and empty.** `macho.rs` emits
`linkedit_data(0x26, function_starts_offset, 1)`. `data_in_code_offset =
function_starts_offset + 1`, and that byte is left zero (an empty ULEB list).

**The UUID is a constant.** `commands.rs:uuid_command` writes the same 16 bytes
(`4d 46 42 00 … 01`) into every binary the compiler emits.

**Section ordinals** (needed for `n_sect`, 1-based across the image), from
`commands.rs:text_segment` / `data_const_segment` / `data_segment` /
`mfb_sign_segment`: `__TEXT,__text` = 1, the second `__TEXT` section = 2, then
`__DATA_CONST` sections in the order `__got` (if imports), `__mod_init_func` (if
initializers), `__const` (if `rodata_len > 0`), then `__DATA,__data` (if writable
data), then `__MFB,.mfbsign` (if signed).

**Signing order.** `encode_mach_o` encodes twice: once to size the ad-hoc code
signature, and again with it. It then `seal`s the content signature, then signs.
`__LINKEDIT` up to the code signature offset is inside `codeLimit`, so a larger
symbol table is covered by both signatures with no change to the signing code.
`inspect_mach_o` reads `covered_end` from `LC_CODE_SIGNATURE`, which also needs no
change.

**How `--debug` reaches the target.** `NativeBackend::write_executable` receives
`debug: DebugOptions`. Each of the five target backends then sets
`image.signing_metadata` after encoding and calls an `os::*` link function that
reads only the image. The image is how link-time decisions travel today
(`signing_metadata`, `rpaths`).

**Names.** Symbols are the native names from
`src/target/shared/nir/symbols.rs:function_symbol` / `symbol_fragment`:
`_mfb_fn_<name>` for user and package functions, `_mfb_ifn_<name>` for
compiler-internal ones, `_mfb_rt_*` / `_mfb_arena_*` for runtime helpers,
`_mfb_str_<n>` for string constants. Every byte outside `[A-Za-z0-9]` is escaped as
`_XX`, for example `_mfb_fn__5F_5Fmfb_5Finit_5Fglobals_5Fbrogue`. These are the
names `--ncode` shows and `mfb spec linker symbols-and-relocations` documents.

### Measured populations

| What | Count | Command |
|---|---|---|
| Symbols in a `--debug` hello-world Mach-O today | 13, all `U` (imports) | `mfb init hello; mfb build --debug hello; nm hello/build/hello.out \| wc -l` → 13 |
| Functions in `examples/brogue` (macOS) | 244 | `mfb build --ncode` then `python3 -c 'len(d["functions"])'` on `brogue.ncode` |
| Data objects in `examples/brogue` | 156 | same, `len(d["dataObjects"])` |
| Duplicate symbol names in brogue | 0 | same, `collections.Counter` over function + data symbols |
| Functions with no instructions in brogue | 0 | same |
| String-table bytes for brogue's names | 6169 (text) + 2165 (data) | same, `sum(len(n)+1)` |
| `EncodedImage { … }` struct literals in `src/` | 54 (1 production: `encode_plan.rs:161`; 53 test-only, including the `#[cfg(test)]` `spike.rs`) | `grep -rn "EncodedImage {" src --include='*.rs' \| grep -v "fn \|struct " \| wc -l` → 54 |
| Target backends that set `image.signing_metadata` | 5 | `grep -rn "image.signing_metadata = " src/target --include='*.rs'` |
| Call sites of the `os::*` link wrappers (the rejected alternative) | 37 | `grep -rn "write_linked_executable(\|write_linked_appdir(\|write_linked_app_bundle(\|link::write_executable(\|link::write_appdir(\|link::write_app_bundle(\|encode_executable_bytes(" src --include='*.rs' \| grep -v "fn " \| wc -l` |

### Verified properties

- **Function offsets are contiguous and ordered.** `encode_plan` assigns each
  function `text_offset` and then adds each instruction's `instruction_size`, with
  no padding between functions (read `encode_plan.rs:encode_plan`). Each function's
  size is therefore the next function's offset minus its own, and the last
  function ends at `image.text.len()`. Linkers append stubs/thunks after
  `image.text.len()`.
- **Symbol names are unique** in brogue (measured above). Not proven for every
  program. The shared builder refuses a duplicate name rather than emitting two
  entries (Phase 1).
- **UNVERIFIED: macOS symbolication keys caches by UUID.** If `sample`/Instruments
  cache names per UUID, two different `--debug` programs that share the constant
  UUID could show each other's names. Phase 3 measures this. See Open Decision 2.

## 3. Design Overview

Three pieces, layered:

1. **Flag.** A new `EncodedImage.debug_symbols: bool` field, set by each target
   backend from `debug.enabled` next to `image.signing_metadata`. Every linker
   decides from the image, as it already does for signing and rpaths.
2. **Shared symbol list.** A new `src/os/debug_symbols.rs` turns
   `EncodedImage.symbols` into a sorted `Vec<DebugSymbol { name, section:
   EncodedSection, offset, size, kind: Function | Object }>`. Sizes come from the
   next symbol in the same section, and the last one in each section runs to the
   section end. It is format-neutral; each writer maps `(section, offset)` to its
   own address with the rule `symbol_vmaddr` already uses. B and C reuse it.
3. **Mach-O writer.** When `debug_symbols` is set: local `N_SECT` entries ahead of
   the imports, `LC_DYSYMTAB` and the indirect table shifted to match, a populated
   `LC_FUNCTION_STARTS`, and (Open Decision 2) a content-derived `LC_UUID`.

**Correctness risk** is in the indirect symbol table. Adding locals ahead of the
imports moves every import's symtab index. If the indirect table or `iundefsym`
is not shifted by the same count, dyld binds stubs to the wrong symbols, and the
symptom is a crash in the first libc call, not a link error. Phase 2 checks this
by **running** a `--debug` program that calls libc, not just by inspecting the
table.

**Design uncertainty** is in which tools accept the result: lldb breakpoints by
name, `atos`, `sample`, and `codesign --verify` after the layout grows. Phase 3
checks each with the real tool.

**Byte-identity is this plan's gate for builds without `--debug`**: that path
must not move. The artifact gate cannot see it, because it stops before linking
(`.ai/testing-gates.md`). So the check is a direct `cmp` of linked outputs from
the saved pre-plan compiler and the new one (Phase 0 / Validation). **`--debug`
outputs are expected to differ** on every target this sub-plan touches (Mach-O
here). A diff in a build without `--debug` is a bug to root-cause: `cmp -l` one
fixture and find the first differing offset. It is never a reason to abandon the
design.

**Rejected alternatives:**

- *A `debug_symbols` parameter on every `os::*` link function.* That is 37 call
  sites against 54 struct literals. It was rejected because link decisions already
  ride on the image (`signing_metadata`, `rpaths`), and the console, `.app` and
  AppDir writers all share one `encode_executable_bytes`, so a field reaches every
  output shape with no signature churn.
- *A sidecar `.map` file.* No debugger or profiler loads one automatically, and the
  goal is tool support.
- *Unescaping names (`f$List$OF$Integer`) or printing MFBASIC display names.* See
  Open Decision 1.
- *External `N_EXT` defined symbols.* Locals suffice for every tool. External
  defined symbols in an executable invite confusion with exports.

## 4. Detailed Design

### 4.1 `EncodedImage.debug_symbols`

`pub(crate) debug_symbols: bool` on `EncodedImage`, documented as: "`--debug`
builds write `symbols` into the output's native symbol table
(`mfb spec linker symbols-and-relocations`). False for every other build, which
keeps those binaries free of internal names (audit-1)." `encode_plan` sets
`false`. Each target backend sets `image.debug_symbols = debug.enabled` on the line
after `image.signing_metadata = …`. All test literals get `debug_symbols: false`,
except the new tests.

### 4.2 `src/os/debug_symbols.rs`

```rust
pub(crate) enum DebugSymbolKind { Function, Object }
pub(crate) struct DebugSymbol<'a> {
    pub(crate) name: &'a str,
    pub(crate) section: EncodedSection,
    pub(crate) offset: usize,
    pub(crate) size: usize,
    pub(crate) kind: DebugSymbolKind,
}
/// `image.symbols` ordered by (section, offset, name), each sized to the next
/// distinct offset in its section (the last to `image.text.len()` / `image.data.len()`).
/// Text → Function, Data → Object. Err on a duplicate name or an offset past the
/// section end.
pub(crate) fn debug_symbols(image: &EncodedImage) -> Result<Vec<DebugSymbol<'_>>, String>
```

Symbols that share an offset all get the size up to the next *distinct* offset,
so an alias is not reported as zero-sized. A data symbol's size includes any
alignment padding before the next object. That is acceptable for names-only
symbols and is documented in the spec text.

### 4.3 Mach-O

With `debug_symbols` true, and `L = debug_symbols(image).len()`:

- **Entries.** Locals at indices `0..L` in the shared order, then the imports
  unchanged at `L..L+imports`. A local's `n_type` is `N_SECT` (`0x0e`). Its
  `n_sect` is the section ordinal from §2: text → 1. Data with `offset <
  rodata_len` → the `__const` ordinal. Other data → the `__data` ordinal. Compute
  the ordinals from the same `has_imports` / `has_init` / `has_rodata` /
  `has_writable` booleans that `encode_unsigned_mach_o` uses; do not count them a
  second time. `n_desc` is 0. `n_value` is the address `symbol_vmaddr` gives,
  with `text_vmaddr = VM_BASE + code_offset` and the rodata/data windows
  `encode_executable_bytes` already computes.
- **String table.** The existing leading NUL and import names stay first,
  unchanged, so import string offsets don't move. Local names are appended after.
- **`LC_DYSYMTAB`**: `ilocalsym=0, nlocalsym=L, iextdefsym=L, nextdefsym=0,
  iundefsym=L, nundefsym=imports`.
- **Indirect symbol table**: `put_u32(L + index)`.
- **Both linkedit paths.** The import-less chained-fixups path (`!needs_data_const`)
  must also write the table when `L > 0`. Today it writes no symtab bytes.
  `linkedit_layout` takes `L` and the function-starts length so every offset is
  computed once.
- **`LC_FUNCTION_STARTS`**: ULEB128 deltas of each text symbol's file offset
  (`code_offset + offset`, relative to the `__TEXT` start at file offset 0), then a
  terminating 0, padded with zeros to a multiple of 8 (as ld64 does). The
  command's size is that padded length, and `data_in_code_offset` follows it.
- **`LC_UUID`** (Open Decision 2, recommended): the first 16 bytes of SHA-256
  over `text || data || symbol string table`, with the RFC 4122 version-4 and
  variant bits set. This is computed in `encode_unsigned_mach_o` before the header
  is written. A build without `--debug` keeps the constant UUID.

With `debug_symbols` false, every function above must produce exactly today's
bytes. That is the byte-identity gate.

## Compatibility / Format Impact

- **Builds without `--debug`**: no change on any target (gated by `cmp`).
- **`--debug` Mach-O**: `__LINKEDIT` grows by `16 × L` symtab bytes, the local
  names, and the function-starts list. `LC_DYSYMTAB` and the indirect table values
  change. Load commands, segments and the content/ad-hoc signature formats are
  unchanged. `mfb info` behaves the same. `--debug` already changes codegen, so no
  gate compares `--debug` binaries to builds without it.
- **`--debug` meaning**: it now also writes symbol tables. This applies to
  `mfb test --debug` too, which shares the backend path.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial work, and mark moot tasks
> `- [x] ~~text~~ (moot: <evidence>)`. Fill each `Commit:` line when the phase
> lands. **An unticked box means NOT DONE.**

### Phase 0: Baseline compiler

Saves the pre-plan compiler, so every later phase can prove that builds without
`--debug` did not move.

- [ ] `cargo build --release` at the pre-plan commit, then
      `cp target/release/mfb /tmp/mfb-166-base`.
- [ ] Record its `git rev-parse --short HEAD` in Corrections as the baseline commit.

Acceptance: `/tmp/mfb-166-base --version` prints the baseline commit.
  Check: `/tmp/mfb-166-base --version` → names the recorded commit (est. 5 min, the release build).
Commit: — (no tree change)

### Phase 1: Flag and shared symbol list

Adds the field and the format-neutral list. No writer reads them yet, so output is
unchanged.

- [ ] `src/arch/image.rs`: add `debug_symbols: bool` (§4.1).
      `src/arch/encode_plan.rs`: set `false`. Add `debug_symbols: false` to the
      53 test literals (`grep -rn "EncodedImage {" src --include='*.rs' | grep -v "fn \|struct "`
      lists all 54).
- [ ] The five target backends (`src/target/{macos_aarch64,linux_aarch64,linux_x86_64,linux_riscv64,win_x86_64}/mod.rs`):
      `image.debug_symbols = debug.enabled;` beside `image.signing_metadata = …`.
- [ ] New `src/os/debug_symbols.rs` (§4.2), registered in `src/os/mod.rs`.
- [ ] Tests in `src/os/debug_symbols.rs`:
      `sizes_run_to_next_symbol_and_last_to_section_end`,
      `aliases_share_the_next_distinct_offset`,
      `text_is_function_data_is_object`, `duplicate_name_is_rejected`,
      `offset_past_section_end_is_rejected`.

Acceptance: the new unit tests pass, and no linked output changes yet.
  Check: `cargo test --bin mfb debug_symbols` → 5 passed (est. 3 min including build).
Commit: —

### Phase 2: Mach-O symbol table, dysymtab, function starts

Writes the locals when `debug_symbols` is true. This phase carries the
indirect-table risk.

- [ ] `src/os/macos/link/commands.rs`: `linkedit_layout` takes the local count and
      the function-starts byte length, and sizes `symbol_count`, `string_size`,
      `function_starts` and `data_in_code_offset` from them, on both the
      data-const and chained-fixups paths.
- [ ] `commands.rs:symbol_table` / `string_table`: emit locals first (§4.3), with
      import strings first in the string table. `dysymtab`: write
      `ilocalsym/nlocalsym/iextdefsym/iundefsym` from the counts.
- [ ] `src/os/macos/link/macho.rs:encode_unsigned_mach_o`: write the shifted
      indirect table (`L + index`). Write the symtab/strtab on the chained-fixups
      path when `L > 0`. Write the ULEB function-starts list.
- [ ] Section-ordinal helper beside `data_const_section_count`, derived from the
      same booleans (§2).
- [ ] Tests in `src/os/macos/link/tests.rs`, parsing the emitted bytes:
      `debug_symbols_lists_locals_before_imports` (n_type, n_sect, n_value against
      `symbol_vmaddr`), `debug_symbols_shift_indirect_table_and_iundefsym`,
      `debug_symbols_on_import_less_image`, and
      `function_starts_lists_every_function`. The flag-off path is covered by the
      existing Mach-O tests passing with no edit beyond the `debug_symbols: false`
      field, plus the byte-identity loop.
- [ ] Signed + debug: in `src/cli/info/tests.rs`, add a macOS case that writes a
      signed image with `debug_symbols: true` and checks that `mfb info` reports
      the signature valid.

Acceptance: a `--debug` macOS program that calls libc **runs correctly** and `nm`
lists its functions.
  Check: `rm -rf /tmp/p166 && mkdir -p /tmp/p166 && mfb init /tmp/p166/hello && mfb build --debug /tmp/p166/hello && /tmp/p166/hello/build/hello.out; echo "exit=$?"; nm /tmp/p166/hello/build/hello.out | grep ' t _mfb_fn_main$'` → the program's greeting, `exit=0`, and one `t _mfb_fn_main` line (est. 1 min). An `exit` other than 0, or a crash, points at the indirect-table shift (§3).
  Check: `cargo test --bin mfb macos::link` → all pass (est. 3 min).
Commit: —

### Phase 3: Tool proof and UUID

Proves that the tools developers actually use read the table, and settles the UUID
question with a measurement.

- [ ] `codesign --verify --verbose build/hello.out` on a `--debug` build → valid.
- [ ] `lldb build/hello.out -o 'b mfb_fn_main' -o run -o bt -o quit` → stops in
      `mfb_fn_main`. The backtrace shows names. Record the output in Corrections.
- [ ] `atos -o build/hello.out -l 0x100000000 <address of _mfb_fn_main from nm>`
      → `_mfb_fn_main`.
- [ ] Build `examples/brogue` and `examples/life` with `--debug`, run
      `sample <pid> 2` on each in turn, and check that each profile names that
      program's own functions. If the second profile shows the first program's
      names, the UUID hypothesis is confirmed. Record the result in Corrections.
- [ ] Per Open Decision 2: content-derived `LC_UUID` in `--debug` builds
      (`commands.rs:uuid_command` takes the 16 bytes; §4.3), plus a test
      `debug_uuid_differs_between_images_and_is_stable`.

Acceptance: lldb breaks by name on a `--debug` binary, and two different `--debug`
programs have different UUIDs.
  Check: `dwarfdump --uuid build/hello.out; dwarfdump --uuid <brogue>/build/brogue.out` → two different UUIDs (est. 1 min).
Commit: —

### Phase 4: Docs and help

`--debug` now means two things. The help text and the spec say so.

- [ ] `src/cli/help.rs:167` and `:201`: `--debug` says it also writes the
      program's function and data names into the executable's symbol table, for
      debuggers and profilers. The `BuildOptions::debug` doc comment in
      `src/cli/build/mod.rs` says the same.
- [ ] `src/docs/spec/tooling/09_debug-report.md`: a "Symbol tables" section (what
      is written, that a build without `--debug` writes none, which formats are
      done), citing `[[src/os/debug_symbols.rs:debug_symbols]]`.
      `src/docs/spec/tooling/07_cli-reference.md:144`: the `--debug` row.
- [ ] `src/docs/spec/linker/04_symbols-and-relocations.md`: a "Debug symbol
      tables" section covering the shared model (§4.2: names verbatim, sizes,
      ordering, no names in builds without `--debug`).
      `src/docs/spec/linker/06_macos-aarch64.md`: locals before imports, the
      dysymtab/indirect shift, function starts, and the debug UUID.

Acceptance: the spec renders and its citations resolve.
  Check: `cargo build && cargo test --bin mfb spec` → pass; `scripts/spec-census.sh --citations` → no new dangling citation (est. 5 min).
Commit: —

## Validation Plan

- Tests: Phase 1 and 2 unit tests, plus the signed + debug `mfb info` case.
- Coverage check: the new `debug_symbols` path runs only when the flag is set.
  The Phase 2 tests set it explicitly, and the Phase 2 runtime check runs a real
  `--debug` binary.
- Runtime proof: Phase 3 (lldb, atos, sample, codesign).
- Byte-identity of builds without `--debug` (run once after Phase 4; est. 3 min):
  ```sh
  for exe in /tmp/mfb-166-base target/release/mfb; do
    for proj in hello_world brogue; do
      for t in macos-aarch64 linux-aarch64 linux-x86_64 linux-riscv64 windows-x86_64; do
        d=/tmp/p166-id/$(basename $exe)/$proj/$t; rm -rf $d; mkdir -p $d
        cp -R examples/$proj/. $d/ && $exe build -q --target $t $d >/dev/null || echo "BUILD FAILED $exe $proj $t"
      done; done; done
  diff <(cd /tmp/p166-id/mfb-166-base && find . -path '*/build/*' -type f | sort | xargs shasum -a 256) \
       <(cd /tmp/p166-id/mfb && find . -path '*/build/*' -type f | sort | xargs shasum -a 256)
  ```
  → no output, and no `BUILD FAILED` line.
- Doc sync: Phase 4.
- Final gate: deferred to plan-166-C, which runs the full suite once after the
  last format lands.

## Open Decisions

1. **Symbol names**: write the native names verbatim (`_mfb_fn_foo_5Fbar`)
   *(recommended)*, or unescape `_XX` back to source spelling. Verbatim names are
   unique by construction, match `--ncode` and the spec, and are what a developer
   types into lldb. Unescaped names read better but put `$` and spaces into
   tool-facing names. Revisit only if profiles prove hard to read.
2. **`--debug` Mach-O UUID**: derive it from the image *(recommended)*, or keep
   the shared constant. The cost is about 10 lines and changes builds with
   `--debug` only. Phase 3 measures whether the shared UUID actually confuses
   `sample`. If it does not, the derived UUID is still correct practice and stays.

## Corrections

(Filled in during execution.)

## Summary

The work is mostly plumbing and one table-layout change. The real risk is the
Mach-O indirect symbol table shift, which fails as a runtime crash rather than a
link error. The Phase 2 acceptance check runs a libc-calling `--debug` binary
for that reason. Builds without `--debug`, codegen, and every artifact-gate golden
are untouched. ELF is plan-166-B and PE is plan-166-C.
