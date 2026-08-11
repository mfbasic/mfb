//! `collections::all` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "Test whether every element of a list satisfies a predicate";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_all OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean
  MUT i AS Integer = 0
  WHILE i < len(value)
    IF NOT predicate(collections::get(value, i)) THEN
      RETURN FALSE
    END IF
    i = i + 1
  END WHILE
  RETURN TRUE
END FUNC";

const DESC: &str = r#"`collections::all` walks `value` from index `0` upward and calls `predicate`
with each element in turn. It returns `FALSE` as soon as a call returns `FALSE`,
without examining any later element, and returns `TRUE` only after every element
has been tested and all matched.

The scan short-circuits: `predicate` is called at most once per element, and no
call is made for elements after the first non-matching one. Callers must not
rely on `predicate` being invoked for the whole list.

For an empty list `all` returns `TRUE`, the vacuous result: there is no element
that fails the test. This is the dual of `collections::any`, which returns
`FALSE` for an empty list.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` is **not** absorbed by `all`: it propagates out of the
`collections::all` call to the caller, where a function-level or inline `TRAP`
may catch it. `all` itself defines no error of its own. Note that a lambda
passed here may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `all`.

`all` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_all` generic and instantiated for the element type like
any other generic function. It does not
mutate `value` and has no other side effects beyond whatever `predicate` does."#;

const EX: &str = r#"Test that every integer in a list is positive:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::all([1, 2, 3], isPos)))
  RETURN 0
END FUNC
```

An empty list satisfies every predicate:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::all(empty, isPos)))
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
  io::print(toString(collections::all(value := [1, 0], predicate := isPos)))
  RETURN 0
END FUNC
```"#;

pub(crate) const ALL: BuiltinFunction = BuiltinFunction::mfb(
    "collections.all",
    "all",
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
