//! `layout_data_objects`: the data blob every executable carries.
//!
//! It packs the emitted data objects into one byte array, reports the length of
//! the read-only prefix, and returns each object's offset. Nothing in process
//! reached it — it runs below `code::lower_module`, on the object-writing path
//! that needs a linker — so the whole function, including its Level-2
//! alignment-ordering row, was uncovered.
//!
//! Three properties, each of which fails silently rather than loudly:
//!
//!   * **the offsets and the blob agree.** They are produced by one pass for
//!     exactly that reason. An offset that disagrees with where the bytes landed
//!     points a relocation into the middle of a neighbouring object.
//!   * **the const → writable page boundary holds.** The read-only prefix is
//!     mapped read-only; a writable object that ended up inside it faults on its
//!     first store, at run time, only in the program that writes it.
//!   * **the Level-2 ordering saves padding and stays deterministic.** It sorts
//!     each partition by descending alignment. Sorting the two partitions
//!     together would move a writable object across the boundary; an unstable
//!     tie-break would make the blob differ between two builds of the same
//!     source, which reads as a flaky byte-identity golden rather than as a
//!     sort.

use crate::codegen::engine::types::{layout_data_objects, CodeDataObject};
use crate::optimizer::{with_opt_level, OptLevel};

/// `(symbol, kind, align, value)` -> a data object.
fn object(symbol: &str, kind: &str, align: usize, value: &str) -> CodeDataObject {
    let size = if kind == "raw" {
        value.len() / 2
    } else {
        value.len() + 9
    };
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: kind.to_string(),
        layout: String::new(),
        align,
        size,
        value: value.to_string(),
    }
}

/// A mixed set: both kinds, several alignments, and a tie to break.
fn objects() -> Vec<CodeDataObject> {
    vec![
        object("c_narrow", "constant", 1, "ab"),
        object("c_wide", "constant", 16, "cdef"),
        object("c_mid", "constant", 8, "gh"),
        object("c_tie_b", "constant", 8, "ij"),
        object("c_tie_a", "constant", 8, "kl"),
        // Declared narrow-first so `-O1` wastes padding here and `-O2` does not.
        object("w_narrow", "raw", 1, "0f"),
        object("w_raw", "raw", 8, "00112233445566778899aabbccddeeff"),
    ]
}

/// Every reported offset is where that object's bytes actually are.
#[test]
fn each_symbols_offset_lands_inside_the_blob_and_on_its_alignment() {
    let objects = objects();
    let (blob, rodata, symbols) = layout_data_objects(&objects).expect("layout");
    assert_eq!(
        symbols.len(),
        objects.len(),
        "every object must get exactly one offset"
    );
    for (symbol, offset) in &symbols {
        let declared = objects
            .iter()
            .find(|o| &o.symbol == symbol)
            .unwrap_or_else(|| panic!("{symbol} is not one of the inputs"));
        assert_eq!(
            offset % declared.align,
            0,
            "`{symbol}` is declared align {} but was placed at {offset}",
            declared.align
        );
        assert!(
            offset + declared.size <= blob.len(),
            "`{symbol}` at {offset} + {} bytes runs past the {}-byte blob",
            declared.size,
            blob.len()
        );
    }
    let mut sorted: Vec<usize> = symbols.iter().map(|(_, at)| *at).collect();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        symbols.len(),
        "two objects were placed at the same offset: {symbols:?}"
    );
    assert!(
        rodata <= blob.len(),
        "the read-only prefix cannot exceed the blob"
    );
}

/// Nothing writable lives inside the read-only prefix.
///
/// The prefix is mapped read-only, so a writable object that ended up in it
/// faults on its first store — at run time, in whichever program writes it.
#[test]
fn the_read_only_prefix_holds_only_constants() {
    let objects = objects();
    let (_, rodata, symbols) = layout_data_objects(&objects).expect("layout");
    assert!(
        rodata > 0,
        "a set with constants must have a read-only prefix"
    );
    for (symbol, offset) in &symbols {
        let kind = objects
            .iter()
            .find(|o| &o.symbol == symbol)
            .map(|o| o.kind.as_str())
            .unwrap_or_default();
        if kind == "constant" {
            assert!(
                *offset < rodata,
                "constant `{symbol}` at {offset} must be inside the {rodata}-byte \
                 read-only prefix"
            );
        } else {
            assert!(
                *offset >= rodata,
                "writable `{symbol}` at {offset} must be outside the {rodata}-byte \
                 read-only prefix, or its first store faults"
            );
        }
    }
}

/// An all-constant set makes the whole blob read-only.
#[test]
fn a_set_with_nothing_writable_is_read_only_to_its_end() {
    let objects: Vec<CodeDataObject> = objects()
        .into_iter()
        .filter(|o| o.kind == "constant")
        .collect();
    let (blob, rodata, _) = layout_data_objects(&objects).expect("layout");
    assert_eq!(
        rodata,
        blob.len(),
        "with no writable object the whole padded blob is the read-only region"
    );
}

/// `-O2`'s alignment ordering saves padding, and never moves an object across
/// the const → writable boundary.
#[test]
fn the_level_two_ordering_shrinks_the_blob_without_crossing_the_boundary() {
    let objects = objects();
    let at_one = with_opt_level(OptLevel(1), || layout_data_objects(&objects)).expect("layout");
    let at_two = with_opt_level(OptLevel(2), || layout_data_objects(&objects)).expect("layout");

    // The WRITABLE span, not the whole blob: the const partition is padded up to
    // a page at the boundary, and 4 KiB of page padding swamps the handful of
    // bytes the ordering saves. The writable partition has no such padding, so it
    // is where the saving is visible at all.
    let writable_span =
        |(blob, rodata, _): &(Vec<u8>, usize, Vec<(String, usize)>)| blob.len() - rodata;
    assert!(
        writable_span(&at_two) < writable_span(&at_one),
        "the writable objects are declared align 1 then align 8, so ordering them \
         by descending alignment must remove the gap: {} bytes at -O2 against {} \
         at -O1",
        writable_span(&at_two),
        writable_span(&at_one)
    );

    // And the const partition really is reordered: the widest-aligned constant
    // is placed first at -O2 and not at -O1, where declaration order stands.
    let first_placed = |symbols: &[(String, usize)], prefix: &str| {
        let mut theirs: Vec<(usize, String)> = symbols
            .iter()
            .filter(|(s, _)| s.starts_with(prefix))
            .map(|(s, at)| (*at, s.clone()))
            .collect();
        theirs.sort();
        theirs.first().map(|(_, s)| s.clone()).unwrap_or_default()
    };
    assert_eq!(
        first_placed(&at_two.2, "c_"),
        "c_wide",
        "-O2 must place the widest-aligned constant first"
    );
    assert_eq!(
        first_placed(&at_one.2, "c_"),
        "c_narrow",
        "-O1 must leave the constants in declaration order"
    );
    // Same partition either way.
    for (blob_symbols, label) in [(&at_one.2, "-O1"), (&at_two.2, "-O2")] {
        let rodata = if label == "-O1" { at_one.1 } else { at_two.1 };
        for (symbol, offset) in blob_symbols.iter() {
            let is_const = symbol.starts_with("c_");
            assert_eq!(
                *offset < rodata,
                is_const,
                "{label}: `{symbol}` crossed the const/writable boundary"
            );
        }
    }
}

/// The blob is deterministic, and ties are broken by symbol.
///
/// Three of the constants share alignment 8. If the sort left their order to
/// chance, two builds of one program would produce different bytes and the
/// byte-identity goldens would read as flaky.
#[test]
fn the_layout_is_deterministic_and_ties_break_by_symbol() {
    let first = with_opt_level(OptLevel(2), || layout_data_objects(&objects())).expect("layout");
    let again = with_opt_level(OptLevel(2), || layout_data_objects(&objects())).expect("layout");
    assert_eq!(
        first.0, again.0,
        "the same input must produce the same bytes"
    );
    assert_eq!(first.2, again.2, "...and the same offsets");

    // Reversing the INPUT order must not change the result: the sort decides.
    let mut reversed = objects();
    reversed.reverse();
    let flipped = with_opt_level(OptLevel(2), || layout_data_objects(&reversed)).expect("layout");
    let order = |symbols: &[(String, usize)]| {
        let mut pairs: Vec<(usize, String)> =
            symbols.iter().map(|(s, at)| (*at, s.clone())).collect();
        pairs.sort();
        pairs.into_iter().map(|(_, s)| s).collect::<Vec<_>>()
    };
    assert_eq!(
        order(&first.2),
        order(&flipped.2),
        "reversing the input changed the placement order, so the tie-break is not \
         on the symbol"
    );
}

/// A malformed `raw` value is rejected rather than truncated.
///
/// The value is hex, and both failures it can have are silent if unchecked: an
/// odd digit count would drop the last nibble, and a non-hex digit would decode
/// as some other byte. Either produces a blob that links and is wrong.
#[test]
fn a_malformed_raw_value_is_rejected() {
    for (value, want) in [
        ("abc", "even digit count"),
        ("zz", "non-hex digit"),
        ("0g", "non-hex digit"),
    ] {
        let err = layout_data_objects(&[object("w", "raw", 1, value)])
            .err()
            .unwrap_or_default();
        assert!(
            err.contains(want),
            "a raw value of {value:?} must be rejected with {want:?}; got {err:?}"
        );
    }
    // Whitespace and `_` are separators, not digits.
    let (blob, _, _) =
        layout_data_objects(&[object("w", "raw", 1, "de ad_be ef")]).expect("layout");
    assert_eq!(&blob[..4], &[0xde, 0xad, 0xbe, 0xef]);
}
