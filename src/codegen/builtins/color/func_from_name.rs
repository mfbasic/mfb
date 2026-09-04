//! `color::fromName` — resolve a CSS named colour.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Resolve a CSS named colour such as `"rebeccapurple"`."#;

const DESC: &str = r#"`fromName` looks `name` up in the CSS Color Level 4 `<named-color>` table and
returns the opaque colour it names. Matching is **case-insensitive** and
surrounding whitespace is trimmed, so a value read straight out of a config file
or a form field resolves without cleaning up first.

Anything not in the table raises `ErrNotFound` (`77050004`) — there is no
best-effort or nearest-colour behaviour. `color::nameOf` is the reverse lookup.

**`green` is not the green you expect.** The CSS keyword `green` is `#008000`, a
dark green; the vivid `#00ff00` most people picture is `lime`. This trips
everyone once, and it fails quietly — you get a colour that merely looks wrong
rather than an error.

CSS spells four greys both ways (`gray`/`grey`, `darkgray`/`darkgrey`,
`lightgray`/`lightgrey`, `slategray`/`slategrey`) and both spellings resolve here
to the same colour. So do the two duplicate pairs `aqua`/`cyan` and
`fuchsia`/`magenta`.

There is deliberately no `transparent`. CSS's `transparent` is `#00000000`, which
`color::rgba(0, 0, 0, 0)` already spells, and a name whose alpha is not `255`
would break `color::nameOf`'s exact-match rule."#;

const EX: &str = r##"Resolve a name, case-insensitively:

```
IMPORT color
IMPORT io

SUB main()
  io::print(color::toHex(color::fromName("RebeccaPurple")))
END SUB
```

The `green` trap — the CSS keyword is dark, and `lime` is the vivid one:

```
IMPORT color
IMPORT io

SUB main()
  io::print("green " & color::toHex(color::fromName("green")))
  io::print("lime  " & color::toHex(color::fromName("lime")))
END SUB
```

An unknown name raises `ErrNotFound` rather than returning a fallback:

```
IMPORT color
IMPORT errorCode
IMPORT io

FUNC lookup(name AS String) AS String
  RETURN color::toHex(color::fromName(name))
  TRAP(err)
    RETURN "unknown (" & toString(err.code = errorCode::ErrNotFound) & ")"
  END TRAP
END FUNC

SUB main()
  io::print(lookup("teal"))
  io::print(lookup("nosuchcolour"))
END SUB
```"##;

/// `strings::trim` then `strings::lower` before the lookup, so the table's keys
/// stay lower-case ASCII and the case-insensitivity lives in exactly one place.
/// `collections::hasKey` before `get` rather than a `getOr` sentinel: every value
/// in the table is a valid packed colour, so there is no in-band value left to mean
/// "absent".
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __color_fromName(name AS String) AS Color
  LET key AS String = strings::lower(strings::trim(name))
  IF NOT collections::hasKey(__COLOR_NAMES, key) THEN
    FAIL error(77050004, "unknown colour name: " & name)
  END IF
  RETURN color::fromPacked(collections::get(__COLOR_NAMES, key))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fromName",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "name",
                desc: "The CSS colour name. Case-insensitive; surrounding whitespace \
                       is ignored.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::COLOR_TYPE),
            errors: vec!["ErrNotFound"],
            body: Body::mfb(BODY, "__color_fromName"),
        }],
    });
}
