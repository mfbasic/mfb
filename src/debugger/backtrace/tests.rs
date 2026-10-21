use super::*;
use crate::debugger::arm64::AARCH64;

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
    let code = words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect::<Vec<_>>();
    state_at(&AARCH64, &code, 0x1000, 0x1000, Some(0x2000))
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

// --- riscv64: real code, through the riscv64 descriptor ---------------------

/// `_mfb_fn_isPrime` of `examples/debug` built for linux-riscv64 (`debug-glibc.out`),
/// from its start at 0x16b0 through its first `ret` and one instruction past it.
const IS_PRIME: &[&str] = &[
    "f3010113", "00113023", "00a13823", "01013e03", "01c13c23", "00200e13", "03c13023", "01813e03",
    "02013e83", "03c13423", "03d13823", "02813e03", "03013e83", "01de4463", "01c0006f", "00000e13",
    "000e0593", "00000513", "00013083", "0d010113", "00008067", "00200e13",
];
/// A glibc 2.41 `libc.so.6` function at 0x27e36, to 0x27f3c: `c.addi16sp`, `c.sdsp`
/// saves of `s0` and `ra`, `c.addi4spn s0,sp,144`, an early epilogue, then
/// `sub sp,sp,a5` (a stack allocation of a size known only at run time).
const PATH_FUNCTION: &[&str] = &[
    "7175", "e122", "fca6", "f8ca", "f4ce", "f0d2", "ecd6", "e4de", "e0e2", "e506", "e8da", "fc66",
    "f86a", "f46e", "0900", "6504", "0012fc17", "032c3c03", "000c3703", "f8e43423", "4701",
    "0004c783", "01054a03", "01154983", "fd17879b", "00053903", "8bae", "8ab2", "0007869b", "eb81",
    "0014c783", "fd17879b", "0007869b", "c3e1", "c2f1", "00094783", "fd17879b", "0007869b", "eb81",
    "00194783", "fd17879b", "0007869b", "cbb1", "cea1", "8756", "85ca", "8526", "f8040693",
    "f7840613", "4b1000ef", "8aaa", "4b01", "c979", "016bb023", "f8843703", "000c3783", "8fb9",
    "4701", "1e079d63", "f7040113", "60aa", "8556", "640a", "74e6", "7946", "79a6", "7a06", "6ae6",
    "6b46", "6ba6", "6c06", "7ce2", "7d42", "7da2", "6149", "8082", "00294683", "f6d5", "0012f797",
    "0467b783", "9792", "639c", "639c", "0a87bc83", "8566", "557580ef", "01250793", "9bc1",
    "40f10133", "8b2a", "85e6", "850a", "865a", "fffff097", "5bc080e7",
];
/// A glibc function at 0x27c04, to 0x27c74, that saves `ra` after its body began
/// (`sd ra,72(sp)` at 0x27c1e) and returns early at 0x27c54.
const LATE_RA: &[&str] = &[
    "715d", "e0a2", "fc26", "0800", "0012f497", "27c4b483", "862a", "609c", "f43e", "4781", "8522",
    "e486", "2b9070ef", "57fd", "cd09", "8522", "4601", "002c", "208000ef", "87aa", "8522", "843e",
    "4a9070ef", "ec09", "67a2", "76a2", "6098", "8f35", "4681", "e715", "60a6", "6406", "74e2",
    "853e", "6161", "8082", "347d", "4785", "0087f463", "57fd", "b7c5", "0012f797", "1667b783",
    "4759", "9792", "c398", "57fd", "b7f9",
];

/// The bytes of instructions as `objdump` prints them: 8 hex digits for a
/// 4-byte one, 4 for a 2-byte one.
fn riscv64_code(words: &[&str]) -> Vec<u8> {
    words
        .iter()
        .flat_map(|word| {
            let value = u32::from_str_radix(word, 16).expect("hex");
            value.to_le_bytes()[..word.len() / 2].to_vec()
        })
        .collect()
}

/// The frame state at `pc` of the function `words` holds, starting at
/// `start`, `size` bytes long.
fn riscv64_state(words: &[&str], start: u64, size: u64, pc: u64) -> FrameState {
    let code = riscv64_code(words);
    state_at(
        &crate::debugger::riscv64::RISCV64,
        &code[..(pc - start) as usize],
        start,
        start,
        Some(start + size),
    )
}

#[test]
fn a_riscv64_compiled_frame_unwinds() {
    let at = |pc| riscv64_state(IS_PRIME, 0x16b0, 0x2c4, pc);
    let body = FrameState {
        sp: Some(-208),
        lr: Some(-208),
        ..FrameState::ENTRY
    };
    // After `addi sp,sp,-208` and `sd ra,0(sp)`.
    assert_eq!(at(0x16b8), body);
    // The epilogue: `ld ra,0(sp)` gives the return address back to `ra`,
    // `addi sp,sp,208` the stack.
    assert_eq!(
        at(0x16fc),
        FrameState {
            sp: Some(-208),
            ..FrameState::ENTRY
        }
    );
    assert_eq!(at(0x1700), FrameState::ENTRY);
    // Past the `ret`, the rest of the body runs in the prologue's frame.
    assert_eq!(at(0x1704), body);
}

#[test]
fn a_riscv64_library_frame_with_a_frame_pointer_unwinds() {
    let at = |pc| riscv64_state(PATH_FUNCTION, 0x27e36, 0x400, pc);
    // `s0` and `ra` saved, `s0` pointed at the CFA.
    let framed = FrameState {
        sp: Some(-144),
        lr: Some(-8),
        fp_saved: Some(-16),
        fp: Some(0),
    };
    assert_eq!(at(0x27e54), framed);
    // `addi sp,s0,-144` then the restores and `c.addi16sp sp,144`.
    assert_eq!(at(0x27ee4), framed);
    assert_eq!(at(0x27f02), FrameState::ENTRY);
    // Past the `ret`, after `sub sp,sp,a5`: only the frame pointer finds the
    // CFA, and the saves are where the prologue put them.
    assert_eq!(at(0x27f3c), FrameState { sp: None, ..framed });
}

#[test]
fn a_riscv64_save_after_the_body_began_counts_after_an_early_return() {
    let at = |pc| riscv64_state(LATE_RA, 0x27c04, 0x74, pc);
    let saved = FrameState {
        sp: Some(-80),
        lr: Some(-8),
        fp_saved: Some(-16),
        fp: Some(-64),
    };
    assert_eq!(at(0x27c20), saved);
    // Past the early `ret` at 0x27c54, `ra` is still where 0x27c1e put it.
    assert_eq!(at(0x27c74), saved);
}

#[test]
fn a_riscv64_frame_before_its_prologue_ran() {
    // At the first instruction the return address is still in `ra`.
    assert_eq!(
        riscv64_state(LATE_RA, 0x27c04, 0x74, 0x27c04),
        FrameState::ENTRY
    );
    // After `c.addi16sp sp,-80` alone: the stack moved, `ra` not yet saved.
    assert_eq!(
        riscv64_state(LATE_RA, 0x27c04, 0x74, 0x27c06),
        FrameState {
            sp: Some(-80),
            ..FrameState::ENTRY
        }
    );
}
