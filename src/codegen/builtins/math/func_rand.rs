//! `math::rand` — uniform inclusive random draw from this thread's PCG64 generator.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType::{Integer, Money};

use super::{overload, req};
const INTRO: &str =
    r#"A uniform random value in an inclusive range, from this thread's generator."#;
const DESC: &str = r#"`rand(min, max)` returns a uniformly-distributed value in the inclusive range
`[min, max]`, drawn from this thread's PCG64 generator (seeded with `math::seed`).
The `(Integer, Integer)` form returns `Integer`; the `(Money, Money)` form returns
`Money` (a uniform amount between two amounts is itself an amount). `min` must not
exceed `max`, else `ErrInvalidArgument`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::rand(1, 6)))
END SUB
```"#;

const MIN_A: &[&str] = &["minimum"];
const MAX_A: &[&str] = &["maximum"];

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let impls: Vec<Implementation> = vec![
        overload(
            vec![req("min", MIN_A, Integer), req("max", MAX_A, Integer)],
            Integer,
            vec!["ErrInvalidArgument"],
            lower_math_rand,
        ),
        overload(
            vec![req("min", MIN_A, Money), req("max", MAX_A, Money)],
            Money,
            vec!["ErrInvalidArgument"],
            lower_math_rand,
        ),
    ];
    pkg.add_function(RegistryFunction {
        name: "rand",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer min, Integer max (or Money, Money)"),
        internal_only: false,
        implementations: impls,
    });
}

/// Target-generic call-site lowering for `math::rand`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_rand(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("rand", args)
}
