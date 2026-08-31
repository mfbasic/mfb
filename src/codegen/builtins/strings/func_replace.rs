//! `strings::replace` — descriptor entry (`Body::Intrinsic`).
//!
//! The `String` overload of `replace` shares its bare native lowering with the
//! `collections::` `List` overload through `builtins::native_builtin_target`
//! (`lower_replace` etc.), so its `Body` is
//! [`Body::Intrinsic`]; the descriptor exists for return-type resolution, arity,
//! errors, and parameter names.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Replace every non-overlapping occurrence of a substring."#;

const DESC: &str = r#"`strings::replace` returns a new `String` in which every non-overlapping
occurrence of `old` in `value` has been replaced with `new`.

Scanning runs left to right. At each match the replacement is emitted and
scanning resumes immediately after the matched region, so matches never overlap
and a replacement is never re-examined: replacing `"aba"` with `"x"` in
`"ababa"` gives `"xba"`, not `"xx"` or `"xa"`. Where `old` does not match, the
original bytes are copied through unchanged.

Matching is an exact byte comparison. `replace` performs no Unicode
normalization, no case folding, and no grapheme-cluster awareness — `old` must
match byte for byte. Because both operands are well-formed UTF-8 and UTF-8 is
self-synchronizing, a byte match is always a whole-scalar match, so the result is
always well-formed UTF-8.

If `old` is the empty string, nothing can match and a copy of `value` is
returned; `replace` never inserts `new` between existing scalars. If `old` is
longer than `value` it likewise cannot match. When `old` does match and `new` is
empty, each match is deleted.

None of the three arguments is mutated. The result is always a new `String` —
when nothing matched, it is its own copy of `value`, so you always get a `String`
back that is independent of the one you passed in.

`old` is also accepted under the name `needle`, and `new` under the name
`replacement`. The bare `replace` name is also defined for lists; see
`mfb man collections replace`.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is transformed exactly as the `String` overload's
and whose attribute spans are remapped by the same edit."#;

const EX: &str = r#"Replace every occurrence, and delete with an empty replacement:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::replace("hello", "l", "x"))
  io::print(strings::replace("banana", "na", ""))
  RETURN 0
END FUNC
```

Matches never overlap, and an empty `old` changes nothing:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::replace("ababa", "aba", "x"))
  io::print(strings::replace("hi", "", "x"))
  RETURN 0
END FUNC
```

Pass the arguments by name:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::replace(value := "hello", old := "l", new := "q"))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "replace",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string to copy from, replacing matches as they are found.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "old",
                    desc: "The substring to search for. Also accepted under the name `needle`. An empty `old`, or one longer than `value`, never matches.",
                    aliases: &["needle"],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "new",
                    desc: "The text written in place of each match. Also accepted under the name `replacement`. May be empty, which deletes each match.",
                    aliases: &["replacement"],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::Intrinsic,
        }],
    });
}
