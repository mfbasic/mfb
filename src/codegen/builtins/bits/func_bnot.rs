//! `bits::bnot` — bitwise NOT (one's complement) of a 64-bit integer.
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/bits/bnot.md`; lowering
//! from the former `src/target/shared/code/builder_bits.rs::lower_bits_not`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Bitwise NOT (one's complement) of a 64-bit integer."#;
const DESC: &str = r#"`bnot` returns the one's complement of `a`: every one of the 64 bit positions is
inverted, so bit *i* of the result is `1` exactly when bit *i* of `a` is `0`, and
`0` otherwise. As a two's-complement arithmetic identity this equals `-(a) - 1`.

The operand and the result are raw two's-complement 64-bit `Integer` bit
patterns; `bnot` does not interpret sign. The operation is total — it is defined
for every input and never raises — has no side effects, and lowers to a single
native AArch64 `mvn` instruction inline rather than calling a runtime helper,
producing identical results on the native and Binary Representation execution
paths.

The name is `bnot` rather than `not` because `NOT` is a reserved logical
(Boolean) keyword and cannot be a package member identifier."#;
const EX: &str = r#"Invert every bit of a value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::bnot(255)
  io::print(toString(result))
END SUB
```

Clear the low byte by ANDing with an inverted mask:

```
IMPORT bits
IMPORT io

SUB main()
  LET value AS Integer = 0x1234
  LET highOnly AS Integer = bits::band(value, bits::bnot(255))
  io::print(toString(highOnly))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bnot",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "a",
                desc: "The operand to invert. Any 64-bit value; treated as a raw bit pattern.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(None, None, Some(lower_bits_bnot)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::bnot`.
pub(crate) fn lower_bits_bnot(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    super::native::lower_bits_not(builder, &args[0])
}
