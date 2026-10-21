//! The riscv64 parts of the Linux backend, which `linux/` reads as `cpu::`.
//!
//! The register sets are `NT_PRSTATUS` (`pc`, `x1`–`x31`, 256 bytes) and
//! `NT_PRFPREG` (`f0`–`f31`, `fcsr`, 264 bytes); the thread pointer is `tp`
//! (`x4`), there being no TLS register set. A breakpoint is `c.ebreak` over a
//! 2-byte instruction and `ebreak` over a 4-byte one; either stops the thread
//! with `pc` on it (`SIGTRAP`, `TRAP_BRKPT`). The kernel has no single step
//! (`PTRACE_SINGLESTEP` fails with `EIO`) and no watchpoints, so a step plants
//! a breakpoint at every address the instruction can go to
//! ([`crate::debugger::riscv64::step`]) and continues the thread, and a
//! watchpoint is refused.

use super::*;
use crate::arch::riscv64::decode;
use crate::debugger::riscv64::step::{next_addresses, Next};

pub(super) use crate::debugger::riscv64::{assign, registers, Float, General};

/// The processor.
pub(super) const ARCH: &Arch = &crate::debugger::riscv64::RISCV64;

/// A `General` to read into.
pub(super) const NO_GENERAL: General = [0; 32];

/// The kernel cannot step a thread: [`Tracee::start_step`] plants breakpoints.
pub(super) const HARDWARE_STEP: bool = false;

/// `c.ebreak` and `ebreak`.
const C_EBREAK: [u8; 2] = 0x9002_u16.to_le_bytes();
const EBREAK: [u8; 4] = 0x0010_0073_u32.to_le_bytes();

/// What `wp` answers: neither the riscv64 kernels the boxes run (6.12, 6.18)
/// has a breakpoint performance event or `NT_*_HW_WATCH` set to hold one.
const NO_WATCHPOINTS: &str =
    "this system has no watchpoints: the riscv64 kernel does not provide them";

/// `pc`: word 0 of `user_regs_struct`.
pub(super) fn pc(general: &General) -> u64 {
    crate::debugger::riscv64::pc(general)
}

/// `sp`: `x2`.
pub(super) fn sp(general: &General) -> u64 {
    crate::debugger::riscv64::sp(general)
}

/// The arena state MFBASIC code keeps in `s11` (`x27`).
pub(super) fn arena(general: &General) -> u64 {
    general[27]
}

/// glibc's thread pointer is 32 bytes further from `pthread_self` than
/// `struct pthread`'s size on riscv64: its `TLS_PRE_TCB_SIZE` is that size plus
/// the 16-byte `tcbhead_t`, rounded up to `struct pthread`'s 32-byte
/// alignment. Measured on box 2232 (glibc 2.41): `tp - pthread_self` 1856,
/// `_thread_db_sizeof_pthread` 1824.
pub(super) fn glibc_gap(size: u64) -> u64 {
    size + 32
}

/// The instruction a breakpoint writes over `original`: one of its size.
pub(super) fn breakpoint(original: &[u8]) -> &'static [u8] {
    if decode::length(original) == 2 {
        &C_EBREAK
    } else {
        &EBREAK
    }
}

impl Tracee {
    /// Thread `tid`'s thread pointer: `tp`, `x4`.
    pub(super) fn thread_pointer(&self, _tid: c_int, general: &General) -> u64 {
        general[4]
    }

    /// Start thread `index` on one instruction: plant a breakpoint at every
    /// address it can go to and let it run. Answers `false`, without running
    /// it, when the instruction is itself a breakpoint (`ebreak`).
    pub(super) fn start_step(&mut self, index: usize) -> Result<bool, String> {
        let tid = self.threads[index].tid;
        let general = self.general(tid)?;
        let pc = pc(&general);
        let code = self.read(pc, 4).or_else(|_| self.read(pc, 2))?;
        let next = next_addresses(
            &code,
            pc,
            &general,
            &mut |address| {
                let bytes = self.read(address, 8)?;
                Ok(u64::from_le_bytes(bytes[..].try_into().expect("8 bytes")))
            },
            &mut |address, length| self.read(address, length),
        )?;
        let Next::To(addresses) = next else {
            return Ok(false);
        };
        for address in addresses {
            if self.step_traps.iter().any(|(at, _)| *at == address) {
                continue;
            }
            // An address no code is mapped at gets no breakpoint: the thread
            // faults on reaching it, and the step reports that, as AArch64's
            // does.
            let Ok(first) = self.read(address, 2) else {
                continue;
            };
            let Ok(original) = self.read(address, decode::length(&first)) else {
                continue;
            };
            if self.write(address, breakpoint(&original)).is_ok() {
                self.step_traps.push((address, original));
            }
        }
        self.continue_step(index)?;
        Ok(true)
    }

    /// Let thread `index`, stopped during its step for something else (a new
    /// thread, an interrupt, a signal left for the next run), go on to its
    /// step's breakpoints. A pending signal stays pending, as on AArch64.
    pub(super) fn continue_step(&mut self, index: usize) -> Result<(), String> {
        let thread = &mut self.threads[index];
        thread.stopped = false;
        match trace(PTRACE_CONT, thread.tid, 0, 0) {
            Ok(_) | Err(ESRCH) => Ok(()),
            Err(error) => Err(format!(
                "cannot resume thread {}: ptrace: {}",
                thread.tid,
                os_error(error)
            )),
        }
    }

    /// Whether thread `tid` stopped on one of the step's breakpoints.
    pub(super) fn on_step_trap(&self, tid: c_int) -> Result<bool, String> {
        let pc = pc(&self.general(tid)?);
        Ok(self.step_traps.iter().any(|(at, _)| *at == pc))
    }

    /// Put back what the step's breakpoints replaced. Once the program has
    /// ended there is nothing to put back.
    pub(super) fn end_step_traps(&mut self) -> Result<(), String> {
        let traps = std::mem::take(&mut self.step_traps);
        if self.exited.is_some() {
            return Ok(());
        }
        for (address, original) in &traps {
            self.write(*address, original)?;
        }
        Ok(())
    }

    /// Refused: the system has no watchpoints.
    pub(super) fn watch_slots(&self, _tid: c_int) -> Result<usize, String> {
        Err(NO_WATCHPOINTS.to_string())
    }

    /// With no watchpoints set there is nothing to write; with one, refused.
    pub(super) fn set_thread_watchpoints(&self, _tid: c_int, _on: bool) -> Result<(), String> {
        if self.watchpoints.is_empty() {
            Ok(())
        } else {
            Err(NO_WATCHPOINTS.to_string())
        }
    }
}
