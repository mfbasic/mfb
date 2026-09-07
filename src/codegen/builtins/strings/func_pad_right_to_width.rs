//! `strings.padRightToWidth` — descriptor + source-backed lowering (bug-528).
//!
//! The body is the shared `__strings_padRightToWidth` in `helper_pad_to_width.rs`
//! (a `WhenUsed` chunk, so a program that never pads to a column width never
//! compiles it).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Pad a string on the right to a given terminal column width."#;

const DESC: &str = r#"`strings::padRightToWidth` returns a new `String` in which copies of `padChar`
are appended to `value` until the result fills as much of `columns` terminal
columns as whole copies of `padChar` allow. Width is measured by
`strings::displayWidth`, so this is the member that lines a table up on screen.

`strings::padRight` measures the same argument in **Unicode scalar values**, which
is the right unit for a fixed-length record field and the wrong one for a column:
`strings::padRight("日本", 4, "-")` is four scalars and six columns, and three
such rows padded to the same scalar count do not line up.

The result **never exceeds** `columns`. When `padChar` occupies two columns and
the gap to fill is odd, the last column is left empty: padding a one-column
`value` to six columns with a two-column `padChar` appends two copies and yields
five columns, not six. Undershooting is the safe direction — a row that is one
column narrow still reads, while one column wide pushes the next column out of
line.

When the display width of `value` already equals or exceeds `columns`, no padding
is added and the result equals `value`. `padRightToWidth` never truncates.

`padChar` is optional and defaults to a single space. When supplied it must be
exactly one Unicode scalar value, and it must occupy **at least one column**: a
zero-width `padChar` — a combining mark, a zero-width joiner — can never reach
the target however many copies are laid down, so it is rejected with
`ErrInvalidArgument`. A negative `columns`, an empty `padChar`, and a `padChar`
of more than one scalar are rejected with the same error. A `columns` so large
that the padded result cannot be built fails with `ErrOutOfMemory` (`77010001`)
rather than producing a wrong answer.

Neither argument is mutated.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is transformed exactly as the `String` overload's
and whose attribute spans are remapped by the same edit."#;

const EX: &str = r#"Build a two-column table whose first column really is the same width on
screen — the scalar-counted `strings::padRight` cannot do this:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  FOR EACH label IN ["ascii", "日本語", "café"]
    io::print("|" & strings::padRightToWidth(label, 10) & "|end|")
  NEXT
  RETURN 0
END FUNC
```

A two-column `padChar` undershoots rather than overshooting, and a value already
wide enough comes back unchanged:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET padded AS String = strings::padRightToWidth("x", 6, "😀")
  io::print(toString(strings::displayWidth(padded)))
  io::print(strings::padRightToWidth("日本語", 4))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "padRightToWidth",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string to pad. Returned as an equal value when its display width is already at least `columns`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "columns",
                    desc: "The target width of the result in TERMINAL COLUMNS, as `strings::displayWidth` measures them — not scalars and not bytes. Must be `0` or greater; `0` never pads. The result never exceeds it.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "padChar",
                    desc: "Optional. The fill character appended toward `columns`; defaults to a single space. Must be exactly one Unicode scalar value occupying at least one column.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::String,
                        expr: " ",
                    },
                },
            ],
            return_type: ParameterType::String,
            errors: vec!["ErrInvalidArgument"],
            body: Body::Rewrite("__strings_padRightToWidth"),
        }],
    });
}
