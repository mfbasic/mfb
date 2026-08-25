//! Loop fission (distribution) — a Level-3 catalog row
//! (`planning/optimizations.md`), on structured NIR loops (the row's own
//! stage note: easiest here): one `FOR` whose body splits into two
//! independent halves becomes two loops over the same range, each half
//! running as its own phase — the locality/vectorization enabler.
//!
//! Fission is fusion in reverse and inherits the identical soundness
//! argument (see `opt1::fuse`): every statement is a flat
//! [`plans::loops::pure_statement`], the two halves are read/write disjoint
//! both ways, the duplicated `start`/`end`/`step` are stable leaves neither
//! half writes (re-evaluating a pure leaf is unobservable), and the
//! function's locals die unobserved on a raise (no `TRAP`, no by-ref
//! captures) so the increment-overflow phase divergence is invisible — the
//! split loops raise the identical error at the identical `loc`, which both
//! copies keep. The split point is the first boundary giving two disjoint
//! halves of at least two statements each (a one-statement phase is loop
//! overhead for nothing).

use crate::target::shared::nir::{NirModule, NirOp, NirValue};

use super::plans::loops::{
    freshened_clone, leaf_names, locals_survive_a_raise, pure_statement, stable_leaf,
    statement_reads, statement_writes,
};
use super::plans::reads::NameUses;

/// Apply the fission row to the whole module. Self-guarded on its catalog
/// level (3); the split count feeds `optimizer::stats`.
pub(crate) fn split(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let mut splits = 0;
    for function in &mut module.functions {
        if locals_survive_a_raise(&function.body) {
            continue;
        }
        let census = NameUses::census(&function.body);
        let mut salt = 0;
        split_in_body(&mut function.body, &census, &mut salt, &mut splits);
    }
    crate::optimizer::stats::count_loops_split(splits);
}

fn split_in_body(ops: &mut Vec<NirOp>, census: &NameUses, salt: &mut u64, splits: &mut u64) {
    for op in ops.iter_mut() {
        match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                split_in_body(then_body, census, salt, splits);
                split_in_body(else_body, census, salt, splits);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    split_in_body(&mut case.body, census, salt, splits);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => split_in_body(body, census, salt, splits),
            _ => {}
        }
    }
    let mut index = 0;
    while index < ops.len() {
        let Some((point, carried)) = split_point(&ops[index]) else {
            index += 1;
            continue;
        };
        // Build the second loop from clones first: it re-declares the loop
        // variable and the carried iteration-mirror binds, so the whole node
        // is freshened (NIR locals are function-unique). Only mutate the
        // original once the verified rename exists.
        let mut candidate = ops[index].clone();
        {
            let NirOp::For { body: slot, .. } = &mut candidate else {
                unreachable!("split_point checked the shape");
            };
            let mut second_half = slot.split_off(point);
            second_half.splice(0..0, carried);
            *slot = second_half;
        }
        let Some(mut freshened) = freshened_clone(std::slice::from_ref(&candidate), census, salt)
        else {
            index += 1;
            continue;
        };
        let second = freshened.remove(0);
        let NirOp::For { body, .. } = &mut ops[index] else {
            unreachable!("split_point checked the shape");
        };
        body.truncate(point);
        ops.insert(index + 1, second);
        *splits += 1;
        index += 2; // both halves are final: fission once per loop
    }
}

/// The first valid split boundary of a fissile `FOR` plus the
/// iteration-mirror binds the second half must re-declare, if any boundary
/// works. Mirror binds (`LET i = <loop var>`) write the same name in both
/// prospective halves' worlds but provably the same value, so they are
/// carried, not counted, in the disjointness tests.
fn split_point(op: &NirOp) -> Option<(usize, Vec<NirOp>)> {
    let NirOp::For {
        name: var,
        start,
        end,
        step,
        body,
        ..
    } = op
    else {
        return None;
    };
    if !stable_leaf(start) || !stable_leaf(end) || !stable_leaf(step) {
        return None;
    }
    if !body.iter().all(pure_statement) {
        return None;
    }
    let mirror = super::fuse::mirror_binds(body, var);
    let is_mirror = |op: &NirOp| {
        matches!(
            op,
            NirOp::Bind {
                name,
                value: Some(NirValue::Local(source)),
                ..
            } if source == var && mirror.contains(name)
        )
    };
    let bounds = leaf_names(&[start, end, step]);
    let strip = |mut set: std::collections::HashSet<String>| {
        for name in &mirror {
            set.remove(name);
        }
        set
    };
    (1..body.len()).find_map(|point| {
        let (first, second) = body.split_at(point);
        let weight = |ops: &[NirOp]| ops.iter().filter(|op| !is_mirror(op)).count();
        if weight(first) < 2 || weight(second) < 2 {
            return None; // both phases need >= 2 real statements
        }
        let writes_first = strip(statement_writes(first));
        let writes_second = strip(statement_writes(second));
        let reads_first = strip(statement_reads(first));
        let reads_second = strip(statement_reads(second));
        let disjoint = writes_first.is_disjoint(&reads_second)
            && writes_first.is_disjoint(&writes_second)
            && writes_second.is_disjoint(&reads_first)
            && bounds.is_disjoint(&writes_first)
            && bounds.is_disjoint(&writes_second);
        if !disjoint {
            return None;
        }
        let needed = statement_reads(second);
        let carried: Vec<NirOp> = first
            .iter()
            .filter(|op| {
                is_mirror(op) && matches!(op, NirOp::Bind { name, .. } if needed.contains(name))
            })
            .cloned()
            .collect();
        Some((point, carried))
    })
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
        with_opt_level(OptLevel(level), || split(&mut module));
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

    /// Two independent two-statement phases distribute into two loops over
    /// the same range.
    #[test]
    fn independent_phases_split() {
        let body = run(
            vec![for_loop(vec![
                assign("a1", local("i")),
                assign("a2", local("a1")),
                assign("b1", local("i")),
                assign("b2", local("b1")),
            ])],
            3,
        );
        assert_eq!(body.len(), 2, "two loops");
        let NirOp::For { body: first, .. } = &body[0] else {
            panic!("first phase");
        };
        let NirOp::For { body: second, .. } = &body[1] else {
            panic!("second phase");
        };
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 2);
    }

    /// A cross-phase dependence (the tail reads the head's write at every
    /// candidate boundary) blocks the split.
    #[test]
    fn dependent_phases_stay_together() {
        let body = run(
            vec![for_loop(vec![
                assign("a1", local("i")),
                assign("a2", local("a1")),
                assign("b1", local("a1")),
                assign("b2", local("b1")),
            ])],
            3,
        );
        assert_eq!(body.len(), 1, "b1 reads a1: no boundary is disjoint");
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let body = run(
            vec![for_loop(vec![
                assign("a1", local("i")),
                assign("a2", local("a1")),
                assign("b1", local("i")),
                assign("b2", local("b1")),
            ])],
            2,
        );
        assert_eq!(body.len(), 1);
    }
}
