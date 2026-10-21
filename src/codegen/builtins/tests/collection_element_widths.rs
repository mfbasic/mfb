//! Every `collections` member over every element width.
//!
//! A list of `Byte`, `Boolean` or `Scalar` is laid out with 1- and 4-byte payloads, a
//! list of `Integer` or `Float` with 8-byte ones, and a list of `String` keeps lookup
//! entries. The members that build or walk a list by hand — `toSet`, `zip`, `chunks`,
//! `window`, `partition`, `insert`, `sum`, the finders — each pick their load width,
//! their stride and whether there is an entry to read from the element type, so a
//! member that was only ever lowered over `Integer` has never run its other arms. The
//! corpus lowers a few fixtures; this lowers the cross product, and asserts the
//! contract that matters: every member that type-checks over a width LOWERS, on a
//! little-endian 64-bit and on a different-ISA backend.
//!
//! Which member/type pairs are valid is the language's rule, not this file's: `sum` is
//! defined over `Integer`/`Float`, `sort` over the orderable elements. The table names
//! them, and the count of programs asserted below stops a row from silently dropping out.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_src, CodeTarget};

const TEMPLATE: &str = "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  LET xs AS List OF {T} = [{a}, {b}, {a}]
  LET ys AS List OF {T} = [{b}, {a}]
  LET r = {expr}
  io::print(\"ok\")
  RETURN 0
END FUNC
";

/// `(member, expression, element types)`; `{T}`, `{a}`, `{b}` are the element type and
/// two of its values.
const MEMBERS: &[(&str, &str, &[&str])] = &[
    (
        "toSet",
        "collections::toSet(xs)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "zip",
        "collections::zip(xs, ys)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "findLastIndex",
        "collections::findLastIndex(xs, LAMBDA(x AS {T}) -> TRUE, 2)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "findLastIndexAll",
        "collections::findLastIndex(xs, LAMBDA(x AS {T}) -> TRUE)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "findIndexAll",
        "collections::findIndex(xs, LAMBDA(x AS {T}) -> TRUE)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "findIndex",
        "collections::findIndex(xs, LAMBDA(x AS {T}) -> TRUE, 0)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "find",
        "collections::find(xs, {a})",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "findList",
        "collections::find(xs, ys)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "chunks",
        "collections::chunks(xs, 2)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "window",
        "collections::window(xs, 2)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "partition",
        "collections::partition(xs, LAMBDA(x AS {T}) -> TRUE)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "insert",
        "collections::insert(xs, 1, {a})",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "filter",
        "collections::filter(xs, LAMBDA(x AS {T}) -> TRUE)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "distinct",
        "collections::distinct(xs)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "sort",
        "collections::sort(xs)",
        &["Byte", "Scalar", "Integer", "Float", "String"],
    ),
    (
        "flatten",
        "collections::flatten([xs, ys])",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "drop",
        "collections::drop(xs, 1)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "take",
        "collections::take(xs, 1)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "contains",
        "collections::contains(xs, {a})",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "repeat",
        "collections::repeat(xs, {a}, 3)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "sum",
        "collections::sum(xs)",
        &["Integer", "Float", "Fixed"],
    ),
    (
        "append",
        "collections::append(xs, {b})",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "appendList",
        "collections::append(xs, ys)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "prepend",
        "collections::prepend(xs, {b})",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "removeAt",
        "collections::removeAt(xs, 0)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "set",
        "collections::set(xs, 0, {b})",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "any",
        "collections::any(xs, LAMBDA(x AS {T}) -> TRUE)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "all",
        "collections::all(xs, LAMBDA(x AS {T}) -> TRUE)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "transform",
        "collections::transform(xs, LAMBDA(x AS {T}) -> x)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "reduce",
        "collections::reduce(xs, {a}, LAMBDA(acc AS {T}, x AS {T}) -> x)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "reduceRight",
        "collections::reduceRight(xs, {a}, LAMBDA(acc AS {T}, x AS {T}) -> x)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "reduceCount",
        "collections::reduce(xs, 0, LAMBDA(acc AS Integer, x AS {T}) -> acc + 1)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "reduceText",
        "collections::reduce(xs, \"\", LAMBDA(acc AS String, x AS {T}) -> acc & \"x\")",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "forEach",
        "collections::forEach(xs, LAMBDA(x AS {T}) -> io::print(\"x\"))",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "sortBy",
        "collections::sortBy(xs, LAMBDA(x AS {T}) -> 1)",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "toList",
        "collections::toList(collections::toSet(xs))",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
    (
        "replace",
        "collections::replace(xs, {a}, {b})",
        &[
            "Byte", "Boolean", "Scalar", "Integer", "Float", "String", "Fixed", "Money",
        ],
    ),
];

/// `(element type, first value, second value)`.
const ELEMENTS: &[(&str, &str, &str)] = &[
    ("Byte", "toByte(1)", "toByte(2)"),
    ("Boolean", "TRUE", "FALSE"),
    ("Scalar", "`a`", "`b`"),
    ("Integer", "1", "2"),
    ("Float", "1.5", "2.5"),
    ("String", "\"a\"", "\"b\""),
    ("Fixed", "1.5F", "2.5F"),
    ("Money", "1.25m", "2.5m"),
];

fn program(expression: &str, element: &(&str, &str, &str)) -> String {
    let (type_, a, b) = element;
    TEMPLATE
        .replace("{expr}", expression)
        .replace("{T}", type_)
        .replace("{a}", a)
        .replace("{b}", b)
}

#[test]
fn every_collections_member_lowers_over_every_element_width_it_accepts() {
    let mut lowered = 0;
    for (member, expression, types) in MEMBERS {
        for type_ in *types {
            let element = ELEMENTS
                .iter()
                .find(|(name, ..)| name == type_)
                .expect("a named element type");
            let source = program(
                &expression
                    .replace("{T}", type_)
                    .replace("{a}", element.1)
                    .replace("{b}", element.2),
                element,
            );
            for target in [CodeTarget::LinuxX86_64, CodeTarget::MacosAarch64] {
                if let Err(err) = try_code_for_src(&source, target, Console) {
                    panic!(
                        "collections::{member} over List OF {type_} must lower on {}: {err}",
                        target.name()
                    );
                }
            }
            lowered += 1;
        }
    }
    assert_eq!(
        lowered, 288,
        "the member/element table changed size; update the count with it"
    );
}

/// The fixed-width element types are all laid out without lookup entries, which is the
/// premise the `zip` and `sum` emitters rest on (their entry-reading arms are gone).
#[test]
fn the_fixed_width_elements_have_a_payload_width() {
    use crate::codegen::collection::layout::kind2_payload_size;
    use crate::types::ParameterType;
    for fixed in [
        ParameterType::Integer,
        ParameterType::Float,
        ParameterType::Fixed,
        ParameterType::Byte,
        ParameterType::Boolean,
        ParameterType::named("Scalar"),
    ] {
        assert!(
            kind2_payload_size(&fixed).is_some(),
            "{fixed} is fixed-width: its payload is addressed at index * width"
        );
    }
    assert!(kind2_payload_size(&ParameterType::String).is_none());
}
