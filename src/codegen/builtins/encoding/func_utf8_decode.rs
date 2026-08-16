//! `encoding::utf8Decode` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! a Custom (resolver-selected) overload; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/utf8Decode.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Decode a UTF-8 byte or code-unit sequence to a `String`."#;
const DESC: &str = r#"`encoding::utf8Decode` interprets `value` as a UTF-8 byte sequence and returns the
corresponding text. Because MFBASIC strings are always well-formed UTF-8, the
input is validated in full before the string is produced: `utf8Decode` accepts
only a well-formed UTF-8 sequence and rejects an invalid lead byte, a missing or
stray continuation byte, a truncated multi-byte sequence, an overlong encoding, a
surrogate code point (`U+D800`–`U+DFFF`), and any scalar above `U+10FFFF`. The
empty list decodes to the empty string.


`utf8Decode` is a **parameter overload** selected by the argument's element type:
a `List OF Byte` is decoded directly, while a `List OF Integer` is first checked
element by element — each unit must lie in `0..255` — then decoded. The overload
is resolved during monomorphization, so the selection is a compile-time decision,
not a runtime dispatch.

It is the inverse of `encoding::utf8Encode`: decoding the bytes (or integers)
that `utf8Encode` produced reconstructs the original string, and any string
round-trips losslessly through the two functions."#;
const EX: &str = r#"Decode raw UTF-8 bytes back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("héllo")
  io::print(encoding::utf8Decode(raw))
END SUB
```

Decode from a `List OF Integer` code-unit list:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = [104, 105]
  io::print(encoding::utf8Decode(units))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    // Parameter overload selected by the argument's element type. Both
    // implementations carry `Body::Intrinsic` (no registry rewrite target), so IR
    // lowering leaves the canonical `encoding.utf8Decode` for the monomorphizer to
    // resolve to `#encoding_utf8Decode`. The two `__encoding_utf8Decode` bodies live
    // in `package.mfb`.
    let value = |ty: ParameterType| Parameter {
        name: "value",
        desc: "The UTF-8 byte or code-unit sequence to decode.",
        aliases: &[],
        ty,
        default: DefaultValue::None,
    };
    pkg.add_function(RegistryFunction {
        name: "utf8Decode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        // Overloaded on the element type; the per-position render only shows the
        // first overload (`List OF Byte`). The union names both accepted forms.
        expected_arguments: Some("List OF Byte or List OF Integer"),
        implementations: vec![
            Implementation {
                params: vec![value(ParameterType::list_of(ParameterType::Byte))],
                return_type: ParameterType::String,
                errors: vec![],
                body: Body::Intrinsic,
            },
            Implementation {
                params: vec![value(ParameterType::list_of(ParameterType::Integer))],
                return_type: ParameterType::String,
                errors: vec![],
                body: Body::Intrinsic,
            },
        ],
    });
}
