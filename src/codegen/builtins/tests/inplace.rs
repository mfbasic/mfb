//! Which of the three `append` lowerings a program gets, and why it matters.
//!
//! `collections::append` has one spelling and three lowerings, chosen by the
//! shape of the call rather than by the member:
//!
//!   * `append_inplace_*` — `xs = append(xs, oneElement)` on a uniquely-owned
//!     `MUT` local, written through the block `xs` already owns
//!     (`collection/assign/builder_inplace_assign.rs`, guards G1–G11).
//!   * `bulk_append_*` — `xs = append(xs, otherList)`, a concatenation, which
//!     copies the whole source payload and so carries its own word-copy loop
//!     (`bulk_append_data_wloop_*`).
//!   * `list_insert_*` — the general rebuild, for every shape the fast path
//!     declines.
//!
//! Guard **G11** is the one this file is aimed at: "commit only for a
//! statically-known single element of the list's element type". Both directions
//! of getting it wrong are memory bugs rather than wrong answers. Taking the
//! in-place path for a concatenation writes a list handle into a slot sized for
//! the element type. Taking the bulk path for a single element runs a copy loop
//! over one element on the hottest path in the collection layer, and nothing
//! behavioural would report either.
//!
//! There is a fourth, for a list held in a RECORD FIELD:
//!
//!   * `inline_append_*` — `rec = WITH rec { f := append(rec.f, x) }`, which
//!     grows the field's bytes inside the record's own block instead of
//!     rebuilding the record.
//!
//! That one has a condition the other three do not, and it is the interesting
//! part of this file: **the field must be the LAST inlined one**. A record's
//! variable-width fields live end to end in a trailing data region, so growing
//! any but the last would move every byte after it — the `inline_append_prefix_*`
//! loop exists to copy the bytes BEFORE the grown field precisely because there
//! must be nothing after it to move.
//!
//! The three families are named by their emitted labels, which is what makes
//! this a test rather than a line-toucher: a rewrite that quietly routed the
//! single-element case through the general insert would still print the right
//! length.

use std::collections::BTreeSet;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::tests::test_support::Stream;
use crate::codegen::engine::types::NativeCodePlan;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

/// `xs = append(xs, one)` on a uniquely-owned MUT local: the shape G11 commits
/// for.
const SINGLE: &str = "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO 20
    xs = collections::append(xs, i)
  NEXT
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
";

/// `xs = append(xs, ys)` — the item type is the LIST type, not the element
/// type, so G11 falls through to the concatenating path.
const BULK: &str = "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  LET ys AS List OF Integer = [7, 8, 9]
  MUT xs AS List OF Integer = []
  FOR i = 0 TO 20
    xs = collections::append(xs, ys)
  NEXT
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
";

/// A single element again, but of a collection element type: the fast path
/// declines and the general insert rebuilds.
const NESTED: &str = "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT rows AS List OF List OF Integer = []
  FOR i = 0 TO 20
    rows = collections::append(rows, [i, i + 1, i + 2])
  NEXT
  io::print(toString(len(rows)))
  RETURN 0
END FUNC
";

fn plan(source: &str) -> &'static NativeCodePlan {
    code_for_src_cached(source, CodeTarget::LinuxX86_64, Console)
}

/// The label stems `main` emitted, with the trailing serial number dropped.
///
/// Labels are numbered per function in emission order, so the number is exactly
/// the part that churns when an unrelated construct is added ahead of the one
/// under test. The stem is the stable name.
fn label_stems(source: &str) -> BTreeSet<String> {
    let main = code_function(plan(source), "main");
    Stream::of(main)
        .labels()
        .into_iter()
        .map(|(_, name)| {
            name.trim_end_matches(|c: char| c.is_ascii_digit())
                .to_string()
        })
        .map(|stem| stem.trim_end_matches('_').to_string())
        .collect()
}

/// True when any stem starts with `family`.
fn has_family(stems: &BTreeSet<String>, family: &str) -> bool {
    stems.iter().any(|stem| stem.starts_with(family))
}

/// A single element takes the in-place path; a concatenation does not.
///
/// If G11 ever committed for the bulk case, `bulk_append_data_wloop_*` — the
/// loop that copies the source list's payload — would disappear and the
/// element-sized slot write would take its place. That is a list handle written
/// where an Integer belongs.
#[test]
fn only_a_single_element_of_the_element_type_appends_in_place() {
    let single = label_stems(SINGLE);
    let bulk = label_stems(BULK);

    assert!(
        has_family(&single, "append_inplace"),
        "`xs = collections::append(xs, i)` on a uniquely-owned MUT local is the \
         shape G11 commits for, and it emitted no `append_inplace_*` labels at \
         all: {single:?}"
    );
    assert!(
        !has_family(&single, "bulk_append"),
        "appending ONE element must not run the concatenation's payload copy \
         loop -- that is a word-copy over a single element on the hottest path \
         in the collection layer: {single:?}"
    );

    assert!(
        has_family(&bulk, "bulk_append"),
        "`xs = collections::append(xs, ys)` is a concatenation (the item type is \
         the list type, not the element type), so G11 must fall through to the \
         bulk path: {bulk:?}"
    );
    assert!(
        bulk.iter().any(|stem| stem == "bulk_append_data_wloop"),
        "the concatenation must copy the whole source payload, and no \
         `bulk_append_data_wloop` was emitted: {bulk:?}"
    );
    assert!(
        !has_family(&bulk, "append_inplace"),
        "committing the in-place path for a concatenation writes a LIST into a \
         slot sized for the element type; `bulk_append` and `append_inplace` \
         must never both appear: {bulk:?}"
    );
}

/// A single element whose type is itself a collection declines the fast path.
///
/// The element is one value of the element type, so G11 is satisfied — the
/// decline happens further down, and the program rebuilds through the general
/// insert. Pinned because the in-place write for a pointer payload would store
/// the source block's pointer into the destination and leave two lists sharing
/// one buffer.
#[test]
fn a_collection_element_rebuilds_through_the_general_insert() {
    let nested = label_stems(NESTED);
    assert!(
        has_family(&nested, "list_insert"),
        "appending to a `List OF List OF Integer` must go through the general \
         insert: {nested:?}"
    );
    assert!(
        !has_family(&nested, "append_inplace"),
        "the in-place write for a collection payload would store the source \
         block's pointer into the destination, leaving two lists sharing one \
         buffer: {nested:?}"
    );
}

/// None of the three shapes calls an out-of-line append helper.
///
/// All three are emitted into the caller. A refactor that moved any of them
/// behind a `bl` would be a real change — an extra call per append — and would
/// silently invalidate every label-family assertion above, because the labels
/// would move to the helper.
#[test]
fn every_append_shape_is_emitted_into_its_caller() {
    for (label, source) in [("single", SINGLE), ("bulk", BULK), ("nested", NESTED)] {
        let main = code_function(plan(source), "main");
        let calls: Vec<String> = main
            .instructions
            .iter()
            .filter(|i| i.op == CodeOp::BranchLink)
            .filter_map(|i| i.get("target"))
            .filter(|target| target.contains("append"))
            .collect();
        assert!(
            calls.is_empty(),
            "{label}: every append shape is emitted into its caller, so `main` \
             must call no out-of-line append helper; it called {calls:?}"
        );
    }
}

/// `WITH rec { b := append(rec.b, x) }` where `b` is the LAST inlined field.
const WITH_LAST_FIELD: &str = "\
IMPORT collections
IMPORT io

TYPE Bag
  a AS List OF Integer
  b AS List OF Integer
END TYPE

FUNC main() AS Integer
  MUT bag AS Bag = Bag[a := [1, 2, 3], b := []]
  FOR i = 0 TO 20
    bag = WITH bag { b := collections::append(bag.b, i) }
  NEXT
  io::print(toString(len(bag.b)))
  RETURN 0
END FUNC
";

/// The same append, on the field that is NOT last.
const WITH_FIRST_FIELD: &str = "\
IMPORT collections
IMPORT io

TYPE Bag
  a AS List OF Integer
  b AS List OF Integer
END TYPE

FUNC main() AS Integer
  MUT bag AS Bag = Bag[a := [], b := [1, 2, 3]]
  FOR i = 0 TO 20
    bag = WITH bag { a := collections::append(bag.a, i) }
  NEXT
  io::print(toString(len(bag.a)))
  RETURN 0
END FUNC
";

/// `a := append(rec.b, …)` — grows a's buffer from b's, which is G18's case.
const WITH_CROSS_FIELD: &str = "\
IMPORT collections
IMPORT io

TYPE Bag
  a AS List OF Integer
  b AS List OF Integer
END TYPE

FUNC main() AS Integer
  MUT bag AS Bag = Bag[a := [], b := [1, 2, 3]]
  FOR i = 0 TO 20
    bag = WITH bag { b := collections::append(bag.a, i) }
  NEXT
  io::print(toString(len(bag.b)))
  RETURN 0
END FUNC
";

/// A record field grows in place only when it is the LAST inlined field.
///
/// A record's variable-width fields live end to end in a trailing data region.
/// Growing the last one extends the block; growing any other would have to shift
/// every byte after it, and the record is rebuilt instead. The
/// `inline_append_prefix_*` copy loop — the bytes BEFORE the grown field — is
/// the shape that only makes sense for the last one.
#[test]
fn a_record_field_grows_in_place_only_when_it_is_the_last_one() {
    let last = label_stems(WITH_LAST_FIELD);
    assert!(
        has_family(&last, "inline_append"),
        "`WITH bag {{ b := append(bag.b, i) }}` on the LAST inlined field is the \
         shape the record-field in-place path exists for: {last:?}"
    );
    assert!(
        last.iter().any(|stem| stem == "inline_append_prefix_wloop"),
        "the in-place record append must copy the bytes BEFORE the grown field; \
         without that loop the field's new bytes land on top of the ones ahead \
         of it: {last:?}"
    );

    let first = label_stems(WITH_FIRST_FIELD);
    assert!(
        !has_family(&first, "inline_append"),
        "`a` is not the last inlined field, so growing it in place would move \
         every byte of `b` that follows it; the record must be rebuilt: {first:?}"
    );
    assert!(
        has_family(&first, "list_insert"),
        "the declining case must still append -- through the general insert, \
         which rebuilds: {first:?}"
    );
}

/// G18: `b := append(rec.a, …)` writes b's buffer from a's, so it rebuilds.
///
/// The two fields are different blocks. Taking the in-place path here would
/// grow `b` in place while reading `a`, which is not the self-append the fast
/// path is written for — and `b`'s own contents would be dropped rather than
/// appended to.
#[test]
fn appending_one_field_into_another_rebuilds() {
    let cross = label_stems(WITH_CROSS_FIELD);
    assert!(
        !has_family(&cross, "inline_append"),
        "`b := append(bag.a, i)` is not a self-append: the in-place path would \
         grow b's bytes from a's and lose b's own: {cross:?}"
    );
}

/// The same in-place record append, over a VARIABLE-WIDTH element type.
const WITH_STRING_FIELD: &str = "\
IMPORT collections
IMPORT io

TYPE Bag
  a AS Integer
  b AS List OF String
END TYPE

FUNC main() AS Integer
  MUT bag AS Bag = Bag[a := 1, b := []]
  FOR i = 0 TO 20
    bag = WITH bag { b := collections::append(bag.b, \"row\" & toString(i)) }
  NEXT
  io::print(toString(len(bag.b)))
  RETURN 0
END FUNC
";

/// A variable-width element type carries a lookup entry array; a fixed-width
/// one does not, and the grow must copy exactly the one that exists.
///
/// A `List OF String`'s elements are not the same size, so the block keeps an
/// entry array beside the data — offset and length per element — and growing it
/// has to move both regions. A `List OF Integer` has no entry array at all
/// (`entry_stride == 0`), and copying a zero-stride array would be a loop that
/// walks the data region instead.
///
/// The failure if the entries are NOT copied is the quiet kind: the data moves
/// to a new block and the entries still describe the old one, so every element
/// read after the first grow returns bytes from wherever that address now is.
#[test]
fn a_variable_width_field_copies_its_entry_array_and_a_fixed_width_one_has_none() {
    let variable = label_stems(WITH_STRING_FIELD);
    assert!(
        has_family(&variable, "inline_append"),
        "a `List OF String` in the last field still grows in place: {variable:?}"
    );
    assert!(
        variable
            .iter()
            .any(|stem| stem == "inline_append_grow_entries_wloop"),
        "a variable-width element type keeps an entry array beside the data, and \
         the grow must copy it: without that loop the entries describe the old \
         block and every element read after the first grow returns bytes from \
         wherever that address now is: {variable:?}"
    );

    let fixed = label_stems(WITH_LAST_FIELD);
    assert!(
        !fixed
            .iter()
            .any(|stem| stem == "inline_append_grow_entries_wloop"),
        "a `List OF Integer` has no entry array (`entry_stride == 0`), so \
         copying one would be a loop walking the data region instead: {fixed:?}"
    );
}
