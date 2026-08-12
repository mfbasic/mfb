//! `encoding::htmlUnescape` — descriptor entry, docs, and source body.
//!
//! Per-member file (mirrors collections/func_*.rs). The descriptor carries
//! an MFBASIC source body; its authored intro/description/examples migrated from
//! `src/docs/man/builtins/encoding/htmlUnescape.md`. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use super::{ov, p, VALTEXT};
use crate::target::shared::registry::BuiltinFunction;

const INTRO: &str =
    r#"Decode HTML/XML named and numeric character references in a `String` back to text."#;
const DESC: &str = r#"`encoding::htmlUnescape` scans `text` grapheme by grapheme and replaces each
character reference — a run that begins with `&` and ends at the next `;` — with
the character it denotes. Every other character, including `&` characters that
are part of a valid reference's expansion, passes through unchanged.


Three reference forms are recognized, distinguished by the text between `&`
and `;`:

- A **hexadecimal numeric** reference `&#x…;` or `&#X…;` (for example
  `&#xE9;`), where the digits after `#x`/`#X` are parsed as base 16.

- A **decimal numeric** reference `&#…;` (for example `&#233;`), where the
  digits after `#` are parsed as base 10.

- A **named** reference `&…;` (for example `&eacute;`), looked up in the
  built-in entity table.

The resolved code point is emitted as UTF-8 text. Any code point in the range
`0`–`1114111` (`0x10FFFF`) is accepted, including surrogate values, which are
not screened out.

The function is **not total**: it fails on a reference that has no `;`
terminator, on a numeric reference whose digits are empty or non-numeric, on an
unknown entity name, and on a numeric reference whose value exceeds `1114111`.
The empty string yields the empty string. `encoding::htmlUnescape` is the
inverse of `encoding::htmlEscape`."#;
#[rustfmt::skip]
const BODY: &str =
r##"FUNC __encoding_htmlUnescape(text AS String) AS String
  LET chars AS List OF String = strings::graphemes(text)
  LET n AS Integer = len(chars)
  MUT out AS String = ""
  MUT i AS Integer = 0
  MUT ch AS String = ""
  MUT body AS String = ""
  MUT j AS Integer = 0
  MUT found AS Boolean = FALSE
  MUT code AS Integer = 0
  WHILE i < n
    ch = collections::get(chars, i)
    IF ch = "&" THEN
      body = ""
      j = i + 1
      found = FALSE
      WHILE j < n AND found = FALSE
        IF collections::get(chars, j) = ";" THEN
          found = TRUE
        ELSE
          body = body & collections::get(chars, j)
          j = j + 1
        END IF
      END WHILE
      IF found = FALSE THEN
        FAIL error(77050003, "malformed entity")
      END IF
      IF strings::startsWith(body, "#x") OR strings::startsWith(body, "#X") THEN
        code = __encoding_parseHex(strings::mid(body, 2, len(body) - 2))
      ELSE
        IF strings::startsWith(body, "#") THEN
          code = __encoding_parseDecimal(strings::mid(body, 1, len(body) - 1))
        ELSE
          code = __encoding_htmlEntity(body)
        END IF
      END IF
      IF code < 0 THEN
        FAIL error(77050003, "unknown entity")
      END IF
      out = out & __encoding_fromCodepoint(code)
      i = j + 1
    ELSE
      out = out & ch
      i = i + 1
    END IF
  END WHILE
  RETURN out
END FUNC"##;
const EX: &str = r#"Decode named references:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::htmlUnescape("&lt;a&gt;"))
END SUB
```

Decode decimal and hexadecimal numeric references:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::htmlUnescape("caf&#233; / caf&#xE9;"))
END SUB
```

Round-trip through `htmlEscape`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET esc AS String = encoding::htmlEscape("5 > 3 & 2 < 4")
  io::print(encoding::htmlUnescape(esc))
END SUB
```"#;

pub(crate) const HTML_UNESCAPE: BuiltinFunction = BuiltinFunction::mfb(
    "encoding.htmlUnescape",
    "htmlUnescape",
    INTRO,
    DESC,
    &[],
    &[ov(&[p("value", VALTEXT, "String")], "String")],
    BODY,
)
.with_example(EX);
