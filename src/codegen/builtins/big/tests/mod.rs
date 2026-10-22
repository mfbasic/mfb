//! `codegen/builtins/big/mod.rs`'s unit tests, moved out of the production file so the coverage
//! denominator measures only production lines (planning/tests.md C15).

use super::{ENDIAN_TYPE_ID, INT_TYPE_ID};
use crate::codegen::registry::registry;
use crate::types::ParameterType;

#[test]
fn big_registered_on_the_clean_room_registry() {
    let pkg = registry().resolve_package("big").expect("big package");
    let source = pkg.get_mfb();
    assert!(source.contains("EXPORT TYPE Int"), "{source}");
    assert!(source.contains("EXPORT ENUM Endian"), "{source}");
    assert!(source.contains("Little"), "{source}");
    assert!(source.contains("Big"), "{source}");
}

#[test]
fn int_and_endian_are_builtin_types() {
    assert!(registry().is_builtin_type("Int"));
    assert!(registry().is_builtin_type("Endian"));
    assert_eq!(
        registry().qualified_builtin_type(INT_TYPE_ID),
        Some(INT_TYPE_ID.to_string())
    );
    assert_eq!(
        registry().qualified_builtin_type(ENDIAN_TYPE_ID),
        Some(ENDIAN_TYPE_ID.to_string())
    );
}

/// Field order is the layout every lowering addresses (plan-127-A §4.2): slot 0 is
/// the magnitude, slot 1 the sign. A reorder must fail here, not as a misread.
#[test]
fn int_field_order_is_magnitude_then_negative() {
    let layout = crate::codegen::registry::builtin_record_layouts()
        .iter()
        .find(|(type_, _)| type_.name() == INT_TYPE_ID)
        .map(|(_, fields)| fields.clone())
        .expect("big.Int has a builtin record layout");
    assert_eq!(
        layout,
        vec![
            (
                "magnitude".to_string(),
                ParameterType::list_of(ParameterType::Byte)
            ),
            ("negative".to_string(), ParameterType::Boolean),
        ]
    );
}

/// Every member and the exact error set of every implementation. An `abi_function`
/// member's declared errors are its registry `errors` vector — the only list
/// `mfb man` renders and the only one a test can pin (plan-127-A Corrections C1).
const MEMBERS: &[(&str, &[&str])] = &[
    ("big.fromInteger", &["ErrOutOfMemory"]),
    ("big.toInteger", &["ErrOverflow"]),
    ("big.fromBytes", &["ErrOutOfMemory"]),
    ("big.toBytes", &["ErrOutOfMemory"]),
    ("big.compare", &[]),
    ("big.equals", &[]),
    ("big.isZero", &[]),
    ("big.sign", &[]),
    ("big.abs", &["ErrOutOfMemory"]),
    ("big.negate", &["ErrOutOfMemory"]),
    ("big.add", &["ErrOutOfMemory"]),
    ("big.subtract", &["ErrOutOfMemory"]),
    ("big.multiply", &["ErrOutOfMemory"]),
    ("big.sum", &["ErrOutOfMemory"]),
    ("big.product", &["ErrOutOfMemory"]),
    ("big.bitLength", &[]),
    ("big.shiftLeft", &["ErrInvalidArgument", "ErrOutOfMemory"]),
    ("big.shiftRight", &["ErrInvalidArgument", "ErrOutOfMemory"]),
    ("big.testBit", &["ErrInvalidArgument"]),
    (
        "big.parse",
        &["ErrInvalidFormat", "ErrInvalidArgument", "ErrOutOfMemory"],
    ),
    ("big.toString", &["ErrOutOfMemory"]),
    (
        "big.toRadixString",
        &["ErrInvalidArgument", "ErrOutOfMemory"],
    ),
    ("big.divide", &["ErrInvalidArgument", "ErrOutOfMemory"]),
    ("big.remainder", &["ErrInvalidArgument", "ErrOutOfMemory"]),
    ("big.divMod", &["ErrInvalidArgument", "ErrOutOfMemory"]),
    ("big.pow", &["ErrInvalidArgument", "ErrOutOfMemory"]),
    ("big.gcd", &["ErrOutOfMemory"]),
    ("big.factorial", &["ErrInvalidArgument", "ErrOutOfMemory"]),
    ("big.modPow", &["ErrInvalidArgument", "ErrOutOfMemory"]),
];

#[test]
fn every_member_declares_exactly_its_errors_and_lowers_natively() {
    let pkg = registry().resolve_package("big").expect("big package");
    assert_eq!(pkg.functions().len(), MEMBERS.len());
    for (name, errors) in MEMBERS {
        let resolved = registry()
            .resolve_func(name)
            .unwrap_or_else(|| panic!("{name} is not registered"));
        for implementation in &resolved.function.implementations {
            assert_eq!(implementation.errors, errors.to_vec(), "{name}");
        }
        // `None` pins that the member stayed an `abi_function`: an inline-lowered
        // body would start feeding the inline-`TRAP` census instead.
        assert_eq!(
            crate::codegen::registry::native_member_declares_error(name),
            None,
            "{name}"
        );
        assert!(
            crate::codegen::registry::abi_function_lower(name).is_some(),
            "{name} has no abi_function lowering"
        );
    }
}

#[test]
fn member_signatures() {
    use crate::codegen::registry::{argument_types, call_return_type_typed};
    let int = || "big.Int".to_string();
    let args = |name: &str| argument_types(name).unwrap_or_else(|| panic!("{name}"));
    let ret = |name: &str| {
        call_return_type_typed(name)
            .map(|t| t.name().into_owned())
            .unwrap_or_else(|| panic!("{name}"))
    };
    assert_eq!(args("big.fromInteger"), vec!["Integer"]);
    assert_eq!(ret("big.fromInteger"), int());
    assert_eq!(args("big.toInteger"), vec![int()]);
    assert_eq!(ret("big.toInteger"), "Integer");
    assert_eq!(
        args("big.fromBytes"),
        vec!["List OF Byte", "Boolean", "big.Endian"]
    );
    assert_eq!(ret("big.fromBytes"), int());
    assert_eq!(args("big.toBytes"), vec![int(), "big.Endian".to_string()]);
    assert_eq!(ret("big.toBytes"), "List OF Byte");
    for name in ["big.compare", "big.equals"] {
        assert_eq!(args(name), vec![int(), int()], "{name}");
    }
    assert_eq!(ret("big.compare"), "Integer");
    assert_eq!(ret("big.equals"), "Boolean");
    assert_eq!(ret("big.isZero"), "Boolean");
    assert_eq!(ret("big.sign"), "Integer");
    for name in ["big.abs", "big.negate"] {
        assert_eq!(args(name), vec![int()], "{name}");
        assert_eq!(ret(name), int(), "{name}");
    }
}

/// The `endian` default pads `big::Endian.Little` (ordinal `0`), typed as the enum
/// so the verifier's table check sees the parameter's own type.
#[test]
fn endian_defaults_to_little() {
    use crate::codegen::registry::default_argument_padding;
    for (name, provided) in [("big.fromBytes", 2), ("big.toBytes", 1)] {
        let padding = default_argument_padding(
            name,
            &vec![ParameterType::Unknown; provided],
            &crate::codegen::builtins::NoTypeKinds,
        );
        assert_eq!(padding.len(), 1, "{name}");
        assert_eq!(padding[0].0, ParameterType::named(ENDIAN_TYPE_ID), "{name}");
        assert_eq!(padding[0].1, "0", "{name}");
    }
}

/// Every member's native body lowers on every backend (plan-127-A Corrections C3):
/// the emitters are exercised in process, and a per-target rejection or a
/// finalizer panic fails here rather than first on a Linux or Windows build.
#[test]
fn every_member_lowers_on_every_backend() {
    let source = r#"IMPORT io
IMPORT big

FUNC main() AS Nothing
  LET a AS big::Int = big::fromInteger(-5)
  LET pair AS List OF Byte = [1, 2]
  LET b AS big::Int = big::fromBytes(pair, FALSE)
  LET c AS big::Int = big::fromBytes(pair, TRUE, big::Endian.Big)
  LET little AS List OF Byte = big::toBytes(a)
  LET wire AS List OF Byte = big::toBytes(c, big::Endian.Big)
  io::print(toString(big::compare(a, b)))
  io::print(toString(big::equals(a, b)))
  io::print(toString(big::isZero(a)))
  io::print(toString(big::sign(a)))
  LET d AS big::Int = big::abs(a)
  LET e AS big::Int = big::negate(b)
  io::print(toString(big::toInteger(d)))
  io::print(toString(len(little) + len(wire) + len(e.magnitude)))
  LET s AS big::Int = big::add(a, b)
  LET t AS big::Int = big::subtract(a, b)
  io::print(toString(big::sign(s)) & toString(big::sign(t)))
  LET values AS List OF big::Int = [a, b, c]
  LET p AS big::Int = big::multiply(a, b)
  LET total AS big::Int = big::sum(values)
  LET prod AS big::Int = big::product(values)
  io::print(toString(big::sign(p)) & toString(big::sign(total)) & toString(big::sign(prod)))
  io::print(toString(big::bitLength(s)) & toString(big::testBit(t, 3)))
  LET left AS big::Int = big::shiftLeft(p, 9)
  LET right AS big::Int = big::shiftRight(total, 2)
  io::print(toString(big::sign(left)) & toString(big::sign(right)))
  LET parsed AS big::Int = big::parse("-1234", 10)
  io::print(big::toString(prod) & big::toRadixString(left, 16) & big::toString(right) & big::toString(parsed))
  LET parts AS big::DivResult = big::divMod(p, b)
  io::print(big::toString(big::divide(p, b)) & big::toString(big::remainder(p, b)) & big::toString(parts.quotient))
  io::print(big::toString(big::pow(a, 5)) & big::toString(big::gcd(p, b)) & big::toString(big::factorial(12)) & big::toString(big::modPow(p, b, c)))
END FUNC
"#;
    for target in crate::testutil::CodeTarget::ALL {
        let code = crate::testutil::code_for_src_on(source, target);
        for (name, _) in MEMBERS {
            let member = name.trim_start_matches("big.");
            let body = code
                .functions
                .iter()
                .find(|f| f.name.contains("big") && f.name.ends_with(member))
                .unwrap_or_else(|| panic!("{}: no lowered body for {name}", target.name()));
            assert!(
                !body.instructions.is_empty(),
                "{}: {name} lowered to an empty body",
                target.name()
            );
        }
    }
}

/// The magnitude is inlined into the record's own block, so a whole-record copy
/// is one byte copy and the slot holds a block-relative offset — the reading every
/// `gen_big/` emitter is written against.
#[test]
fn int_magnitude_field_is_inlined() {
    let model = crate::codegen::engine::builder::TypeModel::builtin_records();
    assert!(crate::codegen::collection::layout::record_field_is_inlined(
        model,
        &ParameterType::list_of(ParameterType::Byte)
    ));
    assert!(
        !crate::codegen::collection::layout::record_field_is_inlined(
            model,
            &ParameterType::Boolean
        )
    );
}
