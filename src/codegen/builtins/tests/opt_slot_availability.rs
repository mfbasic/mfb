//! Stack-slot value availability (`optimizer::opt2::plans::memory`), the fact
//! base store-to-load forwarding and redundant-load elimination share, driven
//! directly over hand-built streams.
//!
//! The contract is the module's: a load is a forwarding candidate only when
//! its slot provably holds a value that a single-definition register still
//! carries; a join keeps a slot only where every predecessor agrees on it; the
//! Store/Load PRE candidate needs every predecessor but exactly one to agree,
//! and that one must reach the join unconditionally.

use crate::codegen::engine::regalloc;
use crate::codegen::engine::regalloc::analysis::build_cfg;
use crate::codegen::engine::types::CodeInstruction;
use crate::optimizer::opt2::plans::memory::{forwardable_loads, Origin};
use crate::optimizer::opt2::plans::ssa;

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

/// `(load index, holder, came from a store)` per forwardable load, and
/// `(load index, gap block, holder, offset)` per partially-available one.
type Facts = (Vec<(usize, String, bool)>, Vec<(usize, usize, String, i64)>);

fn analyze(stream: &[CodeInstruction]) -> Facts {
    let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    let models = regalloc::class_models(&model);
    let blocks = build_cfg(stream);
    let overlay = ssa::build(stream, &blocks, &models);
    let (found, partial) = forwardable_loads(stream, &blocks, &models, &overlay);
    (
        found
            .into_iter()
            .map(|f| {
                (
                    f.inst,
                    f.available.holder.rendered().into_owned(),
                    f.available.origin == Origin::Store,
                )
            })
            .collect(),
        partial
            .into_iter()
            .map(|p| (p.inst, p.gap, p.holder.rendered().into_owned(), p.offset))
            .collect(),
    )
}

#[test]
fn an_empty_stream_has_no_facts() {
    assert_eq!(analyze(&[]), (vec![], vec![]));
}

/// Within a block: a reload after a store is store-to-load forwarding, and a
/// second reload is redundant-load elimination of the first.
#[test]
fn a_reload_reads_the_register_that_put_the_value_there() {
    let stream = vec![
        ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
        ),
        store("%v1", "8"),
        load("%v2", "8"),
        load("%v3", "8"),
        store("%v3", "16"),
        ci("ret", &[]),
    ];
    let (found, partial) = analyze(&stream);
    assert_eq!(
        found,
        vec![(2, "%v1".to_string(), true), (3, "%v2".to_string(), false)]
    );
    assert!(partial.is_empty());
}

/// A diamond whose arms store the slot from different registers: the join
/// keeps nothing (the arms disagree), and the PRE test declines too.
#[test]
fn arms_that_disagree_leave_the_join_nothing() {
    let stream = vec![
        ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
        ),
        ci(
            "mov_imm",
            &[("dst", "%v3"), ("type", "Integer"), ("value", "6")],
        ),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "arm")]),
        store("%v1", "8"),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "arm")]),
        store("%v3", "8"),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "join")]),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
    ];
    assert_eq!(analyze(&stream), (vec![], vec![]));
}

/// The PRE candidate needs exactly one predecessor without the value: two
/// gaps decline.
#[test]
fn two_predecessors_without_the_value_are_not_a_pre_candidate() {
    let stream = vec![
        ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
        ),
        ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
        ci("b.eq", &[("target", "a1")]),
        ci("cmp_imm", &[("lhs", "%v8"), ("rhs", "0")]),
        ci("b.eq", &[("target", "a2")]),
        store("%v1", "8"),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "a1")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "a2")]),
        ci("b", &[("target", "join")]),
        ci("label", &[("name", "join")]),
        load("%v2", "8"),
        store("%v2", "16"),
        ci("ret", &[]),
    ];
    assert_eq!(analyze(&stream), (vec![], vec![]));
}

/// One arm leaves the value in `%v1`, the other has nothing: the PRE
/// candidate names the gap arm — unless `%v1` is redefined after the store,
/// when the register no longer holds what the slot does.
#[test]
fn a_single_gap_is_a_pre_candidate_only_for_a_single_definition_holder() {
    let diamond = |redefine: bool| {
        let mut stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
            ),
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "arm")]),
            store("%v1", "8"),
        ];
        if redefine {
            stream.push(ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "6")],
            ));
        }
        stream.extend([
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "arm")]),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "join")]),
            load("%v2", "8"),
            store("%v2", "16"),
            ci("ret", &[]),
        ]);
        stream
    };
    let (found, partial) = analyze(&diamond(false));
    assert!(found.is_empty());
    assert_eq!(partial, vec![(8, 2, "%v1".to_string(), 8)]);
    assert_eq!(analyze(&diamond(true)), (vec![], vec![]));
}
