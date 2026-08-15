//! `encoding::utf8Encode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! a Custom (resolver-selected) overload; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/utf8Encode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, ParameterType, RegistryFunction,
    RegistryPackage,
};

const INTRO: &str = r#"Encode a `String` to its UTF-8 bytes."#;
const DESC: &str = r#"`encoding::utf8Encode` returns the UTF-8 encoding of `value` — the exact bytes
that make up the string's storage — one element per byte. Because MFBASIC strings
are always UTF-8 text, the result is the string's raw octets in order, with each
element in the range `0..255`.

The function is **total**: every string, including the empty string (which yields
an empty list), encodes successfully, and it never raises a runtime error. The
byte form is exactly `strings::toBytes(value)`; the integer form contains the
identical numeric values widened to `Integer`.

`utf8Encode` is a **return-type overload**: the same `String` argument produces
either a `List OF Byte` or a `List OF Integer`, chosen by the expected
(contextual) type at the call site. A call with no type context to select the
overload is a compile-time `TYPE_OVERLOAD_AMBIGUOUS` error, not a runtime failure;
the overload is resolved during monomorphization.

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

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "utf8Encode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string to encode.",
                aliases: &["text"],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            // Return-type overload: the pre-monomorph default is `List OF Byte`; the
            // `List OF Integer` form is selected by the call-site expected type in
            // syntaxcheck and mangled by the monomorphizer.
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            lowering: Lowering::Helper,
            // `Body::Intrinsic` carries no registry rewrite target, so IR lowering
            // leaves the canonical `encoding.utf8Encode` in place for the
            // monomorphizer to resolve to `#encoding_utf8Encode`. The two
            // `__encoding_utf8Encode` bodies live in `package.mfb`.
            body: Body::Intrinsic,
        }],
    });
}
