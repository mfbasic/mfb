//! Numeric lowering routes that a program reaches only through a particular operand
//! type (`engine/operators/builder_numeric/**`): the upper-bound check of `Byte`
//! arithmetic and the quotient-and-remainder sequence of `Fixed MOD`.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::tests::test_support::Stream;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

fn program(body: &str) -> String {
    format!("IMPORT io\n\nFUNC main() AS Integer\n{body}\n  RETURN 0\nEND FUNC\n")
}

fn labels(source: &str, target: CodeTarget) -> Vec<String> {
    let plan = code_for_src_cached(source, target, crate::target::NativeBuildMode::Console);
    Stream::of(code_function(plan, "main"))
        .labels()
        .into_iter()
        .map(|(_, label)| label)
        .collect()
}

fn count_ops(source: &str, target: CodeTarget, op: CodeOp) -> usize {
    let plan = code_for_src_cached(source, target, crate::target::NativeBuildMode::Console);
    code_function(plan, "main")
        .instructions
        .iter()
        .filter(|i| i.op == op)
        .count()
}

/// A `Byte` result is range-checked after every operation that can leave 0..=255:
/// `+`, `*` and `^` each emit a `byte_overflow` check (the cold block that raises
/// `ErrOverflow`); the same operations over `Integer` emit none.
#[test]
fn byte_arithmetic_checks_its_upper_bound_and_integer_arithmetic_does_not() {
    let byte = program(
        "  LET a AS Byte = toByte(7)\n  LET b AS Byte = toByte(3)\n  \
         LET sum AS Byte = a + b\n  LET product AS Byte = a * b\n  LET power AS Byte = a ^ b\n  \
         io::print(toString(sum) & toString(product) & toString(power))",
    );
    let integer = program(
        "  LET a AS Integer = 7\n  LET b AS Integer = 3\n  \
         LET sum AS Integer = a + b\n  LET product AS Integer = a * b\n  LET power AS Integer = a ^ b\n  \
         io::print(toString(sum) & toString(product) & toString(power))",
    );
    for target in CodeTarget::ALL {
        let checks = |source: &str| {
            labels(source, target)
                .iter()
                .filter(|label| label.starts_with("byte_overflow"))
                .count()
        };
        assert!(
            checks(&byte) >= 3,
            "{}: {} byte_overflow checks",
            target.name(),
            checks(&byte)
        );
        assert_eq!(checks(&integer), 0, "{}", target.name());
    }
}

/// `Fixed MOD Fixed` divides, then multiplies back and subtracts (`sdiv` then
/// `msub`); a program with no `MOD` emits neither.
#[test]
fn a_fixed_remainder_is_a_divide_then_a_multiply_subtract() {
    let with_mod = program(
        "  LET a AS Fixed = 7.50F\n  LET b AS Fixed = 2.00F\n  LET c AS Fixed = a MOD b\n  io::print(toString(c))",
    );
    let without = program(
        "  LET a AS Fixed = 7.50F\n  LET b AS Fixed = 2.00F\n  LET c AS Fixed = a + b\n  io::print(toString(c))",
    );
    for target in [CodeTarget::LinuxX86_64, CodeTarget::MacosAarch64] {
        assert!(
            count_ops(&with_mod, target, CodeOp::SDiv) >= 1,
            "{}",
            target.name()
        );
        assert!(
            count_ops(&with_mod, target, CodeOp::MSub) >= 1,
            "{}",
            target.name()
        );
        assert_eq!(
            count_ops(&without, target, CodeOp::SDiv),
            0,
            "{}",
            target.name()
        );
        assert_eq!(
            count_ops(&without, target, CodeOp::MSub),
            0,
            "{}",
            target.name()
        );
    }
}
