//! Frame-slot promotion (`optimizer::opt2::promote`) declines a whole function
//! whenever a frame access is one it cannot place: the row turns every
//! provably private one-word slot into a register, and "provably" fails the
//! moment any access could reach a slot by a route the row does not model.
//!
//! The row's own tests cover the common refusals; these are the rest — a wide
//! or narrow access, or a frame address handed to a drop helper, that lands
//! outside every registered object; an offset that is not a number; and a
//! stack-pointer copy whose consumer cannot be seen (no destination, the last
//! instruction, or no single-slot helper call before the stream ends). Each
//! must leave the stream exactly as it was and promote nothing.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::CodeInstruction;
use crate::optimizer::opt2::promote::{promote, FrameFacts};
use crate::optimizer::{with_opt_level, OptLevel};

fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut instruction = CodeInstruction::new(op);
    for (name, value) in fields {
        instruction = instruction.physical_field(name, *value);
    }
    instruction
}

fn store(src: &'static str, offset: &'static str) -> CodeInstruction {
    ci(
        "str_u64",
        &[("src", src), ("base", "sp"), ("offset", offset)],
    )
}

fn load(dst: &'static str, offset: &'static str) -> CodeInstruction {
    ci(
        "ldr_u64",
        &[("dst", dst), ("base", "sp"), ("offset", offset)],
    )
}

/// Run the row over `extra` followed by a promotable store/reload pair, with
/// objects at 0 and 8 in a 16-byte frame; return what it promoted and whether
/// the stream changed.
fn run(extra: Vec<CodeInstruction>, trailing: bool) -> (u64, bool) {
    let mut stream = vec![store("%v1", "8"), load("%v2", "8")];
    stream.extend(extra);
    if trailing {
        stream.push(ci("ret", &[]));
    }
    let before: Vec<(CodeOp, Vec<String>)> = stream
        .iter()
        .map(|i| {
            (
                i.op,
                i.fields
                    .iter()
                    .map(|(_, o)| o.rendered().into_owned())
                    .collect(),
            )
        })
        .collect();
    let promoted = with_opt_level(OptLevel(1), || {
        promote(
            &mut stream,
            &FrameFacts {
                object_starts: &[0, 8],
                frame_size: 16,
                zeroed_after_allocation: &[],
            },
        )
    });
    let after: Vec<(CodeOp, Vec<String>)> = stream
        .iter()
        .map(|i| {
            (
                i.op,
                i.fields
                    .iter()
                    .map(|(_, o)| o.rendered().into_owned())
                    .collect(),
            )
        })
        .collect();
    (promoted, before != after)
}

#[test]
fn the_pair_alone_is_promoted() {
    assert_eq!(run(vec![], true), (1, true));
}

#[test]
fn an_access_or_address_outside_every_object_declines_the_function() {
    let wide = ci("ldr_q", &[("dst", "q0"), ("base", "sp"), ("offset", "100")]);
    assert_eq!(
        run(vec![wide], true),
        (0, false),
        "a 16-byte load past the frame"
    );
    let address = vec![
        ci("add_imm", &[("dst", "x0"), ("src", "sp"), ("imm", "100")]),
        ci(
            "bl",
            &[(
                "target",
                crate::codegen::error::constants::DROP_OWNED_STRING_SYMBOL,
            )],
        ),
    ];
    assert_eq!(
        run(address, true),
        (0, false),
        "a drop of a slot past the frame"
    );
}

#[test]
fn a_non_numeric_offset_declines_the_function() {
    let word = store("%v3", "%v9");
    assert_eq!(run(vec![word], true), (0, false), "a full-word store");
    let narrow = ci(
        "ldr_u32",
        &[("dst", "%v4"), ("base", "sp"), ("offset", "%v9")],
    );
    assert_eq!(run(vec![narrow], true), (0, false), "a narrow load");
}

#[test]
fn a_stack_pointer_copy_with_no_visible_consumer_declines_the_function() {
    let no_dst = ci("add_imm", &[("src", "sp"), ("imm", "0")]);
    assert_eq!(run(vec![no_dst], true), (0, false), "no destination");
    let last = ci("add_imm", &[("dst", "%v5"), ("src", "sp"), ("imm", "0")]);
    assert_eq!(
        run(vec![last], false),
        (0, false),
        "the stream's last instruction"
    );
    let no_call = ci("add_imm", &[("dst", "x0"), ("src", "sp"), ("imm", "8")]);
    assert_eq!(
        run(
            vec![no_call, ci("mov", &[("dst", "%v6"), ("src", "%v2")])],
            false
        ),
        (0, false),
        "the stream ends before any helper call"
    );
}
