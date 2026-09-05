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
Consequently an empty-matching pattern inserts the replacement before each scalar
and once at the end: `regex::replace("abc", "a*", "-")` is `"-b-c-"` and
`regex::replace("abc", "(?:)", "-")` is `"-a-b-c-"`.

`replace` refuses an *empty* `pattern`, raising `ErrInvalidArgument`. **This is a
guard on the empty pattern string, not a change to zero-width matching** —
`"a*"`, `"x?"` and `"(?:)"` still match at every position, and
`regex::replace(value, "a*", "-")` still interleaves. The guard exists because a
pattern usually arrives at run time, from a configuration value, a form field or
a `--replace` flag, and an empty one is a normal accident: rewriting the whole
subject is the most destructive answer available for an argument the caller did
not mean to supply, and returning `value` unchanged would report success for a
call that did nothing. `strings::replace` refuses an empty `old` with the same
code, so routing a run-time value to either member gives the same outcome, and
`strings::count` and `strings::split` already refused it before either.

The refusal is only on the rewriting side. `regex::match`, `regex::find`,
`regex::findAll`, `regex::findMatch` and `regex::findAllMatches` all still accept
an empty `pattern` and answer with its zero-width match: `regex::find(v, "")` is
`0`, exactly as `strings::find(v, "")` is.


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
invalid pattern fails with `ErrInvalidFormat`; an *empty* pattern is not
malformed, it is a well-formed pattern this member declines, so it fails with
`ErrInvalidArgument` instead. When `pattern` is valid and matches nothing in
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
```

A zero-width pattern still interleaves; only the *empty* pattern is refused:

```
IMPORT io
IMPORT regex
IMPORT strings

FUNC main() AS Integer
  io::print(regex::replace("abc", "a*", "-"))
  io::print(regex::replace("abc", "(?:)", "-"))
  LET pattern AS String = strings::mid("configured", 0, 0)
  io::print(regex::replace("abc", pattern, "-"))
  RETURN 0
TRAP(err)
  io::print("no pattern was supplied")
  RETURN 0
END TRAP
END FUNC
```"##;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __regex_replace(value AS String, pattern AS String, replacement AS String) AS String
  IF len(pattern) = 0 THEN
    FAIL error(77050002, "Argument value is not valid for the requested operation.")
  END IF
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
                    desc: "The regular expression to compile and search for. It must be a valid pattern in the MFBASIC regex dialect; otherwise the call fails with ErrInvalidFormat. It must also be non-empty: an empty pattern is refused with ErrInvalidArgument, which does not affect patterns such as \"a*\" that match zero-width.",
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
            errors: vec!["ErrInvalidFormat", "ErrInvalidArgument"],
            body: Body::mfb(FUNC_BODY, "__regex_replace"),
        }],
    });
}
