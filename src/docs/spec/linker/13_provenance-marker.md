# Provenance Marker

Every executable any in-tree linker emits carries a vendor note that
positively identifies it as MFBASIC-produced and exposes a small versioned
descriptor. It is **unconditional**: it does not depend on `--sign`, on imports,
or on the build mode, and it is emitted for all three formats — ELF, Mach-O, and
PE — using that format's carrier (a vendor-note command where one exists, a
dedicated read-only section on PE, which has none).

The marker is inert metadata. It does not change the entry point, any code or
data address, or any runtime semantics.

## Owner

The vendor string is exactly the 8 bytes `MFBasic\0` (NUL included) — the ELF
note *name*, the Mach-O `data_owner`, and the first bytes of the PE `.mfbnote`
section. It is what identifies the note; a reader must match on it, not on the
note type. Eight bytes keeps the ELF note name 4-aligned (so the note needs no
name padding) and fits the Mach-O `data_owner[16]` field, which is zero-padded to
its full width. [[src/os/note.rs:MFB_NOTE_OWNER]]

## Descriptor

All three formats carry the **same** 16-byte descriptor as the note contents. All
fields are little-endian. The `MFBasic\0` string is the note name / `data_owner` /
section prefix, not part of this payload. [[src/os/note.rs:mfb_note_descriptor]]

```text
off  size  field               value
0    4     inner magic         "MFB1"
4    2     descriptor version  1
6    2     flags               0 (reserved)
8    2     compiler major      crate version major
10   2     compiler minor      crate version minor
12   2     compiler patch      crate version patch
14   2     pad                 0
```

16 bytes keeps the ELF `descsz` a multiple of 4 (so the descriptor needs no note
padding either) and is 8-aligned for Mach-O. The compiler version comes from the
crate version; a component's leading digit run is parsed, so a pre-release suffix
still yields a number.

## ELF: `PT_NOTE`

The note is a standard `Elf64_Nhdr`-framed note — visible to `readelf -n` — with
`namesz = 8`, `descsz = 16`, and `type = 1`. Neither the name nor the descriptor
needs the note format's 4-byte padding, so the descriptor begins immediately
after the name. [[src/os/linux/link/elf.rs:mfb_note_bytes]]

A `PT_NOTE` program header covers it: `p_flags = R`, `p_align = 4`, `p_vaddr =
p_paddr = image_base + p_offset`, `p_filesz = p_memsz = note length`. Every
encoder — `encode_static_elf`, `encode_static_elf_x86`, and `encode_dynamic_elf`
— emits one. [[src/os/linux/link/elf.rs:note_program_header]]

The note lives in the padding between the program-header table and the text at
`TEXT_FILE_OFFSET`, which is inside the text `PT_LOAD`'s file range — so it is
mapped read-only and readable at runtime, and the text and data file offsets are
unchanged by its presence. A static image places it at
`align(64 + phnum * 56, 8)`; the dynamic image shares the gap with the
interpreter string and places it at `align(interp_end, 8)`.

## Mach-O: `LC_NOTE`

An `LC_NOTE` load command (cmd `0x31`, cmdsize 40) whose `data_owner` is
`MFBasic\0` addresses an out-of-line copy of the descriptor by file offset:
[[src/os/macos/link/commands.rs:note_command]]

```text
cmd            0x31 (LC_NOTE)
cmdsize        40
data_owner[16] "MFBasic\0" + zero pad
offset (u64)   file offset of the descriptor
size   (u64)   16
```

The payload is a 16-byte-aligned region placed after `__DATA` (and after the
optional `__MFB,.mfbsign` block) and before `__LINKEDIT`, which starts on the next
page. It is a bare file region owned by no `LC_SEGMENT_64`: `LC_NOTE` addresses
it by file offset, so it needs no mapping. [[src/os/macos/link/macho.rs:macho_layout]]

Because the payload sits below the `LC_CODE_SIGNATURE` blob it is inside
`codeLimit`, so the ad-hoc signature's page hashes cover it and `codesign -v`
verifies — page hashing is file-offset based, not segment based. The command and
the payload are the same size on both passes of the two-pass signing encode, so
the signature-length settle still converges (see `macos-aarch64`).

## PE: `.mfbnote` section

PE has no `PT_NOTE`/`LC_NOTE` equivalent, so the marker rides in a dedicated
read-only section named `.mfbnote` (exactly 8 bytes, so the COFF section-name
field holds it with no truncation). Its body is the `MFBasic\0` owner followed
immediately by the same 16-byte descriptor — the identical owner+descriptor
framing the ELF note uses, so a reader locates `MFBasic\0` and reads the
descriptor the same way in all three formats. [[src/os/windows/link/mod.rs:write_executable]]

The section is **unconditional** (console and GUI, signed or not) and carries no
data-directory entry. It is placed last — after `.rsrc` when a GUI build has one,
otherwise after the final functional section — at the next section/file-alignment
boundary, with characteristics `CNT_INITIALIZED_DATA | READ`. Because it follows
every other section, its presence shifts only `NumberOfSections` and
`SizeOfImage`; no earlier RVA, the entry point, or any code/data address moves.

## Relationship to executable signing

The marker is additive and orthogonal to the optional MFBASIC executable signing
metadata (the ELF `.mfbsign` section / the Mach-O `__MFB,.mfbsign` segment, which
appear only when the build supplies `signing_metadata`). Both may be present at
once, and the marker is emitted whether or not signing metadata is.

## See Also

* ./mfb spec linker macos-aarch64 — the ad-hoc code signature the payload sits inside
* ./mfb spec package-manager signing — the separate executable signing metadata
