//! `backtrace`/`bt`: a thread's call stack, shown the way lldb shows it.
//!
//! MFBASIC code keeps no frame-pointer chain and an executable carries no
//! unwind tables, so each frame is unwound by reading its function's code from
//! the start to the frame's `pc`, as lldb does for code without unwind tables.
//! The walk tracks how far `sp` has moved from the value it had on entry (the
//! frame's canonical frame address, CFA) and where `lr` (`x30`) and the
//! caller's `fp` (`x29`) were saved. That reads both the compiler's frames
//! (`sub sp, sp, #N` then `str x30, [sp, #off]`, `crate::codegen::engine::util::vreg_frame`)
//! and the frame records system libraries build (`stp x29, x30, [sp, #-N]!`,
//! `mov x29, sp`). An epilogue (`ret`, or a branch out of the function) puts
//! the walk back to the state the prologue left, the state the rest of the
//! body runs in.
//!
//! The walk ends at the program's entry `_main`, whose caller's return address
//! no longer exists, at a frame whose function is not known, or when the next
//! frame would not be further up the stack.

use super::disassemble::at_offset;
use super::names::Collisions;
use super::{FunctionRange, Inferior, Register, RegisterValue};

/// The most frames a backtrace shows.
pub(crate) const MAX_FRAMES: usize = 4096;
/// The longest stretch of a function read to find a frame's state.
const MAX_SCAN: u64 = 1 << 20;
/// The functions with no caller to unwind to: the program's entry, whose
/// return address into the loader is gone once it makes its first call.
const OUTERMOST: &[&str] = &["_main"];
/// The bits of a user-space address: 47 on macOS, 48 on Linux. Return
/// addresses carry no pointer-authentication bits in an arm64 process; masking
/// strips any that a system library's code might have left, which sit above
/// these.
const ADDRESS_MASK: u64 = if cfg!(target_os = "linux") {
    (1 << 48) - 1
} else {
    (1 << 47) - 1
};

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
    /// Where `lr` is saved, as an offset from the CFA; `None` while the return
    /// address is still in `lr`.
    pub(crate) lr: Option<i64>,
    /// Where the caller's `fp` is saved, as an offset from the CFA; `None`
    /// while `fp` still holds the caller's value.
    pub(crate) fp_saved: Option<i64>,
    /// `fp - CFA`, once the function points `fp` at its own frame record.
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
    /// `lr` as the thread holds it; only the innermost frame knows it.
    lr: Option<u64>,
}

/// The frames of `thread`, innermost first, at most `limit` of them.
pub(crate) fn frames(
    inferior: &mut dyn Inferior,
    thread: usize,
    limit: usize,
) -> Result<Vec<Frame>, String> {
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
        pc: value("pc")?,
        sp: value("sp")?,
        fp: value("fp")?,
        lr: Some(value("lr")?),
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
            current.pc.saturating_sub(4)
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
            let words = code
                .chunks_exact(4)
                .map(|word| u32::from_le_bytes(word.try_into().expect("4 bytes")))
                .collect::<Vec<_>>();
            let end = function.size.map(|size| function.start + size);
            state_at(&words, from, function.start, end)
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
    } & ADDRESS_MASK;
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

/// The frame state after running `words`, the code from `from` up to (not
/// including) the frame's instruction, in a function from `start` to `end`
/// (when known); a branch outside it leaves the function.
pub(crate) fn state_at(words: &[u32], from: u64, start: u64, end: Option<u64>) -> FrameState {
    let mut state = FrameState::ENTRY;
    // `x<n> - CFA` for registers set to an address in the frame
    // (`add x17, sp, #off`), so a store through them can be followed.
    let mut bases = [None::<i64>; 31];
    // The state the prologue leaves, which the body runs in.
    let mut body = None::<FrameState>;
    for (index, word) in words.iter().copied().enumerate() {
        let address = from + index as u64 * 4;
        let effect = Effect::of(word, address);
        // A store through a register set from `sp` (a save too far up the
        // frame for one instruction) is part of the prologue too.
        let through_base = matches!(&effect, Effect::Store { base, .. }
            if *base != 31 && bases.get(*base as usize).is_some_and(Option::is_some));
        if body.is_none() && !effect.in_prologue() && !through_base {
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
                        30 => state.lr = None,
                        29 => {
                            state.fp_saved = None;
                            state.fp = None;
                        }
                        _ => {}
                    }
                }
                if let Some(delta) = sp_delta {
                    state.sp = state.sp.map(|sp| sp + delta);
                }
            }
            Effect::SpAddress { register, offset } => {
                let value = state.sp.map(|sp| sp + offset);
                if register == 29 {
                    state.fp = value;
                } else if let Some(slot) = bases.get_mut(register as usize) {
                    *slot = value;
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
                }
            }
            Effect::Hint | Effect::Other => {}
        }
    }
    state
}

/// Record where `fp` and `lr` went, for stores at `pairs` offsets from `base`.
fn saved(state: &mut FrameState, bases: &[Option<i64>; 31], base: u8, pairs: &[(u8, i64)]) {
    let Some(at) = base_offset(state, bases, base) else {
        return;
    };
    for (register, offset) in pairs {
        match register {
            30 => state.lr = Some(at + offset),
            29 => state.fp_saved = Some(at + offset),
            _ => {}
        }
    }
}

/// Move a written-back base register by `delta`.
fn moved(state: &mut FrameState, bases: &mut [Option<i64>; 31], base: u8, delta: i64) {
    match base {
        31 => state.sp = state.sp.map(|sp| sp + delta),
        29 => state.fp = state.fp.map(|fp| fp + delta),
        register => {
            if let Some(slot) = bases.get_mut(register as usize) {
                *slot = slot.map(|at| at + delta);
            }
        }
    }
}

/// `base - CFA` for a store's base register: `sp`, or one set from it.
fn base_offset(state: &FrameState, bases: &[Option<i64>; 31], base: u8) -> Option<i64> {
    match base {
        31 => state.sp,
        29 => state.fp,
        register => bases[register as usize],
    }
}

/// What one instruction does to the frame.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Effect {
    /// `sub`/`add sp, sp, #imm`: `sp` moves by this much.
    AdjustSp(i64),
    /// `sp` set some way the walk cannot follow.
    SpUnknown,
    /// Registers stored at offsets from `base` (31 is `sp`).
    Store {
        base: u8,
        pairs: Vec<(u8, i64)>,
    },
    /// A pre-index store: `base` moves by `delta`, then the stores happen at
    /// offsets from its new value.
    StoreWriteback {
        base: u8,
        pairs: Vec<(u8, i64)>,
        delta: i64,
    },
    /// A post-index store: the stores happen at offsets from `base`, which
    /// then moves by `delta`.
    StoreThenMove {
        base: u8,
        pairs: Vec<(u8, i64)>,
        delta: i64,
    },
    /// Registers loaded; a post-index load from `sp` then moves it.
    Load {
        registers: Vec<u8>,
        sp_delta: Option<i64>,
    },
    /// `add x<register>, sp, #offset` (`mov x29, sp` is offset 0).
    SpAddress {
        register: u8,
        offset: i64,
    },
    /// `sub sp, x29, #offset` (as a negative offset) or `mov sp, x29`.
    SpFromFp {
        offset: i64,
    },
    /// `ret`, `br`, or `b` to `target`: control leaves this straight line.
    Leave {
        target: Option<u64>,
    },
    /// A hint: `nop`, `pacibsp`, `bti`.
    Hint,
    Other,
}

impl Effect {
    fn of(word: u32, address: u64) -> Effect {
        let rd = (word & 31) as u8;
        let rn = ((word >> 5) & 31) as u8;
        let rt2 = ((word >> 10) & 31) as u8;
        // ADD/SUB (immediate), 64-bit: sf=1, S=0.
        if word & 0x3f80_0000 == 0x1100_0000 && word >> 31 == 1 {
            let subtract = word & 0x4000_0000 != 0;
            let shift = if word & 0x0040_0000 != 0 { 12 } else { 0 };
            let immediate = i64::from((word >> 10) & 0xfff) << shift;
            let signed = if subtract { -immediate } else { immediate };
            return match (rd, rn) {
                (31, 31) => Effect::AdjustSp(signed),
                (31, 29) => Effect::SpFromFp { offset: signed },
                (31, _) => Effect::SpUnknown,
                (register, 31) => Effect::SpAddress {
                    register,
                    offset: signed,
                },
                _ => Effect::Other,
            };
        }
        // Any other write to `sp` (`mov sp, x9`, `sub sp, sp, x16`): ADD/SUB
        // (extended register) with Rd = 31.
        if word & 0x1fe0_0000 == 0x0b20_0000 && rd == 31 {
            return Effect::SpUnknown;
        }
        // LDP/STP, 64-bit integer or SIMD: opc and V select the size.
        let pair = word & 0x3a00_0000 == 0x2800_0000;
        if pair {
            let opc = word >> 30;
            let simd = word & 0x0400_0000 != 0;
            let scale = match (simd, opc) {
                (false, 2) => 8,
                (true, 1) => 16,
                (true, 2) => 32,
                _ => 0,
            };
            if scale != 0 {
                let load = word & 0x0040_0000 != 0;
                let mode = (word >> 23) & 3;
                let offset = sign_extend((word >> 15) & 0x7f, 7) * scale;
                // SIMD registers are never `fp` or `lr`.
                let registers = if simd { Vec::new() } else { vec![rd, rt2] };
                return pair_effect(load, mode, rn, registers, offset, scale);
            }
        }
        // STR/LDR (immediate, unsigned offset), 64-bit integer.
        if word & 0xffc0_0000 == 0xf900_0000 {
            let offset = i64::from((word >> 10) & 0xfff) * 8;
            return Effect::Store {
                base: rn,
                pairs: vec![(rd, offset)],
            };
        }
        if word & 0xffc0_0000 == 0xf940_0000 {
            return Effect::Load {
                registers: vec![rd],
                sp_delta: None,
            };
        }
        // STR/LDR (immediate, pre- or post-index), 64-bit integer.
        if word & 0xffe0_0400 == 0xf800_0400 {
            let pre = word & 0x800 != 0;
            let delta = sign_extend((word >> 12) & 0x1ff, 9);
            return if pre {
                Effect::StoreWriteback {
                    base: rn,
                    pairs: vec![(rd, 0)],
                    delta,
                }
            } else {
                Effect::StoreThenMove {
                    base: rn,
                    pairs: vec![(rd, 0)],
                    delta,
                }
            };
        }
        if word & 0xffe0_0400 == 0xf840_0400 {
            return Effect::Load {
                registers: vec![rd],
                sp_delta: (rn == 31).then(|| sign_extend((word >> 12) & 0x1ff, 9)),
            };
        }
        // STR (immediate, unsigned offset) of a SIMD register to `sp`: a
        // callee-saved `d8`–`d15` save, part of a prologue.
        if word & 0x3f40_0000 == 0x3d00_0000 && rn == 31 {
            return Effect::Store {
                base: 31,
                pairs: Vec::new(),
            };
        }
        // HINT: `nop`, `pacibsp`, `bti`, ….
        if word & 0xffff_f01f == 0xd503_201f {
            return Effect::Hint;
        }
        // RET, RETAA, RETAB.
        if word & 0xffff_fc1f == 0xd65f_0000 || word == 0xd65f_0bff || word == 0xd65f_0fff {
            return Effect::Leave { target: None };
        }
        // BR (and its authenticated forms).
        if word & 0xfe9f_f000 == 0xd61f_0000 {
            return Effect::Leave { target: None };
        }
        // B.
        if word & 0xfc00_0000 == 0x1400_0000 {
            let offset = sign_extend(word & 0x03ff_ffff, 26) * 4;
            return Effect::Leave {
                target: Some(address.wrapping_add_signed(offset)),
            };
        }
        Effect::Other
    }

    /// Whether a prologue can hold this instruction: it sets up the frame,
    /// saves registers into it, or is a hint (`pacibsp`, `bti`, `nop`).
    fn in_prologue(&self) -> bool {
        match self {
            Effect::AdjustSp(delta) => *delta <= 0,
            Effect::Store { base, .. }
            | Effect::StoreWriteback { base, .. }
            | Effect::StoreThenMove { base, .. } => *base == 31,
            Effect::SpAddress { .. } | Effect::Hint => true,
            _ => false,
        }
    }
}

/// The effect of an LDP/STP: `mode` 1 is post-index, 2 signed offset, 3
/// pre-index.
fn pair_effect(
    load: bool,
    mode: u32,
    base: u8,
    registers: Vec<u8>,
    offset: i64,
    scale: i64,
) -> Effect {
    if load {
        let sp_delta = (base == 31 && mode != 2).then_some(offset);
        // A pre-index load from `sp` moves it first; every load here restores
        // registers, so only where `sp` ends up matters.
        return Effect::Load {
            registers,
            sp_delta,
        };
    }
    let pairs = registers
        .into_iter()
        .zip([0, scale])
        .map(|(register, at)| (register, at))
        .collect::<Vec<_>>();
    match mode {
        3 => Effect::StoreWriteback {
            base,
            pairs,
            delta: offset,
        },
        1 => Effect::StoreThenMove {
            base,
            pairs,
            delta: offset,
        },
        _ => Effect::Store {
            base,
            pairs: pairs
                .into_iter()
                .map(|(register, at)| (register, at + offset))
                .collect(),
        },
    }
}

fn sign_extend(value: u32, bits: u32) -> i64 {
    let shift = 64 - bits;
    (i64::from(value) << shift) >> shift
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
mod tests {
    use super::*;

    const SUB_SP_0X150: u32 = 0xd105_43ff;
    const ADD_SP_0X150: u32 = 0x9105_43ff;
    const SUB_SP_0X1000: u32 = 0xd140_07ff;
    const SUB_SP_0X10: u32 = 0xd100_43ff;
    const ADD_SP_0X10: u32 = 0x9100_43ff;
    const STR_X30_SP: u32 = 0xf900_03fe;
    const STR_X21_SP: u32 = 0xf900_03f5;
    const STR_X30_SP_16: u32 = 0xf900_0bfe;
    const LDR_X30_SP: u32 = 0xf940_03fe;
    const STP_FP_LR_PRE: u32 = 0xa9bf_7bfd;
    const LDP_FP_LR_POST: u32 = 0xa8c1_7bfd;
    const MOV_FP_SP: u32 = 0x9100_03fd;
    const PACIBSP: u32 = 0xd503_237f;
    const RET: u32 = 0xd65f_03c0;
    const MOV_X0_X1: u32 = 0xaa01_03e0;
    const BL: u32 = 0x9400_0005;

    fn state(words: &[u32]) -> FrameState {
        state_at(words, 0x1000, 0x1000, Some(0x2000))
    }

    #[test]
    fn a_compiled_frame_saves_lr_at_its_offset_from_the_cfa() {
        assert_eq!(state(&[]), FrameState::ENTRY);
        assert_eq!(
            state(&[SUB_SP_0X150]),
            FrameState {
                sp: Some(-0x150),
                ..FrameState::ENTRY
            }
        );
        let body = FrameState {
            sp: Some(-0x150),
            lr: Some(-0x150),
            ..FrameState::ENTRY
        };
        assert_eq!(state(&[SUB_SP_0X150, STR_X30_SP]), body);
        assert_eq!(state(&[SUB_SP_0X150, STR_X30_SP, MOV_X0_X1, BL]), body);
        // Callee-saved registers before lr move its slot up.
        assert_eq!(
            state(&[SUB_SP_0X150, STR_X21_SP, STR_X30_SP_16]).lr,
            Some(-0x150 + 16)
        );
    }

    #[test]
    fn an_epilogue_gives_lr_and_sp_back_and_a_ret_restores_the_body() {
        let prologue = [SUB_SP_0X150, STR_X30_SP, MOV_X0_X1];
        let mut words = prologue.to_vec();
        words.push(LDR_X30_SP);
        assert_eq!(
            state(&words),
            FrameState {
                sp: Some(-0x150),
                ..FrameState::ENTRY
            }
        );
        words.push(ADD_SP_0X150);
        assert_eq!(state(&words), FrameState::ENTRY);
        words.push(RET);
        assert_eq!(state(&words), state(&prologue));
    }

    #[test]
    fn a_large_frame_and_a_call_s_own_stack_space_are_followed() {
        assert_eq!(
            state(&[SUB_SP_0X1000, SUB_SP_0X10, STR_X30_SP]).sp,
            Some(-0x1010)
        );
        let words = [SUB_SP_0X150, STR_X30_SP, MOV_X0_X1, SUB_SP_0X10, BL];
        assert_eq!(state(&words).sp, Some(-0x160));
        assert_eq!(state(&words).lr, Some(-0x150));
        let mut after = words.to_vec();
        after.push(ADD_SP_0X10);
        assert_eq!(state(&after).sp, Some(-0x150));
    }

    #[test]
    fn a_frame_record_saves_fp_and_lr_and_points_fp_at_itself() {
        let record = FrameState {
            sp: Some(-16),
            lr: Some(-8),
            fp_saved: Some(-16),
            fp: Some(-16),
        };
        assert_eq!(state(&[PACIBSP, STP_FP_LR_PRE, MOV_FP_SP]), record);
        assert_eq!(
            state(&[PACIBSP, STP_FP_LR_PRE, MOV_FP_SP, LDP_FP_LR_POST]),
            FrameState::ENTRY
        );
    }

    #[test]
    fn a_branch_out_of_the_function_ends_its_epilogue_but_one_inside_does_not() {
        // `b` back to the start of the function, then `b` past its end.
        let inside = 0x17ff_fffd;
        let outside = 0x1400_1000;
        let prologue = [SUB_SP_0X150, STR_X30_SP, MOV_X0_X1];
        let mut words = prologue.to_vec();
        words.extend([LDR_X30_SP, ADD_SP_0X150, inside]);
        assert_eq!(state(&words), FrameState::ENTRY);
        words.pop();
        words.push(outside);
        assert_eq!(state(&words), state(&prologue));
    }

    #[test]
    fn frames_are_shown_the_way_lldb_shows_them() {
        let range = |image: &str, name: Option<&str>, start| FunctionRange {
            image: image.to_string(),
            name: name.map(str::to_string),
            start,
            size: None,
        };
        let frames = [
            Frame {
                pc: 0x1000,
                function: Some(range("debug.out", Some("_mfb_fn_isPrime"), 0x1000)),
            },
            Frame {
                pc: 0x2048,
                function: Some(range("debug.out", Some("_mfb_fn_countPrimes"), 0x2000)),
            },
            Frame {
                pc: 0x3010,
                function: Some(range("libfoo.dylib", None, 0x3000)),
            },
            Frame {
                pc: 0x4000,
                function: None,
            },
        ];
        let none = Collisions::default();
        assert_eq!(
            lines(0, None, "MFB Main", &frames, true, &none),
            [
                "* thread #0, task = 'MFB Main'",
                "  * frame #0: 0x0000000000001000 debug.out`isPrime",
                "    frame #1: 0x0000000000002048 debug.out`countPrimes + 72",
                "    frame #2: 0x0000000000003010 libfoo.dylib`0x0000000000003000 + 16",
                "    frame #3: 0x0000000000004000",
            ]
        );
        assert_eq!(
            lines(2, Some("worker"), "User Thread", &frames[..1], false, &none),
            [
                "  thread #2, name = 'worker', task = 'User Thread'",
                "    frame #0: 0x0000000000001000 debug.out`isPrime",
            ]
        );
    }

    /// The program's `main` reads as written and the runtime's `_main` by its
    /// symbol, so neither needs a tag.
    #[test]
    fn a_function_nobody_wrote_reads_by_its_symbol() {
        let symbol = |name: &str| crate::debugger::FunctionSymbol {
            name: name.to_string(),
            linked: false,
            loaded: false,
            address: None,
            size: None,
        };
        let collisions = Collisions::of(&[symbol("_mfb_fn_main"), symbol("_main")]);
        let frame = |name: &str, start, pc| Frame {
            pc,
            function: Some(FunctionRange {
                image: "debug.out".to_string(),
                name: Some(name.to_string()),
                start,
                size: None,
            }),
        };
        let frames = [
            frame("_mfb_fn_main", 0x1000, 0x1010),
            frame("_main", 0x2000, 0x2000),
        ];
        assert_eq!(
            lines(0, None, "MFB Main", &frames, false, &collisions)[1..],
            [
                "    frame #0: 0x0000000000001010 debug.out`main + 16",
                "    frame #1: 0x0000000000002000 debug.out`_main",
            ]
        );
    }
}
