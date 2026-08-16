//! `bits::popCount` — count the set bits of a 64-bit integer (population count).
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/bits/popCount.md`; lowering
//! from the former `src/target/shared/code/builder_bits.rs::lower_bits_popcount`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
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

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "popCount",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
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
            body: Body::native(None, None, Some(lower_bits_pop_count)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::popCount`.
pub(crate) fn lower_bits_pop_count(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    super::native::lower_bits_popcount(builder, &args[0])
}
