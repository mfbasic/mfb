//! `strings.upper` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_case_map;
use crate::codegen::builtins::strings::UnicodeCaseMap;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Map a string to uppercase using Unicode full case mapping."#;

const DESC: &str = r#"`strings::upper` returns a new `String` in which every scalar of `value` has been
mapped to its uppercase form. The mapping is applied per Unicode scalar value
across the whole string, using the uppercase table embedded in the runtime.
Scalars with no uppercase mapping — digits, punctuation, symbols, and
already-uppercase letters — are copied through unchanged.

The mapping is *full*, not simple: one scalar may expand into several. The German
sharp s `ß` uppercases to `SS`, so `upper` can return a string that is longer
than its input in both scalars and bytes. Never assume `len` is preserved across
a case mapping.

The mapping is deterministic and locale-independent: it always uses the default
Unicode case conventions and applies no language-specific tailoring, so no
Turkish dotted/dotless-i tailoring is performed. `upper` does not normalize, so
combining sequences stay decomposed; apply `strings::normalizeNfc` first when
that matters.

For caseless *comparison*, prefer `strings::caseFold` over uppercasing or
lowercasing both operands. `value` is not mutated; the result is a new owned
`String`.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is uppercased as above, but **attributes are
dropped** — a full case mapping can change the scalar count within a span, so the
overlay cannot be remapped."#;

const EX: &str = r#"Uppercase a word; uncased scalars pass through:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::upper("hello"))
  io::print(strings::upper("abc-123"))
  RETURN 0
END FUNC
```

Full case mapping can lengthen the string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET german AS String = "straße"
  io::print(strings::upper(german))
  io::print(toString(len(german)))
  io::print(toString(len(strings::upper(german))))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if let Some(value) = builder.static_strings_package_string("strings.upper", args)? {
        let register = builder.load_string_constant(&value)?;
        return Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(register.render()),
            text: "strings.upper".to_string(),
        });
    }
    if args.len() != 1 {
        return Err("strings.upper: no native lowering for these arguments".to_string());
    }
    gen_case_map::lower_strings_case_map(builder, &args[0], UnicodeCaseMap::Upper)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "upper",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string to uppercase. Any `String` is accepted, including the empty string.",
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
