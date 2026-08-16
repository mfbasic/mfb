//! `money::round` — settle a `Money` to a given number of decimal places.
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/money/round.md`; lowering
//! from the former
//! `src/target/shared/code/builder_money.rs::lower_money_round`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::{CodeBuilder, Operand, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str =
    r#"Settle a Money to a given number of decimal places under the current rounding mode"#;
const DESC: &str = r#"`money::round` settles `value` to `decimals` fractional places and returns the
result, still as a `Money`. It is the explicit "compute at five places, book at
two" operation: intermediate `Money` arithmetic keeps all five decimal places that
the type carries, and `money::round` is what settles a line item or an allocation
remainder to whole cents (`decimals` `2`) or another scale when it is time to
record it.

The computation is exact integer arithmetic on the underlying scaled value, with no
floating point anywhere: the raw is divided by `10^(5 - decimals)`, the remainder is
settled through the shared rounding helper, and the quotient is multiplied back to
`Money` scale.

How the remainder settles depends on the mode installed by `money::setRounding`. A
remainder that is not an exact half always goes to the nearer value, under either
mode. At an exact half, `Rounding.Commercial` (the default) rounds away from zero
and `Rounding.Banker` rounds to even — that is, it increments only when the
truncated quotient is odd. Negative amounts round symmetrically: the magnitude is
settled and the sign reapplied, so `money::round(-0.125m, 2)` under
`Rounding.Commercial` is `-0.13`.

`decimals` must be in `0` through `5` inclusive; anything outside that range fails
with `ErrInvalidArgument`. The bounds are not arbitrary: `Money` is scaled to
exactly five decimal places, so `decimals` `5` is the identity and `decimals` `0`
settles to whole currency units — while remaining a `Money`, not an `Integer`.

Rounding can push a near-maximum `Money` past the representable range, because
settling upward returns a quotient one larger before it is scaled back. That
multiply is checked rather than allowed to wrap into a negative amount, so such a
call fails with `ErrOverflow` instead of returning a silently wrong figure.

`money::round` is distinct from `toString(Money)` (presentation rounding, a fixed
half-away-from-zero rule that ignores the current mode) and from `math::round(Money)`
(which leaves the `Money` dimension entirely, yielding the dimensionless whole-unit
`Integer` count); `money::round(value, 0)` is the version that stays money."#;
const EX: &str = r#"Book a taxed line item to whole cents:

```
IMPORT money
IMPORT io

SUB main
  LET price AS Money = 19.99m
  LET line AS Money = price * 1.0825F
  LET booked AS Money = money::round(line, 2)
  io::print(toString(booked))
END SUB
```

The same tie settles differently under each mode:

```
IMPORT money
IMPORT io

SUB main
  money::setRounding(Rounding.Commercial)
  io::print(toString(money::round(0.125m, 2)))
  money::setRounding(Rounding.Banker)
  io::print(toString(money::round(0.125m, 2)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "round",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The amount to settle. Any `Money` is accepted, including zero and \
                           negative amounts.",
                    aliases: &[],
                    ty: ParameterType::Money,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "decimals",
                    desc: "The number of fractional decimal places to keep. Must be `0` through \
                           `5` inclusive; `5` is the identity and `0` settles to whole currency \
                           units.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Money,
            errors: vec!["ErrInvalidArgument", "ErrOverflow"],
            body: Body::native(None, None, Some(lower_money_round)),
        }],
    });
}

/// `money::round(value, decimals)` — settle `value` to `decimals` places under the
/// current mode (plan-29-D §4.4). `decimals` outside `0..5` fails with
/// ErrInvalidArgument; `5` is the identity. Exact integer arithmetic: divide by
/// `10^(5-decimals)`, round the remainder through `emit_apply_rounding`, then
/// re-multiply (which cannot overflow — the product stays within one divisor of the
/// original raw).
pub(crate) fn lower_money_round(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let value = builder.lower_value(&args[0])?;
    // Spill the Money raw before lowering `decimals`: that lowering may emit a
    // `_mfb_*` helper call which clobbers every caller-saved register (the
    // register-lifetime model), destroying `value.location`. Mirror the spill every
    // math sibling (lower_math_min_max/clamp/scalar_binary) already does (bug-200).
    let raw_slot = builder.allocate_stack_object("money_round_raw", 8);
    builder.emit(abi::store_u64(
        &value.location,
        abi::stack_pointer(),
        raw_slot,
    ));
    let decimals = builder.lower_value(&args[1])?;
    let text = format!("money.round({}, {})", value.text, decimals.text);
    let raw = builder.allocate_register()?;
    builder.emit(abi::load_u64(raw, abi::stack_pointer(), raw_slot));
    let dec = decimals.location;

    // decimals must be in 0..=5.
    let lo_ok = builder.label("money_round_lo_ok");
    builder.emit(abi::compare_immediate(&dec, "0"));
    builder.emit(abi::branch_ge(&lo_ok));
    builder.raise_error_bare("ErrInvalidArgument")?;
    builder.emit(abi::label(&lo_ok));
    let hi_ok = builder.label("money_round_hi_ok");
    builder.emit(abi::compare_immediate(&dec, "5"));
    builder.emit(abi::branch_le(&hi_ok));
    builder.raise_error_bare("ErrInvalidArgument")?;
    builder.emit(abi::label(&hi_ok));

    // divisor = 10^(5 - decimals), built by a bounded (<=5) multiply loop.
    let exponent = builder.allocate_register()?;
    builder.emit(abi::move_immediate(exponent, "Integer", "5"));
    builder.emit(abi::subtract_registers(exponent, exponent, &dec));
    let divisor = builder.allocate_register()?;
    builder.emit(abi::move_immediate(divisor, "Integer", "1"));
    let ten = builder.allocate_register()?;
    builder.emit(abi::move_immediate(ten, "Integer", "10"));
    let loop_label = builder.label("money_round_pow_loop");
    let loop_done = builder.label("money_round_pow_done");
    builder.emit(abi::label(&loop_label));
    builder.emit(abi::compare_immediate(exponent, "0"));
    builder.emit(abi::branch_eq(&loop_done));
    builder.emit(abi::multiply_registers(divisor, divisor, ten));
    builder.emit(abi::subtract_immediate(exponent, exponent, 1));
    builder.emit(abi::branch(&loop_label));
    builder.emit(abi::label(&loop_done));

    // q = raw / divisor, r = raw - q*divisor, sign_neg = raw < 0.
    let quotient = builder.allocate_register()?;
    builder.emit(abi::signed_divide_registers(quotient, raw, divisor));
    let remainder = builder.allocate_register()?;
    builder.emit(abi::multiply_subtract_registers(
        remainder, quotient, divisor, raw,
    ));
    let sign_neg = builder.allocate_register()?;
    builder.emit(abi::arithmetic_shift_right_immediate(sign_neg, raw, 63));
    let rounded = builder.allocate_register()?;
    builder.emit_apply_rounding(rounded, quotient, remainder, divisor, sign_neg)?;
    // result = rounded * divisor (back to Money scale). `emit_apply_rounding` can
    // return `q+1`, so for a near-max Money `(q+1)*divisor` can exceed i64::MAX — trap
    // ErrOverflow rather than wrapping to a negative Money (bug-175 A).
    let result = builder.allocate_register()?;
    builder.emit_checked_integer_multiply(result, rounded, divisor)?;
    Ok(ValueResult {
        type_: "Money".to_string(),
        location: Operand::from(result.render()),
        text,
    })
}
