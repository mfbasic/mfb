//! A `List OF Byte` has no lookup entries, so the runtime helpers that build one write none.
//!
//! The read/receive/`toBytes` helpers build a byte list by hand: header, then payload.
//! They once also wrote a lookup entry per byte, guarded by `byte_list_entry_stride()
//! != 0`. Since the entry-free representation (plan-57-D) that stride is zero for
//! every fixed-width element type, so those guarded stores could never run and have
//! been removed. This pins the premise: if a `List OF Byte` ever grows entries again,
//! the helpers need their entry stores back, and this fails first rather than the
//! helpers silently building lists whose lookup table is missing.

use crate::codegen::collection::layout::{byte_list_entry_stride, list_entry_stride};
use crate::types::ParameterType;

#[test]
fn a_byte_list_has_no_lookup_entries() {
    assert_eq!(byte_list_entry_stride(), 0);
    for fixed in [
        ParameterType::Byte,
        ParameterType::Boolean,
        ParameterType::Integer,
        ParameterType::Float,
    ] {
        assert_eq!(
            list_entry_stride(&fixed),
            0,
            "a fixed-width element ({fixed}) is addressed at index * width, no entry"
        );
    }
    assert_ne!(
        list_entry_stride(&ParameterType::String),
        0,
        "a variable-width element keeps its lookup entries"
    );
}
