//! Shared single-operand helper for the unary `bits` members.
//!
//! Consumed by `func_bnot`, `func_clz`/`ctz`, `func_pop_count`, and
//! `func_bswap16`/`bswap32`/`bswap64`. Was a `CodeBuilder` method in the former
//! `src/target/shared/code/builder_bits.rs`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::target::shared::nir::NirValue;
/// Lower a single `bits.*` argument and require it to be `Integer`, returning the
/// lowered value or the shared `does not accept` diagnostic (bug-332 G5).
pub(crate) fn lower_bits_one_integer(
    builder: &mut CodeBuilder,
    function: &str,
    arg: &NirValue,
) -> Result<ValueResult, String> {
    let value = builder.lower_value(arg)?;
    if value.type_ != "Integer" {
        return Err(format!("bits.{function} does not accept {}", value.type_));
    }
    Ok(value)
}
