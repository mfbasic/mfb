//! `collections::any` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Test whether at least one element of a list satisfies a predicate";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_any OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean
  MUT i AS Integer = 0
  WHILE i < len(value)
    IF predicate(collections::get(value, i)) THEN
      RETURN TRUE
    END IF
    i = i + 1
  END WHILE
  RETURN FALSE
END FUNC";

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
non-escaping is `collections::forEach`, not `any`.

`any` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_any` generic and instantiated for the element type like
any other generic function. It does not
mutate `value` and has no other side effects beyond whatever `predicate` does."#;

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

pub(crate) const ANY: BuiltinFunction = BuiltinFunction::mfb(
    "collections.any",
    "any",
    INTRO,
    DESC,
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("predicate", &[], "FUNC(T) AS Boolean"),
    ])],
    BODY,
)
.with_example(EX);
