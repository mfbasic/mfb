# plan-166-C: PE COFF symbol table for `--debug` builds, and plan-166 closeout

Last updated: 2026-10-03
Effort: medium (estimate: 1h–2h, excluding final-suite latency)
Depends on: plan-166-B

Writes the plan-166-A symbol list into a `--debug` Windows `.exe` as a COFF symbol
table, then runs the plan-wide final gate. The outcome: `llvm-nm` /
`llvm-objdump -t` / MinGW-style gdb show named functions at the right addresses
in a `--debug` `.exe` (console and `--app`), llvm's PE reader accepts the image,
and every build without `--debug` is byte-identical.

**Scope limit, stated up front:** WinDbg, Visual Studio, WPA/ETW, VTune and
Superluminal ignore COFF symbols in an image and read only a PDB. This plan does
not make those tools show names. A PDB is a separate, much larger feature that
nobody has asked for (non-goal).

References:

- `src/os/windows/link/mod.rs:write_executable`: section layout, `symbol_rva`,
  `append_thunks`, then the `seal` call.
- `src/os/windows/link/pe.rs:write_image`: writes `PointerToSymbolTable = 0` and
  `NumberOfSymbols = 0` unconditionally (`pe.rs:253-254`), and computes
  `total_file_size`.
- `src/os/inspect.rs:inspect_pe`: `covered_end = bytes.len()`, and finds
  `.mfbsign` / `.mfbnote` by section name.
- `src/os/debug_symbols.rs:debug_symbols` (plan-166-A).
- Microsoft PE/COFF specification, "COFF Symbol Table" and "COFF String Table".
- `mfb spec linker windows-x86_64`.
- The T3 thread "Assess PE Debug Symbol Map" (2026-09-26): option 2 of the three
  options discussed there.

## Prerequisites

The prerequisites are stated once in plan-166-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-166-B is complete | `ls planning/plan-166-B-*` → no match (archived) | MET (2026-10-03, archived in the plan-166-C Phase 1 commit's parent) |
| The baseline compiler still exists | `ls /tmp/mfb-166-base` → exists | MET (2026-10-03) |
If plan-166-B is not complete, this plan cannot start.

| Windows box 206 answers (owner-designated, 2026-10-03) | `ssh test@192.168.1.206 ver` → `Microsoft Windows [Version 10.0.26200.…]` | MET (2026-10-03, re-run: `10.0.26200.9457`) |

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue. If you stop, report all of them.

## 1. Goal

- A `--debug` `windows-x86_64` `.exe` has a COFF symbol table (and string table)
  at `PointerToSymbolTable`, with one entry per `debug_symbols` item that `llvm-nm`
  resolves to `ImageBase + section RVA + offset`.
- The `.exe` parses cleanly in llvm's PE reader, and its mapped image matches a
  build without `--debug` except for the two COFF symbol fields. Console and
  `--app` `--debug` builds run on Windows box 206.
- Builds without `--debug` are byte-identical on all five targets.
- The full suite passes once at the end of plan-166.

### Non-goals (explicit constraints)

- No PDB, no CodeView `RSDS` debug directory, and no change to any data
  directory.
- No change to builds without `--debug`, including signed builds.
- No names for the import thunks appended after `image.text`.
- No change to sections, RVAs, `SizeOfImage`, `CheckSum` (stays 0) or
  `TimeDateStamp` (stays 0). The table is file-only data past the last section,
  never mapped.

## 2. Current State

**Header fields.** `write_image` writes the COFF header with
`PointerToSymbolTable = 0` and `NumberOfSymbols = 0` (`pe.rs:253-254`) for every
build.

**File end.** `write_image` sizes the buffer to `total_file_size`, the maximum
over sections of `file_offset + align_up(bytes.len(), FILE_ALIGNMENT)`. Nothing is
written past the last section. `write_executable` then `seal`s when signed.
`inspect_pe` reports `covered_end = bytes.len()`, so bytes appended inside
`write_image` are covered by the content signature.

**Sections, in order.** `.text` (always, index 1), `.rdata` if
`rodata_size > 0`, `.data` if `data.len() > rodata_size`, `.idata` if imports,
`.rsrc` if `--app`, `.reloc` (always), `.mfbnote` (always), `.mfbsign` if signed.
`.text` holds `image.text` followed by one `THUNK_SIZE` thunk per function import.
`symbol_rva` maps text → `text_rva + offset` and data → `data_base_rva + offset`,
where `data_base_rva` is `.rdata`'s RVA when it exists (`.data` follows it
contiguously; asserted in `write_executable`).

**Tools today**: `nm build/hello.exe` → `no symbols`, and `objdump -t build/hello.exe`
→ an empty `SYMBOL TABLE:` on a `--debug` build (measured 2026-10-03, host llvm 17).

### Measured populations

| What | Count | Command |
|---|---|---|
| Sections a console `.exe` with data and imports has | 6 (`.text`, `.rdata`, `.data`, `.idata`, `.reloc`, `.mfbnote`) | the `section_count` sum in `write_executable` |
| Symbols for brogue | 400 | plan-166-A measured populations |
| Names longer than 8 bytes (they need the string table) | all of them: the shortest native names, `_mfb_str_0` (10 bytes) and `_mfb_fn_` + name (≥ 9), exceed 8 | `symbol_fragment` prefixes, plan-166-A §2 |

### Verified properties

- **The table needs no layout change.** It lives past `total_file_size`, outside
  every section, so no RVA, `SizeOfImage` or section header moves (read
  `pe.rs:write_image`).
- **`Value` is section-relative** (measured 2026-10-03). A hand-patched copy of a
  `--debug` `hello_world.exe` with one record (`Value 0`, `SectionNumber 1`,
  `Type 0x20`, `StorageClass 2`, a long name through the string table) appended
  at the file end, and `PointerToSymbolTable` / `NumberOfSymbols` set:
  `nm` → `140001000 T _mfb_fn_main` (`IMAGE_BASE` + `.text` RVA `0x1000` + 0), and
  `objdump -t` → `[ 0](sec  1)(fl 0x00)(ty  20)(scl   2) (nx 0) 0x00000000 _mfb_fn_main`.
- **The Windows loader accepts the table** (measured 2026-10-03). The same
  patched `.exe` and the unpatched original, both run on box 206 (Windows 11
  10.0.26200) over `ssh test@192.168.1.206`, printed identical output and both
  exited 0.

## 3. Design Overview

`write_image` gains a `coff_symbols: Option<&CoffSymbolTable>` argument.
`write_executable` builds that argument from `debug_symbols(image)` when
`image.debug_symbols` is set. When the argument is present, `write_image` sets
`PointerToSymbolTable` to `total_file_size` (already `FILE_ALIGNMENT`-aligned),
sets `NumberOfSymbols`, and appends the 18-byte records followed by the string
table. When it is absent, every byte is as today.

**Correctness risk** is small and local: a wrong `Value` or `SectionNumber`
gives wrong addresses in tools, not a broken program. The convention is
measured (§2). Phase 1 checks real output against `symbol_rva`.

**Design uncertainty** is low. Both format questions were measured on a
hand-patched `.exe` (§2), and Phase 3 repeats the Windows run with real output.

**Byte-identity is the gate for builds without `--debug`.** **Expected to
differ**: every `--debug` `.exe`.

**Rejected alternatives:**

- *A PDB*: the only route to Microsoft's tools, but its scale is weeks (the
  2026-09-26 thread). Not requested.
- *A sidecar `.map`*: no tool loads it automatically.
- *Putting the table in a section*: it would change `SizeOfImage` and RVAs, and
  it is not where COFF readers look.

## 4. Detailed Design

### 4.1 Records (18 bytes each, one per `debug_symbols` item, in its order)

- `Name[8]`: a name of 8 bytes or fewer is stored inline, NUL-padded. A longer
  name is stored as `u32 0` followed by `u32 offset` into the string table, where
  the offset counts from the start of the string table including its 4-byte
  length field, so the first string is at offset 4.
- `Value: u32`: the offset within the section (measured, §2). Text is
  `offset`. Data is `offset` within `.rdata` when `offset < rodata_size`,
  otherwise `offset - rodata_size` within `.data`.
- `SectionNumber: i16`: the 1-based index of `.text`, `.rdata` or `.data` in the
  emitted section order. It is computed from the same `has_rdata` / `has_data`
  booleans `write_executable` uses.
- `Type: u16`: `0x20` (function) for text, `0` for data.
- `StorageClass: u8`: `IMAGE_SYM_CLASS_EXTERNAL` (2). This is what MinGW emits
  for non-static functions, and what `llvm-nm` shows as `T` / `D`.
- `NumberOfAuxSymbols: u8`: 0.

### 4.2 String table

`u32 total_size` (including these 4 bytes), followed by each long name with a NUL
terminator, in record order.

## Compatibility / Format Impact

- **Builds without `--debug`**: unchanged (gated by `cmp`).
- **`--debug` `.exe`**: two COFF header fields become non-zero, and the file grows
  by `18 × L` bytes plus the string table, past the last section. Mapped image,
  RVAs and data directories are unchanged. `mfb info` behaves the same. A signed
  `--debug` build's content signature covers the table.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial work, and mark moot tasks
> `- [x] ~~text~~ (moot: <evidence>)`. Fill each `Commit:` line when the phase
> lands. **An unticked box means NOT DONE.**

### Phase 1: COFF symbol table

- [x] `src/os/windows/link/pe.rs`: `struct CoffSymbolTable { records: Vec<u8>,
      strings: Vec<u8>, count: u32 }`. `write_image` takes
      `Option<&CoffSymbolTable>`, sets the two header fields, and appends the
      records and strings after `total_file_size`.
- [x] `src/os/windows/link/mod.rs:write_executable`: build the table from
      `debug_symbols(image)` (§4.1–4.2) when `image.debug_symbols` is set, and
      pass `None` otherwise. The other 4 `write_image` callers are tests in
      `pe.rs` (`grep -n "write_image(" src/os/windows/link/*.rs` → `pe.rs:405,
      423, 549, 575`), and they pass `None`. (`mod.rs:coff_symbol_table`.)
- [x] Address check on real output: build `--debug` `examples/hello_world` for
      `windows-x86_64`, and confirm that `nm build/hello_world.exe | grep _mfb_fn_main`
      equals `IMAGE_BASE` (`0x0001_4000_0000`, `pe.rs:21`) plus
      `symbol_rva(image, "_mfb_fn_main", …)`. The section-relative convention was
      measured in §2. A mismatch here is a bug in the section-number or offset
      mapping, not in the convention. → `nm` → `1400017ba T _mfb_fn_main`,
      `140001000 T _main`, `14000e000 R _mfb_str_0`. Cross-checked against the
      relocation-resolved code (relocations use `symbol_rva`): `objdump -d` →
      `140001148: callq 0x1400017ba <_mfb_fn_main>` and
      `leaq 0xc838(%rip), %r10 # 0x14000e000 <_mfb_str_0>`;
      `AddressOfEntryPoint 0x1000` = `_main`'s RVA.
- [x] Tests in `src/os/windows/link/mod.rs` `mod tests`:
      `debug_symbols_fill_coff_header_and_table` (header fields, record count, a
      long name through the string table, `SectionNumber`/`Value` for a text, an
      `.rdata` and a `.data` symbol), `debug_symbols_off_leaves_header_zero`, and
      `signed_debug_build_keeps_mfbsign_and_seals` (`inspect` finds `.mfbsign`,
      and `seal` succeeds). → `cargo test --bin mfb windows::link` → 43 passed.
- [x] Signed + debug in `src/cli/info/tests.rs`: a Windows case like the macOS and
      Linux ones from A and B. → `a_signed_debug_image_reports_intact_contents`
      now covers Mach-O, static ELF, dynamic ELF and PE; `cargo test --bin mfb
      cli::info` → 21 passed.

Acceptance: `llvm-nm` resolves each function to its real address.
  Check: `nm build/hello_world.exe | grep -c ' T _mfb_fn_main'` → 1, at the address computed in the convention check (est. 2 min).
  Check: `cargo test --bin mfb windows::link` → all pass (est. 3 min).
  Result (2026-10-03): `nm build/hello_world.exe | grep -c ' T _mfb_fn_main'` → 1, at
  `1400017ba` (= the call target above); `windows::link` → 43 passed.
Commit: —

### Phase 2: Spec

- [ ] `src/docs/spec/linker/10_windows-x86_64.md`: the COFF symbol table in
      `--debug` builds (placement, record layout, the `Value` convention as
      measured, and that Microsoft's debuggers need a PDB and do not read it).
- [ ] `src/docs/spec/tooling/09_debug-report.md` "Symbol tables": mark PE as done
      and state the WinDbg/VS limitation.

Acceptance: `cargo build && cargo test --bin mfb spec` → pass, and
`scripts/spec-census.sh --citations` → no new dangling citation (est. 5 min).
Commit: —

### Phase 3: PE tool proof and Windows run (local + box 206)

- [ ] `objdump --private-headers build/hello_world.exe` (host llvm-objdump) on a
      `--debug` console build: parses with no warning, and the section table and
      data directories match a build without `--debug` of the same project. Only
      the two COFF symbol fields differ.
- [ ] `objdump -t` on the same file lists the functions with
      `(sec 1)(fl 0x00)(ty 20)(scl 2)`.
- [ ] `--debug --app` build: `nm build/<name>.exe` lists the functions. This proves
      `.rsrc` and the table coexist.

- [ ] Run on Windows box 206: `scp` the `--debug` console `hello_world.exe` and
      the `--debug` `examples/brogue` `.exe` to `C:/Users/test/`, then run each
      with `ssh test@192.168.1.206 "C:\Users\test\<name>.exe 2>nul"` (one ssh per
      program; cmd expands `%ERRORLEVEL%` too early on a combined line). Each
      prints its normal output, and ssh exits 0. Delete the copies afterwards.
- [ ] Launch the `--debug --app` build on 206 using the
      `.ai/remote_systems.md` "visible window from ssh" procedure. The window
      opens.

Acceptance: llvm's PE reader accepts the `--debug` image and finds the names, and
the real `--debug` executables run on Windows.
  Check: `diff <(objdump --private-headers <nodebug>.exe | grep -v -i 'symbol') <(objdump --private-headers <debug>.exe | grep -v -i 'symbol')` → only differences that come from `--debug` codegen (section sizes), with no new section or data directory (est. 2 min).
  Check: the 206 runs above → normal output, and ssh exits 0 (est. 3 min).
Commit: —

### Phase 4: plan-166 closeout (full suite, once)

- [ ] Byte-identity of builds without `--debug` on all five targets: the
      plan-166-A Validation loop → no diff (est. 3 min).
- [ ] `rustup run 1.96.0 cargo fmt --all --check` → clean. `cargo clippy --all-targets`
      → no new warnings in touched files.
- [ ] Full suite once:
      `cargo test --no-fail-fast > /tmp/plan166-run.log 2>&1; echo "EXIT=$?"`
      then `grep -c '^failures:' /tmp/plan166-run.log` → `EXIT=0` and `0`
      (est. UNMEASURED; the run length on this host has not been measured for
      this plan. Record the observed time in Corrections). This is the one place
      the whole suite runs: the linker change touches every executable a runtime
      test builds, and only the full suite covers them all.
- [ ] Archive `plan-166-C` to `planning/completed/` (A and B are archived as they
      complete).

Acceptance: every check in this phase passes as stated.
Commit: —

## Validation Plan

- Tests: Phase 1 unit tests and the signed + debug `mfb info` case.
- Coverage check: the table path runs only when `debug_symbols` is set. The
  Phase 1 tests set it, and Phase 3 runs real `--debug` executables.
- Runtime proof: Phase 1 (llvm-nm addresses match) and Phase 3 (llvm's PE reader
  accepts the image, and the console and `--app` `--debug` builds run on box 206).
- Byte-identity of builds without `--debug`: Phase 4.
- Doc sync: Phase 2.
- Final gate: Phase 4, once for all of plan-166.

## Open Decisions

- None beyond plan-166-A's (names verbatim). A PDB stays out of scope unless it is
  asked for separately.

## Corrections

(Filled in during execution.)

## Summary

The smallest of the three formats: two header fields and a table appended past
the last section, with no layout change. The risks are which `Value` convention
tools expect and whether the loader is happy with a non-zero
`PointerToSymbolTable`. Both were measured before the plan was written: a
hand-patched `.exe` resolved correctly in llvm-nm and ran on box 206. Phase 3
repeats the run with the compiler's real output. Microsoft's own debuggers still won't show
names, because they need a PDB, and the spec says so.
