//! bug-623 B: which bindings provably own the resource record they hold.
//!
//! A scope drop may free a `tcp`/`udp`/`tls` record (`resource_record_freed_at_drop`)
//! only when the binding is the record's ONE owner. Registering a cleanup is not that
//! fact: several shapes hand a binding a record another binding still frees, and each
//! was measured to double-free (or would) once a drop frees records:
//!
//! * `RES c AS Chan = u` — a union wrapping a live concrete binding;
//! * `RES b = passthru(a)` where `passthru` returns its `RES` parameter — the callee
//!   hands back the caller's own record;
//! * `RES y AS Chan = p(x)` / `RES b AS Chan = wrap(a)` — a union parameter returned,
//!   or a union built around a parameter and returned.
//!
//! So the answer is computed, fail-SAFE: a binding owns its record only when EVERY
//! value ever stored into it is provably a fresh record handed to it alone. Anything
//! this pass cannot prove answers "not owned", which keeps the close and skips the
//! free — a bounded leak, never a double free.
//!
//! A stored value is fresh when it is
//! * a `tcp`/`udp`/`tls`/`thread` producer call — every one allocates a new record in
//!   the calling thread's arena — excluding the borrowed-element results
//!   `CodeBuilder::value_aliases_live_resource` already names (`tcp::poll` over a list
//!   and its mirrors, `collections::get`/`getOr`);
//! * a union wrap of such a fresh producer (a wrap of a LOCAL is the alias shape);
//! * the closed default of a `RES x = <fallible> TRAP` temp (a `Bind` with no value)
//!   and the `resultValue` of a fresh `callResult` assigned into it;
//! * a bare local whose own stores are all fresh — the alias chain `RES v = u` whose
//!   root owns the record and hands it over at `RETURN` (the identity skip);
//! * a call to a user function whose every `RETURN` value is fresh by these rules,
//!   inside that function (a parameter is never fresh; a cycle is not fresh).

use crate::codegen::engine::builder::CodeBuilder;
use crate::target::shared::nir::visit::{walk_op, NirVisitor};
use crate::target::shared::nir::{NirFunction, NirOp, NirValue};
use crate::types::ParameterType;
use std::collections::{HashMap, HashSet};

/// The ownership facts one function's lowering consults.
pub(crate) struct RecordOwnership {
    /// Locals of a record-freeable type whose every store is a fresh record.
    pub(crate) owning_locals: HashSet<String>,
    /// Locals whose single store is a bare `Local(src)` — the alias chains a `RETURN`
    /// follows to the binding that actually owns a union's box.
    pub(crate) alias_sources: HashMap<String, String>,
}

/// Where one store into a local comes from, as far as record ownership is concerned.
#[derive(Clone)]
enum Source {
    Fresh,
    Local(String),
    ResultOf(String),
    UserCall(String),
    Unknown,
}

/// `tcp`/`udp`/`tls`/`thread` members that return a resource all return a NEW record
/// (connect/listen/accept/bind, and `thread::accept`'s copy into this arena). The
/// borrowed-element forms are filtered by `value_aliases_live_resource` before this.
fn is_record_producer_target(target: &str) -> bool {
    matches!(
        target.split('.').next(),
        Some("tcp" | "udp" | "tls" | "thread")
    )
}

fn classify(value: Option<&NirValue>, functions: &HashMap<String, &NirFunction>) -> Source {
    let Some(value) = value else {
        // A `Bind` without an initializer: the closed default record the inline-TRAP
        // desugar materializes, which this binding allocated.
        return Source::Fresh;
    };
    match value {
        NirValue::Local(name) => Source::Local(name.clone()),
        NirValue::ResultValue { value } => match value.as_ref() {
            NirValue::Local(name) => Source::ResultOf(name.clone()),
            _ => Source::Unknown,
        },
        NirValue::UnionWrap { value, .. } => match value.as_ref() {
            // `RES c AS Union = u`: the record belongs to `u`.
            NirValue::Local(_) => Source::Unknown,
            inner => classify(Some(inner), functions),
        },
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => {
            if CodeBuilder::value_aliases_live_resource(value) {
                Source::Unknown
            } else if functions.contains_key(target) {
                Source::UserCall(target.clone())
            } else if is_record_producer_target(target) {
                Source::Fresh
            } else {
                Source::Unknown
            }
        }
        _ => Source::Unknown,
    }
}

#[derive(Default)]
struct Stores {
    stores: HashMap<String, Vec<Source>>,
    types: HashMap<String, ParameterType>,
    returns: Vec<Source>,
}

fn collect(f: &NirFunction, functions: &HashMap<String, &NirFunction>) -> Stores {
    struct Collector<'m, 'f> {
        functions: &'m HashMap<String, &'f NirFunction>,
        out: Stores,
    }
    impl NirVisitor for Collector<'_, '_> {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind {
                    name, type_, value, ..
                } => {
                    let source = classify(value.as_ref(), self.functions);
                    self.out
                        .stores
                        .entry(name.clone())
                        .or_default()
                        .push(source);
                    self.out.types.insert(name.clone(), type_.clone());
                }
                NirOp::Assign { name, value } => {
                    let source = classify(Some(value), self.functions);
                    self.out
                        .stores
                        .entry(name.clone())
                        .or_default()
                        .push(source);
                }
                NirOp::Return { value: Some(value) } => {
                    let source = classify(Some(value), self.functions);
                    self.out.returns.push(source);
                }
                _ => {}
            }
            walk_op(self, op);
        }
    }
    let mut collector = Collector {
        functions,
        out: Stores::default(),
    };
    collector.visit_ops(&f.body);
    collector.out
}

struct Resolver<'m, 'f, 'p> {
    functions: &'m HashMap<String, &'f NirFunction>,
    record_type: &'p dyn Fn(&ParameterType) -> bool,
    memo: HashMap<String, bool>,
    visiting_functions: HashSet<String>,
}

impl Resolver<'_, '_, '_> {
    fn function_returns_fresh(&mut self, name: &str) -> bool {
        if let Some(known) = self.memo.get(name) {
            return *known;
        }
        let Some(function) = self.functions.get(name).copied() else {
            return false;
        };
        // Only a function returning a record-freeable type can hand one over, and
        // stopping here keeps the pass off every ordinary value-returning call.
        if !(self.record_type)(&function.returns) {
            return false;
        }
        if !self.visiting_functions.insert(name.to_string()) {
            return false;
        }
        let stores = collect(function, self.functions);
        let params: HashSet<String> = function.params.iter().map(|p| p.name.clone()).collect();
        let fresh = !stores.returns.is_empty()
            && stores
                .returns
                .iter()
                .all(|source| self.source_fresh(source, &stores, &params, &mut HashSet::new()));
        self.visiting_functions.remove(name);
        self.memo.insert(name.to_string(), fresh);
        fresh
    }

    fn local_fresh(
        &mut self,
        name: &str,
        stores: &Stores,
        params: &HashSet<String>,
        visiting: &mut HashSet<String>,
    ) -> bool {
        if params.contains(name) || !visiting.insert(name.to_string()) {
            return false;
        }
        let fresh = match stores.stores.get(name) {
            Some(sources) if !sources.is_empty() => sources
                .iter()
                .all(|source| self.source_fresh(source, stores, params, visiting)),
            _ => false,
        };
        visiting.remove(name);
        fresh
    }

    fn source_fresh(
        &mut self,
        source: &Source,
        stores: &Stores,
        params: &HashSet<String>,
        visiting: &mut HashSet<String>,
    ) -> bool {
        match source {
            Source::Fresh => true,
            Source::Local(name) | Source::ResultOf(name) => {
                self.local_fresh(name, stores, params, visiting)
            }
            Source::UserCall(target) => self.function_returns_fresh(target),
            Source::Unknown => false,
        }
    }
}

/// The record-ownership facts for `function`. `record_type` answers whether a binding
/// of that type can have its record freed at drop (a `resource_record_freed_at_drop`
/// kind, or a resource union with such a variant); only those bindings are resolved.
pub(crate) fn record_ownership(
    function: &NirFunction,
    functions: &HashMap<String, &NirFunction>,
    record_type: &dyn Fn(&ParameterType) -> bool,
) -> RecordOwnership {
    let stores = collect(function, functions);
    let params: HashSet<String> = function.params.iter().map(|p| p.name.clone()).collect();
    let mut resolver = Resolver {
        functions,
        record_type,
        memo: HashMap::new(),
        visiting_functions: HashSet::new(),
    };
    let mut owning_locals = HashSet::new();
    for (name, type_) in &stores.types {
        if record_type(type_) && resolver.local_fresh(name, &stores, &params, &mut HashSet::new()) {
            owning_locals.insert(name.clone());
        }
    }
    let alias_sources = stores
        .stores
        .iter()
        .filter_map(|(name, sources)| match sources.as_slice() {
            [Source::Local(src)] => Some((name.clone(), src.clone())),
            _ => None,
        })
        .collect();
    RecordOwnership {
        owning_locals,
        alias_sources,
    }
}
