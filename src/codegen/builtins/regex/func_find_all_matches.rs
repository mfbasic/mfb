//! `regex::findAllMatches` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__regex_*` body lives here and replaces a
//! `'@@MFB_BODY:findAllMatches@@` marker in package.mfb via assembled_source
//! (which also appends the two generated Unicode tables). Body byte-significant
//! (2-space indent → .ncode columns); do not reformat.
//!
//! The walk is `__regex_matchResults`, the same helper `__regex_findAll` and
//! `__regex_replace` consume, so the match SEQUENCE — including the zero-width
//! rule ("a match ending where the previous one did is skipped by advancing one
//! position") — is shared code rather than a second copy that could drift.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Locate every non-overlapping regular-expression match and return each one's span, text, and groups."#;

const DESC: &str = r#"`regex::findAllMatches` compiles `pattern` as a regular expression, scans
`value` for every non-overlapping match beginning at or after the position
`start`, and returns a `List OF regex::MatchInfo` in left-to-right order. It is the
extracting form of `regex::findAll`: where `findAll` reports only where each
match begins, `findAllMatches` reports how far each one ran, the text it
covered, and what each capturing group captured. When there is no match, the
result is the empty list `[]` rather than a failure.

The matches are exactly the matches `regex::findAll` reports, found by the same
scan, in the same order: for any `value`, `pattern` and `start`, the `start`
field of the nth `MatchInfo` equals the nth index `regex::findAll` returns, and the
list lengths are equal. After each match the scan resumes just past the end of
that match, so matches never overlap and the start indexes strictly increase. A
zero-length match is recorded, and the scan then advances by one scalar to make
progress; a zero-length match is never recorded twice at a position the previous
match already covered. Nothing is matched twice to produce the spans: each
`MatchInfo` reports what its own search already computed.

Each `regex::MatchInfo` reads as follows. `start` is the index of the match's first
scalar and `endIndex` the index one past its last, so the span is half-open —
`endIndex - start` is the match's length in scalars, and a zero-length match has
`start = endIndex`. `text` is the matched text itself, the same text
`regex::replace` would insert for `$0`. `groups` holds one `regex::Group` per
capturing group, indexed by group number, with `groups[0]` restating the whole
match; a group that took no part in that match has `start` and `endIndex` of
`-1` and empty `text`. `names` maps each named group's name to its number, so a
group written `(?<word>\w+)` is read as
`collections::get(m.groups, collections::get(m.names, "word"))` without counting
parentheses.

The whole list is built before it is returned, so a scan of a very long subject
holds one `MatchInfo` per match at once; when only the positions are wanted,
`regex::findAll` is the cheaper call, and when only the first match is wanted,
`regex::findMatch` stops at it.

`start` restricts only where the first match may begin; it does not redefine the
input, so the absolute anchors `\A` and `\z`, and `^` and `$` when the `m` flag
is off, are still evaluated against the whole value. Positions are Unicode
scalar values, never UTF-8 bytes and never grapheme clusters, consistent with
`len` and the `strings` package. A string of `n` scalars has positions `0` … `n`;
position `n` is after the last scalar. The `start` argument and every index in
every returned `MatchInfo` are scalar indexes.

`start` defaults to `0`, meaning the scan begins at the start of `value`. It
must be in the range `0` through the scalar length of `value` inclusive; the
upper bound equals the length so that the scan may begin at the end of the
string (where only a zero-length or end-anchored pattern can match). A negative
`start`, or one greater than the scalar length, is out of range and fails with
`ErrIndexOutOfRange`.

`pattern` is an ordinary runtime `String`, so it may be built or read at run
time; it uses MFBASIC's own portable regex dialect, defined in
`mfb spec stdlib regex` (run `mfb man regex` for the language overview), which
produces identical results on every target and never defers to a host regex
library. Because `String` literals process backslash escapes, a literal
backslash is written `"\\"` — `regex::findAllMatches(value, "\\d+")` extracts
every run of digits. An invalid pattern fails with `ErrInvalidFormat`. Pattern
compilation is checked before `start`, so `ErrInvalidFormat` takes precedence
when both apply.

`findAllMatches` does not mutate `value` or `pattern` and has no side effects."#;

const EX: &str = r##"Extract every run of digits, which `regex::findAll` alone cannot give you
because the lengths differ from match to match:

```
IMPORT regex
IMPORT io

SUB main()
  FOR EACH m IN regex::findAllMatches("a1b22c333", "\\d+")
    io::print(m.text)
  NEXT
END SUB
```

Report each match's span, and rewrite around one:

```
IMPORT regex
IMPORT strings
IMPORT io

SUB main()
  LET text AS String = "a1b22c333"
  FOR EACH m IN regex::findAllMatches(text, "\\d+")
    io::print("[" & toString(m.start) & ", " & toString(m.endIndex) & ") = " & m.text)
  NEXT
  LET first AS regex::MatchInfo = regex::findMatch(text, "\\d+")
  io::print(strings::left(text, first.start) & "#" & strings::right(text, len(text) - first.endIndex))
END SUB
```

Pull a named group out of every match:

```
IMPORT regex
IMPORT collections
IMPORT io

SUB main()
  FOR EACH m IN regex::findAllMatches("2024-06 1999-12", "(?<year>\\d{4})-(?<month>\\d{2})")
    io::print(collections::get(m.groups, collections::get(m.names, "year")).text)
  NEXT
END SUB
```"##;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __regex_findAllMatches(value AS String, pattern AS String, start AS Integer) AS List OF MatchInfo
  LET prog AS __regex_Program = __regex_compile(pattern)
  LET ctx AS __regex_Ctx = __regex_makeCtx(value)
  IF start < 0 OR start > ctx.n THEN
    FAIL error(77050001, "List or string index/range is outside valid bounds.")
  END IF
  MUT out AS List OF MatchInfo = []
  FOR EACH r IN __regex_matchResults(prog, ctx, start)
    out = collections::append(out, __regex_makeMatch(r, value, prog))
  NEXT
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "findAllMatches",
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
                    desc: "The zero-based scalar index at or after which the first match must begin. Defaults to 0. Must be between 0 and the scalar length of value inclusive; start == len(value) is allowed and can match a zero-length or end-anchored pattern. May be passed by name.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Integer,
                        expr: "0",
                    },
                },
            ],
            return_type: ParameterType::list_of(ParameterType::named("MatchInfo")),
            errors: vec!["ErrInvalidFormat", "ErrIndexOutOfRange"],
            body: Body::mfb(FUNC_BODY, "__regex_findAllMatches"),
        }],
    });
}
