//! `money::setRounding` — install the `Money`-arithmetic rounding mode.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTRO: &str = r#"Set the rounding mode used by Money arithmetic on the calling thread"#;
const DESC: &str = r#"`money::setRounding` selects how `Money` arithmetic settles the exact half case.
`mode` is one of the two `Rounding` enum members: `Rounding.Commercial` (round
half **away from zero**, the default) or `Rounding.Banker` (round half to
**even**, which removes the small upward bias of always rounding ties away). The
call returns nothing.

The call is lowered inline to a mask and a single store into the
per-execution-context rounding-mode field in the arena state region. The stored
value is the enum discriminant masked to its low bit, so exactly `0` or `1` is ever
written and a later `money::getRounding` reads back the same member.

The mode is per-execution-context state. A worker thread inherits the spawning
thread's mode at spawn and then changes independently, so setting the mode on one
thread never disturbs another. There is no scoped or automatic restore: the mode
stays as you set it until it is set again, so a routine that changes the mode for
one calculation should read the previous value with `money::getRounding` and put it
back.

The mode applies to every `Money` **arithmetic** rounding site — `money::round`,
dividing a `Money` by a scalar, scaling a `Money` by a `Float` or `Fixed`, and the
`toMoney` / `toFixed` conversions — all of which route through the one shared
rounding helper. It has no bearing on `Fixed`/`Float` rounding, and it does not
change how `toString(Money)` renders a value (presentation rounding is a fixed
half-away-from-zero rule, deliberately independent of the mode).

The `Rounding` enum is referenced bare, like every other builtin type: write
`Rounding.Banker`, not `money::Rounding.Banker`."#;
const EX: &str = r#"Accumulate under banker's rounding, then restore the default:

```
IMPORT money
IMPORT io

SUB main
  money::setRounding(Rounding.Banker)
  io::print(toString(money::round(0.125m, 2)))
  money::setRounding(Rounding.Commercial)
  io::print(toString(money::round(0.125m, 2)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setRounding",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "mode",
                desc: "The mode to install for `Money` arithmetic on the calling thread: \
                       `Rounding.Commercial` or `Rounding.Banker`. Any other type is rejected \
                       at compile time.",
                aliases: &[],
                ty: ParameterType::named("Rounding"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_inline(lower_money_set_rounding),
        }],
    });
}

/// `money::setRounding(mode)` — store `mode & 1` into the arena rounding-mode field.
/// The `Rounding` value arrives as its i64 discriminant. Returns Nothing.
pub(crate) fn lower_money_set_rounding(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let mode = args[0].clone();
    let text = format!("money.setRounding({})", mode.text);
    let masked = builder.allocate_register()?;
    let one = builder.allocate_register()?;
    builder.emit(abi::move_immediate(one, "Integer", "1"));
    builder.emit(abi::and_registers(masked, &mode.location, one));
    builder.emit(abi::store_u64(
        masked,
        ARENA_STATE_REGISTER,
        ARENA_ROUNDING_MODE_OFFSET,
    ));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: abi::return_register(),
        text,
    })
}
