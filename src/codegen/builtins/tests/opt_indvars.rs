//! Induction-variable merging (`optimizer::opt2::indvars`) past the loop: a
//! use of the duplicate counter in a block the increments' block dominates —
//! the loop's exit — reads the survivor too, not only the uses after the
//! increments in their own block.

use crate::codegen::engine::types::CodeInstruction;
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

fn run(stream: &mut [CodeInstruction]) {
    let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    with_opt_level(OptLevel(3), || {
        crate::optimizer::opt2::indvars::simplify(stream, &model)
    });
}

/// Two counters from the same start stepping by 1 in the loop's one block; the
/// second is stored inside the loop and again in the exit block.
fn twin_counters_used_after_the_loop() -> Vec<CodeInstruction> {
    vec![
        /* 0 */ ci("mov", &[("dst", "%v1"), ("src", "%v9")]),
        /* 1 */ ci("mov", &[("dst", "%v2"), ("src", "%v9")]),
        /* 2 */ ci("label", &[("name", "head")]),
        /* 3 */ ci("add_imm", &[("dst", "%v1"), ("src", "%v1"), ("imm", "1")]),
        /* 4 */ ci("add_imm", &[("dst", "%v2"), ("src", "%v2"), ("imm", "1")]),
        /* 5 */ store("%v2", "8"),
        /* 6 */ ci("cmp", &[("lhs", "%v1"), ("rhs", "%v8")]),
        /* 7 */ ci("b.ne", &[("target", "head")]),
        /* 8 */ ci("label", &[("name", "exit")]),
        /* 9 */ store("%v2", "16"),
        /* 10 */ ci("ret", &[]),
    ]
}

#[test]
fn a_use_in_a_block_the_increments_dominate_reads_the_survivor() {
    let mut stream = twin_counters_used_after_the_loop();
    run(&mut stream);
    assert_eq!(
        stream[5].get("src").as_deref(),
        Some("%v1"),
        "the in-loop store reads the survivor"
    );
    assert_eq!(
        stream[9].get("src").as_deref(),
        Some("%v1"),
        "the loop's exit is reached only through the increments' block, so \
         the duplicate's value there is the survivor's"
    );
}
