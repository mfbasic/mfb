//! The AArch64 register file as both backends read and write it.
//!
//! macOS's `ARM_THREAD_STATE64` and Linux's `NT_PRSTATUS` (`user_pt_regs`) are
//! the same 34 words — `x0`–`x28`, `fp`, `lr`, `sp`, `pc`, then the processor
//! state (CPSR / `pstate`) — and macOS's `ARM_NEON_STATE64` and Linux's
//! `NT_FPREGSET` (`user_fpsimd_state`) the same 528 bytes: `v0`–`v31`, `fpsr`,
//! `fpcr`, padding. [`registers`] turns them into the display list and
//! [`assign`] writes one named value back into them.

use super::{Register, RegisterGroup, RegisterValue};

/// The general registers, `sp`, `pc` and the processor state, as words.
pub(crate) type General = [u64; 34];

/// The bits of the processor state the `nzcv` register shows: the N, Z, C and
/// V flags.
pub(crate) const NZCV_BITS: u64 = 0xf000_0000;

/// The SIMD and floating-point registers.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Float {
    pub(crate) v: [u128; 32],
    pub(crate) fpsr: u32,
    pub(crate) fpcr: u32,
}

impl Float {
    pub(crate) fn zeroed() -> Float {
        Float {
            v: [0; 32],
            fpsr: 0,
            fpcr: 0,
        }
    }
}

/// Every register of a thread, in display order: the general-purpose
/// registers, then the system ones, then the SIMD ones. NZCV is the top four
/// bits of the processor state, shown where the `NZCV` system register holds
/// them.
pub(crate) fn registers(general: &General, float: &Float) -> Vec<Register> {
    let x = |name: String, group: RegisterGroup, value: u64| Register {
        role: role(&name),
        name,
        group,
        value: RegisterValue::Bits64(value),
    };
    let mut registers = (0..29)
        .map(|index| x(format!("x{index}"), RegisterGroup::General, general[index]))
        .collect::<Vec<_>>();
    registers.push(x("fp".to_string(), RegisterGroup::General, general[29]));
    registers.push(x("lr".to_string(), RegisterGroup::General, general[30]));
    registers.push(x("sp".to_string(), RegisterGroup::System, general[31]));
    registers.push(x("pc".to_string(), RegisterGroup::System, general[32]));
    registers.push(x(
        "nzcv".to_string(),
        RegisterGroup::System,
        general[33] & NZCV_BITS,
    ));
    registers.push(x(
        "fpsr".to_string(),
        RegisterGroup::System,
        float.fpsr.into(),
    ));
    registers.push(x(
        "fpcr".to_string(),
        RegisterGroup::System,
        float.fpcr.into(),
    ));
    registers.extend(float.v.iter().enumerate().map(|(index, value)| Register {
        name: format!("v{index}"),
        group: RegisterGroup::Simd,
        value: RegisterValue::Bits128(*value),
        role: None,
    }));
    registers
}

/// A register's [`Register::role`]: `x0`–`x7` carry arguments under both MFBASIC's convention
/// and C's, the low ones carry results (four for MFBASIC, two for C), and
/// MFBASIC code keeps three registers for itself. `None` for every other
/// register.
fn role(name: &str) -> Option<&'static str> {
    Some(match name {
        "x0" | "x1" => "MFB arg, MFB result, C arg, C result",
        "x2" | "x3" => "MFB arg, MFB result, C arg",
        "x4" | "x5" | "x6" | "x7" => "MFB arg, C arg",
        "x19" => "Arena",
        "x20" => "Current Thread",
        "x28" => "Closure",
        _ => return None,
    })
}

/// Where a register `jump` sets lives.
pub(crate) enum Slot {
    /// A word of [`General`].
    General(usize),
    /// The flag bits of the processor state.
    Flags,
    Fpsr,
    Fpcr,
}

/// The slot of the register named `name`, as [`registers`] names it: a
/// general-purpose register, `sp`, `pc`, `nzcv`, `fpsr` or `fpcr`.
pub(crate) fn slot(name: &str) -> Result<Slot, String> {
    Ok(match name {
        "fp" => Slot::General(29),
        "lr" => Slot::General(30),
        "sp" => Slot::General(31),
        "pc" => Slot::General(32),
        "nzcv" => Slot::Flags,
        "fpsr" => Slot::Fpsr,
        "fpcr" => Slot::Fpcr,
        other => Slot::General(
            other
                .strip_prefix('x')
                .and_then(|number| number.parse::<usize>().ok())
                .filter(|number| *number < 29)
                .ok_or_else(|| format!("no register '{other}' to set"))?,
        ),
    })
}

/// Write `value` into register `name`: a general word, the flags (CPSR's bits
/// 31–28, the rest of the word kept), or a 32-bit floating-point control word,
/// read through `float` the first time one is needed.
pub(crate) fn assign(
    general: &mut General,
    float: &mut Option<Float>,
    read_float: &mut dyn FnMut() -> Result<Float, String>,
    name: &str,
    value: u64,
) -> Result<(), String> {
    match slot(name)? {
        Slot::General(index) => general[index] = value,
        Slot::Flags => general[33] = (general[33] & !NZCV_BITS) | (value & NZCV_BITS),
        Slot::Fpsr | Slot::Fpcr => {
            if float.is_none() {
                *float = Some(read_float()?);
            }
            let state = float.as_mut().expect("read above");
            let word = u32::try_from(value)
                .map_err(|_| format!("{name} holds 32 bits, not 0x{value:016x}"))?;
            if name == "fpsr" {
                state.fpsr = word;
            } else {
                state.fpcr = word;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    use crate::codegen::engine::operand::{abi_token, AbiConvention, AbiRole};
    use crate::target::shared::abi::realize_abi_positional;
    use crate::target::shared::regmodel::RegisterModel;

    /// The roles `list reg` prints are the compiler's own: each convention's
    /// last argument and result register carries that role, and the pinned
    /// registers are the ones the register model reserves.
    #[test]
    fn roles_name_the_registers_the_compiler_uses() {
        let last = |convention, role, index: u8, label: &str| {
            // The token exists at `index`, so the convention reaches that far.
            abi_token(convention, role, index);
            let register = realize_abi_positional(index);
            assert!(
                super::role(register).is_some_and(|roles| roles.split(", ").any(|r| r == label)),
                "{register} lacks '{label}'"
            );
            let next = format!("x{}", index + 1);
            assert!(
                !super::role(&next).is_some_and(|roles| roles.split(", ").any(|r| r == label)),
                "{next} has '{label}'"
            );
        };
        last(AbiConvention::Mfb, AbiRole::Arg, 7, "MFB arg");
        last(AbiConvention::Mfb, AbiRole::Ret, 3, "MFB result");
        last(AbiConvention::C, AbiRole::Arg, 7, "C arg");
        last(AbiConvention::C, AbiRole::Ret, 1, "C result");
        let model = Aarch64RegisterModel;
        assert_eq!(super::role(model.arena_base()), Some("Arena"));
        assert_eq!(super::role(model.current_thread()), Some("Current Thread"));
        assert_eq!(super::role(model.closure_env()), Some("Closure"));
        let named = registers(&[0; 34], &Float::zeroed())
            .iter()
            .filter(|register| register.role.is_some())
            .map(|register| register.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            named,
            ["x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7", "x19", "x20", "x28"]
        );
    }

    #[test]
    fn the_float_state_is_the_528_bytes_both_kernels_use() {
        assert_eq!(std::mem::size_of::<Float>(), 528);
        assert_eq!(std::mem::size_of::<General>(), 34 * 8);
    }

    #[test]
    fn assigning_keeps_the_rest_of_the_processor_state() {
        let mut general = [0u64; 34];
        general[33] = 0x8000_03c5;
        let mut float = None;
        let reads = std::cell::Cell::new(0);
        let mut read = || {
            reads.set(reads.get() + 1);
            Ok(Float::zeroed())
        };
        for (name, value) in [
            ("x3", 7),
            ("lr", 0x40),
            ("nzcv", 0x6000_0000),
            ("fpcr", 1),
            ("fpsr", 2),
        ] {
            assign(&mut general, &mut float, &mut read, name, value).unwrap();
        }
        assert_eq!(
            (general[3], general[30], general[33]),
            (7, 0x40, 0x6000_03c5)
        );
        assert_eq!(float.map(|state| (state.fpsr, state.fpcr)), Some((2, 1)));
        assert_eq!(reads.get(), 1);
        assert_eq!(
            assign(&mut general, &mut float, &mut read, "x29", 0).err(),
            Some("no register 'x29' to set".to_string())
        );
        assert_eq!(
            assign(&mut general, &mut float, &mut read, "fpsr", 1 << 32).err(),
            Some("fpsr holds 32 bits, not 0x0000000100000000".to_string())
        );
    }
}
