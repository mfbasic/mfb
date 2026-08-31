//! `regex::replace` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__regex_*` body lives here and replaces a
//! `'@@MFB_BODY:replace@@` marker in package.mfb via assembled_source (which
//! also appends the two generated Unicode tables). Body byte-significant
//! (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Replace every non-overlapping regular-expression match using a replacement template."#;

const DESC: &str = r##"`regex::replace` compiles `pattern` as a regular expression and returns a new
`String` in which every non-overlapping match in `value` is replaced by the
expansion of `replacement`. The text before, between, and after matches is copied
unchanged. It is the rewriting form of the package: `regex::match` reports only
whether a match exists, `regex::find` reports where the first one begins,
`regex::findAll` reports the start of every non-overlapping match, and `replace`
produces the rewritten text.

Matches are found left to right by the same leftmost, unanchored search
`regex::findAll` exposes. At each match the engine resolves it by preference
order (earlier alternatives, greedy quantifiers as long as possible, lazy ones as
short as possible), and after each match the scan resumes at the position just
past the end of that match, so the matches are non-overlapping. A zero-length
match is valid; the iterator then advances one scalar so iteration always
terminates and the same empty match is never rewritten twice at one position.
Consequently an empty or empty-matching pattern inserts the replacement before
each scalar and once at the end: `regex::replace("abc", "", "-")` is `"-a-b-c-"`.


Positions are Unicode scalar values, never UTF-8 bytes and never grapheme
clusters, consistent with `len` and the `strings` package.

`replacement` is literal text interleaved with capture references: `$N` or `${N}`
inserts capturing group `N` (`$0` is the whole match), `$name` or `${name}`
inserts a named group, and `$$` inserts a literal `$`. An unbraced reference
takes the longest run of digits it can, so use the braced form to butt a reference
against following text: `${1}0` is group `1` then `"0"`, whereas `$10` is group
`10`. A reference to a group that did not participate in the match, or to an
unknown name or an out-of-range number, expands to the empty string. Replacement
content is therefore always well-formed and is never a source of failure; only an
invalid pattern fails.

`pattern` is an ordinary runtime `String`, so it may be built or read at run
time; it uses MFBASIC's own portable regex dialect, defined in
`mfb spec stdlib regex` (run `mfb man regex` for the language overview), which
produces identical results on every target and never defers to a host regex
library. Because `String` literals process backslash escapes, a literal backslash
is written `"\\"` — `regex::replace(value, "\\d", "#")` rewrites every digit. An
invalid pattern fails with `ErrInvalidFormat`. When `pattern` matches nothing in
`value`, `replace` does not fail; it returns a fresh `String` equal to `value`.

`replace` does not mutate `value`, `pattern`, or `replacement` and has no side
effects."##;

const EX: &str = r##"Replace every match, and reorder capture groups (note the doubled backslashes):

```
IMPORT regex

SUB main()
  LET masked AS String = regex::replace("a1b2", "\\d", "#")
  LET ymd AS String = regex::replace("2024-06-24", "(\\d+)-(\\d+)-(\\d+)", "$3/$2/$1")
END SUB
```

`$$` inserts a literal dollar sign:

```
IMPORT regex

SUB main()
  LET price AS String = regex::replace("5", "5", "$$")
END SUB
```"##;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __regex_replace(value AS String, pattern AS String, replacement AS String) AS String
  LET prog AS __regex_Program = __regex_compile(pattern)
  LET ctx AS __regex_Ctx = __regex_makeCtx(value)
  MUT out AS String = ""
  MUT cursor AS Integer = 0
  FOR EACH r IN __regex_matchResults(prog, ctx, 0)
    LET mstart AS Integer = collections::get(r.caps, 0)
    out = out & strings::mid(value, cursor, mstart - cursor)
    out = out & __regex_expand(replacement, r, value, prog)
    cursor = r.pos
  NEXT
  out = out & strings::mid(value, cursor, ctx.n - cursor)
  RETURN out
END FUNC"#;

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
                    desc: "The subject text searched for a match. It is never modified.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "pattern",
                    desc: "The regular expression to compile and search for. It must be a valid pattern in the MFBASIC regex dialect; otherwise the call fails with ErrInvalidFormat.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "replacement",
                    desc: "The replacement template: literal text plus $ capture references as described above. Always well-formed; never a source of failure.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec!["ErrInvalidFormat"],
            body: Body::mfb(FUNC_BODY, "__regex_replace"),
        }],
    });
}
