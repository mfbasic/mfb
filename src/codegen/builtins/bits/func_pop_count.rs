//! `bits::popCount` — count the set bits of a 64-bit integer (population count).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::mir;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTRO: &str = r#"Count the set (`1`) bits of a 64-bit integer (population count)."#;
const DESC: &str = r#"`popCount` returns the number of set (`1`) bits in `value`, also known as its
Hamming weight or population count.

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern;
`popCount` does not interpret sign, so every one of the 64 bit positions is
inspected regardless of whether `value` is negative. When `value` is `0` no bits
are set and the result is `0`; when every bit is set (the bit pattern `-1`) the
result is `64`. The operation is total — it is defined for every `Integer` and
never raises — and has no side effects. It lowers inline rather than calling a
runtime helper: on AArch64 as a short NEON sequence (move into a vector register,
per-byte `CNT`, then `ADDV` the byte counts into one lane), and on other ISAs as
the portable SWAR (bit-twiddling) sequence over the integer ALU. Both paths
produce identical results on the native and Binary Representation execution
paths."#;
const EX: &str = r#"Count the set bits of a small value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::popCount(255)
  io::print(toString(result))
END SUB
```

The all-ones pattern has 64 set bits:

```
IMPORT bits
IMPORT io

SUB main()
  io::print(toString(bits::popCount(-1)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "popCount",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The 64-bit value to inspect. Any `Integer` is accepted; treated as a raw bit pattern.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower_bits_pop_count),
        }],
    });
}

// 64-bit population-count masks (SWAR Hamming weight), as decimal so they round
// trip through `move_immediate`'s arbitrary-constant path.
const POPCOUNT_MASK_5555: &str = "6148914691236517205"; // 0x5555555555555555
const POPCOUNT_MASK_3333: &str = "3689348814741910323"; // 0x3333333333333333
const POPCOUNT_MASK_0F0F: &str = "1085102592571150095"; // 0x0F0F0F0F0F0F0F0F
const POPCOUNT_MASK_0101: &str = "72340172838076673"; //  0x0101010101010101

/// Target-generic call-site lowering for `bits::popCount` — the 64-bit Hamming
/// weight. `popCount` is the sole consumer, so its body lives here; it still
/// borrows the shared `gen_one_integer::lower_bits_one_integer` operand check.
/// On AArch64 a short NEON sequence (`CNT`/`ADDV`); on other ISAs the portable
/// SWAR over the integer ALU.
pub(crate) fn lower_bits_pop_count(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let value = &args[0];
    if value.type_ != "Integer" {
        return Err(format!("bits.popCount does not accept {}", value.type_));
    }
    let text = format!("bits.popCount({})", value.text);

    // plan-39 K2: on AArch64 the 64-bit Hamming weight is a short NEON sequence —
    // move the value into a `d` register, `CNT` per byte, `ADDV` the 8 byte-counts
    // into lane 0, and move the (0..=64) sum back — instead of the 12-instruction
    // SWAR. Other ISAs keep the portable SWAR below.
    if mir::active_backend().is_aarch64() {
        let dst = builder.allocate_register()?;
        builder.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[0], &value.location));
        builder.emit(abi::vector_cnt8b(abi::VEC_SCRATCH[0], abi::VEC_SCRATCH[0]));
        builder.emit(abi::vector_addv8b(abi::VEC_SCRATCH[0], abi::VEC_SCRATCH[0]));
        builder.emit(abi::vector_extract_to_x(dst, abi::VEC_SCRATCH[0], 0));
        return Ok(ValueResult {
            type_: "Integer".to_string(),
            location: Operand::from(dst.render()),
            text,
        });
    }

    let acc = builder.allocate_register()?;
    let temp = builder.allocate_register()?;
    let mask = builder.allocate_register()?;
    builder.emit(abi::move_register(acc, &value.location));

    // acc = acc - ((acc >> 1) & 0x5555...)
    builder.emit(abi::shift_right_immediate(temp, acc, 1));
    builder.emit(abi::move_immediate(mask, "Integer", POPCOUNT_MASK_5555));
    builder.emit(abi::and_registers(temp, temp, mask));
    builder.emit(abi::subtract_registers(acc, acc, temp));

    // acc = (acc & 0x3333...) + ((acc >> 2) & 0x3333...)
    builder.emit(abi::move_immediate(mask, "Integer", POPCOUNT_MASK_3333));
    let low = builder.allocate_register()?;
    builder.emit(abi::and_registers(low, acc, mask));
    builder.emit(abi::shift_right_immediate(temp, acc, 2));
    builder.emit(abi::and_registers(temp, temp, mask));
    builder.emit(abi::add_registers(acc, low, temp));

    // acc = (acc + (acc >> 4)) & 0x0F0F...
    builder.emit(abi::shift_right_immediate(temp, acc, 4));
    builder.emit(abi::add_registers(acc, acc, temp));
    builder.emit(abi::move_immediate(mask, "Integer", POPCOUNT_MASK_0F0F));
    builder.emit(abi::and_registers(acc, acc, mask));

    // acc = (acc * 0x0101...) >> 56
    builder.emit(abi::move_immediate(mask, "Integer", POPCOUNT_MASK_0101));
    builder.emit(abi::multiply_registers(acc, acc, mask));
    builder.emit(abi::shift_right_immediate(acc, acc, 56));

    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(acc.render()),
        text,
    })
}
