//! `strings.endsWithAny` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_with_any;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether a string ends with any of several suffixes."#;

const DESC: &str = r#"`strings::endsWithAny` returns `TRUE` when `value` ends with at least one of the
strings in `suffixes`, and `FALSE` otherwise. Candidates are tested in list order
and the scan stops at the first match; which candidate matched is not reported,
only that one did.

Each individual test is the same exact byte comparison `strings::endsWith`
performs: the trailing bytes of `value` must equal every byte of the candidate,
in order, with no normalization and no case folding. A candidate longer than
`value` cannot match and is skipped rather than treated as an error.

An empty string appearing as a candidate matches everything, so a `suffixes` list
containing `""` makes the result `TRUE` for any `value`. An empty `suffixes` list
has no candidates and returns `FALSE`. Neither `value` nor the list is modified,
and the call is total — it never fails.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"Test a filename against several image extensions:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET images AS List OF String = [".png", ".jpg", ".gif"]
  io::print(toString(strings::endsWithAny("photo.png", images)))
  RETURN 0
END FUNC
```

An empty candidate list never matches:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET none AS List OF String = []
  io::print(toString(strings::endsWithAny("anything", none)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.endsWithAny: no native lowering for these arguments".to_string());
    }
    gen_with_any::lower_strings_with_any(builder, &args[0], &args[1], true)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "endsWithAny",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string whose trailing bytes are examined. May be empty.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "suffixes",
                    desc: "The candidate suffixes, tested in list order. May be empty, in which case the result is `FALSE`. Entries may themselves be empty; an empty entry always matches.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::String),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
