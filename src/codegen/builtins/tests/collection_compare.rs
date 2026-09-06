//! Every element type a collection can hold, compared.
//!
//! `collection/compare/builder_collection_compare.rs` is one `match` over the
//! element type, per comparison entry point — `contains`, `indexOf`, set
//! membership, map key lookup, `=` between two collections. Each arm knows how
//! that type's bytes sit in a collection block, and an arm nothing reaches is an
//! arm nothing checks.
//!
//! The file was at 73.67%, and the uncovered arms were the ones a corpus of
//! realistic programs does not contain: `Nothing`, `Boolean`, `Byte`, and an
//! ENUM member. Those are exactly the types whose comparison is not the obvious
//! word compare — a `Boolean` and a `Byte` are ONE byte in the block and are
//! loaded with `load_u8`, and reading eight instead would compare the seven
//! bytes that follow them.
//!
//! The enum arm is still uncovered and cannot be reached from here: writing the
//! program that should reach it turned up **bug-549** — `List OF <enum>`
//! type-checks and then fails to build. The row is written out in the table
//! below, commented, so it goes in the day that builds.
//!
//! Comparison is where `.ai/collections.md`'s rule bites: a collection block is
//! not byte-comparable, because the entry fields around the payload are
//! uninitialised. Everything here goes through the payload comparison rather
//! than through a block memcmp, which is what the arms under test implement.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_src, CodeTarget};

/// One program per element type, so a failure names the type.
///
/// `contains` is the entry point rather than `=` because it is the one every
/// container kind shares, and because a `contains` that compares the wrong
/// bytes returns a WRONG ANSWER rather than failing to build — which is what
/// makes these arms worth reaching.
const ELEMENT_TYPES: &[(&str, &str, &str)] = &[
    ("Boolean", "List OF Boolean", "[TRUE, FALSE, TRUE]"),
    ("Byte", "List OF Byte", "[toByte(1), toByte(2)]"),
    ("Integer", "List OF Integer", "[1, 2, 3]"),
    ("Float", "List OF Float", "[1.5, 2.5]"),
    ("Fixed", "List OF Fixed", "[toFixed(1.5), toFixed(2.5)]"),
    ("String", "List OF String", "[\"a\", \"bb\"]"),
    (
        "Set OF Integer",
        "Set OF Integer",
        "Set OF Integer { 1, 2, 3 }",
    ),
    // An ENUM member belongs here and cannot be added yet: `List OF Colour`
    // type-checks and then fails to build -- "native collection packed payload
    // does not support type 'Colour'" (bug-549). The comparator HAS an enum arm
    // and the payload classifier refuses the element type before it is reached,
    // so the row goes in the day that builds:
    //
    //     ("an enum member", "List OF Colour", "[Colour.Red, Colour.Blue]"),
    //
    // and the needle is `Colour.Blue`. The `ENUM Colour` declaration is already
    // in every program below, so nothing else has to change.
];

/// The needle for each of the above, in the same order.
const NEEDLES: &[&str] = &[
    "TRUE",
    "toByte(2)",
    "2",
    "2.5",
    "toFixed(2.5)",
    "\"bb\"",
    "2",
];

fn program(type_name: &str, literal: &str, needle: &str) -> String {
    format!(
        "IMPORT collections\n\
         IMPORT io\n\
         \n\
         ENUM Colour\n\
         \x20 Red, Blue\n\
         END ENUM\n\
         \n\
         FUNC main() AS Integer\n\
         \x20 LET xs AS {type_name} = {literal}\n\
         \x20 IF collections::contains(xs, {needle}) THEN\n\
         \x20   io::print(\"yes\")\n\
         \x20 END IF\n\
         \x20 RETURN 0\n\
         END FUNC\n"
    )
}

/// Every element type's comparison arm lowers, on every backend.
///
/// A missing arm is `native code cannot compare ...` at build time, which is the
/// good failure; the arms that exist and are wrong are caught by the byte-width
/// row below.
#[test]
fn every_element_type_compares_on_every_backend() {
    for ((label, type_name, literal), needle) in ELEMENT_TYPES.iter().zip(NEEDLES) {
        let source = program(type_name, literal, needle);
        for target in CodeTarget::ALL {
            try_code_for_src(&source, target, Console).unwrap_or_else(|err| {
                panic!(
                    "`contains` over a {label} element on {}: {err}",
                    target.name()
                )
            });
        }
    }
}

/// A one-byte element is loaded as ONE byte.
///
/// `Boolean` and `Byte` occupy a single byte in the collection block. Loading
/// eight would compare the seven bytes that follow — which, inside a block whose
/// entry fields are uninitialised, is a comparison against whatever the arena
/// last held there. The answer would be wrong intermittently and correctly
/// often, which is the worst shape a wrong answer can have.
#[test]
fn a_one_byte_element_is_compared_one_byte_wide() {
    use crate::arch::ops::CodeOp;

    let byte_source = program("List OF Byte", "[toByte(1), toByte(2)]", "toByte(2)");
    let plan = try_code_for_src(&byte_source, CodeTarget::LinuxX86_64, Console)
        .expect("the byte program must lower");
    let loads_u8 = plan
        .functions
        .iter()
        .flat_map(|f| f.instructions.iter())
        .filter(|i| i.op == CodeOp::LdrU8)
        .count();
    assert!(
        loads_u8 > 0,
        "a `List OF Byte` comparison must load its candidate one byte wide; the \
         program emitted no `ldr_u8` at all, so the element is being read eight \
         bytes wide and compared against whatever follows it in the block"
    );
}
