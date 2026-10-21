//! riscv64 as the debugger reads it: the register file the Linux backend reads
//! and writes ([`registers`]), and the rules the shared code asks of a
//! processor.
//!
//! Registers are named by their ABI names only (`a0`, `ra`, `s0`, `fa0`), as the
//! compiler and `objdump` name them; `x10` and `fp` name nothing.

mod registers;
pub(crate) use registers::*;

use crate::arch::riscv64::decode::{F, X};

/// Whether `word` names a riscv64 register an address can be in: `pc`, or an
/// integer register by its ABI name (`zero` aside).
fn is_register(word: &str) -> bool {
    word == "pc" || X[1..].contains(&word)
}

/// What `jump` sets, as its refusals say.
const JUMP_SETS: &str = "jump sets ra, sp, gp, tp, t0-t6, s0-s11, a0-a7 and fcsr";

/// Refuse a register `jump` does not set: it sets every integer register but
/// `zero`, and `fcsr`, and moves the `pc` itself.
fn settable(name: &str) -> Result<(), String> {
    if name == "fcsr" || X[1..].contains(&name) {
        return Ok(());
    }
    if name == "pc" {
        return Err(format!(
            "'pc' is where the thread goes: give it as the place, jump <where>; {JUMP_SETS}"
        ));
    }
    if name == "zero" {
        return Err(format!("'zero' is always 0; {JUMP_SETS}"));
    }
    if F.contains(&name) {
        return Err(format!(
            "'{name}' is a floating-point register; {JUMP_SETS}"
        ));
    }
    Err(format!("no register '{name}'; {JUMP_SETS}"))
}

/// Refuse a value register `name` cannot hold: `fcsr` holds 32 bits.
fn holds(name: &str, value: u64) -> Result<(), String> {
    if name == "fcsr" && value > u64::from(u32::MAX) {
        return Err(format!(
            "cannot set fcsr to 0x{value:016x}: it holds 32 bits"
        ));
    }
    Ok(())
}
