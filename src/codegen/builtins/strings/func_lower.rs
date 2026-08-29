//! `strings.lower` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_case_map;
use crate::codegen::builtins::strings::UnicodeCaseMap;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Map a string to lowercase using Unicode full case mapping."#;

const DESC: &str = r#"`strings::lower` returns a new `String` in which every scalar of `value` has been
mapped to its lowercase form. The mapping is applied per Unicode scalar value
across the whole string, using the lowercase table embedded in the runtime.
Scalars with no lowercase mapping — digits, punctuation, symbols, and
already-lowercase letters — are copied through unchanged.

The mapping is *full*, not simple: one scalar may expand into several. `U+0130`
LATIN CAPITAL LETTER I WITH DOT ABOVE (`İ`) lowercases to `i` followed by
`U+0307` COMBINING DOT ABOVE, two scalars, so `lower` can return a string longer
than its input. Never assume `len` is preserved across a case mapping.

The mapping is deterministic and locale-independent: it always uses the default
Unicode case conventions and applies no language-specific tailoring, so no
Turkish dotted/dotless-i tailoring is performed. `lower` does not normalize, so
combining sequences stay decomposed; apply `strings::normalizeNfc` first when
that matters.

For caseless *comparison*, prefer `strings::caseFold` over lowercasing both
operands. `value` is not mutated; the result is a new owned `String`.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is transformed as above, but **attributes are
dropped** — the mapping changes the scalar count within a span, so the overlay
cannot be remapped."#;

const EX: &str = r#"Lowercase a word; uncased scalars pass through:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::lower("HELLO"))
  io::print(strings::lower("ABC-123"))
  RETURN 0
END FUNC
```

Full case mapping can lengthen the string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET dotted AS String = "İ"
  io::print(toString(len(dotted)))
  io::print(toString(len(strings::lower(dotted))))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if let Some(value) = builder.static_strings_package_string("strings.lower", args)? {
        let register = builder.load_string_constant(&value)?;
        return Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(register.render()),
            text: "strings.lower".to_string(),
        });
    }
    if args.len() != 1 {
        return Err("strings.lower: no native lowering for these arguments".to_string());
    }
    gen_case_map::lower_strings_case_map(builder, &args[0], UnicodeCaseMap::Lower)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "lower",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string to lowercase. Any `String` is accepted, including the empty string.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
