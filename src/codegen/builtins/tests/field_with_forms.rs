//! In-place `WITH` stores (`engine/control/builder_control/field_with.rs`), lowered
//! for Linux x86-64 and checked by the stack objects `main` reserves — the slots
//! a store route names (`record_field_inplace`, `record_field_replaced`, ...) are
//! the audit trail of which route a `WITH` took, and a declined update leaves
//! none of them. The programs are those of `tests/runtime/rt_inplace_field_*.rs`
//! and `tests/runtime/rt_record_*.rs`, lowered instead of run.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

/// The kinds of stack object function `name` of `source` reserves, deduplicated.
fn slots_of(source: &str, name: &str) -> Vec<String> {
    let plan = code_for_src_cached(source, CodeTarget::LinuxX86_64, Console);
    let mut kinds: Vec<String> = code_function(plan, name)
        .stack_slots
        .iter()
        .map(|slot| slot.type_.clone())
        .collect();
    kinds.sort();
    kinds.dedup();
    kinds
}

fn has(kinds: &[String], kind: &str) -> bool {
    kinds.iter().any(|k| k == kind)
}

const TYPES: &str = "\
IMPORT collections
IMPORT fs
IMPORT io
IMPORT json

TYPE Rec
  hp AS Integer
  n AS Integer
  xs AS List OF Integer
END TYPE

TYPE P
  hp AS Integer
  n AS Integer
  xs AS List OF Integer
END TYPE

FUNC say(v AS Integer) AS Integer
  IF v = 1 THEN
    io::print(\"say 1\")
  ELSE
    io::print(\"say 2\")
  END IF
  RETURN v
END FUNC

FUNC count(xs AS List OF Integer) AS Integer
  RETURN len(xs)
END FUNC
";

fn rec_main(stmt: &str, init: &str) -> String {
    format!(
        "{TYPES}\nFUNC main() AS Integer\n  MUT r AS Rec = Rec[hp := 1, n := 0, xs := {init}]\n  \
         LET k AS Integer = 5\n  FOR i = 1 TO 4\n    {stmt}\n  NEXT\n  \
         io::print(toString(r.n) & \" \" & toString(len(r.xs)))\n  RETURN 0\nEND FUNC\n"
    )
}

/// A scalar-only `WITH` over a record local, a resource's `STATE` and a global
/// stores into the owner's slots; the slot kind names the owner.
#[test]
fn a_scalar_with_stores_in_place_over_each_kind_of_owner() {
    let local = slots_of(
        &rec_main("r = WITH r { hp := r.hp + 1, n := k }", "[1, 2, 3]"),
        "main",
    );
    assert!(has(&local, "record_field_inplace"), "{local:?}");
    let state = format!(
        "{TYPES}\nFUNC main() AS Integer\n  RES h AS fs::File STATE P = fs::openFile(\"/dev/null\")\n  \
         h.state = P[hp := 1, n := 0, xs := [1, 2, 3]]\n  LET k AS Integer = 5\n  \
         FOR i = 1 TO 4\n    h.state = WITH h.state {{ hp := h.state.hp + 1, n := k }}\n  NEXT\n  \
         io::print(toString(h.state.hp))\n  RETURN 0\nEND FUNC\n"
    );
    assert!(has(&slots_of(&state, "main"), "state_field_inplace"));
    let global = format!(
        "{TYPES}\nMUT g AS Rec = Rec[hp := 1, n := 0, xs := [1]]\n\nFUNC main() AS Integer\n  LET k AS Integer = 5\n  \
         FOR i = 1 TO 4\n    g = WITH g {{ hp := g.hp + 1, n := k }}\n  NEXT\n  \
         io::print(toString(g.hp))\n  RETURN 0\nEND FUNC\n"
    );
    assert!(has(&slots_of(&global, "main"), "global_field_inplace"));
}

/// A mixed `WITH` (a collection arm beside scalars) is the `mixed_with` route,
/// over a record local and over a `STATE` alike.
#[test]
fn a_mixed_with_takes_the_mixed_route() {
    let local = slots_of(
        &rec_main(
            "r = WITH r { hp := r.hp + 1, xs := collections::append(r.xs, k), n := k }",
            "[1, 2, 3]",
        ),
        "main",
    );
    assert!(has(&local, "mixed_with_scalar"), "{local:?}");
}

/// A pointer field and a scalar in one `WITH`: both stored in their slots and
/// the old `json::Json` dropped (`record_field_replaced`).
#[test]
fn a_pointer_and_a_scalar_update_drop_the_displaced_pointee() {
    let source =
        "IMPORT io\nIMPORT json\n\nTYPE Doc\n  j AS json::Json\n  n AS Integer\nEND TYPE\n\n\
         FUNC fresh(v AS Integer) AS json::Json\n  RETURN json::JsonNum[toFloat(v)]\nEND FUNC\n\n\
         FUNC main() AS Integer\n  MUT d AS Doc = Doc[j := json::parse(\"[1,2]\"), n := 0]\n  \
         FOR i = 1 TO 4\n    d = WITH d { j := fresh(i), n := d.n + 1 }\n  NEXT\n  \
         io::print(json::stringify(d.j) & \" \" & toString(d.n))\n  RETURN 0\nEND FUNC\n";
    let kinds = slots_of(source, "main");
    assert!(has(&kinds, "record_field_inplace"), "{kinds:?}");
    assert!(has(&kinds, "record_field_replaced"), "{kinds:?}");
}

const BODIES: &str = "\
IMPORT collections
IMPORT io

TYPE Pt
  x AS Integer
  y AS Integer
END TYPE

TYPE Body
  pos AS Float
  mass AS Float
END TYPE

TYPE Box
  lo AS Pt
  hi AS Pt
  tag AS Integer
END TYPE
";

/// bug-766 B: the n-body pair step. Two `get` elements live across a write are
/// scratch copies — each is copied into the function's record-temp region at its
/// bind, and each `set` item's `WITH` is built in the shared WITH-item region.
#[test]
fn live_elements_across_a_write_are_copied_into_the_record_scratch() {
    let source = format!(
        "{BODIES}\nFUNC main() AS Integer\n  MUT xs AS List OF Body = [Body[pos := 1.0, mass := 2.0], Body[pos := 3.0, mass := 4.0]]\n  \
         FOR i = 0 TO 1\n    LET pi AS Body = collections::get(xs, i)\n    LET pj AS Body = collections::get(xs, 1 - i)\n    \
         xs = collections::set(xs, i, WITH pi {{ pos := pi.pos + pj.pos }})\n    \
         xs = collections::set(xs, 1 - i, WITH pj {{ pos := pj.pos - pi.pos }})\n  NEXT\n  \
         io::print(toString(len(xs)))\n  RETURN 0\nEND FUNC\n"
    );
    let kinds = slots_of(&source, "main");
    assert!(has(&kinds, "rec_scratch"), "{kinds:?}");
    assert!(has(&kinds, "rec_temp_source"), "{kinds:?}");
    assert!(has(&kinds, "rec_temp_with_target"), "{kinds:?}");
    assert!(has(&kinds, "rec_temp_with_value"), "{kinds:?}");
}

/// A `WITH` item of a fixed-size record with an inlined fixed-size field, given
/// as a scalar-only record literal, a field read and a computed value, is built
/// in the WITH-item region: the literal's lanes are spilled field by field.
#[test]
fn a_with_item_with_inlined_fields_is_built_in_the_with_item_region() {
    let source = format!(
        "{BODIES}\nFUNC main() AS Integer\n  MUT bs AS List OF Box = []\n  MUT b AS Box = Box[lo := Pt[x := 0, y := 0], hi := Pt[x := 1, y := 1], tag := 0]\n  \
         FOR i = 0 TO 3\n    b = WITH b {{ lo := Pt[x := i, y := i + 1], tag := b.tag + 1 }}\n    \
         bs = collections::append(bs, WITH b {{ lo := Pt[x := i, y := 7], hi := b.lo, tag := i }})\n  NEXT\n  \
         io::print(toString(len(bs)))\n  RETURN 0\nEND FUNC\n"
    );
    let kinds = slots_of(&source, "main");
    assert!(has(&kinds, "rec_scratch"), "{kinds:?}");
    assert!(has(&kinds, "rec_temp_literal_field"), "{kinds:?}");
    assert!(has(&kinds, "rec_temp_with_value"), "{kinds:?}");
    assert!(has(&kinds, "rec_temp_with_region"), "{kinds:?}");
}

/// bug-487 (`G25`): a `WITH` over a resource's `STATE` stores in place only when
/// no operand can reach a `STATE` assignment; one that does (directly or through
/// a callee) takes the whole-record replace, so the nested write loses to the
/// outer store as `WITH` defines.
#[test]
fn a_state_operand_that_assigns_state_declines_the_in_place_store() {
    let program = |operand: &str| {
        format!(
            "IMPORT fs\nIMPORT io\n\nTYPE St\n  n AS Integer\n  m AS Integer\nEND TYPE\n\n\
             FUNC sideEffect(f AS fs::File STATE St) AS Integer\n  f.state.m = f.state.m + 1\n  RETURN 222\nEND FUNC\n\n\
             FUNC indirect(f AS fs::File STATE St) AS Integer\n  RETURN sideEffect(f)\nEND FUNC\n\n\
             FUNC pure(v AS Integer) AS Integer\n  RETURN v + 1\nEND FUNC\n\n\
             FUNC main() AS Integer\n  RES f AS fs::File STATE St = fs::openFile(\"/dev/null\")\n  \
             f.state = St[n := 0, m := 0]\n  f.state.n = f.state.n + {operand}\n  \
             io::print(toString(f.state.n))\n  RETURN 0\nEND FUNC\n"
        )
    };
    assert!(has(
        &slots_of(&program("pure(41)"), "main"),
        "state_field_inplace"
    ));
    assert!(!has(
        &slots_of(&program("sideEffect(f)"), "main"),
        "state_field_inplace"
    ));
    assert!(!has(
        &slots_of(&program("indirect(f)"), "main"),
        "state_field_inplace"
    ));
}

/// plan-145-G: the same gate over a global owner — an operand that writes the
/// global would have its write discarded by the `WITH` built from the old value.
#[test]
fn a_global_operand_that_stores_the_global_declines_the_in_place_store() {
    let program = |operand: &str| {
        format!(
            "{TYPES}\nMUT g AS Rec = Rec[hp := 1, n := 0, xs := [1]]\n\n\
             FUNC bump() AS Integer\n  g = Rec[hp := 9, n := 9, xs := [9]]\n  RETURN 5\nEND FUNC\n\n\
             FUNC main() AS Integer\n  g = WITH g {{ hp := {operand} }}\n  io::print(toString(g.hp))\n  RETURN 0\nEND FUNC\n"
        )
    };
    assert!(has(
        &slots_of(&program("g.hp + 1"), "main"),
        "global_field_inplace"
    ));
    assert!(!has(
        &slots_of(&program("bump()"), "main"),
        "global_field_inplace"
    ));
}

/// `r = WITH q { .. }` builds a different record from `q`: it is not an update
/// of `r`'s own fields, so no in-place store is taken.
#[test]
fn a_with_over_another_record_is_no_in_place_update() {
    let source = format!(
        "{TYPES}\nFUNC main() AS Integer\n  MUT r AS Rec = Rec[hp := 1, n := 0, xs := [1]]\n  \
         LET q AS Rec = Rec[hp := 2, n := 3, xs := [2]]\n  FOR i = 1 TO 4\n    r = WITH q {{ hp := r.hp + i }}\n  NEXT\n  \
         io::print(toString(r.hp))\n  RETURN 0\nEND FUNC\n"
    );
    assert!(!has(&slots_of(&source, "main"), "record_field_inplace"));
}

const NEST: &str = "\
IMPORT io

TYPE Inner
  a AS Integer
  name AS String
END TYPE

TYPE Outer
  id AS Integer
  inner AS Inner
END TYPE

TYPE Pt
  x AS Integer
  y AS Integer
END TYPE

TYPE Seg
  p AS Pt
  q AS Pt
END TYPE

TYPE Poly
  s AS Seg
  n AS Integer
END TYPE

FUNC mkPt(i AS Integer) AS Pt
  RETURN Pt[x := i, y := i + 1]
END FUNC
";

/// A nested `WITH` whose target is another record's field is a plain value for
/// the field, not an update of this one's sub-block.
#[test]
fn a_nested_with_over_another_places_field_is_not_a_nested_update() {
    let source = format!(
        "{NEST}\nFUNC main() AS Integer\n  MUT o AS Outer = Outer[id := 0, inner := Inner[a := 1, name := \"n\"]]\n  \
         LET g AS Outer = Outer[id := 1, inner := Inner[a := 2, name := \"m\"]]\n  FOR i = 1 TO 4\n    \
         o = WITH o {{ inner := WITH g.inner {{ a := i }} }}\n  NEXT\n  io::print(toString(o.inner.a))\n  RETURN 0\nEND FUNC\n"
    );
    assert!(!has(&slots_of(&source, "main"), "record_nested_inplace"));
}

/// A fixed-size inlined field given a freshly built record (a call's result) is
/// copied over the field and the fresh block freed; a record literal whose own
/// fields are inlined records is taken as a value, not spilled lane by lane.
#[test]
fn an_inlined_field_overwrite_frees_a_fresh_value_and_spills_only_scalar_literals() {
    let source = format!(
        "{NEST}\nFUNC main() AS Integer\n  MUT s AS Seg = Seg[p := Pt[x := 0, y := 0], q := Pt[x := 1, y := 1]]\n  \
         MUT ps AS List OF Seg = []\n  MUT polys AS List OF Poly = []\n  FOR i = 0 TO 3\n    \
         s = WITH s {{ p := mkPt(i) }}\n    \
         ps = collections::append(ps, WITH s {{ p := mkPt(i), q := Pt[x := i, y := 2] }})\n    \
         polys = collections::append(polys, Poly[s := WITH s {{ q := mkPt(i) }}, n := i])\n  NEXT\n  \
         io::print(toString(len(ps)))\n  RETURN 0\nEND FUNC\n"
    )
    .replace("IMPORT io", "IMPORT collections\nIMPORT io");
    let kinds = slots_of(&source, "main");
    assert!(has(&kinds, "record_field_overwrite"), "{kinds:?}");
    assert!(
        has(&kinds, "rec_temp_with_value") || has(&kinds, "rec_temp_literal_field"),
        "{kinds:?}"
    );
}

/// Two variable-width fields replaced in one element `WITH` cannot both be
/// stored where they lie: the element's size could change twice, so the update
/// takes the general write-back (no replace-or-write-back branch).
#[test]
fn two_replaced_fields_in_an_element_update_decline_the_in_place_branch() {
    let source = "\
IMPORT collections
IMPORT io

TYPE Inner
  a AS Integer
  name AS String
END TYPE

TYPE Holder
  label AS String
  inner AS Inner
  n AS Integer
END TYPE

FUNC main() AS Integer
  MUT hs AS List OF Holder = [Holder[label := \"a\", inner := Inner[a := 1, name := \"b\"], n := 0]]
  FOR k = 0 TO 3
    LET h AS Holder = collections::get(hs, 0)
    hs = collections::set(hs, 0, WITH h { label := \"x\" & toString(k), inner := Inner[a := k, name := \"y\" & toString(k)] })
  NEXT
  io::print(toString(len(hs)))
  RETURN 0
END FUNC
";
    let kinds = slots_of(source, "main");
    assert!(!has(&kinds, "element_replace_offset"), "{kinds:?}");
}

/// A record literal whose own fields are inlined records is not spilled lane by
/// lane into a `WITH` item's inlined field: it is built and copied over.
#[test]
fn a_literal_of_inlined_records_in_a_with_item_is_built_as_a_value() {
    let source = format!(
        "IMPORT collections\n{NEST}\nFUNC main() AS Integer\n  MUT pl AS Poly = Poly[s := Seg[p := mkPt(0), q := mkPt(1)], n := 0]\n  \
         MUT polys AS List OF Poly = []\n  FOR i = 0 TO 3\n    \
         polys = collections::append(polys, WITH pl {{ s := Seg[p := mkPt(i), q := mkPt(2)] }})\n  NEXT\n  \
         io::print(toString(len(polys)))\n  RETURN 0\nEND FUNC\n"
    );
    let kinds = slots_of(&source, "main");
    assert!(has(&kinds, "rec_scratch"), "{kinds:?}");
    assert!(!has(&kinds, "rec_temp_literal_field"), "{kinds:?}");
}

/// A nested `WITH` through a record field that owns its own block (the field's
/// record holds a collection, so it is a pointer, not inlined) is peeled as a
/// path level too: the inner scalar is stored into that block where it lies.
#[test]
fn a_nested_with_through_a_pointer_record_field_is_stored_in_the_pointee() {
    let source = "\
IMPORT io

TYPE Bag
  items AS List OF Integer
  n AS Integer
END TYPE

TYPE Wrap
  b AS Bag
  k AS Integer
END TYPE

FUNC main() AS Integer
  MUT w AS Wrap = Wrap[b := Bag[items := [1], n := 0], k := 0]
  FOR i = 1 TO 4
    w = WITH w { b := WITH w.b { n := w.b.n + i } }
  NEXT
  io::print(toString(w.b.n))
  RETURN 0
END FUNC
";
    let kinds = slots_of(source, "main");
    assert!(has(&kinds, "record_field_inplace"), "{kinds:?}");
}

fn tail_program(decl: &str, update: &str) -> String {
    format!(
        "IMPORT collections\nIMPORT io\n\nTYPE R2\n{decl}\nEND TYPE\n\n\
         FUNC main() AS Integer\n  MUT r AS R2 = R2[xs := [1], name := \"a\"]\n  FOR i = 1 TO 4\n    r = WITH r {{ {update} }}\n  NEXT\n  \
         io::print(toString(len(r.xs)))\n  RETURN 0\nEND FUNC\n"
    )
}

/// A collection field is appended in place only when it is the record's LAST
/// inlined field (growing its sub-block extends the block's tail without moving a
/// sibling); with a String after it the append rebuilds through `insert`. A
/// trailing String replaced by a longer one has its own size-checked store.
#[test]
fn an_append_to_a_collection_field_is_in_place_only_when_it_is_last() {
    let append = "xs := collections::append(r.xs, i)";
    let last = slots_of(
        &tail_program("  name AS String\n  xs AS List OF Integer", append),
        "main",
    );
    assert!(has(&last, "inline_append_newrec"), "{last:?}");
    let first = slots_of(
        &tail_program("  xs AS List OF Integer\n  name AS String", append),
        "main",
    );
    assert!(!has(&first, "inline_append_newrec"), "{first:?}");
    assert!(has(&first, "list_insert_result"), "{first:?}");
    let string = slots_of(
        &tail_program(
            "  xs AS List OF Integer\n  name AS String",
            "name := r.name & \"x\"",
        ),
        "main",
    );
    assert!(has(&string, "record_replace_offset"), "{string:?}");
}
