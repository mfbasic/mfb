//! `regex::find` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__regex_*` body lives here and replaces a
//! `'@@MFB_BODY:find@@` marker in package.mfb via assembled_source (which
//! also appends the two generated Unicode tables). Body byte-significant
//! (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Locate the first regular-expression match and return its start index."#;

const DESC: &str = r#"`regex::find` compiles `pattern` as a regular expression, searches `value` for
the first match beginning at or after the position `start`, and returns the
zero-based index where that match starts. It is the locating form of the
package: `regex::match` reports only whether a match exists, `find` reports
where the first one begins, and `regex::findAll` reports the start of every
non-overlapping match.

The search is unanchored and leftmost. A match is sought at each position
`start`, `start+1`, … in turn, and the smallest position at which the pattern
can match is reported; at that position the engine resolves the match by
preference order (earlier alternatives, greedy quantifiers as long as possible,
lazy ones as short as possible), but only the start index is returned. `start`
restricts only where a match may begin; it does not redefine the input, so the
absolute anchors `\A` and `\z`, and `^` and `$` when the `m` flag is off, are
still evaluated against the whole value. For example `regex::find("abc", "^b", 1)`
finds nothing, because `^` is absolute position `0`. A zero-length match is valid
and reports its own start position; an empty or empty-matching pattern matches
immediately at `start`.

Positions are Unicode scalar values, never UTF-8 bytes and never grapheme
clusters, consistent with `len` and the `strings` package. A string of `n`
scalars has positions `0` … `n`; position `n` is after the last scalar. Both the
`start` argument and the returned index are scalar indexes.

`start` defaults to `0`, meaning the search begins at the start of `value`. It
must be in the range `0` through the scalar length of `value` inclusive; the
upper bound equals the length so that a search may begin at the end of the
string (where only a zero-length or end-anchored pattern can match). A negative
`start`, or one greater than the scalar length, is out of range and fails with
`ErrIndexOutOfRange`.

`pattern` is an ordinary runtime `String`, so it may be built or read at run
time; it uses MFBASIC's own portable regex dialect, defined in
`mfb spec stdlib regex` (run `mfb man regex` for the language overview), which
produces identical results on every target and never defers to a host regex
library. Because `String` literals process backslash escapes, a literal
backslash is written `"\\"` — `regex::find(value, "\\d")` searches for the first
digit. An invalid pattern fails with `ErrInvalidFormat`.

When no match exists at or after `start`, `find` raises `ErrNotFound`
(`77050004`). It never returns a sentinel such as `-1`: every value an `Integer`
can hold is a position some other search could legitimately report, so an index
has no spare value that could mean "absent". This is the same contract
`strings::find`, `collections::find`, `collections::findIndex` and
`collections::findLastIndex` use, so moving a search between a literal needle and
a pattern does not change how absence behaves.

Absence is an ordinary outcome for a pattern search, so guard it. `regex::match`
is that guard — it answers the same question with a `Boolean` and never fails on
absence — and `regex::findMatch` is the other route, returning a `MatchInfo`
whose `start` is `-1` when nothing matched. To get an index and a sentinel in one
call, wrap `find` in a `TRAP`:

```
FUNC findOrMinusOne(v AS String, p AS String) AS Integer
  RETURN regex::find(v, p)
TRAP(err)
  RETURN -1
END TRAP
END FUNC
```

Only `find` changes shape on absence. `regex::match` still returns `FALSE`,
`regex::findAll` still returns an empty list, `regex::replace` still returns
`value` unchanged, and `regex::findMatch`/`regex::findAllMatches` still report a
no-match `MatchInfo` and an empty list — each of those return types already has a
value that means "no match", which is exactly what an index does not.

`find` reports only where the match begins. Because a pattern's match *length* is
an output — the caller cannot know it in advance the way it knows `len(needle)`
for a literal — a start index alone cannot be sliced. When the matched text, the
end index, or a capture group is wanted, use `regex::findMatch`, which performs
this same search and returns all of it; `find` is the cheaper call when only the
position is needed.

`find` does not mutate `value` or `pattern` and has no side effects."#;

const EX: &str = r#"Find the first occurrence, and the first at or after a start position:

```
IMPORT regex

SUB main()
  LET firstL AS Integer = regex::find("hello", "l")
  LET nextL AS Integer = regex::find("hello", "l", 3)
END SUB
```

Find the first digit (note the doubled backslash in the String literal):

```
IMPORT regex

SUB main()
  LET firstDigit AS Integer = regex::find("a1b2c3", "\\d")
END SUB
```

Absence raises, so guard with `regex::match` or catch it:

```
IMPORT regex
IMPORT io

SUB main()
  IF regex::match("abc", "\\d") THEN
    io::print("matched at " & toString(regex::find("abc", "\\d")))
  ELSE
    io::print("no match")
  END IF
END SUB
```

The same thing written as a `TRAP`, when the search should not run twice:

```
IMPORT regex
IMPORT io

FUNC main() AS Integer
  io::print("matched at " & toString(regex::find("abc", "\\d")))
  RETURN 0
TRAP(err)
  io::print("no match")
  RETURN 0
END TRAP
END FUNC
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __regex_find(value AS String, pattern AS String, start AS Integer) AS Integer
  LET prog AS __regex_Program = __regex_compile(pattern)
  LET ctx AS __regex_Ctx = __regex_makeCtx(value)
  IF start < 0 OR start > ctx.n THEN
    FAIL error(77050001, "List or string index/range is outside valid bounds.")
  END IF
  LET r AS __regex_Result = __regex_searchFrom(prog, ctx, start)
  IF r.ok = FALSE THEN
    FAIL error(77050004, "Requested item, key, file, or resource was not found.")
  END IF
  RETURN collections::get(r.caps, 0)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "find",
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
            return_type: ParameterType::Integer,
            errors: vec!["ErrInvalidFormat", "ErrIndexOutOfRange", "ErrNotFound"],
            body: Body::mfb(FUNC_BODY, "__regex_find"),
        }],
    });
}
