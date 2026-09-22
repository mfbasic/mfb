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
HTML/XML element content and inside a **quoted** attribute value. It is not enough
for an unquoted attribute: text such as `x onmouseover=alert(1)` contains none of
the five characters and comes back unchanged, so always quote the attribute. It
replaces each of the five metacharacters with its named character reference:


- `&` (ampersand) becomes `&amp;`
- `<` (less-than) becomes `&lt;`
- `>` (greater-than) becomes `&gt;`
- `"` (double quote) becomes `&quot;`
- `'` (apostrophe) becomes `&apos;`

Each character of `text` is considered once, so the `&` that begins each
reference this function writes is never escaped a second time; the result is a
single, correct level of escaping.


Every other character — including whitespace, digits, letters, and non-ASCII
code points — passes through unchanged; only the five characters above are
rewritten. The function is **total**: every `String`, including the empty
string (which yields the empty string), escapes successfully, and it never
raises a runtime error.

The inverse operation is `encoding::htmlUnescape`, which parses named and
numeric character references back into text."#;
#[rustfmt::skip]
const BODY: &str =
r#"' plan-146-F: a single left-to-right grapheme scan. It replaces five
' `strings::replace` passes over `MUT out AS String = text` — a binding that
' COPIED the argument, which is what the `Exempt` row for `encoding::htmlEscape`
' may not do. One pass also makes the "ampersand first" ordering structural
' rather than an invariant of the pass order: each grapheme is emitted once, so
' no reference this function writes can be escaped a second time.
'
' The scan reads `text` a grapheme at a time with `strings::mid`, NOT through
' `strings::graphemes`: that list holds one heap `String` per grapheme and is
' live while `text` still is, which costs ~42 bytes per character of `text` and
' fails the `exempt` peak-live-bytes bound outright (plan-146-F Correction F4).
FUNC __encoding_htmlEscape(text AS String) AS String
  LET n AS Integer = len(text)
  MUT out AS String = ""
  MUT i AS Integer = 0
  MUT ch AS String = ""
  WHILE i < n
    ch = strings::mid(text, i, 1)
    IF ch = "&" THEN
      out = out & "&amp;"
    ELSEIF ch = "<" THEN
      out = out & "&lt;"
    ELSEIF ch = ">" THEN
      out = out & "&gt;"
    ELSEIF ch = "\"" THEN
      out = out & "&quot;"
    ELSEIF ch = "'" THEN
      out = out & "&apos;"
    ELSE
      out = out & ch
    END IF
    i = i + 1
  END WHILE
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
