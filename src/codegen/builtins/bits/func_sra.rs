//! `bits::sra` — arithmetic (sign-filling) right shift of a 64-bit integer.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
const INTRO: &str = r#"Arithmetic (sign-filling) right shift of a 64-bit integer."#;
const DESC: &str = r#"`sra` shifts `value` right by `count` bit positions as a signed quantity.
Vacated high bits are filled with a copy of the sign bit (bit 63 of `value`),
and bits shifted past bit 0 are discarded. A `count` of `0` returns `value`
unchanged.

Both `value` and the result are raw two's-complement 64-bit `Integer` bit
patterns. Because the sign bit is replicated into the vacated high bits,
shifting a negative value keeps it negative — this is the distinction from the
logical right shift `bits::sr`, which zero-fills the vacated high bits. For a
non-negative `value` the two produce identical results. For the left shift see
`bits::sl`.

`sra` is not the same as signed division by a power of two. Because the
discarded low bits are dropped rather than rounded, a negative result is
rounded toward negative infinity, not toward zero: `bits::sra(-1, 1)` is `-1`
where `-1 / 2` is `0`, and `bits::sra(-3, 1)` is `-2`. A `count` of `63`
therefore collapses `value` to `0` when it is non-negative and to `-1` (all
bits set) when it is negative.

Unlike the total bitwise operations, `sra` validates `count`: it first checks
that `count` is in the range `0` to `63` inclusive and raises
`ErrInvalidArgument` for any value outside it, before performing the shift.
Larger shift amounts are not implicitly clamped or reduced modulo the width —
that is the difference from the rotates `bits::rl64` and `bits::rr64`, which
accept any `count` and let the hardware reduce it.

The operation has no side effects and lowers inline to the target-neutral `asrv`
machine op rather than calling a runtime helper. Every backend encodes it
natively: `asrv Xd, Xn, Xm` on AArch64, `sra rd, rs1, rs2` on RISC-V, and a
`mov` of the count into `rcx` followed by `sar dst, cl` on x86-64, whose shift
instruction takes its variable count only in `cl`. The result is identical on
every architecture and on both the native and Binary Representation execution
paths."#;
const EX: &str = r#"Arithmetic shift of a negative value preserves its sign (signed divide by 16):

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::sra(-256, 4)
  io::print(toString(result))
END SUB
```

Sign-extend the low byte of a packed field by shifting it up to bit 63 and back
down:

```
IMPORT bits
IMPORT io

SUB main()
  LET byte AS Integer = 0x80
  LET signed AS Integer = bits::sra(bits::sl(byte, 56), 56)
  io::print(toString(signed))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sra",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The value to shift. Any 64-bit value; treated as a signed two's-complement bit pattern.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "count",
                    desc: "The shift amount in bits. Must be in the range `0` to `63` inclusive; any other value raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec!["ErrInvalidArgument"],
            body: Body::native(None, None, Some(lower_bits_sra)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::sra`.
pub(crate) fn lower_bits_sra(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let (value_reg, count_reg, value_text, count_text) =
        super::gen_two_integers::lower_bits_two_integers(builder, "sra", args)?;
    let valid = builder.label("bits_shift_valid");
    let out_of_range = builder.label("bits_shift_out_of_range");
    builder.emit(abi::compare_immediate(count_reg, "0"));
    builder.emit(abi::branch_lt(&out_of_range));
    builder.emit(abi::compare_immediate(count_reg, "63"));
    builder.emit(abi::branch_le(&valid));
    builder.emit(abi::label(&out_of_range));
    builder.raise_error_bare("ErrInvalidArgument")?;
    builder.emit(abi::label(&valid));
    let dst = builder.allocate_register()?;
    builder.emit(abi::arithmetic_shift_right_variable(
        dst, value_reg, count_reg,
    ));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.sra({value_text}, {count_text})"),
    })
}
