//! `frames`: unwinding a paused thread through a process built from tables —
//! functions with their code, a word map for the stack, and one thread's
//! registers.

use std::collections::HashMap;
use std::ffi::OsString;

use super::*;
use crate::debugger::{Ending, FunctionSymbol, RegisterGroup, RunStop, Step, ThreadFacts};

const SUB_SP_0X150: u32 = 0xd105_43ff;
const SUB_SP_0X10: u32 = 0xd100_43ff;
const STR_X30_SP: u32 = 0xf900_03fe;
const STP_FP_LR_PRE: u32 = 0xa9bf_7bfd;
const MOV_FP_SP: u32 = 0x9100_03fd;
const MOV_X0_X1: u32 = 0xaa01_03e0;
const BL: u32 = 0x9400_0005;
/// `sub sp, sp, x16` (extended register): `sp` moves by an unknown amount.
const SUB_SP_X16: u32 = 0xcb30_63ff;

/// A paused process: its functions (each with its code), its memory as
/// 8-byte words, and the registers of its only thread.
#[derive(Default)]
struct Process {
    functions: Vec<(FunctionRange, Vec<u32>)>,
    words: HashMap<u64, u64>,
    registers: Vec<Register>,
    /// Every code read fails, as for a page the debugger cannot map.
    code_unreadable: bool,
}

impl Process {
    fn function(&mut self, name: &str, start: u64, code: &[u32]) {
        self.functions.push((
            FunctionRange {
                image: "debug.out".to_string(),
                name: Some(name.to_string()),
                start,
                size: Some(code.len() as u64 * 4),
            },
            code.to_vec(),
        ));
    }

    fn thread(&mut self, pc: u64, sp: u64, fp: u64, lr: u64) {
        let register = |name: &str, value| Register {
            name: name.to_string(),
            group: RegisterGroup::System,
            value: RegisterValue::Bits64(value),
            role: None,
        };
        self.registers = vec![
            register("pc", pc),
            register("sp", sp),
            register("fp", fp),
            register("lr", lr),
        ];
    }

    fn walk(&mut self) -> Result<Vec<(u64, Option<String>)>, String> {
        Ok(frames(self, 0, MAX_FRAMES)?
            .into_iter()
            .map(|frame| (frame.pc, frame.function.and_then(|range| range.name)))
            .collect())
    }
}

fn named(pc: u64, name: &str) -> (u64, Option<String>) {
    (pc, Some(name.to_string()))
}

impl Inferior for Process {
    fn pid(&self) -> u32 {
        7
    }
    fn end(&mut self) -> Result<Ending, String> {
        Ok(Ending::Killed)
    }
    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        Ok(Vec::new())
    }
    fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        self.words
            .get(&address)
            .copied()
            .ok_or_else(|| format!("unmapped {address:#x}"))
    }
    fn read_bytes(&mut self, address: u64, _: usize) -> Result<Vec<u8>, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn read_code(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        if self.code_unreadable {
            return Err(format!("no code at {address:#x}"));
        }
        for (range, code) in &self.functions {
            let end = range.start + code.len() as u64 * 4;
            if range.start <= address && address + length as u64 <= end {
                let first = ((address - range.start) / 4) as usize;
                return Ok(code[first..first + length / 4]
                    .iter()
                    .flat_map(|word| word.to_le_bytes())
                    .collect());
            }
        }
        Err(format!("no code at {address:#x}"))
    }
    fn function_at(&mut self, address: u64) -> Result<Option<FunctionRange>, String> {
        Ok(self
            .functions
            .iter()
            .find(|(range, code)| {
                range.start <= address && address < range.start + code.len() as u64 * 4
            })
            .map(|(range, _)| range.clone()))
    }
    fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
        Ok(Vec::new())
    }
    fn data_symbol_at(&mut self, _: u64) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        Ok(Vec::new())
    }
    fn registers(&mut self, _: usize) -> Result<Vec<Register>, String> {
        Ok(self.registers.clone())
    }
    fn set_registers(&mut self, _: usize, _: &[(String, u64)]) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn step(&mut self, _: &[usize], _: u64) -> Result<Vec<Step>, String> {
        Err("read-only".to_string())
    }
    fn restart(&mut self, _: &[(OsString, OsString)]) -> Result<u32, String> {
        Err("read-only".to_string())
    }
    fn entry(&mut self) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn insert_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn remove_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn insert_watchpoint(&mut self, _: u64, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn remove_watchpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn run(&mut self) -> Result<RunStop, String> {
        Err("read-only".to_string())
    }
}

/// `_main` builds a frame record and calls `work`, a compiled frame that
/// saved `lr` at the bottom of its 0x150 bytes.
fn compiled_call() -> Process {
    let mut process = Process::default();
    process.function("_main", 0x1000, &[STP_FP_LR_PRE, MOV_FP_SP, BL, MOV_X0_X1]);
    process.function(
        "_mfb_fn_work",
        0x2000,
        &[SUB_SP_0X150, STR_X30_SP, MOV_X0_X1, BL, MOV_X0_X1],
    );
    process
}

#[test]
fn a_compiled_frame_unwinds_to_main_and_stops_there() {
    let mut process = compiled_call();
    let sp = 0x8000;
    // Paused after `work`'s third instruction; `lr` is saved at `sp`.
    process.thread(0x200c, sp, 0x9000, 0xdead);
    process.words.insert(sp, 0x100c);
    assert_eq!(
        process.walk().unwrap(),
        [named(0x200c, "_mfb_fn_work"), named(0x100c, "_main")]
    );
    // A limit cuts the walk short.
    assert_eq!(frames(&mut process, 0, 1).unwrap().len(), 1);
}

#[test]
fn code_no_function_holds_returns_through_lr_only_while_innermost() {
    let mut process = compiled_call();
    // Paused in code no symbol covers (a library stub), called from `work`:
    // the return address is still in `lr` and `sp` is the CFA.
    let sp = 0x7eb0;
    process.thread(0x5000, sp, 0x9000, 0x2010);
    process.words.insert(sp, 0x100c);
    // `work`'s frame then unwinds from memory, to `_main`.
    let mut wider = process.walk().unwrap();
    assert_eq!(wider.remove(0), (0x5000, None));
    assert_eq!(
        wider,
        [named(0x2010, "_mfb_fn_work"), named(0x100c, "_main")]
    );

    // A caller frame no function holds has no `lr` to fall back on.
    let mut orphan = compiled_call();
    orphan.thread(0x200c, sp, 0x9000, 0xdead);
    orphan.words.insert(sp, 0x6004);
    assert_eq!(
        orphan.walk().unwrap(),
        [named(0x200c, "_mfb_fn_work"), (0x6004, None)]
    );
}

#[test]
fn a_frame_record_unwinds_from_fp_once_sp_is_lost() {
    let mut process = compiled_call();
    // `alloca` saves fp and lr, points fp at them, then moves sp by a
    // register: only `fp` locates the CFA.
    process.function(
        "_alloca_user",
        0x3000,
        &[STP_FP_LR_PRE, MOV_FP_SP, SUB_SP_X16, MOV_X0_X1],
    );
    let fp = 0x7ff0;
    process.thread(0x300c, 0x7000, fp, 0xdead);
    process.words.insert(fp, 0x8800); // the caller's fp
    process.words.insert(fp + 8, 0x2010); // lr, into `work`
    process.words.insert(0x8000, 0x100c); // `work`'s saved lr
    assert_eq!(
        process.walk().unwrap(),
        [
            named(0x300c, "_alloca_user"),
            named(0x2010, "_mfb_fn_work"),
            named(0x100c, "_main"),
        ]
    );
}

#[test]
fn a_frame_whose_cfa_or_return_address_cannot_be_found_ends_the_walk() {
    // `sp` moved by a register before any frame record: no CFA at all.
    let mut lost = Process::default();
    lost.function("_lost", 0x3000, &[SUB_SP_X16, MOV_X0_X1]);
    lost.thread(0x3004, 0x7000, 0x7ff0, 0x2010);
    assert_eq!(lost.walk().unwrap(), [named(0x3004, "_lost")]);

    // A caller frame (no `lr` to read) whose function never saved `lr`.
    let mut unsaved = compiled_call();
    unsaved.function("_leafish", 0x4000, &[SUB_SP_0X10, BL, MOV_X0_X1]);
    unsaved.thread(0x200c, 0x8000, 0x9000, 0xdead);
    unsaved.words.insert(0x8000, 0x4008);
    assert_eq!(
        unsaved.walk().unwrap(),
        [named(0x200c, "_mfb_fn_work"), named(0x4008, "_leafish")]
    );
}

#[test]
fn a_caller_that_is_not_further_up_the_stack_ends_the_walk() {
    // A return address of 0 is the end of the chain.
    let mut zero = compiled_call();
    zero.thread(0x200c, 0x8000, 0x9000, 0xdead);
    zero.words.insert(0x8000, 0);
    assert_eq!(zero.walk().unwrap(), [named(0x200c, "_mfb_fn_work")]);

    // `_flat` keeps `lr` at `sp` without moving it, and the saved return
    // address leads back into `_flat` itself: past the innermost frame, a
    // caller at the same `sp` would repeat forever, so the walk stops.
    let mut flat = Process::default();
    flat.function("_flat", 0x3000, &[STR_X30_SP, BL, MOV_X0_X1]);
    flat.thread(0x3008, 0x8000, 0x9000, 0xdead);
    flat.words.insert(0x8000, 0x3008);
    assert_eq!(
        flat.walk().unwrap(),
        [named(0x3008, "_flat"), named(0x3008, "_flat")]
    );
}

#[test]
fn a_thread_without_the_unwind_registers_is_refused() {
    let mut process = compiled_call();
    process.thread(0x200c, 0x8000, 0x9000, 0xdead);
    process.registers.retain(|register| register.name != "fp");
    assert_eq!(process.walk().unwrap_err(), "thread 0 has no 'fp' register");
    // A 128-bit register is not an address.
    process.registers[0].value = RegisterValue::Bits128(1);
    assert_eq!(process.walk().unwrap_err(), "thread 0 has no 'pc' register");
}

#[test]
fn unreadable_code_or_stack_is_an_error_not_a_short_backtrace() {
    let mut process = compiled_call();
    process.thread(0x200c, 0x8000, 0x9000, 0xdead);
    assert_eq!(process.walk().unwrap_err(), "unmapped 0x8000");
    process.code_unreadable = true;
    assert_eq!(process.walk().unwrap_err(), "no code at 0x2000");
}

#[test]
fn a_signed_return_address_is_stripped_to_a_user_address() {
    let mut process = compiled_call();
    process.thread(0x200c, 0x8000, 0x9000, 0xdead);
    // The pointer-authentication bits sit above every user address.
    process.words.insert(0x8000, 0x002a_0000_0000_0000 | 0x100c);
    assert_eq!(
        process.walk().unwrap(),
        [named(0x200c, "_mfb_fn_work"), named(0x100c, "_main")]
    );
}

#[test]
fn a_frame_at_the_start_of_an_unnamed_function_prints_its_address() {
    let frame = Frame {
        pc: 0x3000,
        function: Some(FunctionRange {
            image: "libfoo.dylib".to_string(),
            name: None,
            start: 0x3000,
            size: None,
        }),
    };
    let none = Collisions::default();
    assert_eq!(
        lines(1, None, "Other", std::slice::from_ref(&frame), false, &none)[1],
        "    frame #0: 0x0000000000003000 libfoo.dylib`0x0000000000003000"
    );
    // Code in no named image is just its function's address.
    let mut bare = frame;
    bare.function.as_mut().unwrap().image.clear();
    assert_eq!(
        lines(1, None, "Other", &[bare], false, &none)[1],
        "    frame #0: 0x0000000000003000 0x0000000000003000"
    );
}
