//! `collections::any` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_any` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str = r#"Test whether at least one element of a list satisfies a predicate"#;

const DESC: &str = r#"`collections::any` walks `value` from index `0` upward and calls `predicate`
with each element in turn. It returns `TRUE` as soon as a call returns `TRUE`,
without examining any later element, and returns `FALSE` only after every
element has been tested and none matched.

The scan short-circuits: `predicate` is called at most once per element, and no
call is made for elements after the first match. Callers must not rely on
`predicate` being invoked for the whole list.

For an empty list `any` returns `FALSE`, since there is no element that could
match. This is the dual of `collections::all`, which returns `TRUE` for an
empty list.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` is **not** absorbed by `any`: it propagates out of the
`collections::any` call to the caller, where a function-level or inline `TRAP`
may catch it. `any` itself defines no error of its own. Note that a lambda
passed here may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `any`.It does not
mutate `value` and has no other side effects beyond whatever `predicate` does.

`T` is inferred from the element type of `value` and may be any type; `any`
imposes no comparability or orderability constraint on `T`, because elements are
never compared to one another — they are only passed to `predicate`. The second
argument must be a function value taking exactly one `T` and returning
`Boolean`."#;

const EX: &str = r#"Test a list of integers for a positive element:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::any([-1, 0, 3], isPos)))
  RETURN 0
END FUNC
```

An empty list never matches:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::any(empty, isPos)))
  RETURN 0
END FUNC
```

Named arguments bind by the declared parameter names `value` and `predicate`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::any(value := [-1, 2], predicate := isPos)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __collections_any OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean
  MUT i AS Integer = 0
  WHILE i < len(value)
    IF predicate(collections::get(value, i)) THEN
      RETURN TRUE
    END IF
    i = i + 1
  END WHILE
  RETURN FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "any",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF T, FUNC(T) AS Boolean"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to scan, in index order starting at `0`. An empty list is accepted and yields `FALSE`. Not modified.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "predicate",
                    desc: "Test applied to each element. Called with one element at a time; the scan stops at the first call that returns `TRUE`. An error it raises propagates to the caller.",
                    aliases: &[],
                    ty: ParameterType::func(vec![ParameterType::var("T")], ParameterType::Boolean),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::mfb(BODY, "__collections_any"),
        }],
    });
}
