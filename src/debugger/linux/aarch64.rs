//! The AArch64 parts of the Linux backend, which `linux/` reads as `cpu::`:
//! the register sets and where `pc`, `sp` and the arena state are in them, the
//! thread pointer (`NT_ARM_TLS`), the breakpoint instruction (`brk #0`), the
//! kernel's single step, and the watchpoints (`NT_ARM_HW_WATCH`).

use super::*;

pub(super) use crate::debugger::arm64::{assign, registers, Float, General};

/// The processor.
pub(super) const ARCH: &Arch = &crate::debugger::arm64::AARCH64;

/// A `General` to read into.
pub(super) const NO_GENERAL: General = [0; 34];

/// The kernel steps a thread one instruction (`PTRACE_SINGLESTEP`).
pub(super) const HARDWARE_STEP: bool = true;

/// `<elf.h>`: the thread pointer's and the watchpoints' register sets.
const NT_ARM_TLS: usize = 0x401;
const NT_ARM_HW_WATCH: usize = 0x403;
/// `brk #0`, the instruction a breakpoint writes.
const BREAKPOINT_INSTRUCTION: [u8; 4] = 0xd420_0000_u32.to_le_bytes();
/// `DBGWCR`: enabled, for EL0 (`PAC` 0b10), on stores (`LSC` 0b10); the
/// byte-address-select bits start at bit 5.
const WCR_STORE_EL0: u64 = 0x1 | (0b10 << 1) | (0b10 << 3);
const WCR_BAS_SHIFT: u64 = 5;
/// `struct user_hwdebug_state`: `dbg_info` and padding, then 16 pairs of
/// `addr`, `ctrl` and padding.
const HW_DEBUG_HEADER: usize = 8;
const HW_DEBUG_SLOT: usize = 16;
const HW_DEBUG_SLOTS: usize = 16;

/// `pc`: word 32 of `user_pt_regs`.
pub(super) fn pc(general: &General) -> u64 {
    general[32]
}

/// `sp`: word 31.
pub(super) fn sp(general: &General) -> u64 {
    general[31]
}

/// The arena state MFBASIC code keeps in `x19`.
pub(super) fn arena(general: &General) -> u64 {
    general[19]
}

/// glibc's thread pointer is `struct pthread`'s size past `pthread_self`
/// (`TLS_PRE_TCB_SIZE` on AArch64 is that size).
pub(super) fn glibc_gap(size: u64) -> u64 {
    size
}

/// The instruction a breakpoint writes over `original`.
pub(super) fn breakpoint(_original: &[u8]) -> &'static [u8] {
    &BREAKPOINT_INSTRUCTION
}

impl Tracee {
    /// Thread `tid`'s thread pointer, `TPIDR_EL0`, from `NT_ARM_TLS`.
    pub(super) fn thread_pointer(&self, tid: c_int, _general: &General) -> u64 {
        self.regset(tid, NT_ARM_TLS, [0u64; 2])
            .map(|tls| tls[0])
            .unwrap_or(0)
    }

    /// Start thread `index` on one instruction: the kernel steps it. Answers
    /// whether it runs (always, here).
    pub(super) fn start_step(&mut self, index: usize) -> Result<bool, String> {
        self.resume(index, true)?;
        Ok(true)
    }

    /// Let thread `index`, stopped during its step for something else (a new
    /// thread, an interrupt, a signal left for later), go on with it.
    pub(super) fn continue_step(&mut self, index: usize) -> Result<(), String> {
        self.resume(index, true)
    }

    /// A kernel step plants no breakpoints, so no stop is on one of a step's.
    pub(super) fn on_step_trap(&self, _tid: c_int) -> Result<bool, String> {
        Ok(false)
    }

    /// A kernel step plants no breakpoints: none to put back.
    pub(super) fn end_step_traps(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// How many watchpoint register pairs the processor has.
    pub(super) fn watch_slots(&self, tid: c_int) -> Result<usize, String> {
        let state = self.regset(
            tid,
            NT_ARM_HW_WATCH,
            [0u8; HW_DEBUG_HEADER + HW_DEBUG_SLOT * HW_DEBUG_SLOTS],
        )?;
        Ok(usize::from(state[0]).min(HW_DEBUG_SLOTS))
    }

    /// Write the watchpoints into thread `tid`, enabled or all off.
    pub(super) fn set_thread_watchpoints(&self, tid: c_int, on: bool) -> Result<(), String> {
        let mut state = [0u8; HW_DEBUG_HEADER + HW_DEBUG_SLOT * HW_DEBUG_SLOTS];
        for (slot, &(address, size)) in self.watchpoints.iter().enumerate().take(HW_DEBUG_SLOTS) {
            let select = ((1u64 << size) - 1) << (address & 7);
            let control = WCR_STORE_EL0 | (select << WCR_BAS_SHIFT);
            let at = HW_DEBUG_HEADER + slot * HW_DEBUG_SLOT;
            state[at..at + 8].copy_from_slice(&(address & !7).to_le_bytes());
            let control = if on { control } else { control & !1 } as u32;
            state[at + 8..at + 12].copy_from_slice(&control.to_le_bytes());
        }
        // Every slot the processor has is written, so a removed watchpoint's
        // slot is cleared.
        let slots = self.watch_slots(tid)?;
        let mut vector = IoVec {
            base: state.as_mut_ptr().cast(),
            len: HW_DEBUG_HEADER + HW_DEBUG_SLOT * slots,
        };
        trace(
            PTRACE_SETREGSET,
            tid,
            NT_ARM_HW_WATCH,
            ptr::addr_of_mut!(vector) as usize,
        )
        .map(|_| ())
        .map_err(|error| {
            format!(
                "cannot set the program's watchpoints: ptrace: {}",
                os_error(error)
            )
        })
    }
}
