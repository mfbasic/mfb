//! `color::fromHex` — parse a CSS-style hex colour.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Parse a CSS-style hex colour in any of its four lengths."#;

const DESC: &str = r#"`fromHex` accepts the four hex forms CSS defines, with or without a leading `#`,
in either digit case:

| Form | Digits | Meaning |
|---|---|---|
| `#rgb` | 3 | each digit doubled — `#f0a` is `#ff00aa` — alpha fully opaque |
| `#rgba` | 4 | each digit doubled, the fourth is alpha |
| `#rrggbb` | 6 | alpha fully opaque |
| `#rrggbbaa` | 8 | as written |

A missing alpha means **opaque**, not transparent, which is what a reader of
`#ff0000` expects and the opposite of what `color::fromPacked` does with the same
24 bits.

Anything else raises `ErrInvalidFormat` (`77050003`): an unsupported length, a
non-hex character, an empty string, a second `#`. The parse is total in the other
direction — every string is either one of the four forms or an error, and there is
no partial or best-effort result.

`color::toHexAlpha` is the lossless inverse: `fromHex(toHexAlpha(c))` is `c` for
every colour. `color::toHex` drops alpha, so it only round-trips an opaque one."#;

const EX: &str = r##"The short form doubles each digit:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::toHexAlpha(color::fromHex("#f0a")))
END SUB
```

The `#` is optional and digit case does not matter:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::toHexAlpha(color::fromHex("3366CC")))
  io::print(color::toHexAlpha(color::fromHex("#3366ccff")))
END SUB
```

A malformed colour raises `ErrInvalidFormat` rather than returning a fallback,
so a bad value in a config file is reported where it is read:

```
IMPORT color
IMPORT errorCode
IMPORT io

FUNC parseOrBlack(text AS String) AS color::Color
  RETURN color::fromHex(text)
  TRAP(err)
    io::print("rejected " & text & ": " & toString(err.code = errorCode::ErrInvalidFormat))
    RETURN color::rgb(0, 0, 0)
  END TRAP
END FUNC

SUB main()
  io::print(color::toHexAlpha(parseOrBlack("#3366cc")))
  io::print(color::toHexAlpha(parseOrBlack("#12345")))
END SUB
```"##;

/// The whole parse is one pass: strip at most one `#`, decode every byte through
/// `__color_hexValue` (which answers `-1` for anything that is not a hex digit, so
/// a stray `#`, a `g` or a non-ASCII byte all land in the same rejection), then
/// branch on the digit count. Decoding *before* the length branch is deliberate —
/// it means a bad character in a well-formed length is rejected too, rather than
/// being read as whatever `-1` arithmetic produces.
#[rustfmt::skip]
const BODY: &str =
r##"FUNC __color_fromHex(text AS String) AS Color
  LET body AS String = strings::stripPrefix(text, "#")
  LET data AS List OF Byte = strings::toBytes(body)
  LET n AS Integer = len(data)
  IF n <> 3 AND n <> 4 AND n <> 6 AND n <> 8 THEN
    FAIL error(77050003, "invalid hex colour length: " & text)
  END IF
  MUT digits AS List OF Integer = []
  MUT i AS Integer = 0
  MUT d AS Integer = 0
  WHILE i < n
    d = __color_hexValue(toInt(collections::get(data, i)))
    IF d < 0 THEN
      FAIL error(77050003, "invalid hex colour digit: " & text)
    END IF
    digits = collections::append(digits, d)
    i = i + 1
  END WHILE
  IF n = 3 THEN
    RETURN Color[toByte(collections::get(digits, 0) * 17), toByte(collections::get(digits, 1) * 17), toByte(collections::get(digits, 2) * 17), toByte(255)]
  END IF
  IF n = 4 THEN
    RETURN Color[toByte(collections::get(digits, 0) * 17), toByte(collections::get(digits, 1) * 17), toByte(collections::get(digits, 2) * 17), toByte(collections::get(digits, 3) * 17)]
  END IF
  IF n = 6 THEN
    RETURN Color[toByte(collections::get(digits, 0) * 16 + collections::get(digits, 1)), toByte(collections::get(digits, 2) * 16 + collections::get(digits, 3)), toByte(collections::get(digits, 4) * 16 + collections::get(digits, 5)), toByte(255)]
  END IF
  RETURN Color[toByte(collections::get(digits, 0) * 16 + collections::get(digits, 1)), toByte(collections::get(digits, 2) * 16 + collections::get(digits, 3)), toByte(collections::get(digits, 4) * 16 + collections::get(digits, 5)), toByte(collections::get(digits, 6) * 16 + collections::get(digits, 7))]
END FUNC"##;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fromHex",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "text",
                desc: "The hex colour: 3, 4, 6 or 8 hex digits, with an optional \
                       leading `#`, in either case.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec!["ErrInvalidFormat"],
            body: Body::mfb(BODY, "__color_fromHex"),
        }],
    });
}
