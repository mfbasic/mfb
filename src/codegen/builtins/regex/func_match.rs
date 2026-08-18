//! `regex::match` — descriptor entry + MFBASIC source body.
//!
//! Per-member file (planning/migrate.md). Source-backed
//! (`Implementation::Mfb`): the `__regex_*` body lives here and replaces a
//! `'@@MFB_BODY:match@@` marker in package.mfb via assembled_source (which
//! also appends the two generated Unicode tables). Body byte-significant
//! (2-space indent → .ncode columns); do not reformat.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether a regular expression matches anywhere in a string."#;

const DESC: &str = r#"`regex::match` compiles `pattern` as a regular expression and returns `TRUE`
when it matches anywhere in `value`, and `FALSE` otherwise. It is the existence
test of the package: it reports only whether some match exists, not where (see
`regex::find`) or what was matched (see `regex::replace`). A zero-length match
counts, so a pattern that can match the empty string — such as `a*`, an anchor
like `^`, or the empty pattern `""` — matches every value, including `""`.

The search is unanchored and leftmost: a match is sought at each position
`0`, `1`, … up to the scalar length of `value`, and the first position at which
the pattern can match makes `match` succeed. To require that the whole value
match, anchor with `\A` and `\z` (or with `^` and `$` when the `m` flag is off).
Matching operates over Unicode scalar values and is Unicode aware: the `(?i)`
flag uses Unicode simple case folding, and `\d`, `\w`, and `\s` use the pinned
Unicode definitions, so results are identical on every target and never defer to
a host regex library.

`pattern` uses MFBASIC's own portable regex dialect, defined in
`mfb spec stdlib regex` (run `mfb man regex` for the language overview). It is
an ordinary runtime `String`, so it may be built or read at run time; because
`String` literals process backslash escapes, a literal backslash must be written
`"\\"` — `regex::match(value, "\\d+")` tests for one or more digits. `match`
never reports absence as a failure; absence simply returns `FALSE`. Only an
invalid pattern fails, with `ErrInvalidFormat`. Because `match` takes no `start`
argument, it raises neither `ErrIndexOutOfRange` nor `ErrNotFound`.


`match` does not mutate `value` or `pattern` and has no side effects."#;

const EX: &str = r#"Test for a substring and an anchored pattern:

```
IMPORT regex

SUB main()
  LET hasEll AS Boolean = regex::match("hello", "ell")
  LET startsH AS Boolean = regex::match("hello", "^h")
END SUB
```

Case-insensitive match and a digit-class test (note the doubled backslash):

```
IMPORT regex

SUB main()
  LET greeting AS Boolean = regex::match("Hello", "(?i)hello")
  LET hasDigit AS Boolean = regex::match("abc123", "\\d+")
END SUB
```"#;

#[rustfmt::skip]
const FUNC_BODY: &str =
r#"FUNC __regex_match(value AS String, pattern AS String) AS Boolean
  LET prog AS __regex_Program = __regex_compile(pattern)
  LET ctx AS __regex_Ctx = __regex_makeCtx(value)
  LET r AS __regex_Result = __regex_searchFrom(prog, ctx, 0)
  RETURN r.ok
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "match",
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
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::mfb(FUNC_BODY, "__regex_match"),
        }],
    });
}
