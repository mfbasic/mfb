//! Loop fusion (jamming) — a Level-3 catalog row
//! (`planning/optimizations.md`), on structured NIR loops (the row's own
//! stage note: far easier here than on a CFG): two *adjacent* `FOR` loops
//! over the identical range become one loop running both bodies.
//!
//! Fusion interleaves iterations that used to run in separate phases, so
//! under MFB's checked semantics eligibility is strict — everything below is
//! what makes the interleave provably unobservable, not a heuristic:
//!
//! - same loop-variable type and structurally identical `start`/`end`/`step`
//!   **stable leaves** (equal constants or the same local, which neither body
//!   writes; resolved through a constant environment, since lowering
//!   pre-binds bounds into per-loop `$for_endN` temps) — same trip count,
//!   and the second loop's bound evaluations (now elided) were pure and
//!   identical. The loop *variables* are distinct per-loop temps
//!   (`$for_iterN`); rather than renaming the second body (a hand-rolled
//!   mutable walk could silently miss a variant), the fused body bridges
//!   them with a per-iteration pure bind `LET <var_b> = <var_a>` — correct
//!   by construction, dissolved by the MIR propagation/DCE rows. The second
//!   variable must occur nowhere outside its loop, and the first variable
//!   nowhere inside the second's (scope-blind census);
//! - both bodies are flat [`plans::loops::pure_statement`]s — no traps, no
//!   calls, no control flow, no observable effects at all beyond local
//!   binds/assigns;
//! - read/write disjointness both ways (writes of one never meet reads *or*
//!   writes of the other), so neither half can see the other's phase;
//! - the function's locals die unobserved on a raise
//!   ([`plans::loops::locals_survive_a_raise`] is false — no `TRAP`, no
//!   by-ref captures): the one remaining divergence, how many iterations of
//!   the second body ran when the FOR *increment itself* overflows mid-loop,
//!   is then invisible (both orders raise the identical error at the
//!   identical location — the first loop's, whose `loc` the fused loop
//!   keeps, since the first loop always raises first in both shapes).
//!
//! Chains fuse left to right (`A B C` → one loop) in a single pass.

use std::collections::HashMap;

use crate::target::shared::nir::{NirModule, NirOp, NirValue};

use super::plans::loops::{
    leaf_names, locals_survive_a_raise, pure_statement, same_stable_leaf, statement_reads,
    statement_writes,
};
use super::plans::reads::NameUses;
#[cfg(test)]
use crate::operators::BinaryOp;

/// Apply the fusion row to the whole module. Self-guarded on its catalog
/// level (3); the fusion count feeds `optimizer::stats`.
pub(crate) fn fuse(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let mut fused = 0;
    for function in &mut module.functions {
        if locals_survive_a_raise(&function.body) {
            continue;
        }
        // One fusion per round with a fresh whole-function census (the
        // variable-locality checks read it); chains converge over rounds.
        loop {
            let census = NameUses::census(&function.body);
            if !fuse_once(&mut function.body, &census, &mut fused) {
                break;
            }
        }
    }
    crate::optimizer::stats::count_loops_fused(fused);
}

/// One fusion attempt anywhere in `ops` (deepest-first). Returns true when a
/// pair fused; the caller re-censuses and retries.
fn fuse_once(ops: &mut Vec<NirOp>, census: &NameUses, fused: &mut u64) -> bool {
    for op in ops.iter_mut() {
        let fused_in_child = match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => fuse_once(then_body, census, fused) || fuse_once(else_body, census, fused),
            NirOp::Match { cases, .. } => cases
                .iter_mut()
                .any(|case| fuse_once(&mut case.body, census, fused)),
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => fuse_once(body, census, fused),
            _ => false,
        };
        if fused_in_child {
            return true;
        }
    }
    // Left-to-right scan with a constant environment: lowering pre-binds FOR
    // bounds into fresh temps (`LET $for_endN = 9`), so the two loops'
    // spellings differ (`Local($for_end3)` vs `Local($for_end4)`) and a pure
    // bound bind sits between them. Bounds compare *through* the environment,
    // and the in-between binds — provably independent of the first loop —
    // move in front of the fused loop (pure, so the move is unobservable;
    // their now-dead spellings are DCE's food).
    let mut env: HashMap<String, NirValue> = HashMap::new();
    let mut index = 0;
    while index < ops.len() {
        if !matches!(ops[index], NirOp::For { .. }) {
            update_env(&mut env, &ops[index]);
            index += 1;
            continue;
        }
        // The run of movable pure binds after the first loop.
        let mut gap_end = index + 1;
        while gap_end < ops.len()
            && movable_past_loop(&ops[gap_end], &ops[index])
            && !matches!(ops[gap_end], NirOp::For { .. })
        {
            gap_end += 1;
        }
        let mut gap_env = env.clone();
        for op in &ops[index + 1..gap_end] {
            update_env(&mut gap_env, op);
        }
        if gap_end < ops.len() && fusible(&ops[index], &ops[gap_end], &env, &gap_env, census) {
            let NirOp::For {
                name: second_var,
                type_: second_type,
                body: mut second,
                ..
            } = ops.remove(gap_end)
            else {
                unreachable!("fusible checked the shape");
            };
            let NirOp::For {
                name: first_var,
                body: first,
                ..
            } = &mut ops[index]
            else {
                unreachable!("fusible checked the shape");
            };
            // The second body's iteration-mirror binds would re-declare the
            // first body's names in the same scope (NIR forbids that); drop
            // them — their reads resolve to the first body's identical bind.
            let mirror: std::collections::HashSet<String> = mirror_binds(first, first_var)
                .intersection(&mirror_binds(&second, &second_var))
                .cloned()
                .collect();
            second.retain(|op| {
                !matches!(
                    op,
                    NirOp::Bind {
                        name,
                        value: Some(NirValue::Local(source)),
                        ..
                    } if source == &second_var && mirror.contains(name)
                )
            });
            if second_var != *first_var && statement_reads(&second).contains(&second_var) {
                // Bridge the second body's iteration temp to the first's — a
                // pure per-iteration bind instead of a risky tree-wide rename.
                first.push(NirOp::Bind {
                    mutable: false,
                    name: second_var,
                    type_: second_type,
                    value: Some(NirValue::Local(first_var.clone())),
                });
            }
            first.extend(second);
            // Slide the gap binds in front of the (fused) loop.
            ops[index..gap_end].rotate_left(1);
            *fused += 1;
            return true;
        }
        update_env(&mut env, &ops[index]);
        index += 1;
    }
    false
}

/// Track which names provably hold a constant at this point of the scan.
fn update_env(env: &mut HashMap<String, NirValue>, op: &NirOp) {
    match op {
        NirOp::Bind {
            name,
            value: Some(value @ NirValue::Const { .. }),
            ..
        } => {
            env.insert(name.clone(), value.clone());
        }
        NirOp::Bind { name, .. } | NirOp::Assign { name, .. } => {
            env.remove(name);
        }
        NirOp::StateAssign { resource, .. } => {
            env.remove(resource);
        }
        _ => {
            for name in super::plans::loops::defined_names(std::slice::from_ref(op)) {
                env.remove(&name);
            }
        }
    }
}

/// A pure bind that can slide leftward past `first` (a FOR loop): it reads
/// nothing the loop writes (body writes or the loop variable) and its own
/// name is not read by the loop.
fn movable_past_loop(op: &NirOp, first: &NirOp) -> bool {
    let NirOp::For {
        name: var,
        body: first_body,
        ..
    } = first
    else {
        return false;
    };
    if !pure_statement(op) || !matches!(op, NirOp::Bind { .. }) {
        return false;
    }
    let NirOp::Bind { name, value, .. } = op else {
        return false;
    };
    let reads = value
        .as_ref()
        .map(super::plans::loops::value_reads)
        .unwrap_or_default();
    let mut writes = statement_writes(first_body);
    writes.insert(var.clone());
    reads.is_disjoint(&writes) && !statement_reads(first_body).contains(name) && name != var
}

/// A bound leaf resolved through the constant environment.
fn resolved<'v>(value: &'v NirValue, env: &'v HashMap<String, NirValue>) -> &'v NirValue {
    if let NirValue::Local(name) = value {
        if let Some(constant) = env.get(name) {
            return constant;
        }
    }
    value
}

fn fusible(
    a: &NirOp,
    b: &NirOp,
    env_a: &HashMap<String, NirValue>,
    env_b: &HashMap<String, NirValue>,
    census: &NameUses,
) -> bool {
    let (
        NirOp::For {
            name: na,
            type_: ta,
            start: sa,
            end: ea,
            step: pa,
            body: ba,
            ..
        },
        NirOp::For {
            name: nb,
            type_: tb,
            start: sb,
            end: eb,
            step: pb,
            body: bb,
            ..
        },
    ) = (a, b)
    else {
        return false;
    };
    if ta != tb {
        return false;
    }
    // Distinct per-loop variables are bridged with a bind; that is scope-safe
    // only when the second variable lives entirely inside its loop and the
    // first variable does not appear in the second loop at all (scope-blind
    // census — shadowing anywhere blocks the fusion).
    if na != nb {
        let b_census = NameUses::census(std::slice::from_ref(b));
        if census.count(nb) != b_census.count(nb) || b_census.count(na) != 0 {
            return false;
        }
    }
    // Bounds compare through each loop's constant environment, so the
    // lowering's per-loop `$for_endN` temps (bound to the same constant)
    // count as identical.
    let bounds_equal =
        |va: &NirValue, vb: &NirValue| same_stable_leaf(resolved(va, env_a), resolved(vb, env_b));
    if !bounds_equal(sa, sb) || !bounds_equal(ea, eb) || !bounds_equal(pa, pb) {
        return false;
    }
    if !ba.iter().all(pure_statement) || !bb.iter().all(pure_statement) {
        return false;
    }
    // Iteration-mirror binds: lowering gives each loop a fresh iteration temp
    // and re-binds the user's variable from it at the body head (`LET i =
    // $for_iterN`) — in *both* bodies. Those binds write the same name but
    // provably the same value each iteration (each body's reads follow its
    // own bind, and the bridge equates the temps), so they are exempt from
    // the disjointness tests.
    let mirror: std::collections::HashSet<String> = mirror_binds(ba, na)
        .intersection(&mirror_binds(bb, nb))
        .cloned()
        .collect();
    let strip = |mut set: std::collections::HashSet<String>| {
        for name in &mirror {
            set.remove(name);
        }
        set
    };
    let writes_a = strip(statement_writes(ba));
    let writes_b = strip(statement_writes(bb));
    let reads_a = strip(statement_reads(ba));
    let reads_b = strip(statement_reads(bb));
    // The fused loop keeps `a`'s bound spellings: protect whatever names
    // those still are after resolution.
    let bounds = leaf_names(&[sa, ea, pa]);
    writes_a.is_disjoint(&reads_b)
        && writes_a.is_disjoint(&writes_b)
        && writes_b.is_disjoint(&reads_a)
        && bounds.is_disjoint(&writes_a)
        && bounds.is_disjoint(&writes_b)
}

/// Names defined exactly once in the (flat) body, by a `Bind` of the loop
/// variable leaf itself. pub(super): fission carries these binds into its
/// second loop under the same reasoning.
pub(super) fn mirror_binds(body: &[NirOp], var: &str) -> std::collections::HashSet<String> {
    let mut defs: HashMap<&str, usize> = HashMap::new();
    for op in body {
        if let NirOp::Bind { name, .. } | NirOp::Assign { name, .. } = op {
            *defs.entry(name.as_str()).or_insert(0) += 1;
        }
    }
    body.iter()
        .filter_map(|op| {
            let NirOp::Bind {
                name,
                value: Some(NirValue::Local(source)),
                ..
            } = op
            else {
                return None;
            };
            (source == var && defs.get(name.as_str()) == Some(&1)).then(|| name.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};
    use crate::target::shared::nir::{NirFunction, NirValue};
    use crate::types::ParameterType;
    use std::collections::HashMap;

    fn run(body: Vec<NirOp>, level: u8) -> Vec<NirOp> {
        let function = NirFunction {
            name: "f".to_string(),
            visibility: "private".to_string(),
            kind: "function".to_string(),
            isolated: false,
            params: vec![],
            returns: ParameterType::Integer,
            body,
            file: "main.mfb".to_string(),
            resource_owners: HashMap::new(),
        };
        let mut module = test_module(vec![function]);
        with_opt_level(OptLevel(level), || fuse(&mut module));
        module.functions.remove(0).body
    }

    fn assign(name: &str, value: NirValue) -> NirOp {
        NirOp::Assign {
            name: name.to_string(),
            value,
        }
    }

    fn for_loop(body: Vec<NirOp>) -> NirOp {
        NirOp::For {
            name: "i".to_string(),
            type_: ParameterType::Integer,
            start: int_const("0"),
            end: int_const("9"),
            step: int_const("1"),
            body,
            loc: Default::default(),
        }
    }

    /// Three adjacent identical-range loops over disjoint pure bodies fuse
    /// into one.
    #[test]
    fn disjoint_pure_loops_fuse_in_chains() {
        let body = run(
            vec![
                for_loop(vec![assign(
                    "a",
                    binary(BinaryOp::Less, local("i"), local("p")),
                )]),
                for_loop(vec![assign(
                    "b",
                    binary(BinaryOp::Less, local("i"), local("q")),
                )]),
                for_loop(vec![assign("c", local("i"))]),
            ],
            3,
        );
        assert_eq!(body.len(), 1, "one fused loop");
        let NirOp::For { body: fused, .. } = &body[0] else {
            panic!("expected the fused For");
        };
        assert_eq!(fused.len(), 3);
    }

    /// A read of the other loop's write, different bounds, or a trap-capable
    /// statement each block fusion.
    #[test]
    fn dependences_and_impure_bodies_block_fusion() {
        let dependent = run(
            vec![
                for_loop(vec![assign("a", local("i"))]),
                for_loop(vec![assign("b", local("a"))]),
            ],
            3,
        );
        assert_eq!(dependent.len(), 2, "b reads a: phases are observable");

        let trapping = run(
            vec![
                for_loop(vec![assign(
                    "a",
                    binary(BinaryOp::Add, local("i"), local("p")),
                )]),
                for_loop(vec![assign("b", local("i"))]),
            ],
            3,
        );
        assert_eq!(trapping.len(), 2, "arithmetic can raise: not fusible");

        let mut different = vec![
            for_loop(vec![assign("a", local("i"))]),
            for_loop(vec![assign("b", local("i"))]),
        ];
        if let NirOp::For { end, .. } = &mut different[1] {
            *end = int_const("8");
        }
        let different = run(different, 3);
        assert_eq!(different.len(), 2, "different trip counts");
    }

    /// A `TRAP` anywhere in the function keeps loops apart — a mid-loop
    /// increment overflow's partial state could be observed by the handler.
    #[test]
    fn a_trap_handler_blocks_fusion() {
        let body = run(
            vec![
                NirOp::Trap {
                    name: "e".to_string(),
                    body: vec![],
                },
                for_loop(vec![assign("a", local("i"))]),
                for_loop(vec![assign("b", local("i"))]),
            ],
            3,
        );
        assert_eq!(body.len(), 3);
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let body = run(
            vec![
                for_loop(vec![assign("a", local("i"))]),
                for_loop(vec![assign("b", local("i"))]),
            ],
            2,
        );
        assert_eq!(body.len(), 2);
    }
}
