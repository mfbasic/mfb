//! `strings.stripPrefix` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_strip;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Remove one leading occurrence of a prefix from a string."#;

const DESC: &str = r#"`strings::stripPrefix` returns `value` with one leading occurrence of `prefix`
removed when `value` begins with `prefix`, and returns `value` unchanged
otherwise.

The match is an exact byte comparison of the leading bytes of `value` against
every byte of `prefix`, with no normalization and no case folding. A match always
covers whole Unicode scalars, so the remainder is always a valid string — but not
necessarily whole characters: stripping `"e"` from `"e\u{0301}"` leaves the lone
combining accent.

Exactly one copy is removed. If `value` begins with `prefix` repeated, only the
first copy is stripped and the rest remain — call `stripPrefix` in a loop to
remove them all. An empty `prefix` matches at the start and removes nothing —
`stripPrefix` acts at a single named position, so the package's empty-needle rule
(see `mfb man strings`) makes it a no-op here rather than an error. A `prefix`
longer than `value` cannot match, and a non-matching `prefix` leaves `value`
alone; all three return an equal string.

No input is rejected: an empty `value`, an empty `prefix`, a `prefix` longer
than `value` and a non-matching `prefix` all succeed. The only error is
`ErrOutOfMemory`, raised when the program runs out of memory. Neither argument
is changed. The result is a value of its own; when there is nothing to strip it
is equal to `value`.

To test for the prefix without removing it, use `strings::startsWith`. To remove
a *set* of leading scalars rather than a fixed substring, use
`strings::trimChars`.

`value` may also be an `AttributedString`: it returns an `AttributedString`
whose text is stripped exactly as for a `String`. Attribute spans that lie
entirely on the removed prefix are dropped, a span that straddles the cut keeps
only its part on the remaining text, and every other span shifts left by the
number of scalars removed."#;

const EX: &str = r#"Remove a leading scheme; a non-matching prefix changes nothing:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::stripPrefix("https://example.com", "https://"))
  io::print(strings::stripPrefix("example.com", "https://"))
  RETURN 0
END FUNC
```

Only one copy is removed:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::stripPrefix("foofoobar", "foo"))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.stripPrefix: no native lowering for these arguments".to_string());
    }
    gen_strip::lower_strings_strip(builder, &args[0], &args[1], false)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "stripPrefix",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string to strip from. May be empty. Returned as an equal copy when it does not begin with `prefix`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "prefix",
                    desc: "The leading substring to remove. May be empty, in which case `value` is returned unchanged.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_inline(lower),
        }],
    });
}
