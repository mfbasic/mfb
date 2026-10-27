//! `strings.graphemeAt` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_graphemes::grapheme_at_window;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Return the extended grapheme cluster at a grapheme index."#;

const DESC: &str = r#"`strings::graphemeAt` returns the single extended grapheme cluster at the
zero-based grapheme `index` of `value`, as a new `String`.

Indexing is by cluster, not by Unicode scalar value (as `len` counts) and not by
UTF-8 byte (as `strings::byteLen` counts). It uses the same enumeration
`strings::graphemes` produces, so `graphemeAt(value, i)` is the element at
position `i` of `strings::graphemes(value)`. An extended grapheme cluster is one
user-perceived character and may be several scalars — a base letter plus
combining marks, a regional-indicator flag pair, or an emoji ZWJ sequence — so
the returned string may hold multiple scalars and many bytes.

`index` is zero-based: `0` selects the first cluster and the last valid index is
`strings::graphemesCount(value) - 1`. An index that is negative, or at or beyond
the cluster count, is out of range and raises `ErrIndexOutOfRange`; it is never
clamped or wrapped. The empty string has no clusters, so every index is out of
range for it.

`value` is not mutated; the returned `String` is a fresh copy of the selected
cluster.

`value` may also be an `AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors). `ErrOutOfMemory` means the program ran out of memory."#;

const EX: &str = r#"Retrieve a cluster by index:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::graphemeAt("abc", 1))
  io::print(strings::graphemeAt("a😀b", 1))
  RETURN 0
END FUNC
```

Guard the index against the cluster count:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET text AS String = "abc"
  IF strings::graphemesCount(text) > 5 THEN
    io::print(strings::graphemeAt(text, 5))
  ELSE
    io::print("too short")
  END IF
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.graphemeAt: no native lowering for these arguments".to_string());
    }
    // bug-992: walk to the wanted cluster inside `value`'s own bytes and copy
    // just that span out. The list of every cluster this used to build, to read
    // one entry of it, had no owner: one whole `List OF String` leaked per call.
    let (ptr, len) = grapheme_at_window(builder, &args[0], &args[1])?;
    let result = builder.emit_materialize_string_from_bytes(&ptr, &len)?;
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::String,
        location: Operand::from(result),
        text: "strings.graphemeAt".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "graphemeAt",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string to index. Any `String` is accepted, though every index is out of range for the empty string.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "index",
                    desc: "The zero-based grapheme index to retrieve. Must satisfy `0 <= index < strings::graphemesCount(value)`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec!["ErrIndexOutOfRange", "ErrOutOfMemory"],
            body: Body::abi_inline(lower),
        }],
    });
}
