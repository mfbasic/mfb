//! `regex::findMatch` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__regex_*` body lives here and replaces a
//! `'@@MFB_BODY:findMatch@@` marker in package.mfb via assembled_source (which
//! also appends the two generated Unicode tables). Body byte-significant
//! (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Locate the first regular-expression match and return its span, its text, and its groups."#;

const DESC: &str = r#"`regex::findMatch` compiles `pattern` as a regular expression, searches `value`
for the first match beginning at or after the position `start`, and returns a
`regex::MatchInfo` describing it. It is the extracting form of the package:
`regex::find` reports *where* the first match begins, and `findMatch` reports
what that match actually was — how far it ran, the text it covered, and what
each capturing group captured.

The search is the same one `regex::find` performs — unanchored, leftmost, and
resolved at the winning position by preference order (earlier alternatives,
greedy quantifiers as long as possible, lazy ones as short as possible). For any
`value`, `pattern` and `start`, `regex::findMatch(value, pattern, start).start`
is exactly `regex::find(value, pattern, start)`. Nothing is matched twice: the
span and the group text come from the one search, not from a second pass over
`value`.

The returned `MatchInfo` reads as follows. `start` is the index of the match's first
scalar and `endIndex` the index one past its last, so the span is half-open —
`endIndex - start` is the match's length in scalars, and a zero-length match has
`start = endIndex`. `text` is the matched text itself, the same text
`regex::replace` would insert for `$0`. `groups` holds one `regex::Group` per
capturing group, indexed by group number, with `groups[0]` restating the whole
match; a group that took no part in the match has `start` and `endIndex` of `-1`
and empty `text`, matching what `$N` expands to for such a group. `names` maps
each named group's name to its number, so a group written `(?<year>\d{4})` is
read as `collections::get(m.groups, collections::get(m.names, "year"))` without
counting parentheses. `names` is empty when the pattern names no groups; use
`collections::hasKey` to test a name before looking it up.

When there is no match at or after `start`, `findMatch` does not fail: it returns
a `MatchInfo` whose `start` and `endIndex` are both `-1`, whose `text` is empty, and
whose `groups` and `names` are empty. Because every real match has `start >= 0`,
testing `m.start >= 0` is the unambiguous "there was a match" test. This is where
`findMatch` and `regex::find` part company: a `MatchInfo` has room for a no-match
value and an index does not, so `find` raises `ErrNotFound` on absence while
`findMatch` reports one. Use `findMatch` when absence should be a value rather
than an error.

Positions are Unicode scalar values, never UTF-8 bytes and never grapheme
clusters, consistent with `len` and the `strings` package. A string of `n`
scalars has positions `0` … `n`; position `n` is after the last scalar. The
`start` argument and every index in the returned `MatchInfo` are scalar indexes.

`start` defaults to `0`, meaning the search begins at the start of `value`. It
must be in the range `0` through the scalar length of `value` inclusive; the
upper bound equals the length so that a search may begin at the end of the
string (where only a zero-length or end-anchored pattern can match). A negative
`start`, or one greater than the scalar length, is out of range and fails with
`ErrIndexOutOfRange`. `start` restricts only where a match may begin; it does
not redefine the input, so the absolute anchors `\A` and `\z`, and `^` and `$`
when the `m` flag is off, are still evaluated against the whole value.

`pattern` is an ordinary runtime `String`, so it may be built or read at run
time; it uses MFBASIC's own portable regex dialect, defined in
`mfb spec stdlib regex` (run `mfb man regex` for the language overview), which
produces identical results on every target and never defers to a host regex
library. Because `String` literals process backslash escapes, a literal
backslash is written `"\\"` — `regex::findMatch(value, "\\d+")` extracts the
first run of digits. An invalid pattern fails with `ErrInvalidFormat`. Pattern
compilation is checked before `start`, so `ErrInvalidFormat` takes precedence
when both apply.

`findMatch` does not mutate `value` or `pattern` and has no side effects. To
extract every match rather than the first, use `regex::findAllMatches`."#;

const EX: &str = r#"Extract the text of the first run of digits, which `regex::find` alone cannot
give you because the length is not known in advance:

```
IMPORT regex
IMPORT io

SUB main()
  LET m AS regex::MatchInfo = regex::findMatch("a1b22c333", "\\d+")
  io::print(m.text)
END SUB
```

Read the span, and handle absence with the `-1` sentinel:

```
IMPORT regex
IMPORT io

SUB main()
  LET m AS regex::MatchInfo = regex::findMatch("abc", "\\d+")
  IF m.start >= 0 THEN
    io::print("matched [" & toString(m.start) & ", " & toString(m.endIndex) & ")")
  ELSE
    io::print("no match")
  END IF
END SUB
```

Read a named capture group by name rather than by counting parentheses:

```
IMPORT regex
IMPORT collections
IMPORT io

SUB main()
  LET m AS regex::MatchInfo = regex::findMatch("due 2024-06", "(?<year>\\d{4})-(?<month>\\d{2})")
  IF collections::hasKey(m.names, "year") THEN
    LET year AS regex::Group = collections::get(m.groups, collections::get(m.names, "year"))
    io::print(year.text)
  END IF
END SUB
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __regex_findMatch(value AS String, pattern AS String, start AS Integer) AS MatchInfo
  LET prog AS __regex_Program = __regex_compile(pattern)
  LET ctx AS __regex_Ctx = __regex_makeCtx(value)
  IF start < 0 OR start > ctx.n THEN
    FAIL error(77050001, "List or string index/range is outside valid bounds.")
  END IF
  LET r AS __regex_Result = __regex_searchFrom(prog, ctx, start)
  IF r.ok = FALSE THEN
    RETURN __regex_noMatch()
  END IF
  RETURN __regex_makeMatch(r, value, prog)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "findMatch",
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
                    name: "start",
                    desc: "The zero-based scalar index at or after which the match must begin. Defaults to 0. Must be between 0 and the scalar length of value inclusive; start == len(value) is allowed and can match a zero-length or end-anchored pattern. May be passed by name.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Integer,
                        expr: "0",
                    },
                },
            ],
            return_type: ParameterType::named("MatchInfo"),
            errors: vec!["ErrInvalidFormat", "ErrIndexOutOfRange"],
            body: Body::mfb(FUNC_BODY, "__regex_findMatch"),
        }],
    });
}
