//! Where a riscv64 thread can be after one instruction: the riscv64 kernel has
//! no single step (`PTRACE_SINGLESTEP` fails with `EIO`), so the Linux backend
//! steps a thread by planting a breakpoint at every address the instruction can
//! go to and letting it run.

use super::General;
use crate::arch::riscv64::decode::{self, Flow};

/// `rt_sigreturn`'s system call number: the `ecall` that ends a signal
/// handler goes back to the `pc` its signal frame saved.
const SYS_RT_SIGRETURN: u64 = 139;
/// Where a signal frame keeps the interrupted `pc`, from `sp` as the handler
/// returns into `rt_sigreturn`: measured on both riscv64 boxes (kernels 6.12
/// and 6.18) by a probe that compared the slot with the interrupted `pc`.
const SIGFRAME_PC: u64 = 304;
/// How far past an `lr` the matching `sc` is looked for.
const ATOMIC_REACH: usize = 16;

/// Where a step can end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Next {
    /// At one of these addresses.
    To(Vec<u64>),
    /// Nowhere: the instruction is a breakpoint (`ebreak`), which a step
    /// reports instead of running.
    Breakpoint,
}

/// The value of integer register `number` in `general` (`zero` is 0).
fn x(general: &General, number: u8) -> u64 {
    match number {
        0 => 0,
        number => general[usize::from(number)],
    }
}

/// Every address the instruction at the start of `code`, at `pc`, can leave
/// the thread at, given its registers `general`; `read_u64` reads memory and
/// `read_code` code. An instruction the decoder does not know goes to the next
/// one, as most do.
pub(crate) fn next_addresses(
    code: &[u8],
    pc: u64,
    general: &General,
    read_u64: &mut dyn FnMut(u64) -> Result<u64, String>,
    read_code: &mut dyn FnMut(u64, usize) -> Result<Vec<u8>, String>,
) -> Result<Next, String> {
    let length = decode::length(code) as u64;
    let after = pc.wrapping_add(length);
    let Some(instruction) = decode::decode(code, pc) else {
        return Ok(Next::To(vec![after]));
    };
    let target = |instruction: &decode::Instruction| instruction.target.unwrap_or(after);
    Ok(Next::To(match instruction.flow {
        Flow::Next => vec![after],
        Flow::Branch => vec![after, target(&instruction)],
        Flow::Jump | Flow::Call => vec![target(&instruction)],
        Flow::Indirect {
            register, offset, ..
        } => vec![x(general, register).wrapping_add_signed(offset) & !1],
        Flow::Return => vec![x(general, 1) & !1],
        Flow::Trap if instruction.word == 0x0000_0073 => {
            // ecall: `rt_sigreturn` goes to the saved `pc`; any other system
            // call returns to the next instruction.
            if x(general, 17) == SYS_RT_SIGRETURN {
                vec![read_u64(x(general, 2).wrapping_add(SIGFRAME_PC))?]
            } else {
                vec![after]
            }
        }
        Flow::Trap => return Ok(Next::Breakpoint),
        Flow::AtomicStart => atomic(pc, after, read_code)?,
    }))
}

/// The addresses an `lr` at `pc` can leave a thread at when it runs on to
/// the end of its sequence: a breakpoint between `lr` and `sc` would make every
/// `sc` fail, so the step plants none there. They are the instruction after
/// the first `sc` within [`ATOMIC_REACH`] instructions, and the target of any
/// branch or jump between that leaves the sequence. With no `sc` that close,
/// the `lr` is stepped alone.
fn atomic(
    pc: u64,
    after: u64,
    read_code: &mut dyn FnMut(u64, usize) -> Result<Vec<u8>, String>,
) -> Result<Vec<u64>, String> {
    let mut at = after;
    let mut exits = Vec::new();
    for _ in 0..ATOMIC_REACH {
        let code = read_code(at, 4).or_else(|_| read_code(at, 2))?;
        let Some(instruction) = decode::decode(&code, at) else {
            break;
        };
        let next = at.wrapping_add(instruction.length as u64);
        let word = instruction.word;
        if word & 0x7f == 0x2f && word >> 27 == 0b00011 {
            let mut addresses = vec![next];
            addresses.extend(
                exits
                    .into_iter()
                    .filter(|target| *target < pc || *target >= next),
            );
            addresses.sort_unstable();
            addresses.dedup();
            return Ok(addresses);
        }
        if let (Flow::Branch | Flow::Jump, Some(target)) = (instruction.flow, instruction.target) {
            exits.push(target);
        }
        at = next;
    }
    Ok(vec![after])
}

#[cfg(test)]
mod tests;
