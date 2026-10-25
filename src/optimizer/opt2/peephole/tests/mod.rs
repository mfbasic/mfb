use super::*;
fn op(name: &str) -> CodeInstruction {
    CodeInstruction::new(name)
}

/// An instruction whose fields the classifier cannot read FLUSHES the slot
/// map, rather than being skipped.
///
/// `forward_stores_to_loads` answers a load from a remembered store, so
/// everything it cannot read has to invalidate what it remembers. Each arm
/// below has its own "and otherwise" case, and all of them were unexecuted
/// because a real emitter never omits a field:
///
///   * a `str` to `sp` with no `offset`/`src` to key the slot by,
///   * an `ldr` from `sp` with no `offset` (a bare `dst` still defines it),
///     and one with neither,
///   * a `mul` (or any implicit-clobber op) with no `dst`,
///   * a float conversion with no `dst`,
///   * and a `DefDst` whose `dst` operand is missing, which clears the map
///     rather than invalidating one register.
///
/// The property is one sentence: **a store the pass cannot read must not
/// leave a stale answer behind it.** If any of these were skipped instead of
/// flushing, the load below would be forwarded from a slot the unreadable
/// store may have just overwritten — the pass would replace a load with a
/// register holding the wrong value, and every later instruction would look
/// correct.
#[test]
fn an_unreadable_instruction_flushes_the_forwarded_slots() {
    for (what, opaque) in [
        (
            "a store to sp with no offset",
            op("str_u64").physical_field("base", "sp"),
        ),
        (
            "a load from sp with no dst",
            op("ldr_u64").physical_field("base", "sp"),
        ),
        (
            "a multiply with no dst",
            op("mul")
                .physical_field("lhs", "x1")
                .physical_field("rhs", "x2"),
        ),
        (
            "a float conversion with no dst",
            op("fcvtzs_x_from_d").physical_field("src", "d1"),
        ),
    ] {
        let mut instructions = vec![
            op("str_u64")
                .physical_field("src", "x8")
                .physical_field("base", "sp")
                .field("offset", "16"),
            opaque,
            op("ldr_u64")
                .physical_field("dst", "x10")
                .physical_field("base", "sp")
                .field("offset", "16"),
            op("ret"),
        ];
        forward_stores_to_loads(&mut instructions, false);
        assert_eq!(
            instructions[2].op,
            CodeOp::LdrU64,
            "{what}: the pass could not read it, so it cannot know the slot \
             at sp+16 still holds x8 -- forwarding the load here replaces it \
             with a register holding whatever was there BEFORE, and every \
             later instruction still looks correct"
        );
    }
}

/// A load from `sp` with a `dst` but no offset defines its register and
/// nothing more.
///
/// The one arm among the unreadable shapes that must NOT flush, and the
/// reason is the difference between the two halves of the map: a load reads
/// memory and writes a register, so the slots stay valid however unreadable
/// its offset is. Invalidating the whole map here would be sound but would
/// silently give up the forwarding for every frame that contains one -- and
/// nothing measures that, which is how a conservative arm written one line
/// too wide survives.
#[test]
fn a_load_with_no_offset_invalidates_only_its_own_register() {
    let mut instructions = vec![
        op("str_u64")
            .physical_field("src", "x8")
            .physical_field("base", "sp")
            .field("offset", "16"),
        op("ldr_u64")
            .physical_field("base", "sp")
            .physical_field("dst", "x9"),
        op("ldr_u64")
            .physical_field("dst", "x10")
            .physical_field("base", "sp")
            .field("offset", "16"),
        op("ret"),
    ];
    forward_stores_to_loads(&mut instructions, false);
    assert_eq!(
        instructions[2].op,
        CodeOp::Mov,
        "the middle instruction writes x9 and reads memory; the slot at sp+16 \
         still holds x8, so the load below is still forwarded"
    );
}

/// A store whose offset is not a number keeps every remembered slot.
///
/// The invalidation compares 8-byte ranges numerically. A symbolic offset
/// (a label, an unresolved frame reference) is not range-comparable, so the
/// pass falls back to exact-string keying and keeps the slots it cannot
/// prove overlap -- the opposite of the flush above, and correct for the same
/// reason: it is the answer that assumes less.
#[test]
fn a_non_numeric_store_offset_is_not_range_compared() {
    let mut instructions = vec![
        op("str_u64")
            .physical_field("src", "x8")
            .physical_field("base", "sp")
            .field("offset", "16"),
        op("str_u64")
            .physical_field("src", "x9")
            .physical_field("base", "sp")
            .field("offset", "#frame_scratch"),
        op("ldr_u64")
            .physical_field("dst", "x10")
            .physical_field("base", "sp")
            .field("offset", "16"),
        op("ret"),
    ];
    forward_stores_to_loads(&mut instructions, false);
    assert_eq!(
        instructions[2].op,
        CodeOp::Mov,
        "sp+16 was stored and the symbolic offset is a different key, so the \
         load is still forwarded from x8"
    );
}

/// The result shuttle `fmov xN,dM ; str xN,[sp,#k]` collapses to `str d dM`
/// when `xN` is dead afterwards, and the reload `ldr xN ; fmov dM,xN`
/// collapses to `ldr d dM` likewise.
#[test]
fn folds_dead_result_and_operand_shuttles() {
    let mut instructions = vec![
        op("fmov_x_from_d")
            .physical_field("dst", "x8")
            .physical_field("src", "d11"),
        op("str_u64")
            .physical_field("src", "x8")
            .physical_field("base", "sp")
            .field("offset", "1120"),
        // x8 redefined here, so it is dead after the store above.
        op("ldr_u64")
            .physical_field("dst", "x8")
            .physical_field("base", "sp")
            .field("offset", "80"),
        op("fmov_d_from_x")
            .physical_field("dst", "d9")
            .physical_field("src", "x8"),
        op("ret"),
    ];
    // AArch64 spellings (`x8`, `d9`), so the AArch64 model supplies the
    // caller-saved set the liveness reads (bug-350).
    remove_fp_shuttles(
        &mut instructions,
        &crate::arch::aarch64::regmodel::Aarch64RegisterModel,
    );
    assert_eq!(instructions.len(), 3);
    assert_eq!(instructions[0].op, CodeOp::StrD);
    assert_eq!(instructions[0].get("src").as_deref(), Some("d11"));
    assert_eq!(instructions[0].get("offset").as_deref(), Some("1120"));
    assert_eq!(instructions[1].op, CodeOp::LdrD);
    assert_eq!(instructions[1].get("dst").as_deref(), Some("d9"));
    assert_eq!(instructions[1].get("offset").as_deref(), Some("80"));
    assert_eq!(instructions[2].op, CodeOp::Ret);
}

/// When the shuttle GPR is still live after the store (read by a later
/// instruction before being redefined), the fold is unsound and must not fire.
#[test]
fn keeps_shuttle_when_gpr_still_live() {
    let mut instructions = vec![
        op("fmov_x_from_d")
            .physical_field("dst", "x8")
            .physical_field("src", "d11"),
        op("str_u64")
            .physical_field("src", "x8")
            .physical_field("base", "sp")
            .field("offset", "1120"),
        // x8 is read here, so it must survive the store.
        op("add")
            .physical_field("dst", "x9")
            .physical_field("lhs", "x8")
            .physical_field("rhs", "x8"),
        op("ret"),
    ];
    let before = instructions.len();
    // AArch64 spellings (`x8`, `d9`), so the AArch64 model supplies the
    // caller-saved set the liveness reads (bug-350).
    remove_fp_shuttles(
        &mut instructions,
        &crate::arch::aarch64::regmodel::Aarch64RegisterModel,
    );
    assert_eq!(instructions.len(), before);
    assert_eq!(instructions[0].op, CodeOp::FMovXFromD);
    assert_eq!(instructions[1].op, CodeOp::StrU64);
}

/// Full-slot, non-overlapping sp stores still forward: the `ldr` from `#8`
/// becomes a `mov` from the register that stored it. (This is the today path —
/// proves the overlap guard did not disturb byte-identical behaviour.)
#[test]
fn forwards_disjoint_full_slot_store() {
    let mut instructions = vec![
        op("str_u64")
            .physical_field("src", "x10")
            .physical_field("base", "sp")
            .field("offset", "8"),
        op("str_u64")
            .physical_field("src", "x11")
            .physical_field("base", "sp")
            .field("offset", "16"),
        op("ldr_u64")
            .physical_field("dst", "x8")
            .physical_field("base", "sp")
            .field("offset", "8"),
    ];
    forward_stores_to_loads(&mut instructions, false);
    assert_eq!(instructions[2].op, CodeOp::Mov);
    assert_eq!(instructions[2].get("dst").as_deref(), Some("x8"));
    assert_eq!(instructions[2].get("src").as_deref(), Some("x10"));
}

/// bug-284 C8: on x86-64 `mul`/`umulh`/`smulh`/`sdiv`/`udiv`/`msub` expand to
/// sequences that clobber rdx:rax in addition to their named `dst`, which the
/// arch-neutral `DefDst` classification ("defines exactly dst") cannot express.
/// A slot whose forwarding source lived in rax/rdx across one of these was not
/// invalidated, so the reload was forwarded to a clobbered register.
#[test]
fn x86_implicit_clobber_ops_flush_forwarding_state() {
    let stream = || {
        vec![
            op("str_u64")
                .physical_field("src", "rax")
                .physical_field("base", "sp")
                .field("offset", "8"),
            // dst is neither rax nor rdx, so `DefDst` invalidates nothing --
            // but the x86 expansion clobbers rax regardless.
            op("udiv")
                .physical_field("dst", "r10")
                .physical_field("lhs", "r11")
                .physical_field("rhs", "r12"),
            op("ldr_u64")
                .physical_field("dst", "r14")
                .physical_field("base", "sp")
                .field("offset", "8"),
        ]
    };

    // On x86 the reload must survive as a real memory load.
    let mut instructions = stream();
    forward_stores_to_loads(&mut instructions, true);
    assert_eq!(
        instructions[2].op,
        CodeOp::LdrU64,
        "x86 must not forward across an op that clobbers rax"
    );

    // On aarch64/riscv64 these ops really do define only `dst`, so the
    // forwarding stays available -- the flush is x86-specific, not a blanket
    // pessimization.
    let mut instructions = stream();
    forward_stores_to_loads(&mut instructions, false);
    assert_eq!(instructions[2].op, CodeOp::Mov);
    assert_eq!(instructions[2].get("src").as_deref(), Some("rax"));
}

/// A later store that *partially overwrites* an 8-byte slot (offset `#12`
/// clobbers bytes 12..16 of the value stored at `#8`) must invalidate that slot,
/// so the reload from `#8` is NOT forwarded to the stale register. Without the
/// range-overlap invalidation this would wrongly rewrite the load to `mov x8,x10`.
#[test]
fn does_not_forward_partially_overwritten_slot() {
    let mut instructions = vec![
        op("str_u64")
            .physical_field("src", "x10")
            .physical_field("base", "sp")
            .field("offset", "8"),
        // Overlaps bytes 12..16 of the store above (|12 - 8| = 4 < 8).
        op("str_u64")
            .physical_field("src", "x11")
            .physical_field("base", "sp")
            .field("offset", "12"),
        op("ldr_u64")
            .physical_field("dst", "x8")
            .physical_field("base", "sp")
            .field("offset", "8"),
    ];
    forward_stores_to_loads(&mut instructions, false);
    // The load survives as a real memory reload.
    assert_eq!(instructions[2].op, CodeOp::LdrU64);
    assert_eq!(instructions[2].get("dst").as_deref(), Some("x8"));
    assert_eq!(instructions[2].get("offset").as_deref(), Some("8"));
}

fn pair(
    op_name: &str,
    first: &str,
    second: &str,
    offset: &str,
    addressing: &str,
) -> CodeInstruction {
    let (a, b) = if op_name == "ldp" {
        ("dst", "dst2")
    } else {
        ("src", "src2")
    };
    op(op_name)
        .physical_field(a, first)
        .physical_field(b, second)
        .physical_field("base", "sp")
        .field("offset", offset)
        .field("addressing", addressing)
        .field("width", "8")
}

fn reload(dst: &str, offset: &str) -> CodeInstruction {
    op("ldr_u64")
        .physical_field("dst", dst)
        .physical_field("base", "sp")
        .field("offset", offset)
}

/// plan-171-A: an `stp` to `sp` is its two slot stores, so a reload of either
/// half forwards from the register that stored it.
#[test]
fn an_sp_store_pair_forwards_both_halves() {
    let mut instructions = vec![
        pair("stp", "x10", "x11", "16", "offset"),
        reload("x8", "16"),
        reload("x9", "24"),
    ];
    forward_stores_to_loads(&mut instructions, false);
    assert_eq!(instructions[1].op, CodeOp::Mov);
    assert_eq!(instructions[1].get("src").as_deref(), Some("x10"));
    assert_eq!(instructions[2].op, CodeOp::Mov);
    assert_eq!(instructions[2].get("src").as_deref(), Some("x11"));
}

/// An `ldp` defines both its destinations: a slot stored from either one is no
/// longer forwardable. A write-back pair moves `sp`, so it flushes everything.
#[test]
fn a_load_pair_invalidates_both_destinations_and_a_write_back_pair_flushes() {
    let mut instructions = vec![
        op("str_u64")
            .physical_field("src", "x10")
            .physical_field("base", "sp")
            .field("offset", "8"),
        op("str_u64")
            .physical_field("src", "x11")
            .physical_field("base", "sp")
            .field("offset", "32"),
        op("str_u64")
            .physical_field("src", "x12")
            .physical_field("base", "sp")
            .field("offset", "48"),
        pair("ldp", "x11", "x10", "64", "offset"),
        reload("x8", "8"),
        reload("x9", "32"),
        reload("x7", "48"),
        pair("stp", "x21", "x22", "-16", "pre"),
        reload("x6", "48"),
    ];
    forward_stores_to_loads(&mut instructions, false);
    assert_eq!(
        instructions[4].op,
        CodeOp::LdrU64,
        "x10 was reloaded by the ldp"
    );
    assert_eq!(
        instructions[5].op,
        CodeOp::LdrU64,
        "x11 was reloaded by the ldp"
    );
    assert_eq!(instructions[6].op, CodeOp::Mov, "x12 still holds slot 48");
    assert_eq!(
        instructions[8].op,
        CodeOp::LdrU64,
        "the write-back pair flushed"
    );
}
