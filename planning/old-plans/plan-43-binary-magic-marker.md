# Binary Magic Marker Plan

Last updated: 2026-07-14
Effort: medium (1h–2h)

Embed an unconditional `"MFBasic\0"` provenance marker in every executable the
built-in linker emits, so any tool (and the runtime itself) can positively
identify an mfb-produced binary and read a small versioned descriptor from it.
The marker uses each format's blessed vendor-note mechanism: an ELF **`PT_NOTE`**
whose note name is `"MFBasic\0"`, and a Mach-O **`LC_NOTE`** whose `data_owner`
is `"MFBasic\0"`. Both carry the **same** descriptor bytes (a symmetric,
self-defined payload), and on macOS the payload sits in a signed region so the
ad-hoc code signature stays valid.

The single behavioral outcome: for a freshly built executable on every target,
`readelf -n` (ELF) / `otool -l` (Mach-O) reports a note owned by `MFBasic`, the
descriptor decodes to the expected fixed struct, and — on macOS arm64 —
`codesign -v` still passes and the binary runs.

References:

- `src/os/linux/link/elf.rs` — the three ELF encoders (`encode_static_elf`,
  `encode_static_elf_x86`, `encode_dynamic_elf`) and `append_elf_signing_section`
  (the existing mfb-owned `.mfb_sign` section precedent).
- `src/os/macos/link/macho.rs` — `encode_mach_o` / `encode_unsigned_mach_o`,
  `macho_layout`/`MachOLayout`, `code_offset`, `load_commands_size`,
  `load_command_count`; the existing `__MFB`/`__sign` signed-region precedent.
- `src/os/macos/link/commands.rs` — load-command emitters (`segment`,
  `mfb_sign_segment`, `linkedit_data`, `code_signature`).
- `src/os/{linux,macos}/link/mod.rs` — `IMAGE_BASE`/`VM_BASE`/`TEXT_FILE_OFFSET`/
  `PAGE_SIZE` constants and `EncodedImage`.
- `src/target/*/mod.rs` — where `signing_metadata` (optional, separate feature) is
  threaded into `EncodedImage`; the marker is added alongside, but is **always
  on**, not gated.
- ELF spec `Elf64_Nhdr`/`PT_NOTE`; Mach-O `struct note_command` (`LC_NOTE`,
  cmd `0x31`, cmdsize 40).
- `.ai/compiler.md` (codegen/runtime gates), acceptance golden harness
  (`scripts/sync-goldens.sh`, `scripts/artifact-gate.sh`).

## 1. Goal

- Every executable emitted by the built-in linker — Linux aarch64/x86_64/riscv64
  (static and dynamic) and macOS aarch64 — contains a vendor note whose
  owner/name is exactly the 8 bytes `MFBasic\0`, carrying an identical, versioned
  descriptor payload.
- ELF: a real `PT_NOTE` program header (visible to `readelf -n`) with
  `namesz = 8`, name `"MFBasic\0"`, and the descriptor as the note descriptor.
- Mach-O: a real `LC_NOTE` load command with `data_owner = "MFBasic\0"` and
  `offset`/`size` pointing at an out-of-line copy of the descriptor placed in a
  region covered by the ad-hoc code signature.
- macOS arm64: `codesign -v <binary>` passes and the binary runs unchanged.

### Non-goals (explicit constraints)

- **Do not change program behavior, entry point, or runtime semantics.** The
  marker is inert metadata; the entry offset and all vmaddrs of code/data must be
  unchanged in meaning.
- **Do not break the existing `signing_metadata` path** (`.mfb_sign` section /
  `__MFB` `__sign` segment, plan-23 repository trust). The marker is additive and
  orthogonal; both may be present at once.
- **Do not invalidate the macOS code signature.** The `LC_NOTE` payload must lie
  within `codeLimit` (before the signature) and the two-pass sign settle in
  `encode_mach_o` must remain correct.
- **Do not repurpose the ELF magic (`0x7F ELF`) or the Mach-O magic
  (`0xFEEDFACF`)**, and do not stash the marker in `e_ident[EI_PAD]` — use the
  proper note mechanisms.
- Marker is **unconditional** (present even without `--sign`); it must not depend
  on `signing_metadata` being `Some`.
- No new public CLI surface, no language-visible change.

## 2. Current State

Three ELF encoders in `src/os/linux/link/elf.rs` hand-assemble the file:
`encode_static_elf` and `encode_static_elf_x86` emit `e_phnum = 2` (text PT_LOAD
R+X at `p_offset 0`, data PT_LOAD R+W page-aligned); `encode_dynamic_elf` emits
`e_phnum = 5` (PT_PHDR, PT_INTERP, two PT_LOAD, PT_DYNAMIC) via the
`program_header` helper. Text always starts at `TEXT_FILE_OFFSET = 0x1000`
(`IMAGE_BASE = 0x400000`); the region `[64 + e_phnum*56, 0x1000)` between the
program-header table and text is currently zero padding and lies inside the text
PT_LOAD's file range (`p_offset 0 .. text_seg_filesz`). `e_ident[9..16]` is zeroed
by `bytes.resize(16, 0)`. There is **no** `PT_NOTE` today. `append_elf_signing_section`
(elf.rs:257) already shows the project appending an mfb-owned artifact — a
`.mfb_sign` `SHT_PROGBITS` section plus a section-header table — after the image
when `signing_metadata` is `Some`; the marker mirrors this precedent but uses a
note instead of a stripped section.

Mach-O is assembled in `src/os/macos/link/macho.rs`. `encode_mach_o` signs in two
passes: build unsigned → compute `code_signature` → rebuild unsigned with the
signature length reserved → recompute signature → append. `code_signature`
(commands.rs:497) is an ad-hoc SHA-256 SuperBlob whose `codeLimit = unsigned.len()`,
i.e. it hashes every file page **before** the `LC_CODE_SIGNATURE` blob. Load
commands are counted/sized by `load_command_count` and `load_commands_size`
(macho.rs:254/281); `code_offset` (macho.rs:242) = `align(32 + load_commands_size, 4)`,
so **any** change to the command set shifts where code begins and therefore every
downstream offset. `macho_layout`/`MachOLayout` (macho.rs:198) already computes an
optional `mfb_sign_file_offset` — a page-aligned region placed **after `__DATA`
and before `__LINKEDIT`** — proving a signed, non-`__LINKEDIT` region works on real
macOS. `mfb_sign_segment` (commands.rs:134) emits that as an `__MFB`/`__sign`
segment. There is **no** `LC_NOTE` today. (Aside: `uuid_command` already embeds
the ASCII bytes `4D 46 42` = "MFB" in the UUID — cosmetic, unrelated.)

`signing_metadata` originates in `src/target/*/mod.rs` (e.g.
`macos_aarch64/mod.rs:289`, `linux_*/mod.rs`), copied into
`EncodedImage.signing_metadata` and read by the encoders. The marker descriptor
will be produced next to that, but always populated.

## 3. Design Overview

Three independent pieces, layered lowest-risk first:

1. **Shared descriptor (`mfb_note_descriptor`)** — one function returning the
   fixed, versioned little-endian payload bytes used verbatim by both formats.
   No callers depend on it until pieces 2/3 wire it in, so it lands first with
   pure unit tests.
2. **ELF `PT_NOTE`** — add one program header (`e_phnum` 2→3 static, 5→6 dynamic)
   and lay the `Elf64_Nhdr`-framed note (`namesz`, `descsz`, `type`, name, desc)
   into the header/text gap below `TEXT_FILE_OFFSET`. Text/data offsets are
   unchanged, so golden churn is confined to the ELF header region. Lowest
   codegen risk.
3. **Mach-O `LC_NOTE`** — the risk concentrates here. Adding the command grows
   `load_commands_size` by 40, which shifts `code_offset` and thus the whole
   image; the payload must land in a signed region and the two-pass signature
   settle must stay correct. This mirrors the proven `__MFB`-segment placement,
   so the payload goes in a new page-aligned region before `__LINKEDIT`, pointed
   at by `LC_NOTE.offset/size`.

**Where correctness risk lives:** Mach-O offset bookkeeping and signing. Every
size/count function (`load_commands_size`, `load_command_count`, `code_offset`,
`macho_layout`) must be updated in lockstep or the produced file is structurally
invalid or unsigned-covering-the-wrong-bytes.

**Rejected alternatives:**
- *`e_ident[EI_PAD]` (7 free bytes) for ELF* — rejected: not self-describing, no
  descriptor room, flagged by strict validators. Non-goal forbids it.
- *A stripped `.note`-less custom ELF section (like `.mfb_sign`)* — rejected: not
  a loadable note, invisible to `readelf -n`, droppable by `strip`. `PT_NOTE` is
  the blessed, discoverable mechanism.
- *Mach-O: inline the descriptor in a load command / a new `__MFB2` segment* —
  rejected: `LC_NOTE` is purpose-built for exactly this and is what the user
  asked for; a bespoke segment is heavier and less standard. (The payload still
  lives in a signed page like `__MFB` does, for proven signing safety.)

## 4. Detailed Design — shared descriptor

`mfb_note_descriptor() -> Vec<u8>` (new, in a shared linker module reachable from
both `os/linux` and `os/macos` — e.g. `src/os/mod.rs` or a small
`src/os/note.rs`). Fixed little-endian layout, v1:

| off | size | field            | value                                        |
|-----|------|------------------|----------------------------------------------|
| 0   | 4    | inner magic      | `b"MFB1"` (0x3142_464D LE)                    |
| 4   | 2    | descriptor version | `1`                                        |
| 6   | 2    | flags            | `0` (reserved)                               |
| 8   | 2    | compiler major   | crate version major                          |
| 10  | 2    | compiler minor   | crate version minor                          |
| 12  | 2    | compiler patch   | crate version patch                          |
| 14  | 2    | pad              | `0` → total 16 bytes, 4/8-aligned            |

16 bytes keeps ELF `descsz` a multiple of 4 (no note padding) and is trivially
8-aligned for Mach-O. The `"MFBasic\0"` string is the **note name / data_owner**,
not part of this descriptor; the descriptor is the note *contents* shared by both
formats. Compiler version comes from `env!("CARGO_PKG_VERSION")` parsed once.

## 5. Detailed Design — ELF PT_NOTE

Note bytes (`Elf64_Nhdr` + payload), built once per file:

```
u32 namesz = 8                 // "MFBasic\0" incl. NUL
u32 descsz = 16                // mfb_note_descriptor().len()
u32 type   = 1                 // vendor-defined note type (NT_MFB)
[8] name   = "MFBasic\0"       // already 4-aligned, no padding
[16] desc  = mfb_note_descriptor()   // 4-aligned, no padding
```

Placement: at `note_off = align(64 + e_phnum*56, 8)`, i.e. immediately after the
(now larger) program-header table, still far below `TEXT_FILE_OFFSET = 0x1000`.
For the dynamic encoder the interpreter string also lives in this gap — place the
note **after** the NUL-terminated interp string (recompute `interp_offset` with
`ph_count = 6`, then `note_off = align(interp_end, 8)`), keeping both inside
`[phdrs_end, 0x1000)`. Add a `PT_NOTE` program header:

```
p_type = 4 (PT_NOTE), p_flags = 4 (R)
p_offset = note_off, p_vaddr = p_paddr = IMAGE_BASE + note_off
p_filesz = p_memsz = note_len, p_align = 4
```

`e_phnum` becomes 3 (static/x86 static) and 6 (dynamic). Text still starts at
`0x1000`, so `data_offset`, entry, relocations, and the dynamic payload are all
unchanged — only the header region grows within the already-reserved padding.
The note lies inside the text PT_LOAD's file range, so it is mapped R and
readable at runtime. All three encoders get the same treatment; factor the note
bytes + phdr emission into one helper to avoid drift.

## 6. Detailed Design — Mach-O LC_NOTE

New always-present region + command:

- **`MachOLayout`** gains `note_file_offset` (payload location). Compute it in
  `macho_layout` as a 16-byte-aligned region placed after `__DATA` / the optional
  `__MFB` sign region and **before** `linkedit_file_offset` (which shifts down by
  the payload size, rounded so `__LINKEDIT` stays page-aligned). Mirror the
  existing `mfb_sign_file_offset` arithmetic exactly.
- **`LC_NOTE` emitter** in `commands.rs`:

  ```
  cmd = 0x31 (LC_NOTE), cmdsize = 40
  data_owner[16] = "MFBasic\0" + zero pad
  offset (u64) = layout.note_file_offset
  size   (u64) = mfb_note_descriptor().len()   // 16
  ```

  Emit it unconditionally in `encode_unsigned_mach_o` (a natural slot is next to
  the other `linkedit_data`/metadata commands, before `LC_CODE_SIGNATURE`).
- **Payload bytes**: write `mfb_note_descriptor()` at `note_file_offset` in the
  file body (same place the `__MFB` metadata is written today). Because
  `note_file_offset < signature_offset ≤ codeLimit`, the page hashes cover it →
  signature stays valid.
- **Bookkeeping (all must move together):**
  - `load_commands_size` += 40 (unconditional).
  - `load_command_count` += 1 (unconditional).
  - `code_offset` recomputes from the new `load_commands_size` — **the whole
    image shifts**; this is expected and drives macOS golden churn.
  - Two-pass signing in `encode_mach_o` is unaffected in structure: the payload
    and command are inside `encode_unsigned_mach_o` and are the same size on both
    passes, so the signature-length settle still converges.

**Signing-coverage risk / Open Decision (§Open Decisions):** whether the kernel
accepts a signed file region not owned by any `LC_SEGMENT`. The `__MFB` segment
wraps its metadata in a segment; the `LC_NOTE` payload as designed is a bare file
gap. Recommended first attempt: bare page (simplest, and page hashing is
file-offset based, not segment based). Fallback if `codesign -v`/launch rejects
it: place the payload inside `__LINKEDIT`'s file range (before the signature) or
wrap it in a tiny read-only segment like `__MFB`.

## Compatibility / Format Impact

- **Changes:** every emitted executable gains one program header (ELF) / one load
  command + a small payload region (Mach-O). File layout shifts on macOS (code
  moves by 40+payload bytes); on ELF only the header region changes (text/data
  offsets fixed). All acceptance goldens that capture emitted binaries
  regenerate.
- **Unchanged:** entry point semantics, runtime behavior, the `signing_metadata`
  feature, ELF/Mach-O magics, language/CLI surface, and the meaning of every
  code/data vmaddr.

## Phases

### Phase 1 — shared descriptor primitive

Land the payload builder with no callers, so it is provably correct before wiring.

- [x] Add `mfb_note_descriptor()` (new `src/os/note.rs` or shared module) plus the
      `MFB_NOTE_OWNER = b"MFBasic\0"` constant.
- [x] Unit tests: descriptor is exactly 16 bytes, starts with `b"MFB1"`, version
      field == 1, and the compiler-version fields match the parsed crate version.

Acceptance: `cargo test` covering the descriptor passes; bytes match the §4 table
exactly. **DONE** — `src/os/note.rs`; 4 unit tests green. A real binary's
descriptor decodes to `MFB1 / v1 / flags 0 / 0.1.0 / pad 0`, matching §4.
Commit: —

### Phase 2 — ELF PT_NOTE (all three encoders)

Lowest codegen risk: header-region-only change, text/data offsets fixed.

- [x] Add a `PT_NOTE` phdr + note bytes helper; wire into `encode_static_elf`,
      `encode_static_elf_x86`, `encode_dynamic_elf`; bump `e_phnum` (2→3, 5→6) and
      place the note per §5 (dynamic: after the interp string).
      **Counts differed from the plan's snapshot**: static was already 3
      (PT_GNU_STACK, bug-224) → **4**; dynamic was already 6/7 (bug-186/bug-187)
      → **7/8**.
- [x] Regenerate ELF acceptance goldens for aarch64/x86_64/riscv64 static +
      dynamic via `scripts/sync-goldens.sh`. **Not needed — zero churn.** The
      goldens capture `-ncode` (codegen output), not linked binaries, and
      `code_offset`/`macho_layout`/`load_commands_size` are confined to
      `src/os/*/link/`. Nothing upstream of the linker moved.
- [x] Tests: an encoder-level test (or acceptance check) asserting a built ELF has
      a `PT_NOTE` with name `MFBasic\0` and the 16-byte descriptor.
      `static_elf_carries_the_mfbasic_provenance_note` (all 3 arches, both static
      encoders), `dynamic_elf_carries_the_mfbasic_provenance_note_past_the_interpreter`
      (all 3 arches), `provenance_note_coexists_with_the_signing_section`.

Acceptance: `readelf -n` on a freshly built Linux binary (each arch/flavor) shows
an `MFBasic` note with the expected descriptor; the binary still runs; ELF golden
diffs are confined to the header region. **DONE on hardware** — `readelf -n`
reports `Owner MFBasic / Data size 0x10 / 4d 46 42 31 01 00 …` and the program
prints and exits 0 on aarch64-glibc (Kali, 2223), x86_64-glibc (Ubuntu, 2228),
riscv64-musl (Alpine, 2229); x86_64-musl (Alpine, 2227) runs (no readelf on that
box). The static encoders were validated on all three arches with a throwaway
raw-`exit(7)` image: `readelf -n` shows the note at 0x120 and each exits 7.
Boxes 2222/2224 were down and there is no riscv64-glibc box, so aarch64-musl and
riscv64-glibc are covered by unit tests + a local note decode only — they differ
from validated combos solely in the interp string.
Commit: —

### Phase 3 — Mach-O LC_NOTE + signed payload (highest-risk work last)

- [x] Add `note_file_offset` to `MachOLayout`/`macho_layout`; add the `LC_NOTE`
      emitter to `commands.rs`; emit it + write the payload in
      `encode_unsigned_mach_o`; bump `load_commands_size` (+40),
      `load_command_count` (+1); verify `code_offset` and the two-pass sign settle.
- [x] Regenerate macOS acceptance goldens (`scripts/sync-goldens.sh`). **Not
      needed — zero churn**, for the reason given in Phase 2. `code_offset` does
      shift the image, but nothing that feeds a golden reads it.
- [x] Tests: assert a built Mach-O has an `LC_NOTE` with `data_owner == "MFBasic\0"`
      whose `offset/size` region equals the descriptor.
      `mach_o_carries_the_mfbasic_provenance_note_inside_the_signed_region` (also
      asserts the payload is below `LC_CODE_SIGNATURE`'s `codeLimit`),
      `provenance_note_coexists_with_the_mfb_sign_segment`, and the macOS-gated
      `noted_mach_o_verifies_and_runs` (`codesign -v` + exit 7, plain and signed).

Acceptance: on macOS arm64, `otool -l` shows the `LC_NOTE` (owner `MFBasic`),
`codesign -v <binary>` passes, and the binary runs. (If signing rejects the bare
region, apply the §6 fallback and re-verify.) **DONE** — on a real `mfb build`
output: `otool -l` reports `cmd LC_NOTE / cmdsize 40 / data_owner MFBasic /
offset 49152 / size 16`, `codesign -v` passes, and the program prints
"Hello World" and exits 0. **The bare-gap placement was accepted — no fallback
was needed**, resolving that Open Decision.
Commit: —

## Validation Plan

- Tests: descriptor unit tests (Phase 1); per-format note-presence assertions
  (Phases 2–3), including that the descriptor bytes round-trip.
- Runtime proof: build a trivial program for each target; ELF → `readelf -n`
  shows the `MFBasic` note and the program runs; macOS arm64 → `otool -l` shows
  `LC_NOTE`/`data_owner MFBasic`, `codesign -v` passes, program runs. Verify the
  `--sign` path still works (marker + `.mfb_sign`/`__MFB` coexist).
- Doc sync: none language-facing; if any spec/doc describes the emitted binary
  layout, note the added `PT_NOTE`/`LC_NOTE`. Update `.ai/compiler.md` only if a
  gate changes.
- Acceptance: full acceptance suite green after golden regeneration
  (`scripts/artifact-gate.sh` for the codegen path; `scripts/test-accept.sh`);
  goldens changed only as described in Compatibility / Format Impact.

## Open Decisions — all resolved

- **Mach-O payload placement** — **RESOLVED: bare 16-byte-aligned gap before
  `__LINKEDIT`**, the recommended first attempt. The kernel and `codesign` both
  accept a signed file region owned by no `LC_SEGMENT`, confirming that page
  hashing is file-offset based, not segment based: `codesign -v` passes and the
  binary runs. No fallback applied. Cost: `__LINKEDIT` rounds up to the next
  16 KiB page after the 16-byte payload, so an unsigned image grows by one page.
  (§6)
- **Note `type` value** — **RESOLVED: fixed `1`** (`MFB_NOTE_TYPE`), as
  recommended. `readelf -n` renders it as `NT_VERSION`, which is harmless — the
  `MFBasic` owner is what identifies the note, and the descriptor's own fields
  carry any discrimination. (§5)
- **Reuse the `__MFB` segment name on Mach-O** — **RESOLVED: no.** The payload is
  a bare `LC_NOTE`-referenced gap, keeping the marker independent of the plan-23
  signing segment. Both are emitted together and verified to coexist
  (`provenance_note_coexists_with_the_mfb_sign_segment`). (§6)

## Summary

The real engineering risk is entirely in Phase 3: the Mach-O size/count/offset
functions must move in lockstep and the payload must stay inside the signed
region, or the binary is invalid or its ad-hoc signature no longer verifies. ELF
(Phase 2) is a contained header-region change because text/data offsets are fixed,
and the shared descriptor (Phase 1) is pure and independently testable. Untouched:
all runtime/entry semantics, the existing `signing_metadata` feature, and every
code/data address — the marker is inert, discoverable provenance metadata using
each format's standard vendor-note mechanism.
