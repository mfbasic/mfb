//! `encoding::htmlEscape` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Escape the five HTML/XML metacharacters in a `String`."#;
const DESC: &str = r#"`encoding::htmlEscape` produces a form of `text` that is safe to embed inside
HTML/XML element content and attribute values. It replaces each of the five
metacharacters with its named character reference:


- `&` (ampersand) becomes `&amp;`
- `<` (less-than) becomes `&lt;`
- `>` (greater-than) becomes `&gt;`
- `"` (double quote) becomes `&quot;`
- `'` (apostrophe) becomes `&apos;`

The ampersand is substituted **first**, before the other four, so that the `&`
introduced by each replacement entity is not escaped a second time; the result
is therefore a single, correct level of escaping.


Every other character — including whitespace, digits, letters, and non-ASCII
code points — passes through unchanged; only the five characters above are
rewritten. The function is **total**: every `String`, including the empty
string (which yields the empty string), escapes successfully, and it never
raises a runtime error.

The inverse operation is `encoding::htmlUnescape`, which parses named and
numeric character references back into text."#;
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __encoding_htmlEscape(text AS String) AS String
  MUT out AS String = text
  out = strings::replace(out, "&", "&amp;")
  out = strings::replace(out, "<", "&lt;")
  out = strings::replace(out, ">", "&gt;")
  out = strings::replace(out, "\"", "&quot;")
  out = strings::replace(out, "'", "&apos;")
  RETURN out
END FUNC"#;
const EX: &str = r#"Escape a fragment before placing it in element content:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::htmlEscape("<a href='#'>Tom & Jerry</a>"))
END SUB
```

Round-trip through `htmlUnescape`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET esc AS String = encoding::htmlEscape("5 > 3 & 2 < 4")
  io::print(esc)
  io::print(encoding::htmlUnescape(esc))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "htmlEscape",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string to escape.",
                aliases: &["text"],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::mfb(BODY, "__encoding_htmlEscape"),
        }],
    });
}
