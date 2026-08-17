//! `collections::reduce` — descriptor entry + target-generic lowering (plan-96).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTO_REDUCE: &str = "Fold a list left to right into a single accumulated value";
const DESC_REDUCE: &str = r#"`collections::reduce` folds `value` into one value. The accumulator starts as
`initial`. The list is walked from the first element to the last, and for each
element the reducer is called as `f(accumulator, element)` — **accumulator
first, element second** — with its return value becoming the accumulator for the
next step. The accumulator left after the final element is the result. It is a
**native** member: the compiler emits the fold loop directly rather than
instantiating an MFBASIC generic.

The fold direction is left, from index 0 upward: the loop starts at the head of
the entry table and advances one entry per step. For a right-to-left fold, use
`collections::reduceRight`.

The accumulator type `U` is fixed by `initial`. `f`'s first parameter type, its
success type, and the type of `initial` must all be that same `U`, while `f`'s
second parameter must be the list element type `T`. `U` may differ from `T`, so
a `List OF String` can be folded into an `Integer`.

When `value` is empty, the loop body never runs, `f` is never called, and
`initial` is returned unchanged.

`value` is not modified. Unlike the other three callback members, `reduce`
deliberately does not free the per-element item it materializes for the
callback, because the reducer is allowed to return that item itself as the new
accumulator — freeing it would turn a leak into a use-after-free. Intermediate
accumulators are likewise left unfreed.

`reduce` raises no domain error of its own. It is classified fallible solely
because a failing `f` propagates: when the reducer returns a non-`Ok` result,
the fold stops immediately at that element, later elements are never visited,
and the reducer's own error is passed through unchanged. No cleanup runs on that
path, since the accumulator may still alias the borrowed `initial`.

An inline `TRAP` on a `reduce` call captures that propagated reducer error at
the call site rather than letting it auto-propagate."#;

const EX: &str = r#"Sum a list with an explicit reducer:

```
IMPORT collections
IMPORT io

FUNC add(total AS Integer, value AS Integer) AS Integer
  RETURN total + value
END FUNC

FUNC main AS Integer
  LET total AS Integer = collections::reduce([1, 2, 3], 10, add)
  io::print(toString(total))
  RETURN 0
END FUNC
```

Fold a `List OF String` into a single `String`, showing that `U` need not equal
`T`'s usual result:

```
IMPORT collections
IMPORT io

FUNC join(text AS String, word AS String) AS String
  RETURN text & word
END FUNC

FUNC main AS Integer
  LET joined AS String = collections::reduce(["hello", "world"], "", join)
  io::print(joined)
  RETURN 0
END FUNC
```

An empty list returns `initial` without calling the reducer:

```
IMPORT collections
IMPORT io

FUNC add(total AS Integer, value AS Integer) AS Integer
  RETURN total + value
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::reduce(empty, 7, add)))
  RETURN 0
END FUNC
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "reduce",
        intro: INTO_REDUCE,
        desc: DESC_REDUCE,
        example: EX,
        expected_arguments: Some("List OF T, U, FUNC(U, T) AS U"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &["collection"],
                    ty: ParameterType::list_of(ParameterType::Var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "initial",
                    desc: "",
                    aliases: &["seed"],
                    ty: ParameterType::Var("U"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "f",
                    desc: "",
                    aliases: &["combine"],
                    ty: ParameterType::func(
                        vec![ParameterType::Var("U"), ParameterType::Var("T")],
                        ParameterType::Var("U"),
                    ),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Arg(1),
            errors: vec![],
            body: Body::native(None, None, Some(lower_reduce)),
        }],
    });
}

/// `collections::reduce(List OF T, U, FUNC(U, T) AS U) AS U`: left fold. The
/// shared fold machinery, walked head-to-tail.
pub(crate) fn lower_reduce(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_collection_reduce_impl(args, false)
}
