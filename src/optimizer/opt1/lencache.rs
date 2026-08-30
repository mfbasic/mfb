//! Codepoint `len()` caching — a Level-3 Opt1 catalog row
//! (`planning/optimizations.md`): compute a String's codepoint count once and
//! reuse it, instead of rescanning the string at every `len()`.
//!
//! This one is MFB-specific and it is not a micro-optimization. A String's
//! *byte* length is an O(1) header read, but `len()` is defined as the
//! **codepoint** count, and `lower_len` lowers it to an inline loop that walks
//! every byte of the string masking off UTF-8 continuation bytes
//! (`builder_collection_layout.rs`). Two `len(s)` in one statement sequence
//! therefore scan the whole string twice, and the classic
//! `FOR i = 0 TO len(s) - 1` shape re-scans it on a schedule the loop rows
//! cannot fix.
//!
//! The rewrite is a plain common-subexpression elimination, done on the NIR
//! where `len` is still a recognizable call rather than the flat byte loop it
//! becomes: bind the first `len(s)` to a fresh local and point the rest of the
//! run at it.
//!
//! **Why only `String`.** For a collection, `len` is already the O(1) count
//! read from the header — there is nothing to cache, and caching would be
//! actively wrong if the collection were reachable through another name that
//! updated the header. The scan is the String case alone, so that is the only
//! case this row claims.
//!
//! **What ends a run.** Any restructuring statement (`IF`, `WHILE`, `FOR`,
//! `DO`, `FOR EACH`, `MATCH`, `TRAP`) ends it, because their bodies execute a
//! variable number of times and a cache bound in this sequence would be
//! neither in scope nor current inside them. Any statement that rebinds or
//! assigns the cached name ends it too — String assignment is how a String
//! changes, so a run that survives one is a run that must not exist. And a
//! name that ever appears as a `LocalRef` is skipped entirely: a slot
//! reference is the one way a callee can change a local underneath us.
//!
//! **Ordering.** The cache bind is inserted immediately before the statement
//! holding the first occurrence, so the only reordering is that `len(s)` is
//! evaluated before the rest of that statement's other subexpressions. `len`
//! on a String cannot trap and has no effect, so that is unobservable.

use std::collections::{HashMap, HashSet};

use super::plans::shape::{nested_bodies, nested_bodies_mut, own_values_mut};
use crate::target::shared::nir::visit::{walk_value, NirVisitor};
use crate::target::shared::nir::{NirModule, NirOp, NirValue};
use crate::types::ParameterType;

/// Apply the row to the whole module. Self-guarded on its catalog level (3).
pub(crate) fn cache(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let mut cached = 0u64;
    for function in &mut module.functions {
        let mut strings: HashSet<String> = function
            .params
            .iter()
            .filter(|param| param.type_ == ParameterType::String)
            .map(|param| param.name.clone())
            .collect();
        let mut aliased = Aliased::default();
        aliased.visit_ops(&function.body);
        collect_string_binds(&function.body, &mut strings);
        strings.retain(|name| !aliased.names.contains(name));
        if strings.is_empty() {
            continue;
        }
        let mut next = 0usize;
        cache_in_body(&mut function.body, &strings, &mut next, &mut cached);
    }
    crate::optimizer::stats::count_len_caches(cached);
}

/// Names that ever appear as a slot reference: a callee holding the slot can
/// change the local, so nothing about it may be cached.
#[derive(Default)]
struct Aliased {
    names: HashSet<String>,
}

impl NirVisitor for Aliased {
    fn visit_value(&mut self, value: &NirValue) {
        if let NirValue::LocalRef { name, .. } = value {
            self.names.insert(name.clone());
        }
        walk_value(self, value);
    }
}

/// Every `String`-typed local bound anywhere in the body.
fn collect_string_binds(ops: &[NirOp], out: &mut HashSet<String>) {
    for op in ops {
        if let NirOp::Bind { name, type_, .. } = op {
            if *type_ == ParameterType::String {
                out.insert(name.clone());
            }
        }
        for body in nested_bodies(op) {
            collect_string_binds(body, out);
        }
    }
}

/// Whether the op restructures control flow, and so ends a run.
fn is_barrier(op: &NirOp) -> bool {
    matches!(
        op,
        NirOp::If { .. }
            | NirOp::While { .. }
            | NirOp::For { .. }
            | NirOp::DoUntil { .. }
            | NirOp::ForEach { .. }
            | NirOp::Match { .. }
            | NirOp::Trap { .. }
    )
}

/// The name an op rebinds or writes, if any — what invalidates a cache.
fn writes(op: &NirOp) -> Option<&str> {
    match op {
        NirOp::Bind { name, .. } | NirOp::Assign { name, .. } => Some(name),
        NirOp::StateAssign { resource, .. } => Some(resource),
        _ => None,
    }
}

/// The String local a `len(...)` call reads, when the call is the cacheable
/// shape.
fn len_of(value: &NirValue, strings: &HashSet<String>) -> Option<String> {
    let NirValue::Call { target, args, .. } = value else {
        return None;
    };
    if target != "len" || args.len() != 1 {
        return None;
    }
    let NirValue::Local(name) = &args[0] else {
        return None;
    };
    strings.contains(name).then(|| name.clone())
}

/// Count the cacheable `len(s)` occurrences in a value.
fn count_in(value: &NirValue, strings: &HashSet<String>, out: &mut HashMap<String, usize>) {
    if let Some(name) = len_of(value, strings) {
        *out.entry(name).or_insert(0) += 1;
        return;
    }
    for child in children(value) {
        count_in(child, strings, out);
    }
}

/// Replace every cacheable `len(name)` in the value with a read of `cache`.
fn replace_in(value: &mut NirValue, name: &str, cache: &str, strings: &HashSet<String>) -> u64 {
    if len_of(value, strings).as_deref() == Some(name) {
        *value = NirValue::Local(cache.to_string());
        return 1;
    }
    let mut fired = 0;
    for child in children_mut(value) {
        fired += replace_in(child, name, cache, strings);
    }
    fired
}

fn children(value: &NirValue) -> Vec<&NirValue> {
    match value {
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => args.iter().collect(),
        NirValue::Closure { captures, .. } => captures.iter().collect(),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::MemberAccess { target: value, .. }
        | NirValue::Unary { operand: value, .. } => vec![value],
        NirValue::WithUpdate {
            target, updates, ..
        } => std::iter::once(&**target)
            .chain(updates.iter().map(|update| &update.value))
            .collect(),
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
            values.iter().collect()
        }
        NirValue::MapLiteral { entries, .. } => entries
            .iter()
            .flat_map(|(key, value)| [key, value])
            .collect(),
        NirValue::Binary { left, right, .. } => vec![left, right],
        _ => Vec::new(),
    }
}

fn children_mut(value: &mut NirValue) -> Vec<&mut NirValue> {
    match value {
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => args.iter_mut().collect(),
        NirValue::Closure { captures, .. } => captures.iter_mut().collect(),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::MemberAccess { target: value, .. }
        | NirValue::Unary { operand: value, .. } => vec![&mut **value],
        NirValue::WithUpdate {
            target, updates, ..
        } => std::iter::once(&mut **target)
            .chain(updates.iter_mut().map(|update| &mut update.value))
            .collect(),
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
            values.iter_mut().collect()
        }
        NirValue::MapLiteral { entries, .. } => entries
            .iter_mut()
            .flat_map(|(key, value)| [key, value])
            .collect(),
        NirValue::Binary { left, right, .. } => vec![&mut **left, &mut **right],
        _ => Vec::new(),
    }
}

/// Cache repeated `len(s)` within each straight-line run of `ops`, recursing
/// into nested bodies first so an inner run is handled in its own scope.
fn cache_in_body(
    ops: &mut Vec<NirOp>,
    strings: &HashSet<String>,
    next: &mut usize,
    cached: &mut u64,
) {
    for op in ops.iter_mut() {
        for body in nested_bodies_mut(op) {
            cache_in_body(body, strings, next, cached);
        }
    }

    // Runs are half-open `[start, end)` statement spans that stop *before* a
    // barrier or a write, never including it.
    //
    // Excluding the barrier itself is the load-bearing part, and it was learned
    // the hard way: a `WHILE` condition is re-evaluated every iteration, and
    // the loop's *body* can assign the very String being cached — an assignment
    // this scan cannot see, because it only inspects the statements of its own
    // sequence. Folding a barrier's own condition into the preceding run
    // therefore froze `len(s)` at its entry value for a loop that reassigned
    // `s`, which hung `rt-behavior/native/native-cbuffer-read-rt` at `-O3`.
    // Ending before the barrier costs the (rare) `FOR` bound and `IF` condition
    // and removes the whole class.
    let mut start = 0usize;
    let mut index = 0usize;
    loop {
        if index >= ops.len() {
            cache_run(ops, start..ops.len(), strings, next, cached);
            break;
        }
        let closes = is_barrier(&ops[index])
            || writes(&ops[index]).is_some_and(|name| strings.contains(name));
        if closes {
            let inserted = cache_run(ops, start..index, strings, next, cached);
            index = index + inserted + 1;
            start = index;
            continue;
        }
        index += 1;
    }
}

/// Cache within one run; returns how many statements were inserted.
fn cache_run(
    ops: &mut Vec<NirOp>,
    span: std::ops::Range<usize>,
    strings: &HashSet<String>,
    next: &mut usize,
    cached: &mut u64,
) -> usize {
    if span.len() < 1 {
        return 0;
    }
    // Count occurrences and remember where each name is first read.
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut first: HashMap<String, usize> = HashMap::new();
    for j in span.clone() {
        let mut here: HashMap<String, usize> = HashMap::new();
        for value in own_values_mut(&mut ops[j]) {
            count_in(value, strings, &mut here);
        }
        for (name, count) in here {
            *counts.entry(name.clone()).or_insert(0) += count;
            first.entry(name).or_insert(j);
        }
    }

    // Only names read more than once are worth a cache; take them in a stable
    // order so the emitted names (and so the codegen) are deterministic.
    let mut worth: Vec<(String, usize)> = counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(name, _)| {
            let at = first[&name];
            (name, at)
        })
        .collect();
    worth.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    let mut inserted = 0usize;
    for (name, at) in worth {
        let cache_name = format!("__len_cache_{}", *next);
        *next += 1;
        // The count above walked exactly these statements through exactly
        // this accessor, so every counted occurrence is reached here and the
        // bind below always has readers. (A partial rewrite would leave a
        // read of a local nothing binds, so this must not be conditional.)
        let mut fired = 0;
        for j in (at + inserted)..(span.end + inserted) {
            for value in own_values_mut(&mut ops[j]) {
                fired += replace_in(value, &name, &cache_name, strings);
            }
        }
        debug_assert!(fired >= 2, "a cached name must have at least two readers");
        ops.insert(
            at + inserted,
            NirOp::Bind {
                mutable: false,
                name: cache_name,
                type_: ParameterType::Integer,
                value: Some(NirValue::Call {
                    target: "len".to_string(),
                    args: vec![NirValue::Local(name)],
                    loc: Default::default(),
                }),
            },
        );
        inserted += 1;
        *cached += fired - 1;
    }
    inserted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};
    use crate::target::shared::nir::{NirFunction, NirParam, NirSourceLoc};

    fn function(params: Vec<(&str, ParameterType)>, body: Vec<NirOp>) -> NirFunction {
        NirFunction {
            name: "f".to_string(),
            visibility: "private".to_string(),
            kind: "function".to_string(),
            isolated: false,
            params: params
                .into_iter()
                .map(|(name, type_)| NirParam {
                    name: name.to_string(),
                    type_,
                    default: None,
                })
                .collect(),
            returns: ParameterType::Integer,
            body,
            file: "main.mfb".to_string(),
            resource_owners: std::collections::HashMap::new(),
        }
    }

    fn module(function: NirFunction) -> NirModule {
        super::super::local_rewrites::testutil::test_module(vec![function])
    }

    fn len_of_local(name: &str) -> NirValue {
        NirValue::Call {
            target: "len".to_string(),
            args: vec![NirValue::Local(name.to_string())],
            loc: NirSourceLoc::default(),
        }
    }

    fn bind(name: &str, value: NirValue) -> NirOp {
        NirOp::Bind {
            mutable: false,
            name: name.to_string(),
            type_: ParameterType::Integer,
            value: Some(value),
        }
    }

    fn run(module: &mut NirModule, level: u8) {
        with_opt_level(OptLevel(level), || cache(module));
    }

    /// Two scans of the same String parameter in one run collapse to one: a
    /// cache bind appears in front and both reads point at it.
    #[test]
    fn a_repeated_scan_is_cached() {
        let mut m = module(function(
            vec![("s", ParameterType::String)],
            vec![bind("a", len_of_local("s")), bind("b", len_of_local("s"))],
        ));
        run(&mut m, 3);
        let body = &m.functions[0].body;
        assert_eq!(body.len(), 3, "one cache bind was inserted");
        match &body[0] {
            NirOp::Bind { name, value, .. } => {
                assert!(name.starts_with("__len_cache_"));
                assert!(matches!(
                    value,
                    Some(NirValue::Call { target, .. }) if target == "len"
                ));
            }
            _ => panic!("expected the cache bind first"),
        }
        for op in &body[1..] {
            match op {
                NirOp::Bind {
                    value: Some(NirValue::Local(name)),
                    ..
                } => assert!(name.starts_with("__len_cache_")),
                _ => panic!("both reads should now be a local read"),
            }
        }
    }

    /// A single scan is left alone - there is nothing to share.
    #[test]
    fn a_lone_scan_is_untouched() {
        let mut m = module(function(
            vec![("s", ParameterType::String)],
            vec![bind("a", len_of_local("s"))],
        ));
        run(&mut m, 3);
        assert_eq!(m.functions[0].body.len(), 1);
    }

    /// Assigning the String between the two reads ends the run: the second
    /// scan is of a different string.
    #[test]
    fn an_assignment_between_them_ends_the_run() {
        let mut m = module(function(
            vec![("s", ParameterType::String)],
            vec![
                bind("a", len_of_local("s")),
                NirOp::Assign {
                    name: "s".to_string(),
                    value: NirValue::Const {
                        type_: ParameterType::String,
                        value: "x".to_string(),
                    },
                },
                bind("b", len_of_local("s")),
            ],
        ));
        run(&mut m, 3);
        assert_eq!(m.functions[0].body.len(), 3, "nothing was cached");
    }

    /// A non-String `len` is the O(1) header read, not a scan: not this row's
    /// business, and caching it could go stale.
    #[test]
    fn a_collection_length_is_not_cached() {
        let mut m = module(function(
            vec![("xs", ParameterType::parse("List OF Integer"))],
            vec![bind("a", len_of_local("xs")), bind("b", len_of_local("xs"))],
        ));
        run(&mut m, 3);
        assert_eq!(m.functions[0].body.len(), 2);
    }

    /// A String reachable through a slot reference can change underneath the
    /// cache, so it is skipped entirely.
    #[test]
    fn an_address_taken_string_is_skipped() {
        let mut m = module(function(
            vec![("s", ParameterType::String)],
            vec![
                bind("a", len_of_local("s")),
                NirOp::Eval {
                    value: NirValue::Call {
                        target: "helper".to_string(),
                        args: vec![NirValue::LocalRef {
                            name: "s".to_string(),
                            type_: ParameterType::String,
                        }],
                        loc: NirSourceLoc::default(),
                    },
                },
                bind("b", len_of_local("s")),
            ],
        ));
        run(&mut m, 3);
        assert_eq!(m.functions[0].body.len(), 3, "nothing was cached");
    }

    /// A loop condition is re-evaluated every iteration and the loop's *body*
    /// can assign the very String being cached — an assignment this scan
    /// cannot see, because it only inspects its own sequence's statements. So
    /// a barrier's own values never join the preceding run.
    ///
    /// Folding them in froze `len(s)` at its entry value for a loop that
    /// reassigned `s`, which hung `rt-behavior/native/native-cbuffer-read-rt`
    /// at `-O3`.
    #[test]
    fn a_loop_condition_never_joins_the_preceding_run() {
        let mut m = module(function(
            vec![("s", ParameterType::String)],
            vec![
                bind("a", len_of_local("s")),
                NirOp::While {
                    kind: crate::ast::LoopKind::While,
                    condition: len_of_local("s"),
                    body: vec![NirOp::Assign {
                        name: "s".to_string(),
                        value: NirValue::Const {
                            type_: ParameterType::String,
                            value: "shorter".to_string(),
                        },
                    }],
                },
            ],
        ));
        run(&mut m, 3);
        let body = &m.functions[0].body;
        assert_eq!(body.len(), 2, "nothing was cached");
        assert!(
            matches!(
                &body[1],
                NirOp::While { condition: NirValue::Call { target, .. }, .. } if target == "len"
            ),
            "the loop still rescans every iteration"
        );
    }
    /// The row is off below `-O3`.
    #[test]
    fn level_two_disables_the_row() {
        let mut m = module(function(
            vec![("s", ParameterType::String)],
            vec![bind("a", len_of_local("s")), bind("b", len_of_local("s"))],
        ));
        run(&mut m, 2);
        assert_eq!(m.functions[0].body.len(), 2);
    }
}
