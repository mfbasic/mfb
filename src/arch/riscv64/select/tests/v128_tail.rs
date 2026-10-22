//! plan-32-D: the v128 run `select_riscv64` accumulates is flushed when a
//! non-v128 op arrives — and also when the instruction list simply ends.

use super::*;
use crate::codegen::engine::mir::{MirInstruction, MirOp};

fn mir(op: MirOp, fields: &[(&'static str, &str)]) -> MirInstruction {
    MirInstruction {
        op,
        fields: fields.iter().map(|(k, v)| (*k, (*v).into())).collect(),
        source: None,
    }
}

fn rendered(out: &[CodeInstruction]) -> Vec<String> {
    out.iter()
        .map(|instruction| {
            let fields: Vec<String> = instruction
                .fields
                .iter()
                .map(|(key, value)| format!("{key}={}", value.render()))
                .collect();
            format!("{} {}", instruction.op.mnemonic(), fields.join(" "))
        })
        .collect()
}

/// A run of RVV-lowerable v128 ops that ends the list is emitted exactly as
/// the same run is when a `ret` follows it: the guard, the RVV arm and the
/// scalar arm are all there, nothing is dropped for want of a closing op.
#[test]
fn a_v128_run_that_ends_the_instruction_list_is_still_emitted() {
    let run = || {
        vec![
            mir(
                MirOp::FAddV,
                &[("dst", "%f0"), ("lhs", "%f1"), ("rhs", "%f2")],
            ),
            mir(
                MirOp::FMulV,
                &[("dst", "%f3"), ("lhs", "%f0"), ("rhs", "%f2")],
            ),
        ]
    };
    let tail = select_riscv64(run());
    let mut closed = run();
    closed.push(mir(MirOp::Ret, &[]));
    let closed = select_riscv64(closed);

    let vops: Vec<String> = tail.iter().filter_map(|i| i.get("vop")).collect();
    assert!(vops.iter().any(|vop| vop == "vfadd.vv"), "{vops:?}");
    assert!(vops.iter().any(|vop| vop == "vfmul.vv"), "{vops:?}");
    assert!(tail
        .iter()
        .any(|i| i.op.mnemonic() == "label" && i.get("name").as_deref() == Some("v128_done_0")));
    assert_eq!(closed.last().map(|i| i.op.mnemonic()), Some("ret"));
    assert_eq!(rendered(&tail), rendered(&closed[..closed.len() - 1]));
}
