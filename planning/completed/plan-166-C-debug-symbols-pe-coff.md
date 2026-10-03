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
Commit: dc2c9d681

### Phase 2: Spec

- [x] `src/docs/spec/linker/10_windows-x86_64.md`: the COFF symbol table in
      `--debug` builds (placement, record layout, the `Value` convention as
      measured, and that Microsoft's debuggers need a PDB and do not read it).
- [x] `src/docs/spec/tooling/09_debug-report.md` "Symbol tables": mark PE as done
      and state the WinDbg/VS limitation.

Acceptance: `cargo build && cargo test --bin mfb spec` → pass, and
`scripts/spec-census.sh --citations` → no new dangling citation (est. 5 min).
  Result (2026-10-03): `cargo test --bin mfb spec` → 44 passed; `--citations` →
  `MISS-PATH 0 / MISS-LINE 0 / MISS-SYMBOL 0`; `--links linker tooling` →
  `unresolved=0`; `--render linker tooling` → 0 leaks. The plan's "MinGW-style
  gdb" and an `llvm-symbolizer` mention were not written into the spec: neither
  tool is on this host to verify (Corrections).
Commit: b028bc7e0

### Phase 3: PE tool proof and Windows run (local + box 206)

- [x] `objdump --private-headers build/hello_world.exe` (host llvm-objdump) on a
      `--debug` console build: parses with no warning, and the section table and
      data directories match a build without `--debug` of the same project. Only
      the two COFF symbol fields differ. → Both parse with 0 stderr lines
      (`bash /tmp/p166-c3.sh`). The header diff is only `--debug` codegen:
      `SizeOfCode`, `SizeOfImage`, the Import/BaseReloc/IAT directory RVAs, and six
      extra kernel32 imports. No new section and no new data directory. Because
      `--debug` changes codegen, the "only the two fields" claim is proven on one
      image instead: `debug_symbols_off_leaves_header_zero` now asserts that the
      same image with the table is the table-less bytes plus the appended table,
      differing only in `PointerToSymbolTable`/`NumberOfSymbols` (Corrections).
- [x] `objdump -t` on the same file lists the functions with
      `(sec 1)(fl 0x00)(ty 20)(scl 2)`. → `[ 1](sec  1)(fl 0x00)(ty  20)(scl   2)
      (nx 0) 0x000007ba _mfb_fn_main`.
- [x] `--debug --app` build: `nm build/<name>.exe` lists the functions. This proves
      `.rsrc` and the table coexist. → 30 `T` symbols, `14000368f T _mfb_fn_main`,
      with `.rsrc` at `0x140015000`.

- [x] Run on Windows box 206: `scp` the `--debug` console `hello_world.exe` and
      the `--debug` `examples/brogue` `.exe` to `C:/Users/test/`, then run each
      with `ssh test@192.168.1.206 "C:\Users\test\<name>.exe 2>nul"` (one ssh per
      program; cmd expands `%ERRORLEVEL%` too early on a combined line). Each
      prints its normal output, and ssh exits 0. Delete the copies afterwards.
      → hello_world printed its greeting list (Abkhaz … Greek), `ssh exit=0`;
      brogue (`rng 1 5`, it is a CLI) printed `0 range 6340` … `generated 16`,
      `ssh exit=0`. Copies deleted.
- [x] Launch the `--debug --app` build on 206 using the
      `.ai/remote_systems.md` "visible window from ssh" procedure. The window
      opens. → `/it` task with `MFB_WINAPP_DUMP=1`: the UI thread read the
      transcript back from the window's edit control and wrote it out
      (`Abkhaz: Салам Адунеи …`, capped by the 512-byte `_mfb_winapp_testbuf`),
      then `exit=0`. The readback runs only when the edit control's HWND is
      non-zero (`emit_main`), so the window existed. Task and files deleted.

Acceptance: llvm's PE reader accepts the `--debug` image and finds the names, and
the real `--debug` executables run on Windows.
  Check: `diff <(objdump --private-headers <nodebug>.exe | grep -v -i 'symbol') <(objdump --private-headers <debug>.exe | grep -v -i 'symbol')` → only differences that come from `--debug` codegen (section sizes), with no new section or data directory (est. 2 min).
  Check: the 206 runs above → normal output, and ssh exits 0 (est. 3 min).
  Result (2026-10-03): the diff shows only codegen-driven size/RVA lines and six
  extra imports, with no new section or directory; both 206 runs → normal output,
  `ssh exit=0`.
Commit: 2d16da988

### Phase 4: plan-166 closeout (full suite, once)

- [x] Byte-identity of builds without `--debug` on all five targets: the
      plan-166-A Validation loop → no diff (est. 3 min). → release build of
      e82f0bb0a (`/tmp/mfb-166-C`): `bash /tmp/p166-identity.sh /tmp/mfb-166-C` →
      `IDENTICAL`, 16 files, no `BUILD FAILED`. The later 1f0a75aa3 touches only
      a test file's lint allow and two pure test-module moves, so no production
      code changed after it.
- [x] `rustup run 1.96.0 cargo fmt --all --check` → clean. `cargo clippy --all-targets`
      → no new warnings in touched files. → The first `--check` flagged hunks in 6
      plan files only, so `cargo fmt --all` ran and was committed (e82f0bb0a); the
      re-check is clean. The first clippy run **failed** (exit 101) on pre-existing
      deny-level errors in files plan-166 never touched:
      `approx_constant` in `tests/runtime/rt_math_fixed_trig_accuracy.rs` and
      `items_after_test_module` in `inplace_dest.rs` / `builder_emit_helpers.rs`.
      They are fixed in 1f0a75aa3 (Corrections), and clippy now exits 0. In
      touched files, the one warning is `windows/link/mod.rs:865`
      `manual_is_multiple_of`, from acaf39979 (2026-09-04), not new.
- [x] Full suite once:
      `cargo test --no-fail-fast > /tmp/plan166-run.log 2>&1; echo "EXIT=$?"`
      then `grep -c '^failures:' /tmp/plan166-run.log` → `EXIT=0` and `0`
      (est. UNMEASURED; the run length on this host has not been measured for
      this plan. Record the observed time in Corrections). This is the one place
      the whole suite runs: the linker change touches every executable a runtime
      test builds, and only the full suite covers them all.
      → It ran three times, because each of the first two found pre-existing bugs
      (Corrections). The final run on 35d720139: `EXIT=0`,
      `grep -c '^failures:'` → `0`, 280 test binaries, 6552 passed / 0 failed,
      `artifact-gate [all]: … 2200 golden(s) checked, 0 diff(s)`, 83.9 min
      (run 2: 73.9 min). After the fixes, the identity loop reran on a fresh
      release build (`/tmp/mfb-166-final`) → `IDENTICAL`, 16 files.
- [x] Archive `plan-166-C` to `planning/completed/` (A and B are archived as they
      complete). → moved in the commit that ticks this box.

Acceptance: every check in this phase passes as stated.
  Result (2026-10-03): all four pass (see each task line).
Commit: f76a2b76a

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

- **Unverified tool claims kept out of the spec (Phase 2).** The plan's outcome
  line names "MinGW-style gdb". No gdb (`gdb`, `x86_64-w64-mingw32-gdb`) and no
  `llvm-symbolizer` is installed on this host (`which` → not found), so the
  spec names only the tools measured: `llvm-nm` and `llvm-objdump -t`.
- **Found: `cargo clippy --all-targets` failed at HEAD (Phase 4).** It exited 101
  on deny-level lints in files no plan-166 commit touches (`git diff 9270cad02
  --stat -- tests/` was empty): `approx_constant` on the deliberate pi/2
  truncations in `rt_math_fixed_trig_accuracy.rs:corpus` (a targeted `#[allow]`
  with the reason), and `items_after_test_module` in
  `src/codegen/collection/assign/inplace_dest.rs` and
  `src/codegen/engine/builder/builder_emit_helpers.rs` (each test module moved
  to the end of its file; the sorted line sets match HEAD). Fixed in its own
  commit, 1f0a75aa3. After the fix: clippy exit 0.
- **Found by the first full-suite run: duplicate macOS TLS data symbols.** The
  first `cargo test --no-fail-fast` run had 2 failures,
  `rt_debug_soak::an_http_read_loop_keeps_live_bytes_constant` and
  `an_http_read_loop_leaves_no_block_behind`, both reporting
  `error: debug symbols: duplicate symbol '_mfb_tls_sym_CFArrayGetCount'`. The
  `--debug` table's duplicate refusal (plan-166-A §4.2) worked as designed.
  The duplicate was real and pre-existing: `--ncode` of a one-line `http::read`
  program listed `_mfb_tls_sym_CFArrayGetCount`, `…CFArrayGetValueAtIndex` and
  `…CFRelease` twice, with identical bytes, because
  `gen_macos::data_objects(server=true)` emitted them for the verify-block table
  and again from `SERVER_SYMBOLS`. Every server-capable macOS TLS build carried
  three dead strings, and relocations resolved to the first copy. Fixed at the
  source, and `NativeCodePlan::validate` now refuses any symbol defined twice,
  so a recurrence fails every build at codegen (5463ca1ec; tests
  `data_objects_define_each_symbol_once`, `a_symbol_defined_twice_is_refused`).
  This shrinks the non-`--debug` output of macOS TLS server-capable programs by
  the three strings; hello_world and brogue (the identity loop) use no TLS. That
  run was then stopped, because its integration tests call the rebuilt `mfb`, and
  the suite was rerun once on the fixed tree.
- **Second full-suite run (73.9 min, `EXIT=101`, 8 failures): the new
  one-definition validator exposed two more pre-existing duplicates.** (1) The
  `LINK` support merge in `builder/mod.rs` extended the module's data objects
  without the module-level dedupe, so a `LINK` program that also used
  `fs`/`thread` defined `_mfb_str_error_resource_closed` (and `_moved`) twice:
  4 unit tests plus the package-fixture test. (2) The macOS app mouse selector
  list re-emitted `_mfb_macapp_sel_modifierFlags`, which the always-on
  `keys::KEY_SELECTORS` already carries: 3 `codegen_mouse_arena_region` tests,
  and `app_mouse_surface` failing to build in the gate. Both are fixed in
  35d720139. The gate's other 2 diffs (`http`/`tls` `macos-aarch64.ncodesum`)
  were the expected effect of the TLS fix. All three re-pinned sums are proven:
  the baseline `.ncode` matches the old sum, and the new one equals the baseline
  minus exactly the unreachable last copy of each duplicated symbol, with
  everything else identical (`/tmp/p166-golddiff.py`). The 7 unit/integration
  tests then passed, and `artifact-gate.sh … http` / `tls` → `0 diff(s)`. A
  third, final full run follows on the finished tree.
- **"Mapped image matches a build without `--debug`" (Goal 2).** That cannot be
  shown by comparing a `--debug` build to a plain one, because `--debug` also
  changes codegen (the debug report helpers and imports). It was strengthened to a
  same-image check in `debug_symbols_off_leaves_header_zero`: identical bytes up
  to the old file end except the two COFF symbol fields.

## Summary

The smallest of the three formats: two header fields and a table appended past
the last section, with no layout change. The risks are which `Value` convention
tools expect and whether the loader is happy with a non-zero
`PointerToSymbolTable`. Both were measured before the plan was written: a
hand-patched `.exe` resolved correctly in llvm-nm and ran on box 206. Phase 3
repeats the run with the compiler's real output. Microsoft's own debuggers still won't show
names, because they need a PDB, and the spec says so.
