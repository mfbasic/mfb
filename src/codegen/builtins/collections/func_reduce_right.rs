//! `collections::reduceRight` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
const INTO_REDUCE_RIGHT: &str =
    "Fold a list into a single value, walking from the last item to the first";
const DESC_REDUCE_RIGHT: &str = r#"`collections::reduceRight` folds `value` into a single accumulated result. The
accumulator starts at `initial`. The function walks the list from the last index
down to index 0, and at each step replaces the accumulator with
`f(accumulator, item)`. When the walk finishes, the accumulator is returned.

The accumulator is the **first** argument of `f` and the list item is the second
— the same argument order `collections::reduce` uses. Only the traversal
direction differs between the two: `reduce` moves from the first item to the
last, `reduceRight` from the last to the first. `f` is therefore declared as
`FUNC(U, T) AS U`, not `FUNC(T, U) AS U`.

For a three-item list `[x, y, z]`, the result is
`f(f(f(initial, z), y), x)`. Direction matters whenever `f` is not associative
and commutative: folding `[1, 2, 3]` from the right with subtraction and an
initial accumulator of `0` yields `((0 - 3) - 2) - 1`, or `-6`.

`f` is called exactly once per item, so an empty `value` calls `f` not at all and
returns `initial` unchanged. `value` is not modified.

The accumulator type `U` need not match the element type `T`; `reduceRight` can
fold a list into a value of an entirely different type, such as building a
`String` from a `List OF Integer`.

`f` is an ordinary MFBASIC function value invoked with an ordinary call. If it
fails at any step, its error propagates out of `reduceRight` to the caller and
can be caught by the caller's `TRAP` block; the partially accumulated value is
discarded. `reduceRight` itself raises no error of its own.

`f` may be a named `FUNC` or a `LAMBDA` expression, since both produce a function
value of the required type."#;

const EX: &str = r#"Subtract each item from an accumulator, right to left:

```
IMPORT io
IMPORT collections

FUNC subtract(acc AS Integer, n AS Integer) AS Integer
  RETURN acc - n
END FUNC

FUNC main AS Integer
  LET total AS Integer = collections::reduceRight([1, 2, 3], 0, subtract)
  io::print(toString(total))
  RETURN 0
END FUNC
```

Fold into a different type — build a reversed `String` from a `List OF String`:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET words AS List OF String = ["a", "b", "c"]
  LET joined AS String = collections::reduceRight(value := words, initial := "", f := LAMBDA(acc AS String, w AS String) -> acc & w)
  io::print(joined)
  RETURN 0
END FUNC
```

An empty list returns `initial` untouched:

```
IMPORT io
IMPORT collections

FUNC subtract(acc AS Integer, n AS Integer) AS Integer
  RETURN acc - n
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::reduceRight(empty, 42, subtract)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "reduceRight",
        intro: INTO_REDUCE_RIGHT,
        desc: DESC_REDUCE_RIGHT,
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
            body: Body::abi_inline_self(lower_reduce_right),
        }],
    });
}

/// `collections::reduceRight(List OF T, U, FUNC(U, T) AS U) AS U`: right fold.
/// The shared fold machinery, walked tail-to-head.
pub(crate) fn lower_reduce_right(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_collection_reduce_impl(args, true)
}
