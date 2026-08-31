//! `strings::byteLen` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Return the UTF-8 byte length of a string."#;

const DESC: &str = r#"`strings::byteLen` returns the number of bytes `value` occupies in its UTF-8
encoding. It counts bytes, not characters: every byte of the encoding is counted
exactly once. The answer is immediate however long the string is — `byteLen`
does not walk the text.

Because UTF-8 uses a variable number of bytes per Unicode scalar value, the
result can exceed `len(value)`, which counts Unicode scalar values. ASCII scalars
occupy one byte each, so the two counts are equal for pure-ASCII text; scalars
outside ASCII occupy two, three, or four bytes each, making the byte length
larger. `byteLen` is therefore always greater than or equal to `len(value)`.

The empty string has a byte length of `0`. `byteLen` inspects `value` only: it
changes nothing and is locale-independent.

To count Unicode scalar values use the bare `len` builtin; to count
user-perceived characters use `strings::graphemesCount`; to obtain the individual
bytes use `strings::toBytes`.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"An ASCII string has one byte per scalar, a non-ASCII one does not:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::byteLen("Hello")))
  io::print(toString(strings::byteLen("😀")))
  RETURN 0
END FUNC
```

Compare byte length with scalar count:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::byteLen("A😀é")))
  io::print(toString(len("A😀é")))
  RETURN 0
END FUNC
```"#;

/// Target-generic `abi_inline` lowering for `strings.byteLen`: the byte length is
/// the leading `u64` count word of the string block, so a single load yields it.
pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 1 {
        return Err("strings.byteLen: no native lowering for these arguments".to_string());
    }
    let value = &args[0];
    builder.require_string("strings.byteLen value", value)?;
    let register = builder.allocate_register();
    builder.emit(abi::load_u64(&register, &value.location, 0));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(register.render()),
        text: format!("strings.byteLen({})", value.text),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "byteLen",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc:
                    "The string to measure. Any `String` is accepted, including the empty string.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
