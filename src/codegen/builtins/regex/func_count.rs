//! `regex::count` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__regex_*` body lives here and replaces a
//! `'@@MFB_BODY:count@@` marker in package.mfb via assembled_source (which
//! also appends the two generated Unicode tables). Body byte-significant
//! (2-space indent → .ncode columns); do not reformat.
//!
//! The walk is `__regex_matchResults`, the same helper `__regex_findAll`,
//! `__regex_findAllMatches`, `__regex_split` and `__regex_replace` consume, so
//! the match SEQUENCE — including the zero-width rule — is shared code rather
//! than a second copy that could drift.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Count the non-overlapping regular-expression matches in a string."#;

const DESC: &str = r#"`regex::count` compiles `pattern` as a regular expression, scans `value` for
every non-overlapping match beginning at or after the position `start`, and
returns how many it found. It is the pattern mirror of `strings::count`, which
counts occurrences of a literal substring, and it reports exactly
`len(regex::findAll(value, pattern, start))` — the same scan, the same matches,
in the same order — without building the list of indices.

The matches counted are the matches `regex::findAll` reports. The search is
leftmost and unanchored; after each match the scan resumes just past the end of
that match, so matches never overlap. A zero-length match is counted, and the
scan then advances by one scalar to make progress; a zero-length match is never
counted twice at a position the previous match already covered. So counting
`"a*"` in `"aba"` reports the two real runs rather than an empty match wedged
between them, and counting `"x*"` in `"abc"` reports `4` — one zero-length match
at each of the four positions `0` … `3`. When there is no match the result is
`0`; the call never fails for want of a match.

`pattern` must not be empty. An empty pattern is refused with
`ErrInvalidArgument` before any scanning occurs, exactly as `strings::count`
refuses an empty needle and as `regex::replace` refuses an empty pattern: an
empty pattern is present at every position, and "every position" is not a count
worth reporting — see `mfb man strings` for the rule and `mfb man regex` for how
`regex` reads it. This is a guard on the empty pattern *string* and nothing else:
a pattern such as `"a*"`, `"x?"` or `"(?:)"` that merely *matches* zero-width text
is an ordinary pattern and is counted as described above. `regex::findAll`
answers for an empty pattern rather than refusing it, so `count` and
`len(findAll(...))` agree on every pattern except that one — the same split
`strings::find` and `strings::count` already have.

`start` restricts only where the first counted match may begin; it does not
redefine the input, so the absolute anchors `\A` and `\z`, and `^` and `$` when
the `m` flag is off, are still evaluated against the whole value. Positions are
Unicode scalar values, never UTF-8 bytes and never grapheme clusters, consistent
with `len` and the `strings` package. A string of `n` scalars has positions `0`
… `n`; position `n` is after the last scalar.

`start` defaults to `0`, meaning the scan begins at the start of `value`. It must
be in the range `0` through the scalar length of `value` inclusive; the upper
bound equals the length so that the scan may begin at the end of the string
(where only a zero-length or end-anchored pattern can match). A negative `start`,
or one greater than the scalar length, is out of range and fails with
`ErrIndexOutOfRange`.

`pattern` is an ordinary runtime `String`, so it may be built or read at run
time; it uses MFBASIC's own portable regex dialect, defined in
`mfb spec stdlib regex` (run `mfb man regex` for the language overview), which
produces identical results on every target and never defers to a host regex
library. Because `String` literals process backslash escapes, a literal
backslash is written `"\\"` — `regex::count(value, "\\d+")` counts the runs of
digits. An invalid pattern fails with `ErrInvalidFormat`. Pattern compilation is
checked before `start`, so `ErrInvalidFormat` takes precedence when both apply.

`count` does not mutate `value` or `pattern` and has no side effects.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"Count the runs of digits (note the doubled backslash in the String literal):

```
IMPORT io
IMPORT regex

FUNC main() AS Integer
  io::print(toString(regex::count("a1b22c333", "\\d+")))
  io::print(toString(regex::count("xyz", "\\d+")))
  RETURN 0
END FUNC
```

Matches never overlap, and the count agrees with `regex::findAll`:

```
IMPORT io
IMPORT regex

FUNC main() AS Integer
  io::print(toString(regex::count("aaaa", "aa")))
  io::print(toString(len(regex::findAll("aaaa", "aa"))))
  RETURN 0
END FUNC
```

Count only the tail of the string by passing an explicit start:

```
IMPORT io
IMPORT regex

FUNC main() AS Integer
  io::print(toString(regex::count("a1b2c3", "\\d", 3)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __regex_count(value AS String, pattern AS String, start AS Integer) AS Integer
  IF len(pattern) = 0 THEN
    FAIL error(77050002, "Argument value is not valid for the requested operation.")
  END IF
  LET prog AS __regex_Program = __regex_compile(pattern)
  LET ctx AS __regex_Ctx = __regex_makeCtx(value)
  IF start < 0 OR start > ctx.n THEN
    FAIL error(77050001, "List or string index/range is outside valid bounds.")
  END IF
  RETURN len(__regex_matchResults(prog, ctx, start))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "count",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The subject text searched for matches. It is never modified.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "pattern",
                    desc: "The regular expression to compile and count. It must be a valid pattern in the MFBASIC regex dialect; otherwise the call fails with ErrInvalidFormat. It must also be non-empty: an empty pattern is refused with ErrInvalidArgument, which does not affect patterns such as \"a*\" that match zero-width.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "start",
                    desc: "The zero-based scalar index at or after which the first counted match must begin. Defaults to 0. Must be between 0 and the scalar length of value inclusive; start == len(value) is allowed and can match a zero-length or end-anchored pattern. May be passed by name.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Integer,
                        expr: "0",
                    },
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec![
                "ErrInvalidFormat",
                "ErrInvalidArgument",
                "ErrIndexOutOfRange",
            ],
            body: Body::mfb(FUNC_BODY, "__regex_count"),
        }],
    });
}
