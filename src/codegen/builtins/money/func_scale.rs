//! `money::scale` — multiply a `Money` by an exact `Rate` with one rounding.

use super::func_convert::emit_positive_rate_scaled;
use super::{RATE_SCALE, RATE_TYPE};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Multiply an amount by an exact rate, rounding once"#;

const DESC: &str = r#"`money::scale(value, rate)` returns `value × rate`, such as a tax or a fee
given as a rate: `money::scale(100m, money::toRate("0.07"))` is `7.00000`.

The product is exact until the end, then rounded once to `Money`'s five places under
the calling thread's rounding mode (see `money::setRounding`). Because the rate is a
`money::Rate`, it is the exact decimal written, unlike scaling by a `Fixed` or
`Float`, which are binary.

A `money::Rate` is above zero, so `scale` multiplies by a positive factor only. To
take a percentage off, scale by the part that remains (`money::toRate("0.93")` for
7% off) or subtract the scaled amount.

- A rate whose `scaled` is zero or negative raises `ErrInvalidArgument` (`77050002`). `money::toRate` never makes one, but a `money::Rate` built by hand can hold one.
- A result outside the `Money` range raises `ErrOverflow` (`77050010`)."#;

const EX: &str = r#"Add 7% tax to a price:

```
IMPORT io
IMPORT money

FUNC main() AS Nothing
  LET price AS Money = 19.99m
  LET tax AS Money = money::scale(price, money::toRate("0.07"))
  io::print(toString(tax, toByte(5)))
  io::print(toString(price + tax))
END FUNC
```

The exact half settles under the rounding mode:

```
IMPORT io
IMPORT money

FUNC main() AS Nothing
  LET half AS money::Rate = money::toRate("0.5")
  money::setRounding(money::Rounding.Commercial)
  io::print(toString(money::scale(0.00001m, half), toByte(5)))
  money::setRounding(money::Rounding.Banker)
  io::print(toString(money::scale(0.00001m, half), toByte(5)))
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "scale",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The amount to multiply.",
                    aliases: &[],
                    ty: ParameterType::Money,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "rate",
                    desc: "The factor. Must be above zero.",
                    aliases: &[],
                    ty: ParameterType::named(RATE_TYPE),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Money,
            errors: vec!["ErrInvalidArgument", "ErrOverflow"],
            body: Body::abi_inline(lower_money_scale),
        }],
    });
}

/// `money::scale(value, rate)` — `R(value.raw × rate.scaled / 10^12)` through
/// `emit_money_rescale`, with the divisor the constant `RATE_SCALE`.
pub(crate) fn lower_money_scale(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let text = format!("money.scale({}, {})", args[0].text, args[1].text);
    let scaled = emit_positive_rate_scaled(builder, &args[1].location)?;
    let divisor = builder.allocate_register();
    builder.emit(abi::move_immediate(
        divisor,
        "Integer",
        &RATE_SCALE.to_string(),
    ));
    let result = builder.allocate_register();
    builder.emit_money_rescale(result, &args[0].location, scaled, divisor)?;
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Money,
        location: Operand::from(result),
        text,
    })
}
