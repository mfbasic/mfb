//! `money::convert` — move a `Money` from one rate to another with one rounding.

use super::RATE_TYPE;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Convert an amount between two currencies given each one's exact rate"#;

const DESC: &str = r#"`money::convert(value, from, to)` returns `value × from / to`: the amount
`value`, held in a currency whose rate is `from`, expressed in the currency whose
rate is `to`. Both rates are measured against the same base currency, so
`money::convert(1000000m, money::toRate("1.0842"), money::toRate("1"))` turns one
million at the rate 1.0842 into `1084200.00000` of the base currency.

The whole calculation is exact. The result is rounded once, to `Money`'s five places,
under the calling thread's rounding mode (see `money::setRounding`); nothing is
rounded on the way. That is why one `convert` can differ in the last place from
`money::scale(money::scale(value, from), …)`, which rounds twice.

The intermediate value is held at twice `Money`'s width, so a large amount and a
large rate do not overflow on the way: only a result outside the `Money` range does.

- A rate whose `scaled` is zero or negative raises `ErrInvalidArgument` (`77050002`). `money::toRate` never makes one, but a `money::Rate` built by hand can hold one.
- A result outside the `Money` range raises `ErrOverflow` (`77050010`)."#;

const EX: &str = r#"Convert euros at 1.0842 to US dollars, the base currency:

```
IMPORT io
IMPORT money

FUNC main() AS Nothing
  LET eur AS money::Rate = money::toRate("1.0842")
  LET usd AS money::Rate = money::toRate("1")
  io::print(toString(money::convert(1000000m, eur, usd)))
END FUNC
```

Ten billion US dollars to yen at a rate of 0.0067 dollars per yen:

```
IMPORT io
IMPORT money

FUNC main() AS Nothing
  LET usd AS money::Rate = money::toRate("1")
  LET jpy AS money::Rate = money::toRate("0.0067")
  io::print(toString(money::convert(10000000000m, usd, jpy), toByte(5)))
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "convert",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The amount to convert, in the currency whose rate is `from`.",
                    aliases: &[],
                    ty: ParameterType::Money,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "from",
                    desc: "The rate of `value`'s currency. Must be above zero.",
                    aliases: &[],
                    ty: ParameterType::named(RATE_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "to",
                    desc: "The rate of the currency to convert into. Must be above zero.",
                    aliases: &[],
                    ty: ParameterType::named(RATE_TYPE),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Money,
            errors: vec!["ErrInvalidArgument", "ErrOverflow"],
            body: Body::abi_inline(lower_money_convert),
        }],
    });
}

/// Load a `Rate` argument's `scaled` value — the word at offset 0 of its record
/// block, since `scaled` is the record's only field and holds an `Integer` inline —
/// and fail with ErrInvalidArgument unless it is above zero. Every `money` member
/// that computes with a `Rate` checks it here, on each call (plan-181-A's
/// check-on-use rule: an exported field cannot stop a caller building
/// `Rate[scaled := 0]`).
pub(super) fn emit_positive_rate_scaled(
    builder: &mut CodeBuilder,
    record: &Operand,
) -> Result<VirtualRegister, String> {
    let scaled = builder.allocate_register();
    builder.emit(abi::load_u64(scaled, record, 0));
    let positive = builder.label("money_rate_positive");
    builder.emit(abi::compare_immediate(scaled, "0"));
    builder.emit(abi::branch_gt(&positive));
    builder.raise_error_bare("ErrInvalidArgument")?;
    builder.emit(abi::label(&positive));
    Ok(scaled)
}

/// `money::convert(value, from, to)` — `R(value.raw × from.scaled / to.scaled)`
/// through `emit_money_rescale`. The three arguments arrive reloaded into
/// registers (`lower_abi_inline_args` spills and reloads a multi-argument call), and
/// nothing below calls a runtime helper before the rescale reads them.
pub(crate) fn lower_money_convert(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let text = format!(
        "money.convert({}, {}, {})",
        args[0].text, args[1].text, args[2].text
    );
    let from = emit_positive_rate_scaled(builder, &args[1].location)?;
    let to = emit_positive_rate_scaled(builder, &args[2].location)?;
    let result = builder.allocate_register();
    builder.emit_money_rescale(result, &args[0].location, from, to)?;
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Money,
        location: Operand::from(result),
        text,
    })
}
