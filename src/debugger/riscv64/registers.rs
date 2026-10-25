//! The riscv64 register file as the Linux backend reads and writes it.
//!
//! `NT_PRSTATUS` is `struct user_regs_struct`: 32 words, `pc` then `x1`–`x31`
//! (`ra`, `sp`, `gp`, `tp`, `t0`–`t2`, `s0`, `s1`, `a0`–`a7`, `s2`–`s11`,
//! `t3`–`t6`), 256 bytes. `NT_PRFPREG` is `struct __riscv_d_ext_state`: `f0`–`f31`
//! and `fcsr`, 264 bytes with its padding. Registers are named by their ABI
//! names only, as the compiler and `objdump` name them. [`registers`] turns them
//! into the display list and [`assign`] writes one named value back.

use crate::arch::riscv64::decode::{F, X};
use crate::debugger::{Register, RegisterGroup, RegisterValue};

/// `pc` and `x1`–`x31`, in the kernel's order.
pub(crate) type General = [u64; 32];

/// The floating-point registers and their control and status register.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Float {
    pub(crate) f: [u64; 32],
    pub(crate) fcsr: u32,
}

impl Float {
    pub(crate) fn zeroed() -> Float {
        Float {
            f: [0; 32],
            fcsr: 0,
        }
    }
}

/// The index of `pc` in [`General`]; `x<n>` is at index `n`.
const PC: usize = 0;
/// The index of `sp` (`x2`).
const SP: usize = 2;

/// Every register of a thread, in display order: the general-purpose
/// registers `ra` to `t6` (`x1`–`x31` without `sp`), then the system ones
/// (`sp`, `pc`, `fcsr`), then the floating-point ones. `zero` is not listed.
pub(crate) fn registers(general: &General, float: &Float) -> Vec<Register> {
    let x = |name: &str, group: RegisterGroup, value: u64| Register {
        role: role(name),
        name: name.to_string(),
        group,
        value: RegisterValue::Bits64(value),
    };
    let mut registers = (1..32)
        .filter(|index| *index != SP)
        .map(|index| x(X[index], RegisterGroup::General, general[index]))
        .collect::<Vec<_>>();
    registers.push(x("sp", RegisterGroup::System, general[SP]));
    registers.push(x("pc", RegisterGroup::System, general[PC]));
    registers.push(x("fcsr", RegisterGroup::System, float.fcsr.into()));
    registers.extend(float.f.iter().enumerate().map(|(index, value)| Register {
        name: F[index].to_string(),
        group: RegisterGroup::Simd,
        value: RegisterValue::Bits64(*value),
        role: None,
    }));
    registers
}

/// A register's [`Register::role`], in the words AArch64's use:
/// `a0`–`a7` carry arguments under both MFBASIC's convention and C's, the low
/// ones carry results (four for MFBASIC, two for C), and MFBASIC code keeps
/// three registers for itself. `None` for every other register.
pub(super) fn role(name: &str) -> Option<&'static str> {
    Some(match name {
        "a0" | "a1" => "MFB arg, MFB result, C arg, C result",
        "a2" | "a3" => "MFB arg, MFB result, C arg",
        "a4" | "a5" | "a6" | "a7" => "MFB arg, C arg",
        "s11" => "Arena",
        "s2" => "Current Thread",
        "s10" => "Closure",
        _ => return None,
    })
}

/// Where a register `jump` sets lives.
enum Slot {
    /// A word of [`General`].
    General(usize),
    Fcsr,
}

/// The slot of the register named `name`: `pc`, a general-purpose register
/// (`sp` included) or `fcsr`.
fn slot(name: &str) -> Result<Slot, String> {
    if name == "pc" {
        return Ok(Slot::General(PC));
    }
    if name == "fcsr" {
        return Ok(Slot::Fcsr);
    }
    X.iter()
        .position(|register| *register == name)
        .filter(|index| *index != 0)
        .map(Slot::General)
        .ok_or_else(|| format!("no register '{name}' to set"))
}

/// Write `value` into register `name`: a general word, or the 32-bit `fcsr`,
/// read through `float` the first time it is needed.
pub(crate) fn assign(
    general: &mut General,
    float: &mut Option<Float>,
    read_float: &mut dyn FnMut() -> Result<Float, String>,
    name: &str,
    value: u64,
) -> Result<(), String> {
    match slot(name)? {
        Slot::General(index) => general[index] = value,
        Slot::Fcsr => {
            if float.is_none() {
                *float = Some(read_float()?);
            }
            let state = float.as_mut().expect("read above");
            state.fcsr = u32::try_from(value)
                .map_err(|_| format!("fcsr holds 32 bits, not 0x{value:016x}"))?;
        }
    }
    Ok(())
}

/// The value of `pc` in a [`General`].
pub(crate) fn pc(general: &General) -> u64 {
    general[PC]
}

/// The value of `sp` in a [`General`].
pub(crate) fn sp(general: &General) -> u64 {
    general[SP]
}

#[cfg(test)]
mod tests;
