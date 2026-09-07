//! `regex::split` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__regex_*` body lives here and replaces a
//! `'@@MFB_BODY:split@@` marker in package.mfb via assembled_source (which
//! also appends the two generated Unicode tables). Body byte-significant
//! (2-space indent → .ncode columns); do not reformat.
//!
//! One pass. The walk is `__regex_matchResults`, the same helper
//! `__regex_findAll`, `__regex_findAllMatches`, `__regex_count` and
//! `__regex_replace` consume, so the match SEQUENCE — including the zero-width
//! rule — is inherited rather than restated, and the engine is never re-run per
//! piece. The body is `__regex_replace`'s loop with the replacement expansion
//! dropped: emit `strings::mid(value, cursor, mstart - cursor)` per match, set
//! `cursor` to the match end, then emit the tail.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Split a string into a list of substrings around every regular-expression match."#;

const DESC: &str = r#"`regex::split` compiles `pattern` as a regular expression, scans `value` left to
right for every non-overlapping match, and returns the text between the matches
as a `List OF String`. The matched text itself is removed from the output. It is
the pattern mirror of `strings::split`, which breaks on a literal delimiter, and
it is the member to reach for when the separator is a *run* — one-or-more
whitespace, a set of alternatives, an optional trailing comma — which a literal
delimiter cannot express.

The pieces are the pieces `strings::split` would produce for the same matches,
and the same counting rule holds: **the result always contains exactly one more
element than the number of matches found, and is therefore never empty.**
Everything else follows from that rule:

- A `pattern` that does not match yields a single-element list holding `value`
  unchanged.
- A match at position `0` yields a leading empty element; a match ending at the
  end of `value` yields a trailing empty element.
- Two adjacent matches yield an empty element between them.
- Splitting the empty string yields a single-element list holding `""`.

Empty pieces are kept, never dropped, so the input can be reconstructed:
joining the result of a `regex::split` puts back everything except the matched
separators. A caller who wants the empty pieces gone filters them, which is a
decision `split` cannot make for you — a leading empty element is the difference
between `",a"` and `"a"`.

The matches removed are exactly the matches `regex::findAll` reports, found by
the same leftmost, unanchored scan, in the same order, so
`len(regex::split(value, pattern))` is `regex::count(value, pattern) + 1`. After
each match the scan resumes just past the end of that match, so matches never
overlap and no scalar is dropped twice. A zero-length match is a separator like
any other, and the scan then advances by one scalar to make progress; a
zero-length match is never taken twice at a position the previous match already
covered. So splitting `"abc"` on `"x*"` — which matches zero-width at every
position — yields the five elements `""`, `"a"`, `"b"`, `"c"`, `""`, one more
than the four matches, and terminates.

`pattern` must not be empty. An empty pattern is refused with
`ErrInvalidArgument` before any scanning occurs, exactly as `strings::split`
refuses an empty delimiter: an empty pattern occurs at every position, and
cutting at every position is not a division of the text — see `mfb man strings`
for the rule and `mfb man regex` for how `regex` reads it. This is a guard on the
empty pattern *string* and nothing else: a pattern such as `"a*"`, `"x?"` or
`"(?:)"` that merely *matches* zero-width text is an ordinary pattern and splits
as described above.

Positions are Unicode scalar values, never UTF-8 bytes and never grapheme
clusters, consistent with `len` and the `strings` package, so a piece can never
begin or end inside a multi-byte scalar.

`pattern` is an ordinary runtime `String`, so it may be built or read at run
time; it uses MFBASIC's own portable regex dialect, defined in
`mfb spec stdlib regex` (run `mfb man regex` for the language overview), which
produces identical results on every target and never defers to a host regex
library. Because `String` literals process backslash escapes, a literal
backslash is written `"\\"` — `regex::split(value, "\\s+")` tokenizes on runs of
whitespace. An invalid pattern fails with `ErrInvalidFormat`.

There is no `limit` parameter, because `strings::split` has none either; every
piece is always returned.

`split` does not mutate `value` or `pattern` and has no side effects. The
returned list and its elements are their own values.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"Tokenize on runs of whitespace — the split a literal delimiter cannot do,
because it cannot collapse a run:

```
IMPORT io
IMPORT regex

FUNC main() AS Integer
  FOR EACH word IN regex::split("the   quick  brown", "\\s+")
    io::print(word)
  NEXT
  RETURN 0
END FUNC
```

Split on any one of several separators, and see that empty pieces are kept:

```
IMPORT io
IMPORT regex
IMPORT collections

FUNC main() AS Integer
  LET parts AS List OF String = regex::split("a,b;c", ",|;")
  io::print(toString(len(parts)))
  io::print(collections::get(parts, 2))
  io::print(toString(len(regex::split(";a;;", ";"))))
  RETURN 0
END FUNC
```

A pattern that never matches returns the whole value as one element:

```
IMPORT io
IMPORT regex
IMPORT collections

FUNC main() AS Integer
  LET parts AS List OF String = regex::split("abc", "\\d+")
  io::print(toString(len(parts)))
  io::print(collections::get(parts, 0))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __regex_split(value AS String, pattern AS String) AS List OF String
  IF len(pattern) = 0 THEN
    FAIL error(77050002, "Argument value is not valid for the requested operation.")
  END IF
  LET prog AS __regex_Program = __regex_compile(pattern)
  LET ctx AS __regex_Ctx = __regex_makeCtx(value)
  MUT out AS List OF String = []
  MUT cursor AS Integer = 0
  FOR EACH r IN __regex_matchResults(prog, ctx, 0)
    LET mstart AS Integer = collections::get(r.caps, 0)
    out = collections::append(out, strings::mid(value, cursor, mstart - cursor))
    cursor = r.pos
  NEXT
  out = collections::append(out, strings::mid(value, cursor, ctx.n - cursor))
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "split",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The text to divide. Any `String` is accepted, including the empty string. It is never modified.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "pattern",
                    desc: "The regular expression matching each separator. It must be a valid pattern in the MFBASIC regex dialect; otherwise the call fails with ErrInvalidFormat. It must also be non-empty: an empty pattern is refused with ErrInvalidArgument, which does not affect patterns such as \"a*\" that match zero-width.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::String),
            errors: vec!["ErrInvalidFormat", "ErrInvalidArgument"],
            body: Body::mfb(FUNC_BODY, "__regex_split"),
        }],
    });
}
