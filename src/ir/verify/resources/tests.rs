//! plan-180-C Phase 2: the back-edge scan decides whether an EXIT/CONTINUE leaves the
//! scanned loop by name, past nested loops, the same way it did by kind.

use super::*;
use crate::ast::LoopKind;

fn exit(kind: LoopKind, target: &str) -> IrOp {
    IrOp::ExitLoop {
        target: Some(target.to_string()),
        kind,
        loc: Default::default(),
    }
}

fn cont(kind: LoopKind, target: &str) -> IrOp {
    IrOp::ContinueLoop {
        target: Some(target.to_string()),
        kind,
        loc: Default::default(),
    }
}

#[test]
fn named_exit_leaves_only_when_no_nested_loop_carries_its_name() {
    let project = crate::ir::test_support::project_fixture("t", vec![], vec![]);
    let env = TypeEnv::build(&project);
    // Scanning `FOR i` with a nested `FOR j` in its body.
    env.back_edge_scan.replace(Some((
        (LoopKind::For, Some("i".to_string())),
        vec![(LoopKind::For, Some("j".to_string()))],
    )));
    // `EXIT FOR i` leaves the scanned loop; `EXIT FOR j` stays inside the body.
    assert!(env.leaves_scanned_loop(&exit(LoopKind::For, "i")));
    assert!(!env.leaves_scanned_loop(&exit(LoopKind::For, "j")));
    // A loop outside the scanned one is left too.
    assert!(env.leaves_scanned_loop(&exit(LoopKind::For, "k")));
}

#[test]
fn named_continue_reaches_the_back_edge_of_its_own_loop() {
    let project = crate::ir::test_support::project_fixture("t", vec![], vec![]);
    let env = TypeEnv::build(&project);
    env.back_edge_scan.replace(Some((
        (LoopKind::For, Some("i".to_string())),
        vec![(LoopKind::For, Some("j".to_string()))],
    )));
    // `CONTINUE FOR i` reaches the scanned loop's back edge: it does not leave.
    assert!(!env.leaves_scanned_loop(&cont(LoopKind::For, "i")));
    // `CONTINUE FOR j` stays inside the nested loop: it does not leave.
    assert!(!env.leaves_scanned_loop(&cont(LoopKind::For, "j")));
    // `CONTINUE FOR k` names a loop outside the scanned one: it leaves.
    assert!(env.leaves_scanned_loop(&cont(LoopKind::For, "k")));
}
