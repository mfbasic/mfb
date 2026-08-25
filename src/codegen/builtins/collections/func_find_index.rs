//! `collections::findIndex` — descriptor entry + generic MFBASIC source body.
//!
//! A source-generic member in the csv/json `func_*.rs` shape: the registered
//! descriptor carries the docs and the generic `__collections_findIndex` body
//! (`Body::Mfb`), which renders into the injected source and is instantiated by
//! the monomorphizer per call site. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

const INTRO: &str =
    r#"Index of the first element at or after a start position that satisfies a predicate"#;

const DESC: &str = r#"`collections::findIndex` scans `value` **forward**, beginning at index `start`
and advancing by one, calling `predicate` with each element. It returns the
zero-based index of the first element for which `predicate` returns `TRUE`. The
scan short-circuits at that element: no later element is examined. When the scan
reaches the end of the list without a match, the call raises `ErrNotFound`
(`77050004`) rather than returning a sentinel index.

`start` defaults to `0`, so the common call form scans the whole list. It is
validated **before** any element is read: the call raises `ErrIndexOutOfRange`
(`77050001`) when `start < 0` or `start > len(value)`. Two consequences are
worth stating precisely:

- `start` equal to `len(value)` is **legal**. It selects an empty scan, so the
  call raises `ErrNotFound`, not `ErrIndexOutOfRange`. `start` strictly greater
  than `len(value)` is the out-of-range case.
- A negative `start` is **not** interpreted as an offset from the end of the
  list. It is simply out of range and raises `ErrIndexOutOfRange`. This is
  deliberately asymmetric with `collections::findLastIndex`, whose `endIndex`
  parameter *does* resolve negative values from the end.

On an empty list every legal `start` is `0`, which is `len(value)`, so
`findIndex` on an empty list raises `ErrNotFound`.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` propagates out of the `collections::findIndex` call to the
caller rather than being reported as a non-match. Note that a lambda passed here
may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `findIndex`.

`findIndex` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_findIndex` generic and instantiated for the element
type like any other generic function. It
does not mutate `value`.

`T` is inferred from the element type of `value` and may be any type;
`findIndex` imposes no comparability or orderability constraint on `T`, because
elements are never compared to one another — they are only passed to
`predicate`. The second argument must be a function value taking exactly one `T`
and returning `Boolean`, and `start`, when supplied, must be an `Integer`."#;

const EX: &str = r#"Find the first positive element:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::findIndex([-1, 0, 3, 4], isPos)))
  RETURN 0
END FUNC
```

Resume the scan past an earlier match by passing `start`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET nums AS List OF Integer = [5, -1, 7, -2]
  LET first AS Integer = collections::findIndex(nums, isPos)
  io::print(toString(collections::findIndex(nums, isPos, first + 1)))
  RETURN 0
END FUNC
```

Handle the no-match case with a function-level `TRAP`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC firstPositive(nums AS List OF Integer) AS Integer
  RETURN collections::findIndex(nums, isPos)

  TRAP(e)
    RETURN -1
  END TRAP
END FUNC

FUNC main AS Integer
  io::print(toString(firstPositive([-3, -2])))
  RETURN 0
END FUNC
```

The third parameter is named `start`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::findIndex([5, -1, 7], isPos, start := 1)))
  RETURN 0
END FUNC
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __collections_findIndex OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean, start AS Integer = 0) AS Integer
  IF start < 0 OR start > len(value) THEN
    FAIL error(77050001, "List or string index/range is outside valid bounds.")
  END IF
  MUT i AS Integer = start
  WHILE i < len(value)
    IF predicate(collections::get(value, i)) THEN
      RETURN i
    END IF
    i = i + 1
  END WHILE
  FAIL error(77050004, "Requested item, key, file, or resource was not found.")
END FUNC"#;

pub(crate) fn register(pkg: &mut crate::codegen::registry::RegistryPackage) {
    use crate::codegen::registry::{
        Body, DefaultValue, Implementation, Parameter, RegistryFunction,
    };
    use crate::types::ParameterType;

    pkg.add_function(RegistryFunction {
        name: "findIndex",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF T, FUNC(T) AS Boolean, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to scan. Not modified.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "predicate",
                    desc: "Test applied to each element from `start` upward; the scan stops at the first call returning `TRUE`. An error it raises propagates to the caller.",
                    aliases: &[],
                    ty: ParameterType::func(vec![ParameterType::var("T")], ParameterType::Boolean),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "start",
                    desc: "Zero-based index at which the forward scan begins. Optional, default `0`. Must satisfy `0 <= start <= len(value)`; a negative value is out of range, not an offset from the end.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill { type_name: ParameterType::Integer, expr: "0" },
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],
            body: Body::mfb(BODY, "__collections_findIndex"),
        }],
    });
}
