//! `encoding::utf8Encode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! a Custom (resolver-selected) overload. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Encode a `String` to its UTF-8 bytes."#;
const DESC: &str = r#"`encoding::utf8Encode` returns the UTF-8 encoding of `value`, one list element
per byte. Because MFBASIC strings
are always UTF-8 text, the result is the string's raw octets in order, with each
element in the range `0..255`.

The function is **total**: every string, including the empty string (which yields
an empty list), encodes successfully, and it never raises a runtime error. The
byte form is exactly `strings::toBytes(value)`; the integer form contains the
identical numeric values widened to `Integer`.

Both forms are listed above, and the note beside them says how one is chosen: the
same `String` argument produces either a `List OF Byte` or a `List OF Integer`,
and only the expected type at the call site decides which. A call with no expected
type is the build-time `TYPE_OVERLOAD_AMBIGUOUS` error, never a runtime failure.
Reach for the byte form for raw octets, and for the integer form when the code
units are about to be used in arithmetic.

The inverse operation is `encoding::utf8Decode`, which accepts either a
`List OF Byte` or a `List OF Integer` and validates it as well-formed UTF-8."#;
const EX: &str = r#"Encode a string to raw UTF-8 bytes:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("héllo")
  io::print(toString(len(raw)))
END SUB
```

Encode to the `List OF Integer` form and round-trip it back:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8Encode("hi")
  io::print(encoding::utf8Decode(units))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    // BOTH forms are registered (bug-530). The member has two `__encoding_utf8Encode`
    // bodies and one descriptor row said so only in prose, so `mfb man encoding
    // utf8Encode` printed a single `Declaration` ending `AS List OF Byte` and the
    // `List OF Integer` form was invisible in the two places a reader looks for a
    // signature. Overload SELECTION is unchanged: the forms share a parameter list,
    // and `match_overload` takes the first implementation that unifies, so the
    // registry still answers `List OF Byte` for a `String` argument exactly as
    // before; the call's expected type still picks the form, in the monomorphizer.
    let value = || Parameter {
        name: "value",
        desc: "The string to encode.",
        aliases: &["text"],
        ty: ParameterType::String,
        default: DefaultValue::None,
    };
    // `Body::Intrinsic` carries no registry rewrite target, so IR lowering leaves the
    // canonical `encoding.utf8Encode` in place for the monomorphizer to resolve to
    // `#encoding_utf8Encode`. The two `__encoding_utf8Encode` bodies live in the
    // package's injected source.
    pkg.add_function(RegistryFunction {
        name: "utf8Encode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        // A multi-implementation member yields no per-position rendering, so the
        // argument-mismatch diagnostic needs the hint spelled out — without it the
        // "expected …" clause would read "no arguments".
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![value()],
                return_type: ParameterType::list_of(ParameterType::Byte),
                errors: vec![],
                body: Body::Intrinsic,
            },
            Implementation {
                params: vec![value()],
                return_type: ParameterType::list_of(ParameterType::Integer),
                errors: vec![],
                body: Body::Intrinsic,
            },
        ],
    });
}
