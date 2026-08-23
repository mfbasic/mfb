//! `money::getRounding` — read the `Money`-arithmetic rounding mode in effect.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTRO: &str = r#"Read the rounding mode currently in effect for Money arithmetic"#;
const DESC: &str = r#"`money::getRounding` returns the `Money` arithmetic rounding mode currently in
effect, as a `Rounding` value. It takes no arguments and always succeeds.

The mode is not a call into a runtime helper — it is lowered inline to a single
load of the per-execution-context rounding-mode field held in the arena state
region, so reading it is as cheap as reading a local. The stored value is exactly
the enum discriminant: `0` for `Rounding.Commercial`, `1` for `Rounding.Banker`,
and only those two values are ever stored, because `money::setRounding` masks its
argument to the low bit before writing.

The mode is per-execution-context state, so `getRounding` reports the mode of the
thread that calls it: the value most recently written by `money::setRounding` on
this thread, or — if this thread has never set it — the mode it inherited from its
spawning thread. A program that has never called `money::setRounding` observes
`Rounding.Commercial`, the default.

The returned mode governs `Money` **arithmetic** rounding only. It does not
describe how `toString(Money)` renders a value — presentation rounding is a fixed
half-away-from-zero rule that ignores the mode entirely.

The `Rounding` enum is referenced bare, like every other builtin type: write
`Rounding.Banker`, not `money::Rounding.Banker`."#;
const EX: &str = r#"Branch on the mode currently in effect:

```
IMPORT money
IMPORT io

SUB main
  IF money::getRounding() = Rounding.Banker THEN
    io::print("banker's rounding is active")
  END IF
END SUB
```

Save the mode, switch it for one calculation, then restore what was there before:

```
IMPORT money
IMPORT io

SUB main
  LET previous AS Rounding = money::getRounding()
  money::setRounding(Rounding.Banker)
  io::print(toString(money::round(0.125m, 2)))
  money::setRounding(previous)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getRounding",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Named("Rounding"),
            errors: vec![],
            body: Body::abi_inline(lower_money_get_rounding),
        }],
    });
}

/// `money::getRounding()` — load the arena rounding-mode field (`0`/`1`) as a
/// `Rounding` value (the enum is i64-carried by its discriminant).
pub(crate) fn lower_money_get_rounding(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let result = builder.allocate_register()?;
    builder.emit(abi::load_u64(
        result,
        ARENA_STATE_REGISTER,
        ARENA_ROUNDING_MODE_OFFSET,
    ));
    Ok(ValueResult {
        origin: None,
        type_: "Rounding".to_string(),
        location: Operand::from(result.render()),
        text: "money.getRounding()".to_string(),
    })
}
