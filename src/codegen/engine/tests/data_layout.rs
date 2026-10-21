//! `layout_data_objects` and `align` (`engine/types/types.rs`): the read-only /
//! writable partition of a plan's data objects.

use crate::codegen::engine::types::{align, layout_data_objects, CodeDataObject, DATA_PAGE_SIZE};

fn obj(symbol: &str, kind: &str, align: usize, value: &str) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: kind.to_string(),
        layout: String::new(),
        align,
        size: 0,
        value: value.to_string(),
    }
}

#[test]
fn partitions_constants_before_writable_with_page_boundary() {
    // A constant and a writable (raw) object: the constant lands in the
    // read-only prefix, the raw object in the writable region on a fresh page.
    let objects = vec![
        obj("_arena", "raw", 8, "0000000000000000"),
        obj("_str", "constant", 8, "hi"),
    ];
    let (bytes, rodata_size, symbols) = layout_data_objects(&objects).unwrap();
    let str_off = symbols.iter().find(|(n, _)| n == "_str").unwrap().1;
    let arena_off = symbols.iter().find(|(n, _)| n == "_arena").unwrap().1;
    // The constant is first (offset 0); the writable object is past the
    // page-aligned boundary.
    assert_eq!(str_off, 0);
    assert!(rodata_size > 0 && rodata_size % DATA_PAGE_SIZE == 0);
    assert!(
        arena_off >= rodata_size,
        "arena must be in the writable region"
    );
    // The constant's bytes (u64 len prefix + "hi" + NUL) sit at offset 0.
    assert_eq!(&bytes[0..8], &2u64.to_le_bytes());
    assert_eq!(&bytes[8..10], b"hi");
}

#[test]
fn no_constants_means_no_rodata() {
    let objects = vec![obj("_arena", "raw", 8, "0000000000000000")];
    let (_, rodata_size, _) = layout_data_objects(&objects).unwrap();
    assert_eq!(rodata_size, 0);
}

#[test]
fn align_zero_does_not_divide_by_zero() {
    // bug-18: a malformed plan could carry align 0; treat 0/1 as "no align".
    assert_eq!(align(1, 0), 1);
    assert_eq!(align(0, 0), 0);
    assert_eq!(align(7, 1), 7);
    assert_eq!(align(1, 8), 8);
    assert_eq!(align(17, 16), 32);
    // And a data object with align 0 lays out without panicking.
    let objects = vec![obj("_x", "raw", 0, "ff")];
    assert!(layout_data_objects(&objects).is_ok());
}
