//! In-place field stores of `engine/control/builder_control/field_replace.rs` for
//! two shapes no other suite lowers.
//!
//! * A swap of two fixed-size inlined record fields, `WITH s { a := s.b, b :=
//!   s.a }`: each new value is a borrow of the owner's own bytes, and the first
//!   overwrite changes the second's source before its copy runs, so the sources
//!   are snapshotted first (`flat_copy_*`). A single such overwrite copies the
//!   borrowed bytes where they lie and snapshots nothing.
//! * A nested `WITH` through a field held behind a pointer (a record with a
//!   function value is not byte-copyable, so its slot is a pointer): the store
//!   walks the pointer level down to the inner block and writes there in place
//!   instead of rebuilding the outer record through the general `WITH`.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

const TYPES: &str = "\
IMPORT io

TYPE Pt
  x AS Float
  y AS Float
END TYPE

TYPE Seg
  a AS Pt
  b AS Pt
  n AS Integer
END TYPE

TYPE Inner
  n AS Integer
  f AS FUNC(Integer) AS Integer
END TYPE

TYPE Outer
  id AS Integer
  inner AS Inner
END TYPE
";

fn main_slot_kinds(body: &str) -> Vec<String> {
    let source = format!("{TYPES}\nFUNC main() AS Integer\n{body}\n  RETURN 0\nEND FUNC\n");
    code_function(
        code_for_src_cached(&source, CodeTarget::LinuxX86_64, Console),
        "main",
    )
    .stack_slots
    .iter()
    .map(|slot| slot.type_.clone())
    .collect()
}

fn has(kinds: &[String], kind: &str) -> bool {
    kinds.iter().any(|k| k == kind)
}

fn segment_loop(update: &str) -> String {
    format!(
        "  MUT s AS Seg = Seg[a := Pt[x := 1.0, y := 2.0], b := Pt[x := 3.0, y := 4.0], n := 0]\n  \
         FOR i = 1 TO 3\n    s = WITH s {{ {update} }}\n  NEXT\n  io::print(toString(s.a.x))"
    )
}

#[test]
fn a_swap_of_inlined_fields_snapshots_its_sources_and_one_overwrite_does_not() {
    let swap = main_slot_kinds(&segment_loop("a := s.b, b := s.a"));
    assert!(has(&swap, "record_field_overwrite"), "{swap:?}");
    assert!(has(&swap, "flat_copy_source"), "{swap:?}");
    let single = main_slot_kinds(&segment_loop("a := s.b, n := i"));
    assert!(has(&single, "record_field_overwrite"), "{single:?}");
    assert!(!has(&single, "flat_copy_source"), "{single:?}");
}

#[test]
fn a_nested_with_through_a_pointer_field_stores_in_place() {
    let kinds = main_slot_kinds(
        "  MUT o AS Outer = Outer[id := 1, inner := Inner[n := 1, f := LAMBDA(v AS Integer) -> v]]\n  \
         FOR i = 1 TO 3\n    o = WITH o { inner := WITH o.inner { n := o.inner.n + i } }\n  NEXT\n  \
         io::print(toString(o.inner.n))",
    );
    assert!(has(&kinds, "record_field_inplace"), "{kinds:?}");
    assert!(!has(&kinds, "with_target"), "{kinds:?}");
}
