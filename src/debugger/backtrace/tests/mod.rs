use super::*;

mod effects;
mod walk;

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
