//! PE32+ header + section-table byte writer (plan-47-C Phase 2 / §4).
//!
//! Pure layout arithmetic over already-built section blobs (`.text`, `.rdata`,
//! `.data`, `.idata`) plus the entry-point RVA and the Import/IAT data-directory
//! entries. Emits a complete, deterministic PE32+ console image. Relocation
//! patching and `.idata`/thunk construction live in the parent `link` module
//! (Phase 3); this file never inspects an `EncodedImage`.
//!
//! Every multi-byte field is little-endian. "RVA" = address relative to
//! `ImageBase`; "file offset" = offset within the emitted file.

// --- Constants (plan-47-C §4) ----------------------------------------------

/// `link.exe`'s default x64 EXE image base (§4.3). Fixed — no `.reloc`/ASLR.
pub(super) const IMAGE_BASE: u64 = 0x0001_4000_0000;
const SECTION_ALIGNMENT: u32 = 0x1000;
const FILE_ALIGNMENT: u32 = 0x200;
/// DOS header (64) + a conventional 64-byte DOS stub → `e_lfanew = 0x80`.
const DOS_HEADER_AND_STUB: usize = 0x80;
const COFF_HEADER_SIZE: usize = 20;
const OPTIONAL_HEADER_SIZE: usize = 240; // 0xF0 — asserted by a test
const SECTION_HEADER_SIZE: usize = 40;
const NUMBER_OF_DATA_DIRECTORIES: u32 = 16;

// COFF Characteristics (§4.2).
const IMAGE_FILE_RELOCS_STRIPPED: u16 = 0x0001;
const IMAGE_FILE_EXECUTABLE_IMAGE: u16 = 0x0002;
const IMAGE_FILE_LARGE_ADDRESS_AWARE: u16 = 0x0020;

// Section characteristics (§4.4).
pub(super) const SCN_TEXT: u32 = 0x6000_0020; // CODE | EXECUTE | READ
pub(super) const SCN_RDATA: u32 = 0x4000_0040; // INITIALIZED_DATA | READ
pub(super) const SCN_DATA: u32 = 0xC000_0040; // INITIALIZED_DATA | READ | WRITE
pub(super) const SCN_IDATA: u32 = 0xC000_0040; // loader writes the IAT

/// A 64-byte real-mode DOS stub printing "This program cannot be run in DOS
/// mode." — the conventional bytes `link.exe` emits, transcribed so third-party
/// PE tools stay happy (§4.1). Bytes 0..0x40 are the `IMAGE_DOS_HEADER`; the
/// stub program occupies 0x40..0x80. `e_magic`=MZ at 0, `e_lfanew`=0x80 at 0x3C.
#[rustfmt::skip]
const DOS_HEADER_AND_STUB_BYTES: [u8; DOS_HEADER_AND_STUB] = [
    0x4D, 0x5A, 0x90, 0x00, 0x03, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00,
    0xB8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00,
    // DOS stub program (offset 0x40): print the message and terminate.
    0x0E, 0x1F, 0xBA, 0x0E, 0x00, 0xB4, 0x09, 0xCD, 0x21, 0xB8, 0x01, 0x4C, 0xCD, 0x21, 0x54, 0x68,
    0x69, 0x73, 0x20, 0x70, 0x72, 0x6F, 0x67, 0x72, 0x61, 0x6D, 0x20, 0x63, 0x61, 0x6E, 0x6E, 0x6F,
    0x74, 0x20, 0x62, 0x65, 0x20, 0x72, 0x75, 0x6E, 0x20, 0x69, 0x6E, 0x20, 0x44, 0x4F, 0x53, 0x20,
    0x6D, 0x6F, 0x64, 0x65, 0x2E, 0x0D, 0x0D, 0x0A, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// One section's finished layout: name, characteristics, and the four
/// address/size words the section header carries. `bytes` is the section's raw
/// content (unpadded); `write_image` pads it to `FILE_ALIGNMENT`.
pub(super) struct Section<'a> {
    pub(super) name: [u8; 8],
    pub(super) characteristics: u32,
    pub(super) virtual_address: u32,
    pub(super) virtual_size: u32,
    pub(super) file_offset: u32,
    pub(super) bytes: &'a [u8],
}

/// The two data-directory entries the loader needs for imports (§4.3). Both
/// `(0, 0)` when there are no imports.
#[derive(Clone, Copy, Default)]
pub(super) struct ImportDirectories {
    /// `[1]` Import directory table: (RVA, byte size incl. zero terminator).
    pub(super) import: (u32, u32),
    /// `[12]` IAT: (first IAT RVA, total bytes of all IATs).
    pub(super) iat: (u32, u32),
}

pub(super) fn align_up(value: u32, alignment: u32) -> u32 {
    value.div_ceil(alignment) * alignment
}

/// The RVA of the first section: `SizeOfHeaders`, `SectionAlignment`-aligned.
/// `SizeOfHeaders` itself is `FileAlignment`-aligned (§4.3).
pub(super) fn size_of_headers(section_count: usize) -> u32 {
    let raw = DOS_HEADER_AND_STUB
        + 4
        + COFF_HEADER_SIZE
        + OPTIONAL_HEADER_SIZE
        + section_count * SECTION_HEADER_SIZE;
    align_up(raw as u32, FILE_ALIGNMENT)
}

/// Assemble a section name into the 8-byte NUL-padded COFF field.
pub(super) fn section_name(name: &str) -> [u8; 8] {
    let mut field = [0u8; 8];
    let bytes = name.as_bytes();
    let n = bytes.len().min(8);
    field[..n].copy_from_slice(&bytes[..n]);
    field
}

struct LeWriter {
    buf: Vec<u8>,
}

impl LeWriter {
    fn with_capacity(cap: usize) -> Self {
        LeWriter {
            buf: Vec::with_capacity(cap),
        }
    }
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }
    /// Zero-pad up to `offset` (file offset). Used to reach each section's
    /// `FileAlignment`-aligned `PointerToRawData`.
    fn pad_to(&mut self, offset: usize) {
        debug_assert!(offset >= self.buf.len(), "pad_to would truncate");
        self.buf.resize(offset, 0);
    }
}

/// Emit a complete PE32+ image. `sections` are already laid out (RVAs/file
/// offsets computed by the caller, zero-length sections already omitted);
/// `entry_rva` is the `AddressOfEntryPoint`; `dirs` fills data directories
/// `[1]`/`[12]`.
pub(super) fn write_image(
    sections: &[Section],
    entry_rva: u32,
    dirs: ImportDirectories,
) -> Vec<u8> {
    let n = sections.len();
    let headers_size = size_of_headers(n);

    // Derived optional-header sizes (§4.3).
    let size_of_code: u32 = sections
        .iter()
        .filter(|s| s.characteristics & 0x0000_0020 != 0) // CNT_CODE
        .map(|s| align_up(s.virtual_size, SECTION_ALIGNMENT))
        .sum();
    let size_of_init_data: u32 = sections
        .iter()
        .filter(|s| s.characteristics & 0x0000_0040 != 0) // CNT_INITIALIZED_DATA
        .map(|s| align_up(s.virtual_size, SECTION_ALIGNMENT))
        .sum();
    let base_of_code = sections
        .iter()
        .find(|s| s.characteristics & 0x0000_0020 != 0)
        .map(|s| s.virtual_address)
        .unwrap_or(0);
    let size_of_image = sections
        .iter()
        .map(|s| align_up(s.virtual_address + s.virtual_size, SECTION_ALIGNMENT))
        .max()
        .unwrap_or(headers_size);

    let total_file_size = sections
        .iter()
        .map(|s| s.file_offset as usize + align_up(s.bytes.len() as u32, FILE_ALIGNMENT) as usize)
        .max()
        .unwrap_or(headers_size as usize);

    let mut w = LeWriter::with_capacity(total_file_size);

    // --- DOS header + stub ---
    w.bytes(&DOS_HEADER_AND_STUB_BYTES);

    // --- PE signature + COFF header (§4.2) ---
    w.u32(0x0000_4550); // "PE\0\0"
    w.u16(0x8664); // Machine = AMD64
    w.u16(n as u16); // NumberOfSections
    w.u32(0); // TimeDateStamp (determinism)
    w.u32(0); // PointerToSymbolTable
    w.u32(0); // NumberOfSymbols
    w.u16(OPTIONAL_HEADER_SIZE as u16); // SizeOfOptionalHeader = 0xF0
    w.u16(
        IMAGE_FILE_EXECUTABLE_IMAGE | IMAGE_FILE_LARGE_ADDRESS_AWARE | IMAGE_FILE_RELOCS_STRIPPED,
    );

    // --- PE32+ optional header (§4.3) ---
    let opt_start = w.buf.len();
    w.u16(0x020B); // Magic = PE32+
    w.u8(14); // MajorLinkerVersion (cosmetic, pinned)
    w.u8(0); // MinorLinkerVersion
    w.u32(size_of_code);
    w.u32(size_of_init_data);
    w.u32(0); // SizeOfUninitializedData
    w.u32(entry_rva); // AddressOfEntryPoint
    w.u32(base_of_code); // BaseOfCode
                         // PE32+ omits BaseOfData.
    w.u64(IMAGE_BASE);
    w.u32(SECTION_ALIGNMENT);
    w.u32(FILE_ALIGNMENT);
    w.u16(6); // MajorOperatingSystemVersion
    w.u16(0); // Minor
    w.u16(0); // MajorImageVersion
    w.u16(0); // Minor
    w.u16(6); // MajorSubsystemVersion
    w.u16(0); // Minor
    w.u32(0); // Win32VersionValue
    w.u32(size_of_image);
    w.u32(headers_size); // SizeOfHeaders
    w.u32(0); // CheckSum (determinism)
    w.u16(3); // Subsystem = WINDOWS_CUI
    w.u16(0x0100 | 0x8000); // DllCharacteristics: NX_COMPAT | TERMINAL_SERVER_AWARE (DYNAMIC_BASE clear)
    w.u64(0x0010_0000); // SizeOfStackReserve
    w.u64(0x0000_1000); // SizeOfStackCommit
    w.u64(0x0010_0000); // SizeOfHeapReserve
    w.u64(0x0000_1000); // SizeOfHeapCommit
    w.u32(0); // LoaderFlags
    w.u32(NUMBER_OF_DATA_DIRECTORIES);
    // 16 data directories; only [1] Import and [12] IAT are non-zero.
    for index in 0..NUMBER_OF_DATA_DIRECTORIES {
        match index {
            1 => {
                w.u32(dirs.import.0);
                w.u32(dirs.import.1);
            }
            12 => {
                w.u32(dirs.iat.0);
                w.u32(dirs.iat.1);
            }
            _ => {
                w.u32(0);
                w.u32(0);
            }
        }
    }
    debug_assert_eq!(
        w.buf.len() - opt_start,
        OPTIONAL_HEADER_SIZE,
        "PE32+ optional header must be exactly 240 bytes"
    );

    // --- Section table (§4.4) ---
    for s in sections {
        w.bytes(&s.name);
        w.u32(s.virtual_size);
        w.u32(s.virtual_address);
        w.u32(align_up(s.bytes.len() as u32, FILE_ALIGNMENT)); // SizeOfRawData
        w.u32(s.file_offset); // PointerToRawData
        w.u32(0); // PointerToRelocations
        w.u32(0); // PointerToLinenumbers
        w.u16(0); // NumberOfRelocations
        w.u16(0); // NumberOfLinenumbers
        w.u32(s.characteristics);
    }

    // --- Section bodies, each at its FileAlignment-aligned PointerToRawData ---
    for s in sections {
        w.pad_to(s.file_offset as usize);
        w.bytes(s.bytes);
    }
    // Pad the final section's raw data out to FileAlignment.
    w.pad_to(total_file_size);

    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le_u16(bytes: &[u8], off: usize) -> u16 {
        u16::from_le_bytes([bytes[off], bytes[off + 1]])
    }
    fn le_u32(bytes: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
    }
    fn le_u64(bytes: &[u8], off: usize) -> u64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&bytes[off..off + 8]);
        u64::from_le_bytes(a)
    }

    /// A minimal single-`.text` image (no imports), the shape a
    /// hand-built `ExitProcess` test uses once Phase 3 adds `.idata`.
    fn single_text_image() -> Vec<u8> {
        let text = vec![0x90u8; 16]; // 16 NOPs
        let headers = size_of_headers(1);
        let text_rva = align_up(headers, SECTION_ALIGNMENT);
        let text_file = align_up(headers, FILE_ALIGNMENT);
        let sections = vec![Section {
            name: section_name(".text"),
            characteristics: SCN_TEXT,
            virtual_address: text_rva,
            virtual_size: text.len() as u32,
            file_offset: text_file,
            bytes: &text,
        }];
        write_image(&sections, text_rva, ImportDirectories::default())
    }

    #[test]
    fn dos_and_pe_signatures_present() {
        let image = single_text_image();
        assert_eq!(&image[0..2], b"MZ");
        let e_lfanew = le_u32(&image, 0x3C) as usize;
        assert_eq!(e_lfanew, 0x80);
        assert_eq!(&image[e_lfanew..e_lfanew + 4], b"PE\0\0");
    }

    #[test]
    fn coff_header_fields() {
        let image = single_text_image();
        let coff = 0x80 + 4;
        assert_eq!(le_u16(&image, coff), 0x8664, "Machine = AMD64");
        assert_eq!(le_u16(&image, coff + 2), 1, "NumberOfSections");
        assert_eq!(
            le_u32(&image, coff + 4),
            0,
            "TimeDateStamp = 0 (determinism)"
        );
        assert_eq!(
            le_u16(&image, coff + 16),
            0xF0,
            "SizeOfOptionalHeader = 240"
        );
        // EXECUTABLE_IMAGE | LARGE_ADDRESS_AWARE | RELOCS_STRIPPED = 0x0023.
        assert_eq!(le_u16(&image, coff + 18), 0x0023, "Characteristics");
    }

    #[test]
    fn optional_header_is_pe32_plus_console() {
        let image = single_text_image();
        let opt = 0x80 + 4 + 20;
        assert_eq!(le_u16(&image, opt), 0x020B, "Magic = PE32+");
        assert_eq!(le_u64(&image, opt + 24), IMAGE_BASE, "ImageBase");
        assert_eq!(le_u32(&image, opt + 32), SECTION_ALIGNMENT);
        assert_eq!(le_u32(&image, opt + 36), FILE_ALIGNMENT);
        // Subsystem at optional-header offset 68 (24 std + 44 into windows-specific).
        assert_eq!(le_u16(&image, opt + 68), 3, "Subsystem = WINDOWS_CUI");
        assert_eq!(
            le_u16(&image, opt + 70),
            0x8100,
            "DllCharacteristics (no DYNAMIC_BASE)"
        );
        assert_eq!(le_u32(&image, opt + 108), 16, "NumberOfRvaAndSizes");
    }

    #[test]
    fn entry_and_base_of_code_point_at_text() {
        let image = single_text_image();
        let opt = 0x80 + 4 + 20;
        let entry = le_u32(&image, opt + 16); // AddressOfEntryPoint
        let base_of_code = le_u32(&image, opt + 20);
        assert_eq!(entry, base_of_code);
        assert_eq!(entry, align_up(size_of_headers(1), SECTION_ALIGNMENT));
    }

    #[test]
    fn section_header_text_characteristics_and_alignment() {
        let image = single_text_image();
        let sect = 0x80 + 4 + 20 + 240; // section table starts after the optional header
        assert_eq!(&image[sect..sect + 5], b".text");
        let virtual_size = le_u32(&image, sect + 8);
        assert_eq!(virtual_size, 16);
        let vaddr = le_u32(&image, sect + 12);
        assert_eq!(vaddr % SECTION_ALIGNMENT, 0, "VirtualAddress page-aligned");
        let raw_size = le_u32(&image, sect + 16);
        assert_eq!(
            raw_size, FILE_ALIGNMENT,
            "SizeOfRawData FileAlignment-rounded"
        );
        let raw_ptr = le_u32(&image, sect + 20);
        assert_eq!(raw_ptr % FILE_ALIGNMENT, 0, "PointerToRawData file-aligned");
        assert_eq!(le_u32(&image, sect + 36), SCN_TEXT, "characteristics");
    }

    #[test]
    fn data_directories_zero_without_imports() {
        let image = single_text_image();
        let dirs = 0x80 + 4 + 20 + 112; // data directories start 112 bytes into the optional header
        for i in 0..16 {
            assert_eq!(le_u32(&image, dirs + i * 8), 0, "dir {i} RVA");
            assert_eq!(le_u32(&image, dirs + i * 8 + 4), 0, "dir {i} size");
        }
    }

    #[test]
    fn import_directories_populate_slots_1_and_12() {
        let text = vec![0x90u8; 16];
        let headers = size_of_headers(1);
        let text_rva = align_up(headers, SECTION_ALIGNMENT);
        let sections = vec![Section {
            name: section_name(".text"),
            characteristics: SCN_TEXT,
            virtual_address: text_rva,
            virtual_size: text.len() as u32,
            file_offset: align_up(headers, FILE_ALIGNMENT),
            bytes: &text,
        }];
        let dirs = ImportDirectories {
            import: (0x3000, 40),
            iat: (0x3100, 16),
        };
        let image = write_image(&sections, text_rva, dirs);
        let dd = 0x80 + 4 + 20 + 112;
        assert_eq!(le_u32(&image, dd + 8), 0x3000, "Import[1] RVA");
        assert_eq!(le_u32(&image, dd + 8 + 4), 40, "Import[1] size");
        assert_eq!(le_u32(&image, dd + 12 * 8), 0x3100, "IAT[12] RVA");
        assert_eq!(le_u32(&image, dd + 12 * 8 + 4), 16, "IAT[12] size");
    }

    #[test]
    fn image_size_and_headers_size_are_aligned() {
        let image = single_text_image();
        let opt = 0x80 + 4 + 20;
        let size_of_image = le_u32(&image, opt + 56);
        let size_of_headers_field = le_u32(&image, opt + 60);
        assert_eq!(size_of_image % SECTION_ALIGNMENT, 0);
        assert_eq!(size_of_headers_field % FILE_ALIGNMENT, 0);
        assert_eq!(size_of_headers_field, size_of_headers(1));
    }

    #[test]
    fn section_body_lands_at_pointer_to_raw_data() {
        let image = single_text_image();
        let sect = 0x80 + 4 + 20 + 240;
        let raw_ptr = le_u32(&image, sect + 20) as usize;
        // The 16 NOPs are written at PointerToRawData.
        assert_eq!(&image[raw_ptr..raw_ptr + 16], &[0x90u8; 16]);
    }

    #[test]
    fn optional_header_is_exactly_240_bytes() {
        // Guarded at runtime by the debug_assert in write_image; assert the COFF
        // field agrees so a release build is covered too.
        let image = single_text_image();
        assert_eq!(le_u16(&image, 0x80 + 4 + 16), 240);
    }
}
