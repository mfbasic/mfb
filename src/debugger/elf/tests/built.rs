//! Files the in-tree linker never writes, built byte by byte: a shared
//! library hashed with `DT_GNU_HASH` and carrying a `.plt` section, and the
//! dynamic tables the reader has to survive without (no strings, no hash, a
//! hash claiming too many symbols), plus odd `.symtab` entries and import
//! stubs that stop the stub walk.

use super::*;

/// One file: its bytes, everything at `vaddr == offset` in one `PT_LOAD`.
struct File {
    bytes: Vec<u8>,
}

const PHDRS: usize = 0x40;
const DYNSTR: usize = 0x200;
const DYNSYM: usize = 0x300;
const HASH: usize = 0x400;
const JMPREL: usize = 0x500;
const GOT: u64 = 0x600;
const DYNAMIC: usize = 0x700;
const PLT: u64 = 0x800;
const SYMTAB: usize = 0x900;
const STRTAB: usize = 0x980;
const SHSTRTAB: usize = 0x9c0;
const SECTIONS: usize = 0xa00;
const SIZE: usize = 0xc00;

/// `.dynstr`: the soname, a needed library, three symbol names.
const STRINGS: &[u8] = b"\0libx.so\0libc.so.6\0foo\0bar\0baz\0";
const LIBX: u64 = 1;
const LIBC: u64 = 9;
const FOO: u32 = 19;
const BAR: u32 = 23;
const BAZ: u32 = 27;

/// A symbol-table entry: name offset, `st_info`, section index, value, size.
fn sym(name: u32, info: u8, shndx: u16, value: u64, size: u64) -> Vec<u8> {
    let mut entry = name.to_le_bytes().to_vec();
    entry.push(info);
    entry.push(0);
    entry.extend(shndx.to_le_bytes());
    entry.extend(value.to_le_bytes());
    entry.extend(size.to_le_bytes());
    entry
}

/// `(binding << 4) | type`.
fn info(binding: u8, kind: u8) -> u8 {
    (binding << 4) | kind
}

impl File {
    /// A shared library: `foo` (a function it defines), `bar` (an import
    /// through the `.plt`), `baz` (an object it defines), hashed by
    /// `DT_GNU_HASH`; `.symtab` holds a function, a section symbol, an
    /// unnamed function and an undefined one; a section table names `.plt`.
    fn library() -> File {
        let mut file = File {
            bytes: vec![0; SIZE],
        };
        file.header(ET_DYN, 2, SECTIONS as u64, 5, 4);
        file.phdr(0, PT_LOAD, PF_X | 4, 0, SIZE as u64);
        file.phdr(1, PT_DYNAMIC, 4, DYNAMIC as u64, 16 * 9);
        file.put(DYNSTR, STRINGS);
        let symbols = [
            vec![0; SYMBOL_SIZE],
            sym(FOO, info(STB_GLOBAL, STT_FUNC), 1, 0x880, 8),
            sym(BAR, info(STB_GLOBAL, STT_FUNC), 0, 0, 0),
            sym(BAZ, info(STB_WEAK, STT_OBJECT), 1, 0x610, 8),
        ]
        .concat();
        file.put(DYNSYM, &symbols);
        // nbuckets 1, symoffset 1, one bloom word; bucket 0 starts the chain
        // at symbol 1; the chain ends (bit 0) at symbol 3.
        let mut hash = [1u32, 1, 1, 6]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>();
        hash.extend([0; 8]);
        hash.extend(
            [1u32, 0x10, 0x20, 0x31]
                .iter()
                .flat_map(|word| word.to_le_bytes()),
        );
        file.put(HASH, &hash);
        let rela = [
            GOT.to_le_bytes(),
            ((2u64 << 32) | u64::from(R_AARCH64_JUMP_SLOT)).to_le_bytes(),
            0u64.to_le_bytes(),
        ]
        .concat();
        file.put(JMPREL, &rela);
        file.dynamic(&[
            (DT_NEEDED, LIBC),
            (DT_SONAME, LIBX),
            (DT_STRTAB, DYNSTR as u64),
            (DT_STRSZ, STRINGS.len() as u64),
            (DT_SYMTAB, DYNSYM as u64),
            (DT_GNU_HASH, HASH as u64),
            (DT_JMPREL, JMPREL as u64),
            (DT_PLTRELSZ, 24),
        ]);
        // `.symtab`: null, `work`, a section symbol, an unnamed function, an
        // undefined function.
        let symtab = [
            vec![0; SYMBOL_SIZE],
            sym(1, info(STB_GLOBAL, STT_FUNC), 1, 0x8a0, 4),
            sym(11, info(0, 3), 1, 0x800, 0),
            sym(0, info(STB_GLOBAL, STT_FUNC), 1, 0x8b0, 4),
            sym(6, info(STB_GLOBAL, STT_FUNC), 0, 0, 0),
        ]
        .concat();
        file.put(SYMTAB, &symtab);
        file.put(STRTAB, b"\0work\0puts\0sect\0");
        file.put(SHSTRTAB, b"\0.plt\0.symtab\0.strtab\0.shstrtab\0");
        // Section headers: null, .plt, .symtab (linked to .strtab), .strtab,
        // .shstrtab.
        file.section(1, 1, 1, PLT, PLT, 0x30, 0);
        file.section(2, 6, SHT_SYMTAB, 0, SYMTAB as u64, 5 * 24, 3);
        file.section(3, 14, 3, 0, STRTAB as u64, 16, 0);
        file.section(4, 22, 3, 0, SHSTRTAB as u64, 32, 0);
        file
    }

    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
    }

    fn put_u16(&mut self, at: usize, value: u16) {
        self.put(at, &value.to_le_bytes());
    }

    fn put_u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }

    fn put_u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }

    fn header(&mut self, kind: u16, phnum: u16, shoff: u64, shnum: u16, shstrndx: u16) {
        self.put(0, &ELF_MAGIC);
        self.bytes[4] = ELFCLASS64;
        self.bytes[5] = ELFDATA2LSB;
        self.put_u16(16, kind);
        self.put_u16(18, EM_AARCH64);
        self.put_u64(24, 0x880);
        self.put_u64(32, PHDRS as u64);
        self.put_u64(40, shoff);
        self.put_u16(54, 56);
        self.put_u16(56, phnum);
        self.put_u16(58, 64);
        self.put_u16(60, shnum);
        self.put_u16(62, shstrndx);
    }

    fn phdr(&mut self, index: usize, kind: u32, flags: u32, offset: u64, size: u64) {
        let at = PHDRS + index * 56;
        self.put_u32(at, kind);
        self.put_u32(at + 4, flags);
        self.put_u64(at + 8, offset);
        self.put_u64(at + 16, offset);
        self.put_u64(at + 32, size);
        self.put_u64(at + 40, size);
    }

    fn dynamic(&mut self, tags: &[(u64, u64)]) {
        self.put(DYNAMIC, &[0; 16 * 9]);
        for (index, (tag, value)) in tags.iter().enumerate() {
            self.put_u64(DYNAMIC + index * 16, *tag);
            self.put_u64(DYNAMIC + index * 16 + 8, *value);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn section(
        &mut self,
        index: usize,
        name: u32,
        kind: u32,
        address: u64,
        offset: u64,
        size: u64,
        link: u32,
    ) {
        let at = SECTIONS + index * 64;
        self.put_u32(at, name);
        self.put_u32(at + 4, kind);
        self.put_u64(at + 16, address);
        self.put_u64(at + 24, offset);
        self.put_u64(at + 32, size);
        self.put_u32(at + 40, link);
    }

    fn read(&self) -> Option<Elf> {
        read(&self.bytes)
    }
}

#[test]
fn a_gnu_hashed_library_names_its_symbols_imports_and_plt() {
    let elf = File::library().read().expect("a well-formed library");
    assert_eq!(elf.soname.as_deref(), Some("libx.so"));
    assert_eq!(elf.needed, ["libc.so.6"]);
    // No PT_PHDR: the headers are where the load holding them maps them.
    assert_eq!(elf.program_headers, Some(PHDRS as u64));
    assert_eq!(
        elf.dynamic
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect::<Vec<_>>(),
        ["", "foo", "bar", "baz"]
    );
    assert_eq!(elf.slots, [(2, GOT)]);
    assert_eq!(elf.plt, Some((PLT, 0x30)));
    assert_eq!(
        elf.plt_entry(PLT + 0x24),
        Some((PLT + 0x20, "bar".to_string()))
    );
    // Unnamed, undefined and non-function symbols are no defined function.
    assert_eq!(
        elf.defined_functions(),
        [
            ("foo".to_string(), 0x880, 8, true),
            ("work".to_string(), 0x8a0, 4, true),
        ]
    );
}

#[test]
fn an_executable_skips_unnamed_undefined_and_section_symbols() {
    let image = executable(&File::library().bytes).expect("readable");
    assert_eq!(
        image
            .functions
            .iter()
            .map(|function| (function.name.as_str(), function.location.clone()))
            .collect::<Vec<_>>(),
        [
            (
                "work",
                Location::Defined {
                    address: 0x8a0,
                    size: 4
                }
            ),
            ("bar", Location::Import(Some(GOT))),
        ]
    );
    assert!(image.data.is_empty());
    assert!(image.stubs.is_empty());
}

#[test]
fn a_gnu_hash_with_no_chain_counts_only_the_symbols_before_it() {
    let mut file = File::library();
    // Bucket 0 empty: nothing past symoffset is hashed.
    file.put_u32(HASH + 24, 0);
    let elf = file.read().unwrap();
    assert_eq!(elf.dynamic.len(), 1);
}

#[test]
fn dynamic_tables_without_strings_or_a_hash_name_nothing() {
    let mut file = File::library();
    file.dynamic(&[(DT_SYMTAB, DYNSYM as u64), (DT_GNU_HASH, HASH as u64)]);
    let elf = file.read().unwrap();
    assert!(elf.needed.is_empty() && elf.dynamic.is_empty());

    let mut file = File::library();
    file.dynamic(&[
        (DT_STRTAB, DYNSTR as u64),
        (DT_STRSZ, STRINGS.len() as u64),
        (DT_SYMTAB, DYNSYM as u64),
        (DT_RELA, JMPREL as u64),
    ]);
    let elf = file.read().unwrap();
    assert!(elf.dynamic.is_empty() && elf.slots.is_empty());
}

#[test]
fn a_hash_table_claiming_too_many_symbols_is_refused() {
    let mut file = File::library();
    file.put_u32(HASH, 1);
    file.put_u32(HASH + 4, (MAX_DYNAMIC_SYMBOLS + 1) as u32);
    file.dynamic(&[
        (DT_STRTAB, DYNSTR as u64),
        (DT_STRSZ, STRINGS.len() as u64),
        (DT_HASH, HASH as u64),
    ]);
    assert_eq!(file.read(), None);
}

#[test]
fn the_stub_walk_stops_where_the_code_cannot_be_read_or_names_no_import() {
    // An executable segment whose file bytes run past the end of the file.
    let mut file = File::library();
    file.put_u64(PHDRS + 32, SIZE as u64 + 0x100);
    assert!(executable(&file.bytes).unwrap().stubs.is_empty());

    // A well-formed stub at the end of the code loading a slot no
    // relocation binds: `adrp x16, 0` / `ldr x17, [x16, #0x7f8]` / `br x17`.
    let mut file = File::library();
    let end = SIZE - 12;
    file.put_u32(end, 0x9000_0010);
    file.put_u32(end + 4, 0xf943_fe11);
    file.put_u32(end + 8, 0xd61f_0220);
    assert!(executable(&file.bytes).unwrap().stubs.is_empty());

    // No executable segment at all.
    let mut file = File::library();
    file.put_u32(PHDRS + 4, 4);
    assert!(executable(&file.bytes).unwrap().stubs.is_empty());
}

#[test]
fn a_fixed_address_executable_is_not_position_independent() {
    let mut file = File::library();
    file.put_u16(16, ET_EXEC);
    let elf = file.read().unwrap();
    assert!(!elf.position_independent);
    assert_eq!(elf.code(), [(0, SIZE as u64)]);
}
