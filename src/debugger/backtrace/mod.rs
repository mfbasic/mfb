//! `backtrace`/`bt`: a thread's call stack, shown the way lldb shows it.
//!
//! MFBASIC code keeps no frame-pointer chain and an executable carries no
//! unwind tables, so each frame is unwound by reading its function's code from
//! the start to the frame's `pc`, as lldb does for code without unwind tables.
//! The walk tracks how far `sp` has moved from the value it had on entry (the
//! frame's canonical frame address, CFA) and where the link register and the
//! caller's frame pointer were saved, reading each instruction through the
//! processor's [`Arch::effect`]. On AArch64 (`lr` is `x30`, `fp` `x29`) that
//! reads both the compiler's frames (`sub sp, sp, #N` then `str x30, [sp,
//! #off]`, `crate::codegen::engine::util::vreg_frame`) and the frame records
//! system libraries build (`stp x29, x30, [sp, #-N]!`, `mov x29, sp`). An
//! epilogue (a return, or a jump out of the function) puts the walk back to the
//! state the prologue left, the state the rest of the body runs in.
//!
//! The walk ends at the program's entry `_main`, whose caller's return address
//! no longer exists, at a frame whose function is not known, or when the next
//! frame would not be further up the stack.

use super::arch::{Arch, Effect, Reg};
use super::disassemble::{at_offset, instructions};
use super::names::Collisions;
use super::{FunctionRange, Inferior, Register, RegisterValue};

/// The most frames a backtrace shows.
pub(crate) const MAX_FRAMES: usize = 4096;
/// The longest stretch of a function read to find a frame's state.
const MAX_SCAN: u64 = 1 << 20;
/// The functions with no caller to unwind to: the program's entry, whose
/// return address into the loader is gone once it makes its first call.
const OUTERMOST: &[&str] = &["_main"];

/// One frame: where it is, and the function holding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) pc: u64,
    pub(crate) function: Option<FunctionRange>,
}

/// What a function's code has done to the stack by some instruction, relative
/// to the CFA: the `sp` it had on entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameState {
    /// `sp - CFA`, or `None` once `sp` was set in a way the walk cannot follow
    /// (`mov sp, x29`, a register-sized adjustment).
    pub(crate) sp: Option<i64>,
    /// Where the link register is saved, as an offset from the CFA; `None`
    /// while the return address is still in it.
    pub(crate) lr: Option<i64>,
    /// Where the caller's frame pointer is saved, as an offset from the CFA;
    /// `None` while the frame pointer still holds the caller's value.
    pub(crate) fp_saved: Option<i64>,
    /// `fp - CFA`, once the function points the frame pointer at its own frame
    /// record.
    pub(crate) fp: Option<i64>,
}

impl FrameState {
    const ENTRY: FrameState = FrameState {
        sp: Some(0),
        lr: None,
        fp_saved: None,
        fp: None,
    };
}

/// The registers one frame was running with: enough to find its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Registers {
    pc: u64,
    sp: u64,
    fp: u64,
    /// The link register as the thread holds it; only the innermost frame
    /// knows it.
    lr: Option<u64>,
}

/// The frames of `thread`, innermost first, at most `limit` of them.
pub(crate) fn frames(
    inferior: &mut dyn Inferior,
    thread: usize,
    limit: usize,
) -> Result<Vec<Frame>, String> {
    let arch = inferior.arch();
    let registers = inferior.registers(thread)?;
    let value = |name: &str| -> Result<u64, String> {
        registers
            .iter()
            .find(|register: &&Register| register.name == name)
            .and_then(|register| match register.value {
                RegisterValue::Bits64(value) => Some(value),
                RegisterValue::Bits128(_) => None,
            })
            .ok_or_else(|| format!("thread {thread} has no '{name}' register"))
    };
    let mut current = Registers {
        pc: value(arch.pc)?,
        sp: value(arch.sp)?,
        fp: value(arch.frame)?,
        lr: Some(value(arch.link)?),
    };
    let mut frames = Vec::new();
    while frames.len() < limit.min(MAX_FRAMES) {
        // A return address is the instruction after the call, which may be
        // past the end of the calling function (`bl _exit` as its last
        // instruction): look the caller up by the call itself.
        let innermost = frames.is_empty();
        let lookup = if innermost {
            current.pc
        } else {
            current.pc.saturating_sub(arch.alignment)
        };
        let function = inferior
            .function_at(lookup)?
            .filter(|range| range.start <= lookup);
        frames.push(Frame {
            pc: current.pc,
            function: function.clone(),
        });
        let Some(next) = caller(inferior, function.as_ref(), current)? else {
            break;
        };
        // Each caller's frame is further up the stack; only a function that
        // has not yet moved `sp` shares its caller's.
        if next.pc == 0 || next.sp < current.sp || (next.sp == current.sp && !innermost) {
            break;
        }
        current = next;
    }
    Ok(frames)
}

/// The registers the caller of the frame running with `registers` in
/// `function` resumes with, or `None` where the walk ends.
fn caller(
    inferior: &mut dyn Inferior,
    function: Option<&FunctionRange>,
    registers: Registers,
) -> Result<Option<Registers>, String> {
    let arch = inferior.arch();
    let state = match function {
        Some(function) => {
            if function
                .name
                .as_deref()
                .is_some_and(|name| OUTERMOST.contains(&name))
            {
                return Ok(None);
            }
            let length = (registers.pc - function.start).min(MAX_SCAN);
            let from = registers.pc - length;
            let code = inferior.read_code(from, length as usize)?;
            let end = function.size.map(|size| function.start + size);
            state_at(arch, &code, from, function.start, end)
        }
        // Code no image names: only the innermost frame can be unwound, and
        // only as a function that has not yet touched the stack.
        None if registers.lr.is_some() => FrameState::ENTRY,
        None => return Ok(None),
    };
    let cfa = match (state.sp, state.fp) {
        (Some(sp), _) => registers.sp.wrapping_add_signed(-sp),
        (None, Some(fp)) => registers.fp.wrapping_add_signed(-fp),
        (None, None) => return Ok(None),
    };
    let pc = match state.lr {
        Some(offset) => inferior.read_u64(cfa.wrapping_add_signed(offset))?,
        None => match registers.lr {
            Some(lr) => lr,
            None => return Ok(None),
        },
    } & arch.address_mask;
    let fp = match state.fp_saved {
        Some(offset) => inferior.read_u64(cfa.wrapping_add_signed(offset))?,
        None => registers.fp,
    };
    Ok(Some(Registers {
        pc,
        sp: cfa,
        fp,
        lr: None,
    }))
}

/// The frame state after running `code`, the code from `from` up to (not
/// including) the frame's instruction, in a function from `start` to `end`
/// (when known); a jump outside it leaves the function.
pub(crate) fn state_at(
    arch: &Arch,
    code: &[u8],
    from: u64,
    start: u64,
    end: Option<u64>,
) -> FrameState {
    let mut state = FrameState::ENTRY;
    // `register - CFA` for registers set to an address in the frame
    // (`add x17, sp, #off`), so a store through them can be followed.
    let mut bases = Bases::default();
    // The constant each register holds, where the code put one there.
    let mut constants = Bases::default();
    // The state the prologue leaves, which the body runs in.
    let mut body = None::<FrameState>;
    // Whether control has left the straight line yet (an epilogue ran).
    let mut left = false;
    for (address, bytes) in instructions(arch, code, from) {
        let effect = (arch.effect)(bytes, address);
        // A store through a register set from `sp` (a save too far up the
        // frame for one instruction) is part of the prologue too.
        let through_base = matches!(&effect, Effect::Store { base, .. }
            if *base != Reg::Sp && bases.get(*base).is_some());
        // Loading a constant changes nothing about the frame: it neither
        // continues a prologue nor ends one.
        let constant = matches!(effect, Effect::Constant { .. } | Effect::ConstantAdd { .. });
        if body.is_none() && !effect.in_prologue() && !through_base && !constant {
            body = Some(state);
        }
        match effect {
            Effect::AdjustSp(delta) => state.sp = state.sp.map(|sp| sp + delta),
            Effect::SpUnknown => state.sp = None,
            Effect::Store { base, pairs } => saved(&mut state, &bases, base, &pairs),
            Effect::StoreWriteback { base, pairs, delta } => {
                // A pre-index store moves its base first, then stores.
                moved(&mut state, &mut bases, base, delta);
                saved(&mut state, &bases, base, &pairs);
            }
            Effect::StoreThenMove { base, pairs, delta } => {
                saved(&mut state, &bases, base, &pairs);
                moved(&mut state, &mut bases, base, delta);
            }
            Effect::Load {
                registers,
                sp_delta,
            } => {
                for register in registers {
                    match register {
                        Reg::Link => state.lr = None,
                        Reg::Frame => {
                            state.fp_saved = None;
                            state.fp = None;
                        }
                        Reg::Sp | Reg::Other(_) => {}
                    }
                }
                if let Some(delta) = sp_delta {
                    state.sp = state.sp.map(|sp| sp + delta);
                }
            }
            Effect::SpAddress { register, offset } => {
                let value = state.sp.map(|sp| sp + offset);
                if register == Reg::Frame {
                    state.fp = value;
                } else {
                    bases.set(register, value);
                }
            }
            Effect::SpFromFp { offset } => state.sp = state.fp.map(|fp| fp + offset),
            Effect::Leave { target } => {
                let leaves = match target {
                    None => true,
                    Some(target) => target < start || end.is_some_and(|end| target >= end),
                };
                if leaves {
                    state = body.unwrap_or(state);
                    left = true;
                }
            }
            Effect::Constant { register, value } => constants.set(register, value),
            Effect::ConstantAdd {
                register,
                from,
                offset,
            } => constants.set(register, constants.get(from).map(|value| value + offset)),
            Effect::SpByRegister { register, subtract } => {
                state.sp = match (state.sp, constants.get(register)) {
                    (Some(sp), Some(value)) if subtract => Some(sp - value),
                    (Some(sp), Some(value)) => Some(sp + value),
                    _ => None,
                }
            }
            Effect::Hint | Effect::Other => {}
        }
        // A compiler may schedule a prologue's saves among the body's first
        // instructions (riscv64 glibc saves `ra` after them): until an epilogue
        // runs, a save the body's state lacks belongs to it too.
        if let (Some(body), false) = (&mut body, left) {
            body.lr = body.lr.or(state.lr);
            body.fp_saved = body.fp_saved.or(state.fp_saved);
            body.fp = body.fp.or(state.fp);
        }
    }
    state
}

/// `register - CFA` for each register other than `sp` and the frame pointer
/// that holds an address in the frame: the link register's, then the others'
/// by number.
#[derive(Default)]
struct Bases {
    link: Option<i64>,
    others: [Option<i64>; 32],
}

impl Bases {
    fn get(&self, register: Reg) -> Option<i64> {
        match register {
            Reg::Link => self.link,
            Reg::Other(number) => self.others.get(usize::from(number)).copied().flatten(),
            Reg::Sp | Reg::Frame => None,
        }
    }

    fn set(&mut self, register: Reg, value: Option<i64>) {
        match register {
            Reg::Link => self.link = value,
            Reg::Other(number) => {
                if let Some(slot) = self.others.get_mut(usize::from(number)) {
                    *slot = value;
                }
            }
            Reg::Sp | Reg::Frame => {}
        }
    }
}

/// Record where the frame pointer and the link register went, for stores at
/// `pairs` offsets from `base`.
fn saved(state: &mut FrameState, bases: &Bases, base: Reg, pairs: &[(Reg, i64)]) {
    let Some(at) = base_offset(state, bases, base) else {
        return;
    };
    for (register, offset) in pairs {
        match register {
            Reg::Link => state.lr = Some(at + offset),
            Reg::Frame => state.fp_saved = Some(at + offset),
            Reg::Sp | Reg::Other(_) => {}
        }
    }
}

/// Move a written-back base register by `delta`.
fn moved(state: &mut FrameState, bases: &mut Bases, base: Reg, delta: i64) {
    match base {
        Reg::Sp => state.sp = state.sp.map(|sp| sp + delta),
        Reg::Frame => state.fp = state.fp.map(|fp| fp + delta),
        register => bases.set(register, bases.get(register).map(|at| at + delta)),
    }
}

/// `base - CFA` for a store's base register: `sp`, or one set from it.
fn base_offset(state: &FrameState, bases: &Bases, base: Reg) -> Option<i64> {
    match base {
        Reg::Sp => state.sp,
        Reg::Frame => state.fp,
        register => bases.get(register),
    }
}

/// The backtrace lines for one thread, lldb's layout: a `thread #<id>` line,
/// then a `frame #<n>: <pc> <image>`<function> + <offset>` line per frame.
/// `current` marks the thread and its innermost frame with `*`.
pub(crate) fn lines(
    thread: usize,
    name: Option<&str>,
    task: &str,
    frames: &[Frame],
    current: bool,
    collisions: &Collisions,
) -> Vec<String> {
    let mark = if current { "*" } else { " " };
    let mut header = format!("{mark} thread #{thread}");
    if let Some(name) = name {
        header.push_str(&format!(", name = '{name}'"));
    }
    header.push_str(&format!(", task = '{task}'"));
    let mut lines = vec![header];
    for (number, frame) in frames.iter().enumerate() {
        let mark = if current && number == 0 { "*" } else { " " };
        let mut line = format!("  {mark} frame #{number}: 0x{:016x}", frame.pc);
        if let Some(place) = place(frame, collisions) {
            line.push(' ');
            line.push_str(&place);
        }
        lines.push(line);
    }
    lines
}

/// `image`function + offset`, as lldb names a frame's place; `None` for code
/// no image names.
fn place(frame: &Frame, collisions: &Collisions) -> Option<String> {
    let function = frame.function.as_ref()?;
    let offset = frame.pc - function.start;
    let name = match &function.name {
        Some(name) => at_offset(name, offset, collisions),
        None if offset == 0 => format!("0x{:016x}", function.start),
        None => format!("0x{:016x} + {offset}", function.start),
    };
    Some(if function.image.is_empty() {
        name
    } else {
        format!("{}`{name}", function.image)
    })
}

#[cfg(test)]
mod tests;
