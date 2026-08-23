//! `math::seed` — reseed this thread's PCG64 random generator.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Integer, Nothing};

use super::{overload, req};
const INTRO: &str = r#"Reseed this thread's random generator."#;
const DESC: &str = r#"`seed(value)` reseeds this thread's PCG64 generator so a subsequent sequence of
`math::rand` draws is reproducible. It returns Nothing. Seeding is per-execution
context: a worker thread inherits the spawning thread's stream and then diverges
independently."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  math::seed(42)
  io::print(toString(math::rand(1, 6)))
END SUB
```"#;

const SEED_A: &[&str] = &["seed"];

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let impls: Vec<Implementation> = vec![overload(
        vec![req("value", SEED_A, Integer)],
        Nothing,
        vec![],
        lower_math_seed,
    )];
    pkg.add_function(RegistryFunction {
        name: "seed",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer"),
        internal_only: false,
        implementations: impls,
    });
}

/// Target-generic call-site lowering for `math::seed`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_seed(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("seed", args)
}
