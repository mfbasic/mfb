# plan-166-B: ELF `.symtab` for `--debug` builds

Last updated: 2026-10-03
Effort: large (estimate: 3h–1d)
Depends on: plan-166-A

Writes the plan-166-A symbol list into a `--debug` Linux executable as an ELF
`.symtab` / `.strtab`. A `.symtab` is only reachable through a section header
table, and builds without `--debug` and without signing have **no section header
table at all**, so most of this work is building one. The outcome: `nm` /
`readelf -s` / gdb / `perf` see named functions in every `--debug` ELF the
compiler emits (aarch64, x86_64 and riscv64; glibc and musl; console and AppDir),
and every build without `--debug` stays byte-identical.

References:

- `src/os/linux/link/mod.rs:encode_executable_bytes`: chooses the static or
  dynamic writer, then `seal`s.
- `src/os/linux/link/elf.rs:encode_static_elf`, `encode_static_elf_x86`,
  `encode_dynamic_elf`, `append_elf_signing_section`, `section_header`.
- `src/os/inspect.rs:inspect_elf`, `elf_section`: `mfb info` finds `.mfbsign` by
  name through the section header table. `covered_end` is the whole file.
- `src/os/debug_symbols.rs:debug_symbols` (plan-166-A).
- `mfb spec linker static-and-dynamic-output`, `linux-aarch64`, `linux-x86_64`,
  `linux-riscv64`.

## Prerequisites

The prerequisites are stated once in plan-166-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-166-A is complete | `ls planning/plan-166-A-*` → no match (archived to `planning/completed/`) | MET (2026-10-03, archived in this commit) |
| `EncodedImage.debug_symbols` and `os::debug_symbols` exist | `grep -n "debug_symbols" src/arch/image.rs src/os/debug_symbols.rs` → hits in both | MET (2026-10-03) |
| The baseline compiler from plan-166-A Phase 0 still exists | `ls /tmp/mfb-166-base` → exists | MET (2026-10-03) |

If plan-166-A is not complete, this plan cannot start.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue. If you stop, report all of them.

## 1. Goal

- A `--debug` Linux build on any of the three architectures, either libc flavor,
  console or AppDir, has a section header table with `.text`, `.data` (and
  `.rodata` where the dynamic writer splits it), `.symtab`, `.strtab`, `.shstrtab`,
  and `.mfbsign` when signed. `.symtab` lists every `debug_symbols` entry as
  `STB_LOCAL` / `STT_FUNC` or `STT_OBJECT`, with its runtime address and size.
- Every build without `--debug`, signed or unsigned, is byte-identical to the
  pre-plan compiler.

### Non-goals (explicit constraints)

- No change to unsigned builds without `--debug` (the plan-166-A constraint).
  ~~Including the signed-build section table~~ (corrected: a signed **dynamic**
  table gains `.dynstr` + `.dynamic` to fix a found readelf error; see
  Corrections. The signed static table is unchanged.)
- No section headers for the rest of the dynamic payload (`.dynsym`, `.rela.*`,
  `.got`, `.plt`). The loader reads it through program headers.
  (Corrected: `.dynstr` and `.dynamic` *are* described, because readers require
  a section covering `PT_DYNAMIC` once any section table exists.)
- No `.dynsym` entries for internal symbols. That would export names at runtime.
- No names for the PLT stubs appended after `image.text`.
- No change to program headers, load addresses, `ET_EXEC` / `ET_DYN` choice, or
  the content-signature format.

## 2. Current State

**Three ELF writers, one shared tail.** `encode_executable_bytes` picks
`encode_static_elf_x86` (x86_64, no imports), `encode_static_elf` (aarch64/riscv64,
no imports) or `encode_dynamic_elf` (any import). All three write
`e_shoff = 0`, `e_shnum = 0`, `e_shentsize = 0`, `e_shstrndx = 0`
(`elf.rs` header writes, e.g. lines 78–86 and 158–166). When signed, each one ends
by calling `append_elf_signing_section`. That appends the `.mfbsign` body, then
`.shstrtab` (`"\0.mfbsign\0.shstrtab\0"`), then a three-entry section header table
(NULL, `.mfbsign`, `.shstrtab`), and patches `e_shoff`, `e_shentsize = 64`,
`e_shnum = 3`, `e_shstrndx = 2`. `encode_executable_bytes` then `seal`s.
`inspect_elf` reports `covered_end = bytes.len()`, so anything appended before the
seal is covered by the content signature.

**Layout facts the section headers must match** (read from the writers):

- Text: file offset `TEXT_FILE_OFFSET` (`0x1000`). Address
  `IMAGE_BASE + 0x1000` for static output (`ET_EXEC`, `IMAGE_BASE = 0x400000`), or
  `DYN_IMAGE_BASE + 0x1000` for dynamic output (`ET_DYN`, base 0). Its length is
  `text.len()`, which includes the PLT stubs appended after `image.text`.
- Data: file offset `align(0x1000 + text.len(), PAGE_SIZE)`, address = base + that
  offset, length `image.data.len()`. The dynamic writer places the
  `DynamicPayload` after it, which is not part of `.data`. The dynamic writer
  carves the read-only prefix `rodata_size = image.rodata_size.min(data.len())`
  into its own `PT_LOAD`. The static writers do not (their relocations use
  `rodata: None`).
- Symbol addresses use the same rule as `patch_relocations`: `symbol_vmaddr` with
  `rodata: None` on Linux. Data symbols are therefore `data_vmaddr + offset`,
  whether or not the dynamic writer maps a prefix read-only.

**Tools today**: `nm build/hello-glibc.out` → `no symbols` on a `--debug`
linux-aarch64 and linux-x86_64 build (measured 2026-10-03, host llvm-nm).

### Measured populations

| What | Count | Command |
|---|---|---|
| ELF writer functions that write the ELF header | 3 | `grep -n "^pub(super) fn encode_.*elf" src/os/linux/link/elf.rs` |
| Callers of `append_elf_signing_section` | 3 (all three writers) | `grep -c "append_elf_signing_section(" src/os/linux/link/elf.rs` → 4, including the definition |
| Symbols for brogue | 400 (244 functions, 156 data objects) | plan-166-A measured populations |
| Linux targets × flavors | 3 × 2 | `ls build/` after a `--target linux-<arch>` build → `<name>-glibc.out`, `<name>-musl.out` |
| Remote tool availability (owner: 2226 and 2229 are the boxes for this plan; re-probed 2026-10-03 one tool per `command -v`: 2229 has readelf, objdump, nm, addr2line, no gdb) | 2229 (riscv64 musl): `readelf`, `objdump`, `nm`, `addr2line`; no `gdb`, no `perf`. 2226 (aarch64 glibc, Debian 12): none installed, but a rootless install works (§Phase 3) and gives gdb 13.1 plus binutils 2.40, and `gdb -batch -ex starti` traces an mfb binary there | `ssh -p 2229 test@127.0.0.1 'command -v gdb perf readelf objdump nm addr2line'`; on 2226 the Phase 3 recipe, then `root/usr/bin/gdb --version` → `GNU gdb (Debian 13.1-3) 13.1` (2026-10-03) |

### Verified properties

- **Signed and `--debug` coexist only if one table holds every section.** The
  signing path writes its own complete table and patches the header. A second
  table would orphan the first. The design therefore generalizes
  `append_elf_signing_section` into one builder (read
  `elf.rs:append_elf_signing_section`).
- **`inspect_elf` / `elf_section` find `.mfbsign` by name**, so extra sections
  before it are fine (read `inspect.rs:elf_section`).
- **All three writers sign the same way.** Each ends with
  `if let Some(metadata) = signing_metadata { append_elf_signing_section(…) }`
  (read `encode_static_elf`, `encode_static_elf_x86`, `encode_dynamic_elf`).

## 3. Design Overview

One new function replaces the signing tail:
`append_section_table(bytes, header_fields, sections: &[ElfSection])`. It appends
each non-`NOBITS` section body that is not already in the file (symtab, strtab,
`.mfbsign`, shstrtab), then the header table, and patches `e_shoff` /
`e_shentsize` / `e_shnum` / `e_shstrndx`. Each writer builds the section list from
its own layout values:

- **unsigned, no `--debug`**: call nothing (unchanged, still no table);
- **signed, no `--debug`**: `[NULL, .mfbsign, .shstrtab]`, which must reproduce
  today's bytes exactly (the existing signing tests stay green unedited, and the
  `cmp` gate covers it);
- **`--debug`**: `[NULL, .text, (.rodata), .data, .symtab, .strtab, (.mfbsign),
  .shstrtab]`.

**Correctness risk** is low for running programs: section headers are not read
by the kernel or the dynamic loader. The risk is in tools, which reject or
misread a table whose offsets, `sh_link`, `sh_info` or `st_shndx` are wrong.
Phase 3 checks with `readelf -S -s` (which is strict) and runs the binary.

**Design uncertainty** is low. These are the standard ELF structures.

**Byte-identity is the gate for builds without `--debug`** (signed and
unsigned), checked by `cmp` against `/tmp/mfb-166-base`. **Expected to differ**:
every `--debug` ELF output.

**Rejected alternatives:**

- *Keep `append_elf_signing_section` and add a second appender for symbols.* Two
  tables can't coexist, and patching the header twice is fragile.
- *`STB_GLOBAL` symbols.* `.symtab` requires locals first, with `sh_info` = first
  global. All-local is simpler, and tools treat local `STT_FUNC` the same.
- *Section headers for the whole dynamic payload.* Not needed for symbols, and
  out of scope.

## 4. Detailed Design

### 4.1 Section list for a `--debug` build

| # | Name | `sh_type` | `sh_flags` | `sh_addr` | `sh_offset` | `sh_size` | `sh_link` | `sh_info` | `sh_addralign` | `sh_entsize` |
|---|---|---|---|---|---|---|---|---|---|---|
| 0 | (null) | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| 1 | `.text` | PROGBITS (1) | ALLOC\|EXECINSTR (6) | text vmaddr | `0x1000` | `text.len()` | 0 | 0 | 16 | 0 |
| 2 | `.rodata` (dynamic writer, `rodata_size > 0` only) | PROGBITS | ALLOC (2) | data vmaddr | data offset | `rodata_size` | 0 | 0 | 16 | 0 |
| 3 | `.data` | PROGBITS | ALLOC\|WRITE (3) | data vmaddr + rodata part | data offset + rodata part | `data.len()` − rodata part | 0 | 0 | 16 | 0 |
| 4 | `.symtab` | SYMTAB (2) | 0 | 0 | 8-aligned, past the file end | `24 × (1 + L)` | index of `.strtab` | `1 + L` (all local) | 8 | 24 |
| 5 | `.strtab` | STRTAB (3) | 0 | 0 | after `.symtab` | len | 0 | 0 | 1 | 0 |
| 6 | `.mfbsign` (signed only) | PROGBITS (1) | 0 | 0 | as today | as today | 0 | 0 | 1 | 0 |
| 7 | `.shstrtab` | STRTAB | 0 | 0 | after the above | len | 0 | 0 | 1 | 0 |

The `.mfbsign` row copies today's `section_header(…, 1, 0, 0, …, 1, 0)` values
exactly. Indices shift when `.rodata` or `.mfbsign` is absent. `e_shstrndx` is the
`.shstrtab` index.

### 4.2 Symbol entries (24 bytes each)

Entry 0 is all zero. Then one entry per `debug_symbols` item:

- `st_name`: the offset into `.strtab`, which starts with NUL;
- `st_info`: `(STB_LOCAL << 4) | type`, where type is `STT_FUNC` (2) for a
  Function and `STT_OBJECT` (1) for an Object;
- `st_other`: 0;
- `st_shndx`: `.text` for text. For data, `.rodata` when `offset < rodata_size`
  and `.rodata` exists, otherwise `.data`;
- `st_value`: `symbol_vmaddr(…, rodata: None)` with the writer's text/data
  vmaddr;
- `st_size`: from `debug_symbols`.

## Compatibility / Format Impact

- **Builds without `--debug`**: unchanged, signed or not (gated by `cmp` and the
  unedited signing tests).
- **`--debug` ELF**: gains a section header table and the bytes it points at,
  past the end of the last `PT_LOAD`. No program header, address or loaded byte
  changes. `mfb info` behaves the same. The content signature covers the new
  bytes, because the seal runs after them.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial work, and mark moot tasks
> `- [x] ~~text~~ (moot: <evidence>)`. Fill each `Commit:` line when the phase
> lands. **An unticked box means NOT DONE.**

### Phase 1: One section-table builder (no behavior change)

Replaces `append_elf_signing_section` with the general builder, used only for the
signed case. Output is unchanged.

- [x] `src/os/linux/link/elf.rs`: add `struct ElfSection` and
      `append_section_table`. Re-express `append_elf_signing_section` as a call to
      it with `[NULL, .mfbsign, .shstrtab]`. (`append_section_table` adds the
      NULL and `.shstrtab` entries itself; `mfbsign_section` builds the row.)
- [x] Tests: the existing signed-ELF tests in `src/os/linux/link/tests.rs` and
      `src/cli/info/tests.rs` pass **unedited**.

Acceptance: signed and unsigned builds without `--debug` are byte-identical to
the baseline.
  Check: `cargo test --bin mfb linux::link` then `cargo test --bin mfb cli::info` → all pass, and `git diff --stat -- src/os/linux/link/tests.rs src/cli/info/tests.rs` is empty (est. 4 min).
  Check: the plan-166-A byte-identity loop, restricted to the three Linux targets → no diff (est. 2 min).
  Result (2026-10-03): `cargo test --bin mfb linux::link` → 45 passed;
  `cargo test --bin mfb cli::info` → 21 passed; the `git diff --stat` of both
  test files → empty; the identity loop over all five targets (a superset) →
  `IDENTICAL`, 16 files. Signed builds cannot be compared by `cmp`: each
  `--sign` build mints a one-off key, so two signed builds differ by design.
  The unedited signing tests are what cover the signed table (Corrections).
Commit: f720c47c1

### Phase 2: `.symtab` for `--debug`

Fills in the section list from §4.1 and the symbol entries from §4.2 when
`image.debug_symbols` is true.

- [x] `elf.rs`: each of the three writers builds its section list from its own
      layout values (§2) and calls `append_section_table` when `debug_symbols`
      or signed. The dynamic writer passes `rodata_size`. (Shared tail
      `append_elf_sections(bytes, &LoadedSections{…}, debug, signing)`; the static
      writers gained a `debug: Option<&[DebugSymbol]>` parameter, the dynamic one
      reads `image.debug_symbols`.)
- [x] `elf.rs`: `symtab_bytes(image, text_vmaddr, data_vmaddr, shndx)` and
      `strtab_bytes(…)` from `debug_symbols(image)`. (One `symtab_bytes` returns
      both tables, so a name offset can never drift from its string.)
- [x] Tests in `src/os/linux/link/tests.rs` that parse the emitted bytes:
      `debug_symbols_static_aarch64`, `debug_symbols_static_x86`,
      `debug_symbols_dynamic_with_rodata`. Each checks `e_shnum`, the
      `.symtab` `sh_link`/`sh_info`, every symbol's `st_shndx`, `st_value`
      (against `symbol_vmaddr`) and `st_size`. Also
      `debug_symbols_signed_keeps_mfbsign_findable` (`inspect` finds
      `.mfbsign`, and `seal` succeeds), and `debug_symbols_off_writes_no_table`.
      → `cargo test --bin mfb linux::link` → 50 passed. The `cli::info` signed +
      debug test gained static and dynamic ELF cases → passes.

Acceptance: every `--debug` Linux target and flavor lists the program's functions.
  Check: `for t in linux-aarch64 linux-x86_64 linux-riscv64; do mfb build -q --debug --target $t /tmp/p166/hello; for f in /tmp/p166/hello/build/*.out; do nm $f | grep -c ' t _mfb_fn_main'; done; done` → `1` six times (est. 1 min).
  Result (2026-10-03, dev compiler, `bash /tmp/p166-b2check.sh`): `1` for each of
  aarch64/x86_64/riscv64 × glibc/musl (six). `objdump -h` on the aarch64 glibc build
  lists `.text`, `.rodata`, `.data`, `.symtab`, `.strtab`, `.shstrtab`, no warning.
Commit: 9b5cb44a4

### Phase 3: Tool proof on Linux

- [x] Strict parse: `ssh -p 2229 test@127.0.0.1` with a copied `--debug`
      riscv64 musl build: `readelf -S -s <file>` prints no warnings and lists the
      functions. Then run the binary → its normal output and exit status. Box
      2229 is used because it is up, has binutils, and riscv64 is native there.
      → First run: `readelf: Error: no .dynamic section in the dynamic segment`
      (a pre-existing bug in signed dynamic builds too; fixed, see Corrections).
      After the fix: `readelf -S -s -d --wide hello-musl.out` → exit 0, 0 stderr
      lines, sections `.text .rodata .data .dynstr .dynamic .symtab .strtab
      .shstrtab`, `2: 0000000000001308 140 FUNC LOCAL DEFAULT 1 _mfb_fn_main`;
      `./hello-musl.out` → `Hello World`, exit 0.
- [x] Native aarch64 on 2226 (aarch64 glibc). gdb is not installed there; set it
      up without root (measured working 2026-10-03; `/tmp` may have been
      cleared since):
      ```sh
      ssh -p 2226 test@127.0.0.1 'mkdir -p /tmp/p166tools && cd /tmp/p166tools &&
        apt-get download gdb binutils binutils-aarch64-linux-gnu libbinutils libctf0 \
          libctf-nobfd0 libbabeltrace1 libsource-highlight4v5 libdebuginfod1 \
          libboost-regex1.74.0 && for f in *.deb; do dpkg -x $f root; done'
      ```
      Then copy a `--debug` `examples/brogue` `-glibc.out` there and run
      `LD_LIBRARY_PATH=$PWD/root/usr/lib/aarch64-linux-gnu root/usr/bin/gdb -nx -batch -ex 'b mfb_fn_main' -ex run -ex bt <file>`.
      It should stop in `mfb_fn_main` with a named backtrace.
      → The tools were still in `/tmp/p166tools`. gdb needs `b _mfb_fn_main` (ELF
      keeps the leading `_`; see Corrections). `gdb -nx -batch -ex 'b _mfb_fn_main'
      -ex 'b _mfb_fn_dumpRng' -ex 'run rng 1 3' -ex bt -ex continue -ex bt` →
      `Breakpoint 1, 0x0000aaaaaaad5614 in _mfb_fn_main ()`, `#1 … in _main ()`;
      `Breakpoint 2, … in _mfb_fn_dumpRng ()`, `#1 … in _mfb_fn_main ()`,
      `#2 … in _main ()`.
- [x] `addr2line -f -e <file> <address of _mfb_fn_main>` on 2229 (riscv64 build)
      → `_mfb_fn_main`. Run
      `root/usr/bin/aarch64-linux-gnu-readelf -S -s` on the 2226 aarch64 build →
      no warnings. → `addr2line -f -e hello-musl.out 0x1308 0x1310` →
      `_mfb_fn_main` twice (`??:?` for the line: names only). The 2226 readelf on
      the brogue `-glibc.out` → exit 0, 0 stderr lines, 250 FUNC / 200 OBJECT.
- [x] Linux AppDir: `mfb build --debug --app --target linux-aarch64` and `nm` on
      the ELF inside `build/<name>-glibc.AppDir/` lists the functions. This
      proves the AppDir path shares `encode_executable_bytes`. → `--app` alone
      leaves only the AppImage, so `--app-debug` (which keeps the AppDir) was used:
      `nm …/hello_world-glibc.AppDir/usr/bin/hello_world` → 43 `t` symbols,
      `000000000000293c t _mfb_fn_main`; musl AppDir → 1 match.

Acceptance: a debugger on Linux stops at a function by name.
  Check: the 2226 gdb transcript above shows `Breakpoint 1, … mfb_fn_main` (est. 10 min including the tool setup; a full gdb session is the only check that proves a debugger accepts the table).
  Result (2026-10-03): `Breakpoint 1, 0x0000aaaaaaad5614 in _mfb_fn_main ()`.
Commit: —

### Phase 4: Spec

- [ ] `src/docs/spec/linker/05_static-and-dynamic-output.md`: a `--debug` builds
      section covering the section header table, the `.symtab` contents, and
      that a build without `--debug` has none (a signed build has the
      three-entry `.mfbsign` table).
- [ ] `src/docs/spec/linker/07_linux-aarch64.md`, `08_linux-x86_64.md`,
      `09_linux-riscv64.md`: one line each pointing to it, if they describe the
      section table.
- [ ] `src/docs/spec/tooling/09_debug-report.md` "Symbol tables": mark ELF as
      done.

Acceptance: `cargo build && cargo test --bin mfb spec` → pass, and
`scripts/spec-census.sh --citations` → no new dangling citation (est. 5 min).
Commit: —

## Validation Plan

- Tests: Phase 1 (unchanged signing tests) and Phase 2 (new parse tests).
- Coverage check: the three writers each get a `--debug` parse test. The signed
  path is covered by the unedited existing tests plus the new signed + debug test.
- Runtime proof: Phase 3 (readelf strict parse, gdb breakpoint, addr2line, and the
  binary still runs).
- Byte-identity of builds without `--debug`: the plan-166-A loop, all five
  targets, after Phase 4 (est. 3 min).
- Doc sync: Phase 4.
- Final gate: in plan-166-C.

## Open Decisions

- None beyond plan-166-A's (names verbatim).

## Corrections

- **Signed byte-identity is covered by the unedited tests, not by `cmp`.** The
  plan names `cmp` against the baseline for signed builds without `--debug`.
  That cannot work: `--sign` mints a fresh one-off signing key per build
  (`ExecutableSigning.signing_private`), so two signed builds of one project
  differ even on one compiler. The signed section table is instead pinned by the
  existing signing tests, which run unedited
  (`encode_static_elf_x86_appends_signing_section` and the `cli::info` signed
  cases), and the unsigned outputs by the `cmp` loop.
- **Static writer signature (Phase 2).** The static writers never received the
  image, so `encode_static_elf` / `encode_static_elf_x86` gained a trailing
  `debug: Option<&[DebugSymbol]>`. The 11 existing test call sites in
  `src/os/linux/link/tests.rs` pass `None` there. That is a mechanical argument,
  not a weakened assertion: every existing assertion is unchanged.
- **Found bug: a dynamic ELF with section headers had no `.dynamic` section.**
  GNU readelf 2.4x on box 2229 printed `readelf: Error: no .dynamic section in
  the dynamic segment` for the first `--debug` riscv64 musl build. This predates
  the plan. A base-compiler (9270cad02) riscv64 musl build given the exact
  pre-plan signed table (`/tmp/p166-signedtable.py`, a byte-for-byte replay of
  the old `append_elf_signing_section`) prints the same error. So every **signed**
  dynamic build already had it. Fix: whenever the dynamic writer emits a table
  (signed or `--debug`), it now includes `.dynstr` and `.dynamic` (`SHT_DYNAMIC`,
  `sh_link` → `.dynstr`, `sh_entsize` 16), covering exactly `PT_DYNAMIC`
  (`append_elf_sections`; tests `signed_dynamic_table_names_the_dynamic_segment`,
  `assert_dynamic_section_covers_pt_dynamic`). The §1 non-goal "no change to the
  signed-build section table" and the §Non-goals "no section headers for the
  dynamic payload" are corrected accordingly. A signed dynamic build's table is
  now `[NULL, .dynstr, .dynamic, .mfbsign, .shstrtab]`. The signed **static** table
  is unchanged (`[NULL, .mfbsign, .shstrtab]`; static images have no
  `PT_DYNAMIC`), and unsigned builds without `--debug` still carry no table, so
  the `cmp` gate is unaffected. AGENTS.md "never leave a bug you found" outranks
  the plan's constraint.
- **gdb breakpoint name on ELF.** The plan's `b mfb_fn_main` comes from the
  Mach-O habit: lldb and atos drop the leading `_` from Mach-O symbols. gdb on ELF
  uses the symbol verbatim: `b mfb_fn_main` → `Function "mfb_fn_main" not
  defined.`, `b _mfb_fn_main` → breaks. The spec says so.

## Summary

The work is building a section header table where there was none, and folding
the signing table into it so signed `--debug` builds stay verifiable. Loaders
ignore section headers, so a mistake shows up in tools, not as crashes. Phase 3
uses the strictest reader available (`readelf`) and a real gdb session. Builds
without `--debug`, signed or not, keep their exact bytes.
