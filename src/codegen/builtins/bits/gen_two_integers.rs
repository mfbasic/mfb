//! Shared two-operand helper for the binary / shift / rotate `bits` members.
//!
//! Consumed by `func_band`/`bor`/`bxor`, `func_sl`/`sr`/`sra`, and
//! `func_rl32`/`rr32`/`rl64`/`rr64`. Was a `CodeBuilder` method in the former
//! `src/target/shared/code/builder_bits.rs`; the `abi::` register operands are
//! passed by value (`VirtualRegister` is `Copy`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
/// Lower the two `Integer` operands of a binary `bits` op into fresh registers,
/// spilling the first across the second lowering so a temporary reset cannot
/// clobber it.
pub(crate) fn lower_bits_two_integers(
    builder: &mut CodeBuilder,
    function: &str,
    args: &[NirValue],
) -> Result<(VirtualRegister, VirtualRegister, String, String), String> {
    let left = builder.lower_value(&args[0])?;
    if left.type_ != "Integer" {
        return Err(format!("bits.{function} does not accept {}", left.type_));
    }
    let left_slot = builder.allocate_stack_object("bits_left", 8);
    builder.emit(abi::store_u64(
        &left.location,
        abi::stack_pointer(),
        left_slot,
    ));
    let right = builder.lower_value(&args[1])?;
    if right.type_ != "Integer" {
        return Err(format!("bits.{function} does not accept {}", right.type_));
    }
    let right_slot = builder.allocate_stack_object("bits_right", 8);
    builder.emit(abi::store_u64(
        &right.location,
        abi::stack_pointer(),
        right_slot,
    ));
    builder.reset_temporary_registers();
    let left_reg = builder.allocate_register()?;
    let right_reg = builder.allocate_register()?;
    builder.emit(abi::load_u64(left_reg, abi::stack_pointer(), left_slot));
    builder.emit(abi::load_u64(right_reg, abi::stack_pointer(), right_slot));
    Ok((left_reg, right_reg, left.text, right.text))
}
