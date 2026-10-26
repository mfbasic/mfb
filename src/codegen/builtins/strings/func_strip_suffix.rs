//! `strings.stripSuffix` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_strip;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Remove one trailing occurrence of a suffix from a string."#;

const DESC: &str = r#"`strings::stripSuffix` returns `value` with one trailing occurrence of `suffix`
removed when `value` ends with `suffix`, and returns `value` unchanged otherwise.

The match is an exact byte comparison of the trailing bytes of `value` against
every byte of `suffix`, with no normalization and no case folding. A match always
covers whole Unicode scalars, so the remainder is always a valid string — but not
necessarily whole characters: stripping `"\u{0301}"` from `"cafe\u{0301}"` leaves
`"cafe"`.

Exactly one copy is removed. If `value` ends with `suffix` repeated, only the
last copy is stripped and the earlier ones remain — call `stripSuffix` in a loop
to remove them all. An empty `suffix` matches at the end and removes nothing —
`stripSuffix` acts at a single named position, so the package's empty-needle rule
(see `mfb man strings`) makes it a no-op here rather than an error. A `suffix`
longer than `value` cannot match, and a non-matching `suffix` leaves `value`
alone; all three return an equal string.

No input is rejected: an empty `value`, an empty `suffix`, a `suffix` longer
than `value` and a non-matching `suffix` all succeed. The only error is
`ErrOutOfMemory`, raised when the program runs out of memory. Neither argument
is changed. The result is a value of its own; when there is nothing to strip it
is equal to `value`.

To test for the suffix without removing it, use `strings::endsWith`. To remove a
*set* of trailing scalars rather than a fixed substring, use
`strings::trimChars`.

`value` may also be an `AttributedString`: it returns an `AttributedString`
whose text is stripped exactly as for a `String`. Attribute spans that lie
entirely on the removed suffix are dropped, a span that straddles the cut keeps
only its part on the remaining text, and every other span is unchanged."#;

const EX: &str = r#"Remove a file extension; a non-matching suffix changes nothing:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::stripSuffix("photo.png", ".png"))
  io::print(strings::stripSuffix("photo.png", ".jpg"))
  RETURN 0
END FUNC
```

Only one copy is removed:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::stripSuffix("foobarbar", "bar"))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.stripSuffix: no native lowering for these arguments".to_string());
    }
    gen_strip::lower_strings_strip(builder, &args[0], &args[1], true)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "stripSuffix",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string to strip from. May be empty. Returned as an equal copy when it does not end with `suffix`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "suffix",
                    desc: "The trailing substring to remove. May be empty, in which case `value` is returned unchanged.",
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
