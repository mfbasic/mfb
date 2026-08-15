//! `regex::find` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__regex_*` body lives here and replaces a
//! `'@@MFB_BODY:find@@` marker in package.mfb via assembled_source (which
//! also appends the two generated Unicode tables). Body byte-significant
//! (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, ParameterType, RegistryFunction,
    RegistryPackage,
};

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
digit. An invalid pattern fails with `ErrInvalidFormat`; when no match exists at
or after `start`, `find` returns `-1` rather than failing. Because every real
match position is `>= 0`, `-1` is an unambiguous "no match" sentinel. (This
differs from `strings::find`, which fails with `ErrNotFound` on absence.)

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

Handle absence with the `-1` sentinel:

```
IMPORT regex
IMPORT io

SUB main()
  LET i AS Integer = regex::find("abc", "\\d")
  IF i >= 0 THEN
    io::print("matched at " & toString(i))
  ELSE
    io::print("no match")
  END IF
END SUB
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
    RETURN -1
  END IF
  RETURN collections::get(r.caps, 0)
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "find",
        intro: INTRO,
        desc: DESC,
        example: EX,
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
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::mfb(FUNC_BODY, "__regex_find"),
        }],
    });
}
