//! Mach-O files the in-tree linker does not write, made by patching one it
//! does: load commands removed or malformed, symbols that are stabs or
//! absolute, indirect entries that are local, and an indirect table past the
//! end of the file.

use super::*;

const LC_SEGMENT_64: u32 = 0x19;

/// The offset of the first load command `cmd`, walking the command list.
fn command(bytes: &[u8], cmd: u32) -> usize {
    let mut at = 32;
    for _ in 0..u32_at(bytes, 16).unwrap() {
        if u32_at(bytes, at).unwrap() == cmd {
            return at;
        }
        at += u32_at(bytes, at + 4).unwrap() as usize;
    }
    panic!("no load command {cmd:#x}");
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Turn the load command `cmd` into one the reader does not know.
fn without(bytes: &[u8], cmd: u32) -> Vec<u8> {
    let mut bytes = bytes.to_vec();
    let at = command(&bytes, cmd);
    put_u32(&mut bytes, at, 0x7fff_0000);
    bytes
}

fn sample() -> Vec<u8> {
    linked(&["_main", "_mfb_fn_main"], &["_malloc", "_free"], true)
}

#[test]
fn a_file_without_an_entry_symbols_or_indirect_table_reads_what_it_has() {
    let bytes = sample();
    assert_eq!(read(&without(&bytes, LC_MAIN)).unwrap().entry, None);
    let bare = read(&without(&bytes, LC_SYMTAB)).unwrap();
    assert!(bare.functions.is_empty() && bare.data.is_empty() && bare.stubs.is_empty());
    assert!(bare.entry.is_some());
    // Without the indirect symbol table no import has a slot or a stub.
    let image = read(&without(&bytes, LC_DYSYMTAB)).unwrap();
    assert!(image.stubs.is_empty());
    assert_eq!(
        image
            .functions
            .iter()
            .filter(|function| function.location == Location::Import(None))
            .count(),
        2
    );
}

#[test]
fn malformed_load_commands_and_tables_are_refused() {
    let bytes = sample();
    // A load command shorter than its own header.
    let mut short = bytes.clone();
    put_u32(&mut short, 32 + 4, 4);
    assert_eq!(read(&short), None);
    // A segment claiming more sections than it carries.
    let mut sections = bytes.clone();
    let segment = command(&sections, LC_SEGMENT_64);
    put_u32(&mut sections, segment + 64, 1000);
    assert_eq!(read(&sections), None);
    // An indirect symbol table past the end of the file.
    let mut indirect = bytes.clone();
    let dysymtab = command(&indirect, LC_DYSYMTAB);
    put_u32(&mut indirect, dysymtab + 56, u32::MAX - 8);
    assert_eq!(read(&indirect), None);
}

#[test]
fn stabs_absolute_symbols_and_local_indirect_entries_are_skipped() {
    let bytes = sample();
    let symtab = command(&bytes, LC_SYMTAB);
    let symoff = u32_at(&bytes, symtab + 8).unwrap() as usize;
    let image = read(&bytes).unwrap();
    let names = |image: &Image| {
        image
            .functions
            .iter()
            .map(|function| function.name.clone())
            .collect::<Vec<_>>()
    };
    let before = names(&image);
    let first = before[0].clone();
    // The first symbol a stab, the second absolute: neither is a function.
    let mut patched = bytes.clone();
    patched[symoff + 4] = N_STAB;
    patched[symoff + NLIST_SIZE + 4] = 0x2;
    let after = names(&read(&patched).unwrap());
    assert_eq!(after.len(), before.len() - 2, "{after:?}");
    assert!(!after.contains(&first));

    // Every indirect entry local: no import has a slot, and no stub a name.
    let mut local = bytes.clone();
    let dysymtab = command(&local, LC_DYSYMTAB);
    let indirectoff = u32_at(&local, dysymtab + 56).unwrap() as usize;
    let nindirect = u32_at(&local, dysymtab + 60).unwrap() as usize;
    for entry in 0..nindirect {
        put_u32(&mut local, indirectoff + entry * 4, INDIRECT_SYMBOL_LOCAL);
    }
    let image = read(&local).unwrap();
    assert!(image.stubs.is_empty());
    assert!(image
        .functions
        .iter()
        .all(|function| !matches!(function.location, Location::Import(Some(_)))));
}
