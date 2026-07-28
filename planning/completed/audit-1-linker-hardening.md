# Audit 1 — Native Linker & Executable Hardening

**Scope.** Code-grounded review of MFBASIC's hand-written Mach-O and ELF emitters
and executable hardening: `src/os/macos/link/{mod,macho,commands}.rs`,
`src/os/macos/{mod,object}.rs`, `src/os/linux/link/{mod,elf}.rs`,
`src/os/linux/{flavor,mod,object}.rs`, relocation realization in
`src/arch/{aarch64,x86_64}/reloc.rs` and `.../encode/emitter.rs`, and the build
entry `src/cli/build.rs`.

**Trust-boundary note.** This is a compile-time tool: relocation offsets, targets,
and symbol names are produced by the compiler's own encoder, not from untrusted
runtime input. So reloc-arithmetic issues below are **correctness/robustness**
concerns (silent miscompile / build-time panic), not remote-input memory
corruption. The hardening gaps, by contrast, ship in *every* end-user binary and
directly weaken the runtime security posture of programs users distribute.

**What is already correct (verified in the writer, so not re-listed as findings):**
- macOS `MH_PIE` **is** set — header flags `0x0020_0085` at `macho.rs:72` include
  `MH_PIE (0x200000)`. macOS binaries are position-independent / ASLR-capable.
- No RWX segment anywhere. macOS `__TEXT` init/maxprot = `5/5` (R+X,
  `commands.rs:17-18`); `__DATA`/`__DATA_CONST` = `3/3` (R+W, `commands.rs:62-63,
  111-112`). ELF x86 static/dynamic split R+X text (`p_flags=5`) from R+W data
  (`p_flags=6`) — `elf.rs:89,99,183-189,191-201`. W^X holds on those paths.
- The macOS ad-hoc code signature's CodeDirectory `codeLimit` = `unsigned.len()`
  (`commands.rs:506,512`), i.e. the byte offset where the signature blob begins,
  and it hashes every 4 KiB page below that (`commands.rs:519-521`). All code +
  load-command + data pages are covered; the two-pass encode
  (`macho.rs:13-27`) reserves the signature size so offsets are stable.
- No internal symbol-name leakage: the macOS symtab/strtab emit **only** imported
  libc symbols (`commands.rs:434-454`, `string_table`/`symbol_table` iterate
  `image.imports`); the dynamic ELF `.dynstr` holds only libraries + imported
  symbols + version strings (`elf.rs:347-359`); the static ELF has no symbol
  table at all. Internal MFBASIC function names in `EncodedImage.symbols` are used
  for reloc resolution only and are never written to the file.

---

## LNK-01 — HIGH: Linux binaries are non-PIE (`ET_EXEC`) at a fixed load address — ASLR fully defeated

**Location:** `src/os/linux/link/elf.rs:14,73` (`e_type = ET_EXEC`), `src/os/linux/link/mod.rs:7` (`IMAGE_BASE = 0x400000`)

**Issue:** All three ELF emitters set `e_type = 2` (`ET_EXEC`):
`encode_static_elf` (`elf.rs:14`), `encode_static_elf_x86` (`elf.rs:73`), and
`encode_dynamic_elf` (`elf.rs:143`). Every `PT_LOAD` `p_vaddr` is the compile-time
constant `IMAGE_BASE = 0x400000` (`mod.rs:7`, used at `elf.rs:34-35,91,163-201`).
An `ET_EXEC` image with fixed `p_vaddr`s is loaded at exactly `0x400000` on every
run — the kernel cannot randomize the main image. A PIE would be `e_type = 3`
(`ET_DYN`) with `p_vaddr`s expressed as offsets from 0, letting the loader pick a
random base. The Mach-O side already does the position-independent equivalent
(`MH_PIE`), so this is a Linux-only regression from the macOS baseline.

**Trigger:** With the main executable at a known fixed address, an attacker who
finds a memory-safety bug in a shipped MFBASIC program (e.g. via the runtime's
own `unsafe` FFI helpers, `dlopen`'d user `LINK` libraries, or a bug in generated
code) gets the entire text + data + GOT at predictable addresses for free. This
removes the single most valuable mitigation ASLR provides — no infoleak is needed
to build a ROP/JOP chain or to locate the writable arena global. It is an
**exploitation amplifier**: it does not create a bug, but it makes any future one
far more reliable.

**Fix:** Emit PIE. Set `e_type = 3` (`ET_DYN`) in all three encoders. Change
`IMAGE_BASE` to `0` so `p_vaddr`/`p_offset` are load-relative (a conventional PIE
uses base 0; the loader adds a random slide). This requires the entry
(`e_entry`), all `program_header` vaddrs, the GOT/PLT stub math in
`append_import_stubs`/`emit_import_stub` (`mod.rs:255-308`), and
`symbol_vmaddr`/`patch_relocations` to compute **relative** page-deltas — which
the AArch64 `adrp`/`page21` path and x86 `rel32`/GOTPCREL path already do
(they are all PC-relative), so the internal patches are slide-safe once the base
is 0. The static (interpreter-less) ELF path additionally needs the kernel to
apply the ELF as `ET_DYN`; for a no-interpreter static-PIE the program must also
self-relocate any absolute pointers — of which the emitter currently has none in
the static path except the fixed `p_vaddr`s, so setting base 0 + `ET_DYN`
suffices there. A `DT_FLAGS_1 = DF_1_PIE` entry in the dynamic table
(`elf.rs:560-588`) is also expected by some loaders/tools for the dynamic path.

---

## LNK-02 — MEDIUM: No `PT_GNU_STACK` program header — executable-stack default is left to the loader

**Location:** `src/os/linux/link/elf.rs:56-115` (static x86), `elf.rs:3-47` (static aarch64), `elf.rs:117-227` (dynamic) — no `PT_GNU_STACK` emitted anywhere

**Issue:** None of the three ELF emitters emit a `PT_GNU_STACK` (`p_type =
0x6474e551`) program header. Verified by absence: `elf.rs` emits exactly the
`PT_LOAD`(s), `PT_INTERP`, `PT_PHDR`, and `PT_DYNAMIC` headers (`e_phnum` is
hardcoded to 1, 2, or 5 at `elf.rs:26,82,126`) and there is no writer for a
`GNU_STACK` header. In the absence of `PT_GNU_STACK`, the stack executability is
determined by loader/kernel policy for the target ABI, which historically has
defaulted to **executable** on several configurations (the whole reason
`PT_GNU_STACK` exists). This most acutely affects the interpreter-less static
x86 path (`encode_static_elf_x86`), where no dynamic loader is present to impose
a policy — the kernel's `READ_IMPLIES_EXEC` legacy behavior can then map the
stack `RWX`.

**Trigger:** An executable (or `READ_IMPLIES_EXEC`) stack turns any stack-based
memory-safety bug in a shipped program into classic shellcode injection rather
than requiring ROP. Combined with LNK-01's fixed addresses, this is a large
gift to an attacker.

**Fix:** Add one `program_header(&mut bytes, 0x6474e551, /*p_flags*/ 6 /*RW,
not X*/, 0, 0, 0, 0, 0, 0)` to each encoder and bump `e_phnum` accordingly (the
static paths from 1→2 / 2→3, the dynamic path from 5→6). `RW` (flags `6`)
requests a non-executable stack.

---

## LNK-03 — MEDIUM: No RELRO — GOT and dynamic-relocation targets stay writable for the process lifetime

**Location:** `src/os/linux/link/elf.rs:191-201` (`__DATA` `PT_LOAD` is `p_flags=6` RW, contains the GOT), no `PT_GNU_RELRO` and no `DT_BIND_NOW`/`DF_BIND_NOW` in the dynamic table (`elf.rs:560-588`); macOS `__DATA_CONST` segment flags = 0, missing `SG_READ_ONLY` (`commands.rs:47-97`, segment `flags` field written as the trailing `put_u32(bytes, 0)` in `segment`/via `data_const_segment`)

**Issue (Linux):** The GOT lives inside the writable R+W data `PT_LOAD`
(`got_offset` within the data segment, `elf.rs:405-406,497,569`), and the
dynamic table sets `DT_FLAGS = DF_TEXTREL(?)`… actually `DT_FLAGS (20) = 7`
(`elf.rs:573`) which is `DF_ORIGIN|DF_SYMBOLIC|DF_TEXTREL` — notably **not**
`DF_BIND_NOW`, and there is no `DT_BIND_NOW (24)` and no `PT_GNU_RELRO`
(`0x6474e552`) program header. So the GOT is bound lazily/at-load but then
remains writable for the entire process lifetime. `DT_FLAGS = 7` including
`DF_TEXTREL (0x4)` also advertises text relocations, which some hardened loaders
treat as a reason to keep text pages writable — worth auditing separately, since
the emitter's internal relocations are all applied at build time and there should
be no runtime text relocs.

**Issue (macOS):** `__DATA_CONST` (which holds the GOT and `__mod_init_func`
pointers) is created with `initprot/maxprot = 3/3` (R+W) and segment `flags = 0`
(`commands.rs:62-65`). dyld makes a `__DATA_CONST` segment read-only after fixups
**only when the `SG_READ_ONLY (0x10)` segment flag is set**; here the segment
`flags` word is 0, so the GOT stays writable at runtime — the Apple equivalent of
missing RELRO.

**Trigger:** A writable GOT is the canonical target for converting an arbitrary
(or even relative) write primitive into control-flow hijack: overwrite a
resolved libc function pointer (e.g. the slot for a function the program calls in
a loop) and redirect execution. RELRO/`SG_READ_ONLY` removes this by making the
GOT read-only once binding completes.

**Fix (Linux):** Emit a `PT_GNU_RELRO` program header (`p_type = 0x6474e552`,
`p_flags = 4` R-only) covering the GOT (and ideally the whole
data-relocation region), and add `DT_FLAGS_1 (0x6ffffffb) |= DF_1_NOW` plus
`DT_BIND_NOW (24)` (or `DT_FLAGS |= DF_BIND_NOW 0x8`) to the dynamic table so the
loader resolves everything before flipping RELRO to read-only. Also drop
`DF_TEXTREL` from `DT_FLAGS` if there are genuinely no runtime text relocations.
**Fix (macOS):** Set the `SG_READ_ONLY (0x10)` flag in the `__DATA_CONST`
segment `flags` field (`data_const_segment`, `commands.rs`) so dyld protects the
GOT post-fixup.

---

## LNK-04 — MEDIUM: AArch64 static ELF maps the data section Read+Execute (executable constant data)

**Location:** `src/os/linux/link/elf.rs:9,31-42` (`encode_static_elf`)

**Issue:** The AArch64 static ELF emitter creates a **single** `PT_LOAD` with
`p_flags = 5` (R+X, `elf.rs:32`) whose `p_filesz`/`p_memsz` = `file_size =
TEXT_FILE_OFFSET + text.len() + data.len()` (`elf.rs:9,36-37`) and appends the
data bytes directly after text inside that same segment (`elf.rs:41-42`). The
program's constant data is therefore mapped **executable** (and, separately, is
read-only — so a program that writes the arena global on this path would also
`SIGSEGV`, implying this path is only reached by trivial import-free programs).
Contrast the x86 static path (`encode_static_elf_x86`, `elf.rs:56-115`), which
correctly emits a second R+W `PT_LOAD` for data. The AArch64 static path is a
leftover single-segment layout.

**Trigger:** Executable constant data is unnecessary attack surface: every string
literal, table, and blob in `.data` becomes a potential ROP/JOP gadget source and
a landing pad for code injection, undermining W^X on this path. Lower severity
only because AArch64 normally links libc dynamically (this path requires a
zero-import program), so it is rarely emitted.

**Fix:** Give `encode_static_elf` the same two-segment layout as
`encode_static_elf_x86`: an R+X (`p_flags=5`) `PT_LOAD` for `[0, text_end)` and a
page-aligned R+W (`p_flags=6`) `PT_LOAD` for data, and bump `e_phnum` from 1 to 2.
This both removes executable data and makes the arena-global write valid on this
path.

---

## LNK-05 — LOW: No AArch64 BTI / PAC (`GNU_PROPERTY`) — CFI hardening entirely absent

**Location:** `src/os/linux/link/elf.rs` (no `PT_GNU_PROPERTY` / `.note.gnu.property`), `src/os/macos/link/macho.rs:310-319` (`build_version`, no arm64e / PAC ABI)

**Issue:** No emitter produces a `GNU_PROPERTY` note advertising
`GNU_PROPERTY_AARCH64_FEATURE_1_BTI` (or `_PAC`, `_GCS`). Verified by absence:
grep for `GNU_PROPERTY`/`BTI`/`PAC`/`0x6474e553` across `src/` returns nothing,
and the AArch64 code generator emits no `bti` landing-pad instructions at
function/branch-target entries (the import stub at `mod.rs:302-307` is
`adrp;ldr;br` with no `bti c`). On macOS, `build_version` (`commands.rs:310-319`)
targets the standard arm64 (not arm64e) ABI, so pointer authentication is not
in effect.

**Trigger:** Without BTI, an attacker performing JOP/COP can land indirect
branches (`br`/`blr`) on any instruction, not just marked entry points; without
PAC, return addresses and function pointers are unprotected. These are
defense-in-depth mitigations; their absence broadens the gadget space for
control-flow-hijack chains but does not by itself create a bug — hence LOW.

**Fix:** For ELF, emit a `PT_GNU_PROPERTY` program header + `.note.gnu.property`
note with `GNU_PROPERTY_AARCH64_FEATURE_1_AND` bit `BTI (0x1)` set, **and** have
the AArch64 backend emit `bti c`/`bti jc` at every indirect-branch target
(function entries, import stubs) — the note without the landing pads will fault.
This is a codegen + linker change and should be sequenced behind BTI landing-pad
support. PAC/arm64e on macOS is a larger ABI change and is best treated as a
separate future track.

---

## LNK-06 — LOW: Relocation value computations silently truncate out-of-range branch/page deltas (no range check)

**Location:** `src/os/macos/link/mod.rs:470-473` and `src/os/linux/link/mod.rs:348-351` (`branch_imm26`); `page21` deltas at `macos/link/mod.rs:207-210` / `linux/link/mod.rs:115-118`; x86 `rel32` at `linux/link/mod.rs:198,212-213,227-228,233-234`

**Issue:** `branch_imm26` computes `(target - source) / 4` and masks to 26 bits
(`& 0x03ff_ffff`) with **no check** that the signed delta fits ±128 MiB:

```rust
fn branch_imm26(source: usize, target: usize) -> u32 {
    let delta = target as isize - source as isize;
    ((delta / 4) as i32 as u32) & 0x03ff_ffff   // silently truncates if out of range
}
```

The AArch64 `page21` path likewise computes `page_delta = (...) >> 12` and takes
`encoded as u32` then masks to 21 bits (`macos/link/mod.rs:207-216`) with no
range validation; the x86 `rel32` paths do `(target - (site+4)) as i32` which
wraps silently if the delta exceeds `i32` range. Because these are
compiler-generated offsets on images far smaller than 128 MiB / 4 GiB, the bug is
not currently reachable — but there is no guard, so a future large-image build
would silently emit a branch/address that resolves to the *wrong* in-image
location (a jump into arbitrary code/data) rather than failing the build.

**Trigger:** Not attacker-triggerable from program input (these are build-time,
compiler-produced values). The risk is a silent miscompile as image size grows:
a truncated `branch26` sends a call to a wrong offset inside `__TEXT`, which is a
control-flow-integrity failure introduced by the linker itself. Defensive, not
exploitable today.

**Fix:** Range-check before masking and return `Err` on overflow. For
`branch_imm26`: assert `(-(1<<27)..(1<<27)).contains(&delta)` (±128 MiB, and
4-byte-aligned) else error. For `page21`: assert the signed 33-bit page delta
fits 21 signed bits after the `>>12`. For x86 `rel32`: check the `i64` delta fits
`i32::MIN..=i32::MAX`. All of these already have an `Err` return path in
`patch_relocations`, so surfacing a "relocation out of range" error is a small,
localized change.

---

## LNK-07 — LOW/NTH: Relocation application uses unchecked slice writes into the image buffer (build-time panic on bad offset)

**Location:** `src/os/macos/link/mod.rs:475-481` and `src/os/linux/link/mod.rs:357-363` (`read_u32`/`write_u32`)

**Issue:** `read_u32`/`write_u32` index `bytes[offset..offset+4]` directly:

```rust
fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
```

`relocation.offset` is not validated against `text.len()` before the slice.
Rust bounds-checks the slice, so an out-of-range `offset` is a **panic (build
abort)**, not memory corruption — hence NTH, not a memory-safety hole. Offsets
originate from the encoder (`emitter.rs:1012,1047`, recorded as `self.text.len()`
at emit time), so they are in range by construction; there is currently no path
for a corrupted offset. Listed for completeness because the emitter and the
patcher are decoupled (the patcher trusts `EncodedRelocation.offset` blindly), so
a future refactor that reorders/rewrites text after relocations are recorded
would turn this into a hard-to-diagnose panic.

**Trigger:** Not runtime-exploitable. A compiler-internal invariant violation
(text buffer resized/reordered after reloc recording) crashes the build with a
raw index-out-of-bounds panic instead of a diagnostic.

**Fix:** Validate `offset + 4 <= text.len()` at the top of `patch_relocations`
(or inside `write_u32`/`read_u32` returning `Result`) and emit a descriptive
"relocation offset out of range" error, consistent with the existing `Err`
returns in the same function.

---

## Severity summary

| ID | Severity | Title |
|----|----------|-------|
| LNK-01 | HIGH | Linux binaries non-PIE (`ET_EXEC`) at fixed base — ASLR defeated |
| LNK-02 | MEDIUM | No `PT_GNU_STACK` — exec-stack default left to loader (worst on static x86) |
| LNK-03 | MEDIUM | No RELRO (Linux) / missing `SG_READ_ONLY` on `__DATA_CONST` (macOS) — GOT stays writable |
| LNK-04 | MEDIUM | AArch64 static ELF maps data R+X (executable constant data) |
| LNK-05 | LOW | No AArch64 BTI/PAC `GNU_PROPERTY` — CFI hardening absent |
| LNK-06 | LOW | Reloc value math silently truncates out-of-range deltas (no guard) |
| LNK-07 | NTH | Unchecked slice writes when applying relocations (build-time panic, not corruption) |

Not findings (verified correct): macOS `MH_PIE`; no RWX anywhere; ad-hoc code
signature covers all pages up to the signature; no internal-symbol/source-path
leakage in symbol/string tables.
