//! plan-171-A: the `stp`/`ldp` pair forms. Oracle words: Apple clang 17
//! `clang -c -arch arm64` of the spelled lines, read back with `otool -t`.

use super::*;

fn pair(
    op: &str,
    first: &str,
    second: &str,
    base: &str,
    offset: &str,
    mode: &str,
    width: &str,
) -> CodeInstruction {
    let (a, b) = if op == "ldp" {
        ("dst", "dst2")
    } else {
        ("src", "src2")
    };
    CodeInstruction::new(op)
        .physical_field(a, first)
        .physical_field(b, second)
        .physical_field("base", base)
        .field("offset", offset)
        .field("addressing", mode)
        .field("width", width)
}

fn emit(instruction: &CodeInstruction) -> Result<u32, String> {
    let mut encoder = fresh_encoder();
    encoder.emit_instruction(instruction)?;
    assert_eq!(encoder.text.len(), 4, "a pair is one word");
    assert_eq!(instruction_size(instruction).unwrap(), 4);
    Ok(u32::from_le_bytes(encoder.text[..4].try_into().unwrap()))
}

/// Every width × addressing mode × load/store, including the imm7 boundaries
/// (X −512 and 504, Q −1024 and 1008) and `xzr` as stored data.
#[test]
fn encodes_every_pair_form() {
    let cases: &[(&str, [&str; 6], u32, &str)] = &[
        (
            "stp",
            ["x21", "x22", "sp", "16", "offset", "8"],
            0xa901_5bf5,
            "stp x21, x22, [sp, #16]",
        ),
        (
            "stp",
            ["x0", "x1", "x2", "0", "offset", "8"],
            0xa900_0440,
            "stp x0, x1, [x2]",
        ),
        (
            "stp",
            ["x21", "lr", "sp", "-512", "pre", "8"],
            0xa9a0_7bf5,
            "stp x21, x30, [sp, #-512]!",
        ),
        (
            "stp",
            ["x21", "x30", "sp", "-32", "pre", "8"],
            0xa9be_7bf5,
            "stp x21, x30, [sp, #-32]!",
        ),
        (
            "stp",
            ["x3", "x4", "x5", "504", "offset", "8"],
            0xa91f_90a3,
            "stp x3, x4, [x5, #504]",
        ),
        (
            "stp",
            ["x3", "x4", "x5", "-512", "offset", "8"],
            0xa920_10a3,
            "stp x3, x4, [x5, #-512]",
        ),
        (
            "stp",
            ["xzr", "xzr", "x9", "8", "offset", "8"],
            0xa900_fd3f,
            "stp xzr, xzr, [x9, #8]",
        ),
        (
            "stp",
            ["x7", "x8", "x9", "16", "post", "8"],
            0xa881_2127,
            "stp x7, x8, [x9], #16",
        ),
        (
            "ldp",
            ["x21", "x22", "sp", "16", "offset", "8"],
            0xa941_5bf5,
            "ldp x21, x22, [sp, #16]",
        ),
        (
            "ldp",
            ["x21", "x30", "sp", "32", "post", "8"],
            0xa8c2_7bf5,
            "ldp x21, x30, [sp], #32",
        ),
        (
            "ldp",
            ["x21", "x30", "sp", "504", "post", "8"],
            0xa8df_fbf5,
            "ldp x21, x30, [sp], #504",
        ),
        (
            "ldp",
            ["x3", "x4", "x5", "-512", "offset", "8"],
            0xa960_10a3,
            "ldp x3, x4, [x5, #-512]",
        ),
        (
            "ldp",
            ["x3", "x4", "x5", "504", "pre", "8"],
            0xa9df_90a3,
            "ldp x3, x4, [x5, #504]!",
        ),
        (
            "ldp",
            ["x1", "x2", "x2", "0", "offset", "8"],
            0xa940_0841,
            "ldp x1, x2, [x2]",
        ),
        (
            "stp",
            ["d8", "d9", "sp", "32", "offset", "16"],
            0xad01_27e8,
            "stp q8, q9, [sp, #32]",
        ),
        (
            "stp",
            ["q8", "q9", "sp", "-1024", "pre", "16"],
            0xada0_27e8,
            "stp q8, q9, [sp, #-1024]!",
        ),
        (
            "ldp",
            ["d8", "d9", "sp", "1008", "post", "16"],
            0xacdf_a7e8,
            "ldp q8, q9, [sp], #1008",
        ),
        (
            "ldp",
            ["v14", "v15", "x3", "-16", "offset", "16"],
            0xad7f_bc6e,
            "ldp q14, q15, [x3, #-16]",
        ),
        (
            "stp",
            ["q0", "q31", "x1", "1008", "offset", "16"],
            0xad1f_fc20,
            "stp q0, q31, [x1, #1008]",
        ),
    ];
    for (op, [first, second, base, offset, mode, width], word, spelling) in cases {
        let instruction = pair(op, first, second, base, offset, mode, width);
        assert_eq!(emit(&instruction).unwrap(), *word, "{spelling}");
    }
}

/// Every form the producers must never build is an encode error, not a wrong word.
#[test]
fn refuses_unencodable_and_unpredictable_pairs() {
    let refused: &[(&str, [&str; 6], &str)] = &[
        ("stp", ["x1", "x2", "sp", "4", "offset", "8"], "unaligned"),
        ("stp", ["x1", "x2", "sp", "-520", "offset", "8"], "imm7"),
        ("ldp", ["x1", "x2", "sp", "512", "offset", "8"], "imm7"),
        ("stp", ["q1", "q2", "sp", "8", "offset", "16"], "unaligned"),
        ("stp", ["q1", "q2", "sp", "-1040", "pre", "16"], "imm7"),
        ("ldp", ["q1", "q2", "sp", "1024", "post", "16"], "imm7"),
        ("ldp", ["x1", "x1", "sp", "0", "offset", "8"], "twice"),
        ("ldp", ["q3", "q3", "x0", "0", "offset", "16"], "twice"),
        ("ldp", ["x2", "x3", "x2", "16", "post", "8"], "write-back"),
        ("ldp", ["x3", "x2", "x2", "-16", "pre", "8"], "write-back"),
        ("stp", ["x2", "x3", "x2", "-16", "pre", "8"], "write-back"),
        ("stp", ["sp", "x3", "x2", "0", "offset", "8"], "XZR"),
        (
            "stp",
            ["x1", "x3", "xzr", "0", "offset", "8"],
            "zero register",
        ),
        (
            "stp",
            ["x1", "x3", "x2", "0", "sideways", "8"],
            "addressing",
        ),
        ("stp", ["x1", "x3", "x2", "0", "offset", "4"], "width"),
    ];
    for (op, [first, second, base, offset, mode, width], why) in refused {
        let error = emit(&pair(op, first, second, base, offset, mode, width)).unwrap_err();
        assert!(
            error.contains(why),
            "{op} {first},{second},[{base},{offset}] {mode}: {error}"
        );
    }
}

/// A write-back through `sp` may name any data register: register 31 there is
/// XZR, never the base (the frame's `stp x21, x30, [sp, #-16]!`).
#[test]
fn write_back_through_sp_is_not_a_base_conflict() {
    let word = emit(&pair("ldp", "x21", "x22", "sp", "16", "post", "8")).unwrap();
    assert_eq!(word, 0xa8c1_5bf5);
}
