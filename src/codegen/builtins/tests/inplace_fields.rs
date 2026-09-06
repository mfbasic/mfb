//! The in-place record-field mutators other than `append`.
//!
//! `collection/assign/builder_inplace_assign.rs` has a `try_inplace_*` function
//! per (container, operation) pair, and `append` is one of eight:
//!
//!     try_inplace_set_add_assign                    s = add(s, x)
//!     try_inplace_remove_key_assign                 m = removeKey(m, k)
//!     try_inplace_record_field_set_add_assign       rec.s = add(rec.s, x)
//!     try_inplace_record_field_set_remove_assign    rec.s = remove(rec.s, x)
//!     try_inplace_record_field_remove_key_assign    rec.m = removeKey(rec.m, k)
//!     try_inplace_record_field_remove_at_assign     rec.l = removeAt(rec.l, i)
//!     try_inplace_record_field_set_assign           rec.l = set(rec.l, i, v)
//!
//! Each rewrites `x = op(x, …)` into a write through the block `x` already owns
//! instead of building a new container and dropping the old one, and each has
//! its own guard list for when that would be unsound. Nothing in process
//! reached any of them: the corpus mutates plain locals, not record fields, so
//! the whole record-field half of the file was the branch not taken.
//!
//! Held together in one program because they share the record: what makes the
//! record-field forms different from the plain ones is that the container lives
//! inside another block, and a test that gave each its own record would not be
//! exercising the same thing.

use crate::codegen::engine::tests::test_support::Stream;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

/// A record holding a Set, a Map and a List, each mutated through `WITH`.
const FIELD_MUTATORS: &str = "\
IMPORT collections
IMPORT io

TYPE Store
  tags AS Set OF Integer
  index AS Map OF String TO Integer
  rows AS List OF Integer
END TYPE

FUNC main() AS Integer
  MUT store AS Store = Store[tags := Set OF Integer { 1, 2, 3 }, index := Map OF String TO Integer { \"a\" := 1, \"b\" := 2 }, rows := [10, 20, 30, 40]]
  FOR i = 0 TO 8
    store = WITH store { tags := collections::add(store.tags, i) }
  NEXT
  store = WITH store { tags := collections::remove(store.tags, 2) }
  store = WITH store { index := collections::removeKey(store.index, \"a\") }
  store = WITH store { rows := collections::removeAt(store.rows, 1) }
  store = WITH store { rows := collections::set(store.rows, 0, 99) }
  io::print(toString(len(store.tags)))
  io::print(toString(len(store.index)))
  io::print(toString(len(store.rows)))
  RETURN 0
END FUNC
";

/// The same four operations on plain locals, for the comparison below.
const LOCAL_MUTATORS: &str = "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT tags AS Set OF Integer = Set OF Integer { 1, 2, 3 }
  MUT index AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }
  MUT rows AS List OF Integer = [10, 20, 30, 40]
  FOR i = 0 TO 8
    tags = collections::add(tags, i)
  NEXT
  tags = collections::remove(tags, 2)
  index = collections::removeKey(index, \"a\")
  rows = collections::removeAt(rows, 1)
  rows = collections::set(rows, 0, 99)
  io::print(toString(len(tags)) & toString(len(index)) & toString(len(rows)))
  RETURN 0
END FUNC
";

/// The label stems `main` emitted, with the trailing serial dropped.
fn label_stems(source: &str) -> Vec<String> {
    let plan = code_for_src_cached(source, CodeTarget::LinuxX86_64, Console);
    let main = code_function(plan, "main");
    let mut stems: Vec<String> = Stream::of(main)
        .labels()
        .into_iter()
        .map(|(_, name)| {
            name.trim_end_matches(|c: char| c.is_ascii_digit())
                .trim_end_matches('_')
                .to_string()
        })
        .collect();
    stems.sort();
    stems.dedup();
    stems
}

/// Every record-field mutator lowers to a real body, on a record holding all
/// three container kinds.
///
/// The assertion is that the program lowers AND that the mutators are emitted
/// into `main` rather than behind a call: these are in-place rewrites, and one
/// that quietly fell back to "build a new container, drop the old" would still
/// print the right lengths.
#[test]
fn the_record_field_mutators_lower_and_are_emitted_inline() {
    let stems = label_stems(FIELD_MUTATORS);
    assert!(
        !stems.is_empty(),
        "the program must lower to a body with labels in it"
    );
    let plan = code_for_src_cached(FIELD_MUTATORS, CodeTarget::LinuxX86_64, Console);
    let main = code_function(plan, "main");
    let calls: Vec<String> = main
        .instructions
        .iter()
        .filter(|i| i.op == crate::arch::ops::CodeOp::BranchLink)
        .filter_map(|i| i.get("target"))
        .filter(|target| {
            target.contains("collections") && !target.contains("drop") && !target.contains("copy")
        })
        .collect();
    assert!(
        calls.is_empty(),
        "the record-field mutators are emitted into their caller; `main` called \
         {calls:?} instead, which means one of them fell back to the general \
         rebuild"
    );
}

/// A record field and a plain local reach DIFFERENT lowerings.
///
/// This is the row that says the record-field half is being exercised at all. A
/// container inside a record is not a container in a stack slot: the write has
/// to go through the record's block and, for a variable-width payload, move
/// everything after the field. If both programs emitted the same labels, the
/// record-field functions would not be running and every other row here would
/// be testing the plain path twice.
#[test]
fn a_container_in_a_record_field_lowers_differently_from_a_local() {
    let field = label_stems(FIELD_MUTATORS);
    let local = label_stems(LOCAL_MUTATORS);
    let only_in_field: Vec<&String> = field.iter().filter(|s| !local.contains(s)).collect();
    assert!(
        !only_in_field.is_empty(),
        "mutating a container held in a record field must emit something \
         mutating a local does not; the two programs produced the same label \
         set, so the record-field lowerings are not being reached"
    );
}
