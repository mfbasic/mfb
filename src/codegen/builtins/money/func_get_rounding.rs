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
effect, as a `money::Rounding` value. It takes no arguments and always succeeds.

Reading the mode is as cheap as reading a local variable, so there is no reason
to cache it. The answer is always one of the two `money::Rounding` members — never an
unset or out-of-range value.

The mode is per-thread, so `getRounding` reports the mode of the
thread that calls it: the value most recently written by `money::setRounding` on
this thread, or — if this thread has never set it — the mode it inherited from its
spawning thread. A program that has never called `money::setRounding` observes
`money::Rounding.Commercial`, the default.

The returned mode governs `Money` **arithmetic** rounding only. It does not
describe how `toString(Money)` renders a value — presentation rounding is a fixed
half-away-from-zero rule that ignores the mode entirely.

The `money::Rounding` enum is referenced bare, like every other builtin type: write
`money::Rounding.Banker`, not `money::Rounding.Banker`."#;
const EX: &str = r#"Branch on the mode currently in effect:

```
IMPORT money
IMPORT io

SUB main
  IF money::getRounding() = money::Rounding.Banker THEN
    io::print("banker's rounding is active")
  END IF
END SUB
```

Save the mode, switch it for one calculation, then restore what was there before:

```
IMPORT money
IMPORT io

SUB main
  LET previous AS money::Rounding = money::getRounding()
  money::setRounding(money::Rounding.Banker)
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
            return_type: ParameterType::named("Rounding"),
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
    let result = builder.allocate_register();
    builder.emit(abi::load_u64(
        result,
        ARENA_STATE_REGISTER,
        ARENA_ROUNDING_MODE_OFFSET,
    ));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::named("Rounding"),
        location: Operand::from(result.render()),
        text: "money.getRounding()".to_string(),
    })
}
