### 1. Unquoted attributes remain unsafe
UNIT:      man-page:encoding/htmlEscape
CLAIM:     "`encoding::htmlEscape` produces a form of `text` that is safe to embed inside HTML/XML element content and attribute values."
VERDICT:   misleading
EVIDENCE:  The body at `src/codegen/builtins/encoding/func_html_escape.rs:39` replaces only `&`, `<`, `>`, `"`, and `'`. The probe printed `x onmouseover=alert(1) `` unchanged, so placing that result in an unquoted HTML attribute can still create another attribute.
SUGGESTED: `encoding::htmlEscape` escapes these five characters for element content and quoted attribute values. It does not make an unquoted HTML attribute value safe.

### 2. Return-versus-mutation behavior is omitted
UNIT:      man-page:encoding/htmlEscape
CLAIM:     "The string to escape."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_html_escape.rs:39-46` assigns transformed text to local `out` and returns it. The probe printed `A&B` followed by `A&amp;B` after calling `htmlEscape(original)`, confirming the input remains unchanged.
SUGGESTED: Returns a new escaped `String`; `value` is unchanged.