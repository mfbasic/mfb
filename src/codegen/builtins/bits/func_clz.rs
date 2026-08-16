//! `bits::clz` — count the leading zero bits of a 64-bit integer.
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/bits/clz.md`; lowering
//! from the former `src/target/shared/code/builder_bits.rs::lower_bits_count_zeros`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Count the leading zero bits of a 64-bit integer."#;
const DESC: &str = r#"`clz` returns the number of zero bits that precede the most significant set (`1`)
bit of `value`, counting down from bit 63 (the highest bit) toward bit 0.

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern; `clz`
does not interpret sign. When `value` is `0` there is no set bit, so all 64 bits
count as leading zeros and the result is `64`. When bit 63 is set the result is
`0`. The operation is total — it is defined for every `Integer` and never raises
— has no side effects, and lowers to a single native AArch64 count-leading-zeros
instruction rather than calling a runtime helper, producing identical results on
the native and Binary Representation execution paths."#;
const EX: &str = r#"Count the leading zeros of a small value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::clz(255)
  io::print(toString(result))
END SUB
```

The all-zero pattern has 64 leading zeros:

```
IMPORT bits
IMPORT io

SUB main()
  io::print(toString(bits::clz(0)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "clz",
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
            body: Body::native(None, None, Some(lower_bits_clz)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::clz`.
pub(crate) fn lower_bits_clz(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    super::native::lower_bits_count_zeros(builder, "clz", &args[0])
}
