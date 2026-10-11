//! bug-623 B: which bindings provably own the resource record they hold.
//!
//! A scope drop may free a `tcp`/`udp`/`tls` record (`resource_record_freed_at_drop`)
//! only when the binding is the record's ONE owner. Registering a cleanup is not that
//! fact: several shapes hand a binding a record another binding still frees, and each
//! was measured to double-free (or would) once a drop frees records:
//!
//! * `RES c AS Chan = u` — a union wrapping a live concrete binding;
//! * `RES b = passthru(a)` where `passthru` returns its resource parameter — the callee
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
//!
//! A union wrapping a local (`RES c AS Union = u`) is an ALIAS of `u`'s record: its own
//! cleanup frees only its box, and it never owns the record. Returned, it is fresh for
//! the caller exactly when the `RETURN` retires `u`'s close (`builder_exits`), which the
//! codegen does from bind-time alias maps. So a wrap counts as fresh only under the
//! conditions that make those maps coincide with this pass: the union local has that one
//! store, every bare-local hop to the wrapped root has one store, and the root itself is
//! fresh and has not floated into a collection.
//!
//! bug-645 asks the same question one container out: which OWNER COLLECTIONS may have
//! their owned-list drain free what it closes. See [`owning_collections`] — the rules,
//! and the direction they fail in, are this module's.

use crate::codegen::engine::builder::CodeBuilder;
use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{NirFunction, NirMatchPattern, NirOp, NirValue};
use crate::types::ParameterType;
use std::collections::{HashMap, HashSet};

/// The ownership facts one function's lowering consults.
pub(crate) struct RecordOwnership {
    /// Locals of a record-freeable type whose every store is a fresh record.
    pub(crate) owning_locals: HashSet<String>,
    /// Locals whose single store is a bare `Local(src)` — the alias chains a `RETURN`
    /// follows to the binding that actually owns a union's box.
    pub(crate) alias_sources: HashMap<String, String>,
    /// bug-645: owner-collection locals whose owned-list drain provably owns every
    /// block it can reach — see [`owning_collections`].
    pub(crate) owning_collections: HashSet<String>,
    /// bug-753: `RES`-block collections whose copying reassignment may free the
    /// block it replaces — see [`block_collections`].
    pub(crate) reassign_free_collections: HashSet<String>,
    /// bug-753: owning nested collections whose inner blocks are theirs alone, so
    /// the scope-exit free releases each inner block, then the outer one.
    pub(crate) inner_owning_collections: HashSet<String>,
    /// bug-999: the call nodes (by address) whose aggregate result nothing but
    /// their own statement can hold, mapped to whether the result's block is one
    /// the callee built — see [`CollectionUses::exclusive_temps`].
    pub(crate) exclusive_temps: HashMap<usize, bool>,
    /// bug-1045: the stores that put a value from an inner scope into a container
    /// declared further out — see [`StoreFloats`].
    pub(crate) store_floats: StoreFloats,
    /// bug-1053: the `RES` collections whose own block the binding frees at its
    /// scope's end: `owning_collections`, and every collection that is its block's
    /// one holder whoever reads its elements — see [`block_owning_collections`].
    pub(crate) block_owning_collections: HashSet<String>,
    /// bug-1053: whether a container of this function that is NOT proved the one
    /// holder of the records it lists may still free them at run time, by the
    /// owner word — see [`runtime_reclaim`].
    pub(crate) runtime_reclaim: bool,
}

/// bug-1045: the stores that carry a resource from an inner scope into a container
/// declared in an enclosing one.
///
/// `mfb spec language resource-management` 15.6: adding a reference to a resource to
/// a collection raises the resource's owning scope to the collection's when that
/// scope outlives the current owner. The escape analysis decides that for a `RES`
/// binding named as the element (`ResOwner::Float`). It has no answer for a value
/// that only REACHES a resource: a record with a `RES` field bound in a loop body
/// and appended to a list declared before the loop, or that record's field
/// appended to a `List OF RES T`. The record's binding adopted the resource for the
/// loop body's scope, which closed it at the end of every iteration while the outer
/// list still held it.
///
/// So the raise is done where the store is: every named operand of an assignment
/// to a container that is bound in a scope INSIDE the container's is walked, and
/// each resource it reaches is pushed, adopting, onto the container's owned-list
/// (`CodeBuilder::emit_store_float`). The push takes a record only when it is
/// unowned or owned by a scope inside the container's, so a handle owned further
/// out is left alone; the inner scope's drain then finds the record owned by
/// another scope and skips it.
#[derive(Default, Clone)]
pub(crate) struct StoreFloats {
    /// The operand nodes (by address) the store raises, mapped to the container
    /// they are stored into.
    pub(crate) operands: HashMap<usize, String>,
    /// The containers such a store assigns: each needs its owned-lists from its
    /// bind, like a float target.
    pub(crate) containers: HashSet<String>,
    /// Per container, the root locals of its raised operands.
    sources: HashMap<String, Vec<StoreSource>>,
    /// The raised roots that are records: an assignment of one is a copy by value,
    /// so the block it installs is fresh.
    record_roots: HashSet<String>,
    /// The operand nodes the ownership walk sets aside as the hand-over itself:
    /// those rooted at a record (copied by value where it is stored) or at a `RES`
    /// binding. An operand rooted at a COLLECTION is not one of them: a collection
    /// is stored by pointer, so the store really is a second holder of its block.
    handed_over: HashSet<usize>,
    /// The containers whose drain may only CLOSE what such a store handed it — see
    /// [`StoreFloatProof`].
    pub(crate) close_only: HashSet<String>,
    /// bug-1053: whether some assignment to a binding that reaches a resource
    /// names, anywhere this pre-pass does not raise, a binding of an inner scope
    /// that reaches one too (`keep = collections::filter(xs, …)` with `xs` bound
    /// in a nested block). What such a store carries outward keeps the inner
    /// scope's token, so the outer binding reads it after that scope has exited —
    /// see [`runtime_reclaim`].
    unraised_outward_store: bool,
}

/// One root local a [`StoreFloats`] store raises out of.
#[derive(Clone, PartialEq)]
struct StoreSource {
    name: String,
    /// The address of the op that declares it (a name may be bound again in a
    /// sibling scope, and that is another binding).
    declared_at: usize,
    /// For a `FOR EACH` variable, the local it iterates: the variable is a
    /// reference to an element of that local, which owns what the element reaches.
    iterates: Option<String>,
}

/// The named, side-effect-free operands of a stored value that can carry a
/// resource into the store's target: a local, a field chain or union extract over
/// a local, through the constructors, literals, `WITH` and `collections`
/// insertions that place them. The NIR-only twin of
/// `CodeBuilder::named_store_operands` (the module-level store, bug-806).
pub(crate) fn store_operands<'v>(value: &'v NirValue, out: &mut Vec<&'v NirValue>) {
    match value {
        NirValue::Local(_) => out.push(value),
        NirValue::MemberAccess { .. } | NirValue::UnionExtract { .. } => {
            if store_operand_root(value).is_some() {
                out.push(value);
            }
        }
        NirValue::Constructor { args, .. } => {
            for arg in args {
                store_operands(arg, out);
            }
        }
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
            for item in values {
                store_operands(item, out);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (_, entry) in entries {
                store_operands(entry, out);
            }
        }
        NirValue::UnionWrap { value, .. } => store_operands(value, out),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            store_operands(target, out);
            for update in updates {
                store_operands(&update.value, out);
            }
        }
        NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. }
            if target.starts_with("collections.")
                && matches!(
                    crate::codegen::builtins::native_builtin_target(target),
                    Some("append" | "prepend" | "insert" | "set")
                ) =>
        {
            for arg in args {
                store_operands(arg, out);
            }
        }
        _ => {}
    }
}

/// The local a store operand reads through: the local itself, or the root of a
/// field chain or union extract over one.
pub(crate) fn store_operand_root(value: &NirValue) -> Option<&str> {
    match value {
        NirValue::Local(name) => Some(name),
        NirValue::MemberAccess { target, .. } => store_operand_root(target),
        NirValue::UnionExtract { value, .. } => store_operand_root(value),
        _ => None,
    }
}

/// bug-1045: find the stores [`StoreFloats`] describes. `container` answers whether
/// a binding of that type reaches a resource without being one (a collection or a
/// record); a `RES` binding is recognised by its entry in `resource_owners`.
fn collect_store_floats(
    function: &NirFunction,
    container: &dyn Fn(&ParameterType) -> bool,
    record_container: &dyn Fn(&ParameterType) -> bool,
) -> StoreFloats {
    struct Scan<'p> {
        function: &'p NirFunction,
        container: &'p dyn Fn(&ParameterType) -> bool,
        record_container: &'p dyn Fn(&ParameterType) -> bool,
        depth: usize,
        /// The bindings in scope: the depth of the scope that owns what each
        /// reaches (for a `FOR EACH` variable, its iterable's), its type, the
        /// address of the op that declares it, and a `FOR EACH` variable's iterable.
        bound: HashMap<String, (usize, ParameterType, usize, Option<String>)>,
        scopes: Vec<Vec<String>>,
        out: StoreFloats,
    }
    impl NirVisitor for Scan<'_> {
        fn visit_ops(&mut self, ops: &[NirOp]) {
            self.depth += 1;
            self.scopes.push(Vec::new());
            for op in ops {
                self.visit_op(op);
            }
            for name in self.scopes.pop().unwrap_or_default() {
                self.bound.remove(&name);
            }
            self.depth -= 1;
        }

        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { name, type_, .. } => {
                    self.bound.insert(
                        name.clone(),
                        (self.depth, type_.clone(), op as *const NirOp as usize, None),
                    );
                    if let Some(scope) = self.scopes.last_mut() {
                        scope.push(name.clone());
                    }
                }
                NirOp::Assign { name, value } => {
                    self.note_unraised_outward_store(name, value);
                    let target = self
                        .bound
                        .get(name)
                        .filter(|(_, type_, _, _)| (self.container)(type_))
                        .map(|(depth, _, _, _)| *depth);
                    if let Some(target_depth) = target {
                        let mut operands = Vec::new();
                        store_operands(value, &mut operands);
                        for operand in operands {
                            let Some(root) = store_operand_root(operand) else {
                                continue;
                            };
                            let Some((depth, type_, declared_at, iterates)) = self.bound.get(root)
                            else {
                                continue;
                            };
                            let source = StoreSource {
                                name: root.to_string(),
                                declared_at: *declared_at,
                                iterates: iterates.clone(),
                            };
                            // The same scope, or the target itself: ownership is
                            // unchanged (15.6).
                            if root == name || *depth <= target_depth {
                                continue;
                            }
                            // An element of a container that reaches a resource
                            // reaches one too, whatever the variable's own type.
                            let element = source.iterates.is_some();
                            let res_binding =
                                !element && self.function.resource_owners.contains_key(root);
                            if !element {
                                match self.function.resource_owners.get(root) {
                                    // The escape analysis already floated it, at its bind.
                                    Some(crate::ir::resource_escape::ResOwner::Float(_)) => {
                                        continue
                                    }
                                    Some(_) => {}
                                    None if (self.container)(type_) => {}
                                    None => continue,
                                }
                            }
                            let address = operand as *const NirValue as usize;
                            if (self.record_container)(type_) {
                                self.out.record_roots.insert(root.to_string());
                                self.out.handed_over.insert(address);
                            } else if res_binding
                                || (element
                                    && !crate::codegen::engine::types::typed_is_collection_type(
                                        type_,
                                    ))
                            {
                                self.out.handed_over.insert(address);
                            }
                            self.out.operands.insert(address, name.clone());
                            self.out.containers.insert(name.clone());
                            let sources = self.out.sources.entry(name.clone()).or_default();
                            if !sources.contains(&source) {
                                sources.push(source);
                            }
                        }
                    }
                }
                // A `FOR EACH` variable is a reference to an element of its
                // iterable: what it reaches is owned by the iterable's scope or
                // further out (anything stored into the iterable from deeper was
                // raised to it), so that is the scope a store out of it raises
                // from. An iterable that is no local of this body (a parameter, a
                // call) is the caller's or the statement's: nothing to raise.
                NirOp::ForEach {
                    name,
                    type_,
                    iterable,
                    body,
                } => {
                    self.visit_value(iterable);
                    let owner = store_operand_root(iterable)
                        .and_then(|root| self.bound.get(root).map(|b| (b.0, root.to_string())));
                    if let Some((depth, root)) = owner {
                        self.bound.insert(
                            name.clone(),
                            (
                                depth,
                                type_.clone(),
                                op as *const NirOp as usize,
                                Some(root),
                            ),
                        );
                    }
                    self.visit_ops(body);
                    self.bound.remove(name);
                    return;
                }
                _ => {}
            }
            walk_op(self, op);
        }
    }
    impl Scan<'_> {
        /// bug-1053: record an assignment that carries a reference outward without
        /// raising it (`StoreFloats::unraised_outward_store`). The target is any
        /// binding of this body that reaches a resource (a container, or a `RES`
        /// binding); the carried names are the locals bound in a scope inside the
        /// target's that reach one, named anywhere in the value except as an
        /// operand the store raises or that the escape analysis floated. A call
        /// whose result the assignment adopts raises everything the result
        /// reaches, whatever its arguments were.
        fn note_unraised_outward_store(&mut self, name: &str, value: &NirValue) {
            if self.out.unraised_outward_store {
                return;
            }
            let reaches = |scan: &Self, name: &str, type_: &ParameterType| {
                (scan.container)(type_) || scan.function.resource_owners.contains_key(name)
            };
            let Some((target_depth, raises)) = self
                .bound
                .get(name)
                .filter(|(_, type_, _, _)| reaches(self, name, type_))
                .map(|(depth, type_, _, _)| (*depth, (self.container)(type_)))
            else {
                return;
            };
            if crate::codegen::cleanup::owned::builder_owned_cleanup::adopts_from_call(value) {
                return;
            }
            struct Names<'v> {
                raised: Vec<&'v NirValue>,
                found: Vec<String>,
            }
            impl NirVisitor for Names<'_> {
                fn visit_value(&mut self, value: &NirValue) {
                    if self
                        .raised
                        .iter()
                        .any(|operand| std::ptr::eq(*operand, value))
                    {
                        return;
                    }
                    if let NirValue::Local(name) = value {
                        self.found.push(name.clone());
                    }
                    walk_value(self, value);
                }
            }
            let mut names = Names {
                raised: Vec::new(),
                found: Vec::new(),
            };
            // Only an assignment to a container raises its operands (the `Assign`
            // arm above); into a `RES` binding nothing is raised.
            if raises {
                store_operands(value, &mut names.raised);
            }
            names.visit_value(value);
            let unraised = names.found.iter().any(|root| {
                if root == name {
                    return false;
                }
                let Some((depth, type_, _, iterates)) = self.bound.get(root) else {
                    return false;
                };
                if *depth <= target_depth {
                    return false;
                }
                if matches!(
                    self.function.resource_owners.get(root),
                    Some(crate::ir::resource_escape::ResOwner::Float(_))
                ) {
                    return false;
                }
                iterates.is_some() || reaches(self, root, type_)
            });
            if unraised {
                self.out.unraised_outward_store = true;
            }
        }
    }
    let mut scan = Scan {
        function,
        container,
        record_container,
        depth: 0,
        bound: HashMap::new(),
        scopes: Vec::new(),
        out: StoreFloats::default(),
    };
    scan.visit_ops(&function.body);
    scan.out
}

/// Where one store into a local comes from, as far as record ownership is concerned.
#[derive(Clone)]
enum Source {
    Fresh,
    Local(String),
    /// A union wrap of a bare local: an alias of that local's record.
    WrapLocal(String),
    ResultOf(String),
    UserCall(String),
    /// bug-648: a store an inline-`TRAP` temp is only LENT (a borrowed element, an
    /// aliasing `RECOVER`). The temp's run-time owner flag keeps its drop off such a
    /// value, so the temp's OWN drop ignores this store ([`Resolver::lent_temp_fresh`]);
    /// read through anything else — an alias hop, a `RETURN` — it is not fresh.
    Lent,
    Unknown,
}

/// `fs`/`tcp`/`udp`/`tls`/`thread` members that return a resource all return a NEW
/// record (open/connect/listen/accept/bind, and `thread::accept`'s copy into this
/// arena). The borrowed-element forms are filtered by `value_aliases_live_resource`
/// before this.
///
/// bug-647: `fs` was missing, so no `fs::File` binding was ever an owning local and its
/// 96 B record leaked on every bind — 192 B under an inline `TRAP`, which materializes a
/// second record for the error-path binding. Every `fs` member that returns a resource is
/// an opener that allocates a fresh record — `open`, `openFile`, `openFileNoFollow`,
/// `openWithin`, `createTempFile`, and no other `fs` member returns `fs::File` (checked
/// against the rendered `mfb man fs --all` declarations) — so the same reasoning that
/// admits the other four packages admits this one.
///
/// plan-158-A: `audio` likewise. Its only members that return a resource are the four
/// opens (`openInput`/`openOutput` and their device overloads), and each backend's open
/// allocates a fresh `H_RECORD_SIZE` (= `RESOURCE_RECORD_SIZE`) record in this arena;
/// the streams are not sendable, so no transfer copy exists. Missing, every audio
/// binding leaked its 96 B record (measured: 96 B per open/close cycle on macOS and
/// Windows).
fn is_record_producer_target(target: &str) -> bool {
    matches!(
        target.split('.').next(),
        Some("fs" | "tcp" | "udp" | "tls" | "thread" | "audio")
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
            NirValue::Local(name) => Source::WrapLocal(name.clone()),
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
        lent: crate::codegen::resource::cleanup::trap_ownership::TrapOwnership,
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
                    let source = if self.lent.lent_temps.contains(name)
                        && !self.lent.store_is_owned(value)
                    {
                        Source::Lent
                    } else {
                        classify(Some(value), self.functions)
                    };
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
        lent: crate::codegen::resource::cleanup::trap_ownership::collect_trap_ownership(&f.body),
        out: Stores::default(),
    };
    collector.visit_ops(&f.body);
    collector.out
}

struct Resolver<'m, 'f, 'p> {
    functions: &'m HashMap<String, &'f NirFunction>,
    record_type: &'p dyn Fn(&ParameterType) -> bool,
    /// The floated locals of each function being resolved, innermost last: a local whose
    /// close obligation floated into a collection never owns its record.
    floats: Vec<HashSet<String>>,
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
        // A callee's floats are its own: a local that floated there is never fresh.
        self.floats.push(float_set(function));
        let fresh = !stores.returns.is_empty()
            && stores
                .returns
                .iter()
                .all(|source| self.source_fresh(source, &stores, &params, &mut HashSet::new()));
        self.floats.pop();
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
        let floated = self.floats.last().is_some_and(|f| f.contains(name));
        if params.contains(name) || floated || !visiting.insert(name.to_string()) {
            return false;
        }
        let fresh = match stores.stores.get(name) {
            // An alias union: fresh only through the strict single-store wrap walk.
            Some(sources) if sources.iter().any(|s| matches!(s, Source::WrapLocal(_))) => {
                match sources.as_slice() {
                    [Source::WrapLocal(src)] => match wrap_root(src, stores) {
                        Some(root) => self.local_fresh(&root, stores, params, visiting),
                        None => false,
                    },
                    _ => false,
                }
            }
            Some(sources) if !sources.is_empty() => sources
                .iter()
                .all(|source| self.source_fresh(source, stores, params, visiting)),
            _ => false,
        };
        visiting.remove(name);
        fresh
    }

    /// bug-648: [`Self::local_fresh`] for an inline-`TRAP` temp that is lent some of
    /// its stores, asked only for the temp's own drop. That drop runs only while the
    /// owner flag says the slot holds an owned value, so the record it may free is one
    /// of the OWNED stores — the closed default, a produced record — and only those
    /// have to be fresh.
    fn lent_temp_fresh(&mut self, name: &str, stores: &Stores, params: &HashSet<String>) -> bool {
        let floated = self.floats.last().is_some_and(|f| f.contains(name));
        if params.contains(name) || floated {
            return false;
        }
        let mut visiting = HashSet::from([name.to_string()]);
        stores.stores.get(name).is_some_and(|sources| {
            sources
                .iter()
                .filter(|source| !matches!(source, Source::Lent))
                .all(|source| self.source_fresh(source, stores, params, &mut visiting))
        })
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
            Source::Local(name) | Source::ResultOf(name) | Source::WrapLocal(name) => {
                self.local_fresh(name, stores, params, visiting)
            }
            Source::UserCall(target) => self.function_returns_fresh(target),
            Source::Lent | Source::Unknown => false,
        }
    }
}

/// The locals of `function` whose close obligation floated into a collection.
fn float_set(function: &NirFunction) -> HashSet<String> {
    function
        .resource_owners
        .iter()
        .filter(|(_, owner)| matches!(owner, crate::ir::resource_escape::ResOwner::Float(_)))
        .map(|(name, _)| name.clone())
        .collect()
}

/// The concrete binding a wrap of `src` aliases: follow bare-local hops, each of which
/// must be that local's only store. `None` when a hop has another store (a name reused in
/// sibling scopes), so the pass and the codegen's bind-time maps cannot disagree.
fn wrap_root(src: &str, stores: &Stores) -> Option<String> {
    let mut current = src.to_string();
    for _ in 0..64 {
        match stores.stores.get(&current).map(Vec::as_slice) {
            Some([Source::Local(next)]) => current = next.clone(),
            Some([_]) => return Some(current),
            _ => return None,
        }
    }
    None
}

/// The record-ownership facts for `function`. `record_type` answers whether a binding
/// of that type can have its record freed at drop (a `resource_record_freed_at_drop`
/// kind, or a resource union with such a variant); only those bindings are resolved.
///
/// `res_collection_type` answers whether a collection binding of that type carries
/// resources it may own (a `List OF RES T`, or any collection reaching one).
pub(crate) fn record_ownership(
    function: &NirFunction,
    functions: &HashMap<String, &NirFunction>,
    record_type: &dyn Fn(&ParameterType) -> bool,
    res_collection_type: &dyn Fn(&ParameterType) -> bool,
) -> RecordOwnership {
    let stores = collect(function, functions);
    let params: HashSet<String> = function.params.iter().map(|p| p.name.clone()).collect();
    let mut resolver = Resolver {
        functions,
        record_type,
        floats: vec![float_set(function)],
        memo: HashMap::new(),
        visiting_functions: HashSet::new(),
    };
    let mut owning_locals = HashSet::new();
    for (name, type_) in &stores.types {
        // An alias union never owns the record, whatever its root does.
        let wraps = stores
            .stores
            .get(name)
            .is_some_and(|s| s.iter().any(|s| matches!(s, Source::WrapLocal(_))));
        if wraps {
            continue;
        }
        if !record_type(type_) {
            continue;
        }
        let lent = stores
            .stores
            .get(name)
            .is_some_and(|s| s.iter().any(|s| matches!(s, Source::Lent)));
        let fresh = if lent {
            resolver.lent_temp_fresh(name, &stores, &params)
        } else {
            resolver.local_fresh(name, &stores, &params, &mut HashSet::new())
        };
        if fresh {
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
    // bug-1025: a record that reaches a resource, as opposed to a collection that
    // does. Both are owner containers; only a record is copied by value.
    let record_container = |type_: &ParameterType| {
        res_collection_type(type_)
            && !crate::codegen::engine::types::typed_is_collection_type(type_)
    };
    // bug-1045: the stores that raise an inner scope's value into an outer container.
    let container_type = |type_: &ParameterType| res_collection_type(type_) && !record_type(type_);
    let mut store_floats = collect_store_floats(function, &container_type, &record_container);
    let (owning_collections, exclusive_temps, block_owning_collections) = owning_collections(
        function,
        functions,
        record_type,
        res_collection_type,
        &stores,
        &params,
        &mut store_floats,
    );
    let runtime_reclaim = runtime_reclaim(
        function,
        functions,
        record_type,
        res_collection_type,
        &stores,
        &store_floats,
    );
    let (reassign_free_collections, inner_owning_collections) = block_collections(
        function,
        functions,
        &stores,
        &owning_collections,
        &record_container,
        &store_floats,
    );
    RecordOwnership {
        owning_locals,
        alias_sources,
        owning_collections,
        reassign_free_collections,
        inner_owning_collections,
        exclusive_temps,
        store_floats,
        block_owning_collections,
        runtime_reclaim,
    }
}

/// bug-1053: whether a container this function cannot PROVE the one holder of the
/// records it lists may free them anyway, at run time.
///
/// The proof ([`owning_collections`]) reads the function's text and fails for any
/// reading it cannot follow, most of which leave no holder behind at all: a handle
/// bound on its own (`RES f = k.h`), a copy that is stored in the same scope, a
/// `LAMBDA` that only reads its capture, a callee that keeps an element in a list
/// it returns. A close-only drain then kept every such record (112 bytes for a
/// file, on every call).
///
/// What actually decides the question is already in the record: its owner word
/// names the ONE scope that closes it (`codegen::resource::owner`). Every store
/// that lets a reference outlive that scope raises the word first — the float of
/// a `RES` binding into an outer container, the store-time raise of anything
/// else ([`StoreFloats`]), the adoption of a call's result, the module-level
/// raise, a `RETURN`'s release and the caller's claim. So when the scope that
/// holds the token exits, every remaining reference is in that scope or inside
/// it, and is gone with it. The drain may free what it closes, provided the free
/// waits for the other cleanups of the same exit that may still read the owner
/// word: `CodeBuilder::emit_owned_list_drain` takes the record and
/// `emit_owned_list_reclaims` frees it after the last of them.
///
/// Two stores carry a reference outward WITHOUT raising it, and a function that
/// can be on either side of one keeps its close-only drains (the records are
/// closed; reading one afterwards is `ErrResourceClosed`, not a read of freed
/// memory):
///
/// * **A store through a captured reference.** A `LAMBDA` that assigns a `MUT`
///   container of its enclosing function (`collections::forEach(xs, LAMBDA(h) ->
///   keep = collections::append(keep, h))`) writes a longer-lived list from code
///   the storing function's own pre-pass never sees as a store, and any call that
///   reaches that lambda — through a function value, any number of frames away —
///   can hand it a record. This is asked of the whole program
///   ([`module_stores_through_a_captured_reference`]): no such lambda anywhere,
///   no such store.
/// * **An assignment this pre-pass does not raise** (`keep =
///   collections::filter(xs, …)` with `xs` bound in a nested block): asked of
///   this function (`StoreFloats::unraised_outward_store`).
fn runtime_reclaim(
    function: &NirFunction,
    functions: &HashMap<String, &NirFunction>,
    record_type: &dyn Fn(&ParameterType) -> bool,
    res_collection_type: &dyn Fn(&ParameterType) -> bool,
    stores: &Stores,
    store_floats: &StoreFloats,
) -> bool {
    if store_floats.unraised_outward_store {
        return false;
    }
    // Asked only of a function that can have such a container at all: the last
    // answer costs a walk of the program.
    let binds_a_container = stores.types.values().any(res_collection_type)
        || function
            .params
            .iter()
            .any(|param| res_collection_type(&param.type_));
    let reaches = |type_: &ParameterType| res_collection_type(type_) || record_type(type_);
    binds_a_container && !module_stores_through_a_captured_reference(functions, &reaches)
}

/// bug-1053: whether any function of the program assigns, through a by-reference
/// capture, a binding that reaches a resource. `reaches` answers for a type.
///
/// A by-reference capture is bound at the top of the lambda's body as `Bind name
/// = Capture { by_ref: true }`; a write through it is an `Assign` to that name.
fn module_stores_through_a_captured_reference(
    functions: &HashMap<String, &NirFunction>,
    reaches: &dyn Fn(&ParameterType) -> bool,
) -> bool {
    struct Writes<'n> {
        captured: &'n HashSet<&'n str>,
        found: bool,
    }
    impl NirVisitor for Writes<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::Assign { name, .. } = op {
                if self.captured.contains(name.as_str()) {
                    self.found = true;
                }
            }
            if !self.found {
                walk_op(self, op);
            }
        }
    }
    functions.values().any(|function| {
        let captured: HashSet<&str> = function
            .body
            .iter()
            .filter_map(|op| match op {
                NirOp::Bind {
                    name,
                    type_,
                    value: Some(NirValue::Capture { by_ref: true, .. }),
                    ..
                } if reaches(type_) => Some(name.as_str()),
                _ => None,
            })
            .collect();
        if captured.is_empty() {
            return false;
        }
        let mut writes = Writes {
            captured: &captured,
            found: false,
        };
        writes.visit_ops(&function.body);
        writes.found
    })
}

/// bug-753: the two block-free verdicts beyond [`owning_collections`].
///
/// * **Reassign-free.** A copying `c = collections::append(c, …)` installs a new
///   block and copies `c`'s entries into it. The block it replaces may be freed when
///   every store into `c` is fresh, no call keeps `c`'s block, no element is read
///   back out of it, and `c` is named outside a call argument only by `RETURN`. A
///   `RETURN` hands on the CURRENT block, never one a reassignment already replaced,
///   so it is no reason to keep the old ones. The free is shallow: entries are
///   references (a `RES` element) or inner-block pointers the new block now holds.
/// * **Inner-owning.** A nested `List OF List OF RES T` holds its inner lists by
///   POINTER (bug-801), so its scope-exit free may release them only when no other
///   holder can exist. That is proved for a sole-owner binding whose every store is
///   `collections::chunks`/`window`: their bodies are audited to append each fresh
///   `__collections_slice` piece exactly once (`FRESH_BUILTIN_BODIES`).
fn block_collections(
    function: &NirFunction,
    functions: &HashMap<String, &NirFunction>,
    stores: &Stores,
    owning: &HashSet<String>,
    record_container: &dyn Fn(&ParameterType) -> bool,
    store_floats: &StoreFloats,
) -> (HashSet<String>, HashSet<String>) {
    let mut callees = CalleeParamUses {
        record_container: Some(record_container),
        ..CalleeParamUses::default()
    };
    let uses = CollectionUses::of_with(function, functions, &mut callees, store_floats);
    let mut reassign_free = HashSet::new();
    let mut inner_owning = HashSet::new();
    for (name, type_) in &stores.types {
        if !CodeBuilder::is_res_collection_block(type_) {
            continue;
        }
        if uses.block_is_fresh(name)
            && !uses.kept.contains(name)
            && !uses.element_readers.contains(name)
            && !uses.escaping_outside_return.contains(name)
        {
            reassign_free.insert(name.clone());
        }
        let nested = matches!(type_, ParameterType::ListOf(inner)
            if CodeBuilder::is_res_marked_resource_collection(inner));
        if nested && owning.contains(name) && uses.chunk_stores.get(name).copied().unwrap_or(false)
        {
            inner_owning.insert(name.clone());
        }
    }
    (reassign_free, inner_owning)
}

/// The audited builtin bodies whose nested result owns each inner block it holds.
const INNER_OWNING_BUILTIN_BODIES: &[&str] = &["#collections_chunks$", "#collections_window$"];

/// bug-645: which owner collections an owned-list drain may RECLAIM through, not just
/// close through.
///
/// The drain walks the `{record, next}` nodes a floated `RES` bind pushed (§15.6) and
/// closes each element. Closing is unconditional — a close is idempotent. *Freeing* is
/// not, so it asks the same question this module asks everywhere else, in the same
/// fail-SAFE direction: unless the collection is provably the ONE owner of every block
/// the drain would release, the drain keeps today's close-only behaviour and the memory
/// stays leaked. A bounded leak is acceptable; a double free is not.
///
/// A collection `c` qualifies only when all four hold:
///
/// * **Its own block is fresh.** Every store into `c` is a collection LITERAL or a
///   self-receiving `collections::` mutator (`c = collections::append(c, …)`), whose
///   lowering either grows the block in place or allocates a replacement and frees the
///   original. Anything else — `MUT d = c`, a field read, a call that hands back
///   somebody else's list — means a second holder may exist, so the block is not freed.
/// * **Nothing else holds `c`.** `c` never appears as a bare `Local` outside a call's
///   ARGUMENT position. An argument creates no second owner (no callee frees a
///   `List OF RES` block: `is_freeable_flat_value` is false for one, so no
///   `OwnedValue` cleanup can exist for it anywhere), while a bind, a `RETURN`, a
///   record field, a union wrap or a closure capture does. A `FOR EACH` over `c`
///   does only when its loop variable leaves the loop (bug-1000).
/// * **No element read back out is kept.** A `collections::get`/`getOr` — or a
///   borrowed `tcp`/`udp`/`tls::poll` — "yields a POINTER to the one resource"
///   (§15.6), so a freed record could still be live in the reader's hands. One
///   written straight into an argument of a call that does not keep it is a use
///   that ends with the call (bug-1000); bound, returned, stored or passed to a
///   callee that keeps it, it disqualifies `c`.
/// * **Every floated element is a fresh record that does not escape.** Fresh by this
///   module's existing rules (a producer call, a closed `TRAP` default, a wrap of one
///   of those); a wrap of a LOCAL is an alias whose record another binding still frees,
///   and is refused. And the element local itself never appears outside an argument
///   position, so no `RETURN c` / `LET keep = c` hands the record on.
///
/// bug-1000: "outside an argument position" is not the whole of it. An argument
/// is a use only when the callee is read and found not to keep it, or is a natively
/// lowered builtin: a call through a function value, or one handed a callback that
/// keeps what it is passed, reaches code that may store the reference anywhere
/// ([`CollectionUses::call_reaches_unread_code`]). Such an argument is kept, for the
/// collection and for an element alike. (A named callee that keeps its parameter
/// keeps a collection's block, but is no hazard for an element: see
/// `kept_by_unread_code`.)
///
/// The float exclusion in [`Resolver::local_fresh`] is lifted for exactly the elements
/// of the collection being asked about: "this local floated into a collection" is the
/// reason its own binding must not free the record, and the reason the collection may.
fn owning_collections(
    function: &NirFunction,
    functions: &HashMap<String, &NirFunction>,
    record_type: &dyn Fn(&ParameterType) -> bool,
    res_collection_type: &dyn Fn(&ParameterType) -> bool,
    stores: &Stores,
    params: &HashSet<String>,
    store_floats: &mut StoreFloats,
) -> (HashSet<String>, HashMap<usize, bool>, HashSet<String>) {
    // bug-1025: a record that reaches a resource, as opposed to a collection that
    // does (`record_ownership`'s own `record_container`).
    let record_container = &|type_: &ParameterType| {
        res_collection_type(type_)
            && !crate::codegen::engine::types::typed_is_collection_type(type_)
    };
    let mut floats: HashMap<String, Vec<String>> = HashMap::new();
    for (name, owner) in &function.resource_owners {
        if let crate::ir::resource_escape::ResOwner::Float(collection) = owner {
            floats
                .entry(collection.clone())
                .or_default()
                .push(name.clone());
        }
    }
    let mut callees = CalleeParamUses {
        record_container: Some(record_container),
        ..CalleeParamUses::default()
    };
    let mut uses = CollectionUses::of_with(function, functions, &mut callees, store_floats);
    let exclusive_temps = std::mem::take(&mut uses.exclusive_temps);
    let all_floated: HashSet<String> = floats.values().flatten().cloned().collect();
    let mut owning = HashSet::new();
    // bug-651: a `RES`-marked collection that no element ever floats into still owns its
    // own block, and nothing freed it — 48 B per binding for a `MUT xs AS List OF RES X
    // = []` in a loop, with no resource involved at all. bug-645 registered that free
    // only alongside an owned-list drain, which requires a float, so the empty case fell
    // through every branch.
    //
    // The three conditions are the SAME ones the floated loop below applies to a
    // container; only the per-element check is absent, and vacuously so — there are no
    // elements to own. Asking them here rather than trusting the empty literal is what
    // keeps a container that is returned, aliased, or read through `collections::get`
    // out of the set.
    for (name, type_) in &stores.types {
        if !res_collection_type(type_) || floats.contains_key(name) {
            continue;
        }
        if uses.block_held_elsewhere(name) {
            continue;
        }
        if !uses.block_is_fresh(name) {
            continue;
        }
        owning.insert(name.clone());
    }
    for (collection, elements) in &floats {
        // A float target with no bind in this body (a parameter) takes the strict
        // rule: its type is not known here.
        let held = match stores.types.get(collection) {
            Some(_) => uses.block_held_elsewhere(collection),
            None => uses.block_held_elsewhere_strict(collection),
        };
        if held {
            continue;
        }
        if !uses.block_is_fresh(collection) {
            continue;
        }
        // A callee's floats stay excluded, and so do the floats of every OTHER
        // collection in this function; only this collection's own elements are asked.
        let mut other_floats = all_floated.clone();
        for element in elements {
            other_floats.remove(element);
        }
        let mut resolver = Resolver {
            functions,
            record_type,
            floats: vec![other_floats],
            memo: HashMap::new(),
            visiting_functions: HashSet::new(),
        };
        let elements_owned = elements.iter().all(|element| {
            !uses.escaping.contains(element)
                // bug-1000: nor handed to code the walk cannot read.
                && !uses.kept_by_unread_code.contains(element)
                && !stores
                    .stores
                    .get(element)
                    .is_some_and(|s| s.iter().any(|s| matches!(s, Source::WrapLocal(_))))
                && resolver.local_fresh(element, stores, params, &mut HashSet::new())
        });
        if elements_owned {
            owning.insert(collection.clone());
        }
    }
    store_floats.close_only = StoreFloatProof {
        function,
        record_container,
        stores,
        params,
        store_floats,
        uses: &uses,
        owning: &owning,
        resolver: Resolver {
            functions,
            record_type,
            floats: vec![all_floated],
            memo: HashMap::new(),
            visiting_functions: HashSet::new(),
        },
        memo: HashMap::new(),
        visiting: HashSet::new(),
    }
    .close_only();
    let block_owning = block_owning_collections(stores, &uses, &owning);
    (owning, exclusive_temps, block_owning)
}

/// bug-1053: the `RES` collections whose own block the binding frees at its
/// scope's end.
///
/// [`owning_collections`] asks one question for two things: whether the drain may
/// free the RECORDS the collection lists, and whether the binding may free the
/// collection's own BLOCK. A second reader of an element answers "no" to both,
/// and it is no holder of the block: `pick(hs)` that appends a `MATCH` case
/// binding of each element to the list it returns keeps `hs`'s elements, and
/// `hs`'s block (160 bytes for two handles) was kept with them on every call.
///
/// The block of a `List OF RES T` or `Map OF K TO RES T` is held by whatever
/// holds its pointer, and nothing else: an element read out of it is a record
/// pointer (or a copy of a union box), never a pointer into a block that outlives
/// the read. So the binding is its block's one holder when
///
/// * every store into it installs a fresh block (the same rule as
///   [`owning_collections`]);
/// * it is named, outside a call's argument list, only as a `FOR EACH` iterable:
///   a bind (`MUT d = c` shares the block), a `RETURN`, a constructor, literal
///   or capture each hold the pointer;
/// * no call keeps the block: it is not a `collections` member's non-receiver
///   argument, not handed to code the walk cannot read, and not an argument a
///   callee's body names anywhere but in such positions
///   (`CollectionUses::block_kept`).
///
/// Asked only of a collection whose elements are resources. One whose elements
/// are themselves collections is held to the whole of [`owning_collections`]:
/// what a `FOR EACH` over it yields IS a block.
fn block_owning_collections(
    stores: &Stores,
    uses: &CollectionUses,
    owning: &HashSet<String>,
) -> HashSet<String> {
    let mut block_owning = owning.clone();
    for (name, type_) in &stores.types {
        if CodeBuilder::is_res_marked_resource_collection(type_)
            && uses.block_is_fresh(name)
            && !uses.escaping_beyond_iteration.contains(name)
            && !uses.block_kept.contains(name)
        {
            block_owning.insert(name.clone());
        }
    }
    block_owning
}

/// bug-1045: the containers whose drain may only CLOSE the records a
/// [`StoreFloats`] store handed them, never free them.
///
/// A container's drain frees what it closes when it is the one owner of
/// everything it lists ([`owning_collections`]). A record a store raised into it
/// came out of another binding, so that binding must be shown to leave no other
/// holder behind, with the raising stores themselves set aside (they are the
/// hand-over, and [`CollectionUses`] skips them):
///
/// * a record that reaches the resource (`LET k AS HB = mk()`) must itself be an
///   owning container by [`owning_collections`]' rules: a fresh block, named
///   nowhere that keeps it. Its own drain then frees whatever it still owns at its
///   scope's end (a store that did not run) and skips what was raised;
/// * a `RES` binding the escape analysis left in its own scope must be a fresh
///   producer record that does not escape, by the floated element's rule;
/// * a `FOR EACH` variable is a reference to an element of its iterable, so the
///   iterable must be an owning container, and proved in turn when it is itself a
///   container such a store raised into;
/// * a collection is held by pointer where it is stored, so nothing is proved for
///   it.
///
/// A source stored into several containers needs no rule of its own. Each store
/// raises the record to its container's scope when that scope is further out, so
/// the record ends owned by the outermost of them, and a drain closes and frees
/// only a record its own scope still owns: one list frees it, and every other
/// container that holds the reference is in that scope or inside it.
///
/// Anything else answers close-only: the records are closed by the container's
/// scope, as the language requires, and their blocks are kept, never freed twice.
/// [`StoreFloatProof::close_only`] is the answer.
struct StoreFloatProof<'a, 'm, 'f, 'p, 'u> {
    function: &'a NirFunction,
    record_container: &'a dyn Fn(&ParameterType) -> bool,
    stores: &'a Stores,
    params: &'a HashSet<String>,
    store_floats: &'a StoreFloats,
    uses: &'a CollectionUses<'u, 'f, 'p>,
    owning: &'a HashSet<String>,
    resolver: Resolver<'m, 'f, 'a>,
    memo: HashMap<String, bool>,
    visiting: HashSet<String>,
}

impl StoreFloatProof<'_, '_, '_, '_, '_> {
    fn close_only(mut self) -> HashSet<String> {
        let store_floats = self.store_floats;
        store_floats
            .sources
            .keys()
            .filter(|container| !self.proved(container))
            .cloned()
            .collect()
    }

    /// Whether everything raised into `container` came out of a binding that
    /// leaves no other holder behind. A `FOR EACH` variable answers for its
    /// iterable, which may itself be such a container, so the answer is computed
    /// on demand; a cycle answers "not proved".
    fn proved(&mut self, container: &str) -> bool {
        if let Some(known) = self.memo.get(container) {
            return *known;
        }
        let store_floats = self.store_floats;
        let Some(sources) = store_floats.sources.get(container) else {
            return true;
        };
        if !self.visiting.insert(container.to_string()) {
            return false;
        }
        let proved = sources.iter().all(|source| self.source_proved(source));
        self.visiting.remove(container);
        self.memo.insert(container.to_string(), proved);
        proved
    }

    fn source_proved(&mut self, source: &StoreSource) -> bool {
        let name = &source.name;
        let unheld = !self.uses.escaping.contains(name)
            && !self.uses.kept.contains(name)
            && !self.uses.kept_by_unread_code.contains(name);
        let record = self
            .stores
            .types
            .get(name)
            .is_some_and(self.record_container);
        if let Some(iterable) = &source.iterates {
            return unheld && self.owning.contains(iterable) && self.proved(iterable);
        }
        if self.function.resource_owners.contains_key(name) && !record {
            return unheld
                && !self
                    .stores
                    .stores
                    .get(name)
                    .is_some_and(|s| s.iter().any(|s| matches!(s, Source::WrapLocal(_))))
                && self
                    .resolver
                    .local_fresh(name, self.stores, self.params, &mut HashSet::new());
        }
        record && self.owning.contains(name)
    }
}

/// Where each local is NAMED, for [`owning_collections`]: the positions that can create
/// a second holder of a block, and the calls that hand a resource pointer back out of a
/// collection.
struct CollectionUses<'m, 'f, 'p> {
    /// Every function in the program, to ask what a callee does with an argument.
    functions: &'f HashMap<String, &'f NirFunction>,
    /// What each callee's parameters are known to do, shared across the walk.
    callees: &'m mut CalleeParamUses<'p>,
    /// Locals named as a bare `Local` somewhere other than a call's argument list.
    escaping: HashSet<String>,
    /// bug-801: locals whose BLOCK a call keeps — a `collections` member's non-receiver
    /// argument (`collections::append(out, piece)` stores `piece`'s block pointer in
    /// `out`), or an argument the callee's own body keeps. Asked of collections only:
    /// a resource element passed this way is the float the owning list exists for.
    kept: HashSet<String>,
    /// Collections named as an argument to a borrowed-element read
    /// (`CodeBuilder::value_aliases_live_resource`): `collections::get`/`getOr`, or a
    /// `*::poll` over a socket list.
    element_readers: HashSet<String>,
    /// Per local: whether EVERY store into it is a fresh collection block.
    fresh_block: HashMap<String, bool>,
    /// bug-753: the locals in `escaping` named somewhere other than a `RETURN`'s value.
    escaping_outside_return: HashSet<String>,
    /// bug-753: per local, whether every store is a call to an
    /// [`INNER_OWNING_BUILTIN_BODIES`] body.
    chunk_stores: HashMap<String, bool>,
    /// bug-753: every `RETURN`'s value, for asking whether the function hands back
    /// only blocks it built.
    returns: Vec<NirValue>,
    /// Set while the walk is inside a `RETURN`'s value.
    in_return: bool,
    /// bug-1000: the locals in `escaping` named somewhere other than as a
    /// `FOR EACH` iterable. Iterating a collection leaves no holder behind unless
    /// the loop variable itself goes somewhere (see [`Self::owner_held_elsewhere`]).
    escaping_beyond_iteration: HashSet<String>,
    /// bug-1000: every `FOR EACH <variable> IN <local>`, as `(local, variable)`.
    ///
    /// bug-1028: and the three references a `MATCH` makes, which live by the same
    /// rule (see [`is_match_view`]): the scrutinee temporary of the local it was
    /// bound from, as `(local, temporary)`; a case binding of that temporary, as
    /// `(temporary, binding)`; and the temporary of the collection a borrowed
    /// element was read from, as `(collection, temporary)`.
    iterations: Vec<(String, String)>,
    /// Set while the walk is on a `FOR EACH`'s iterable, or on the one local a
    /// `MATCH` reference is derived from.
    in_iterable: bool,
    /// bug-1000: set, for the walk of one call argument, when that argument is a
    /// borrowed-element read whose result the receiving call does not keep.
    transient_borrow: bool,
    /// bug-1000: collections a borrowed-element read names only transiently (see
    /// `transient_borrow`). Kept apart from `element_readers` for the float
    /// targets that still take the strict rule
    /// ([`Self::block_held_elsewhere_strict`]).
    transient_readers: HashSet<String>,
    /// bug-999: call nodes (by address) whose aggregate result is their statement's
    /// alone, so the statement adopts what the result reaches as a binding of the
    /// same call would (`LET t = mkList()` then `len(t)`): claim at once, and at
    /// the end of the statement close AND free. Three facts make it so:
    ///
    /// * the call is written straight into an argument of a call that does not
    ///   keep that argument (a natively lowered builtin that reads through the
    ///   pointer, or a body read and found not to keep it), does not consume it
    ///   and reaches no code this walk cannot read;
    /// * that receiving call can only answer with a plain scalar
    ///   ([`Self::call_returns_plain_scalar`]), so nothing of the argument comes
    ///   back out of it;
    /// * the call itself runs a function BODY whose parameters are all plain
    ///   scalars ([`Self::call_takes_only_plain_scalars`]), so what its result
    ///   reaches unowned came out of its own frame.
    ///
    /// The value says whether the result's own block is one the callee built
    /// ([`Self::user_call_returns_fresh_block`]). Every other position an
    /// aggregate-returning call can be written in is left alone: a constructor
    /// argument, a member read, a `FOR EACH` iterable and a bound, assigned,
    /// stored or returned value can all keep the aggregate past the statement
    /// without owning what it reaches.
    exclusive_temps: HashMap<usize, bool>,
    /// bug-1000: the locals handed to code this walk cannot read
    /// ([`Self::call_reaches_unread_code`]): a callback that keeps what it is
    /// passed, or a call through a function value. Asked of a floated ELEMENT,
    /// which `kept` cannot be: an element is in `kept` by construction (the
    /// `append` that floats it), and a named callee that keeps its parameter is
    /// no hazard for one — what it keeps dies with its frame, comes back as a
    /// result the caller's binding adopts by the owner test, or is raised to the
    /// module instance. Unread code can hold a reference to an outer scope's
    /// list (a `LAMBDA`'s by-reference capture), which none of those cover.
    kept_by_unread_code: HashSet<String>,
    /// bug-1000: the locals and parameters of this function that hold a function
    /// value — a call through one, or a call handed one, runs code this walk
    /// cannot read.
    function_values: HashSet<String>,
    /// bug-1045: the operand nodes (by address) a [`StoreFloats`] store raises into
    /// its container out of a record or a `RES` binding (`StoreFloats::handed_over`).
    /// Such an operand is the hand-over itself, not a second holder:
    /// the store floats every resource it reaches to the container's scope, and the
    /// source's own drain skips what it no longer owns. The walk sets it aside.
    store_operands: HashSet<usize>,
    /// bug-1045: the raised roots that are records (copied by value where stored).
    store_record_roots: HashSet<String>,
    /// bug-1053: the locals whose own BLOCK a call can keep: the subset of `kept`
    /// that leaves out a callee that keeps only what it reads out of the argument
    /// (an element, a `MATCH` case binding, a field), which is a second reader of
    /// the elements and no holder of the block. See [`block_owning_collections`].
    block_kept: HashSet<String>,
}

/// Per callee, per parameter position: whether the callee keeps that argument's block
/// past the call (bug-801). Memoized; a function still being resolved answers
/// fail-safe.
#[derive(Default)]
struct CalleeParamUses<'p> {
    /// bug-1025: whether a binding of that type is a record that reaches a
    /// resource (see [`CollectionUses::visit_op`], the copy arm). `None` knows no
    /// such type, which answers "held" for every copy.
    record_container: Option<&'p dyn Fn(&ParameterType) -> bool>,
    known: HashMap<String, Vec<bool>>,
    /// bug-1053: per callee, per parameter: whether the callee keeps the
    /// argument's own block (`CollectionUses::block_kept`).
    known_block: HashMap<String, Vec<bool>>,
    visiting: HashSet<String>,
    /// bug-753: per function, whether every `RETURN` hands back a block it built.
    fresh_returns: HashMap<String, bool>,
    fresh_visiting: HashSet<String>,
    /// bug-1000: per call target, whether it is a builtin.
    builtins: HashMap<String, bool>,
}

impl<'m, 'f, 'p> CollectionUses<'m, 'f, 'p> {
    fn of(
        function: &NirFunction,
        functions: &'f HashMap<String, &'f NirFunction>,
        callees: &'m mut CalleeParamUses<'p>,
    ) -> Self {
        Self::of_with(function, functions, callees, &StoreFloats::default())
    }

    /// [`Self::of`] for the function whose [`StoreFloats`] are known.
    fn of_with(
        function: &NirFunction,
        functions: &'f HashMap<String, &'f NirFunction>,
        callees: &'m mut CalleeParamUses<'p>,
        store_floats: &StoreFloats,
    ) -> Self {
        let mut collector = CollectionUses {
            functions,
            callees,
            escaping: HashSet::new(),
            kept: HashSet::new(),
            element_readers: HashSet::new(),
            fresh_block: HashMap::new(),
            escaping_outside_return: HashSet::new(),
            chunk_stores: HashMap::new(),
            returns: Vec::new(),
            in_return: false,
            escaping_beyond_iteration: HashSet::new(),
            iterations: Vec::new(),
            in_iterable: false,
            transient_borrow: false,
            transient_readers: HashSet::new(),
            exclusive_temps: HashMap::new(),
            kept_by_unread_code: HashSet::new(),
            function_values: function
                .params
                .iter()
                .filter(|param| matches!(param.type_, ParameterType::Func(..)))
                .map(|param| param.name.clone())
                .collect(),
            store_operands: store_floats.handed_over.clone(),
            store_record_roots: store_floats.record_roots.clone(),
            block_kept: HashSet::new(),
        };
        collector.visit_ops(&function.body);
        collector
    }

    fn is_store_operand(&self, value: &NirValue) -> bool {
        self.store_operands
            .contains(&(value as *const NirValue as usize))
    }

    /// Which of `target`'s arguments it keeps, when its body is in the program. `None`
    /// for a natively lowered builtin, whose arguments are uses (see `visit_value`).
    fn callee_keeps(&mut self, target: &str) -> Option<Vec<bool>> {
        if let Some(known) = self.callees.known.get(target) {
            return Some(known.clone());
        }
        let callee = self.functions.get(target).copied()?;
        if !self.callees.visiting.insert(target.to_string()) {
            // Recursion: answer "escapes" until the cycle resolves, never "owned".
            return Some(vec![true; callee.params.len()]);
        }
        let inner = CollectionUses::of(callee, self.functions, self.callees);
        let keeps: Vec<bool> = callee
            .params
            .iter()
            // bug-1000: a parameter the callee only iterates is not kept, by the
            // same rule as a local (`block_held_elsewhere`).
            .map(|p| {
                inner.escaping_beyond_iteration.contains(&p.name)
                    || inner.kept.contains(&p.name)
                    || inner.iterated_reference_leaves(&p.name, 64)
            })
            .collect();
        // bug-1053: the block alone. What the callee reads out of the parameter
        // and keeps is not the parameter's block.
        let block_keeps: Vec<bool> = callee
            .params
            .iter()
            .map(|p| {
                inner.escaping_beyond_iteration.contains(&p.name)
                    || inner.block_kept.contains(&p.name)
            })
            .collect();
        self.callees.visiting.remove(target);
        self.callees.known.insert(target.to_string(), keeps.clone());
        self.callees
            .known_block
            .insert(target.to_string(), block_keeps);
        Some(keeps)
    }

    /// bug-1053: [`Self::callee_keeps`] for the argument's own block. Call it
    /// after `callee_keeps(target)`, which fills both answers; a callee still
    /// being resolved (recursion) answers "keeps".
    fn callee_block_keeps(&self, target: &str) -> Option<Vec<bool>> {
        if let Some(known) = self.callees.known_block.get(target) {
            return Some(known.clone());
        }
        self.functions
            .get(target)
            .map(|callee| vec![true; callee.params.len()])
    }

    /// bug-1000: whether a call to `target` with `args` can hand its arguments to
    /// code that keeps them where this walk cannot see.
    ///
    /// The argument-position rule ("an argument is a use, not a second owner") is
    /// about a callee whose body is read (`callee_keeps`) or a natively lowered
    /// builtin, which reads through the pointer and returns. Two calls are neither:
    ///
    /// * **A call through a function value** (`cb(h)`): the target is a local, so
    ///   the code it runs is unknown here.
    /// * **A call handed a callback** (`collections::forEach(xs, LAMBDA(h) -> keep
    ///   = collections::append(keep, h))`): the builtin passes each element to the
    ///   callback, which may store it in a longer-lived collection. Before this,
    ///   `xs` still counted as the one owner, its drain freed the record, and the
    ///   outer list read freed memory. A callback named here (a `LAMBDA` or a
    ///   function reference) is read: it is harmless when none of the parameters
    ///   that can carry a reference is kept by its body. A callback held in a
    ///   local cannot be read and counts as keeping.
    fn call_reaches_unread_code(&mut self, target: &str, args: &[NirValue]) -> bool {
        let through_function_value = self.function_values.contains(target)
            || (!self.functions.contains_key(target) && !self.is_builtin(target));
        if through_function_value {
            return true;
        }
        args.iter().any(|arg| match arg {
            NirValue::Closure { name, .. } | NirValue::FunctionRef { name, .. } => {
                self.callback_keeps_a_reference(name)
            }
            NirValue::Local(name) => self.function_values.contains(name),
            _ => false,
        })
    }

    /// `builtins::is_builtin_call`, remembered per target: the walk asks it of every
    /// call that names no function in the program.
    fn is_builtin(&mut self, target: &str) -> bool {
        if let Some(known) = self.callees.builtins.get(target) {
            return *known;
        }
        let builtin = crate::codegen::builtins::is_builtin_call(target);
        self.callees.builtins.insert(target.to_string(), builtin);
        builtin
    }

    /// Whether the function `name`, used as a callback, keeps any parameter that
    /// can carry a reference. A parameter of a plain scalar type cannot (a
    /// `reduce` accumulator returned by its reducer is not a holder of anything).
    fn callback_keeps_a_reference(&mut self, name: &str) -> bool {
        let Some(callback) = self.functions.get(name).copied() else {
            return true;
        };
        let Some(keeps) = self.callee_keeps(name) else {
            return true;
        };
        callback
            .params
            .iter()
            .zip(keeps)
            .any(|(param, kept)| kept && !is_plain_scalar(&param.type_))
    }

    /// bug-999: whether `value` calls a function with a body all of whose
    /// parameters are plain scalars. Nothing such a call is given can carry a
    /// reference in, so an unowned record in its result is one its own frame
    /// produced and released: no argument's holder can be among them.
    fn call_takes_only_plain_scalars(&self, value: &NirValue) -> bool {
        // A plain call only: a `CallResult` yields a `Result` wrapper, not the value.
        let NirValue::Call { target, .. } = value else {
            return false;
        };
        self.functions.get(target).is_some_and(|callee| {
            callee
                .params
                .iter()
                .all(|param| is_plain_scalar(&param.type_))
        })
    }

    /// bug-999: whether a call to `target` can only ever answer with a plain scalar
    /// — a value that cannot hold a reference to anything its arguments reached. A
    /// function with a body answers by its declared return; a builtin by every
    /// overload it declares.
    fn call_returns_plain_scalar(&self, target: &str) -> bool {
        if let Some(callee) = self.functions.get(target) {
            return is_plain_scalar(&callee.returns);
        }
        let (package, member) = target.split_once('.').unwrap_or(("general", target));
        crate::codegen::registry::registry()
            .resolve_package(package)
            .and_then(|package| package.function(member))
            .is_some_and(|function| {
                !function.implementations.is_empty()
                    && function
                        .implementations
                        .iter()
                        .all(|implementation| is_plain_scalar(&implementation.return_type))
            })
    }

    /// Whether a holder other than `name` itself can still reach its block or read a
    /// resource pointer out of it after `name`'s scope exits.
    ///
    /// bug-1000: two readings leave no holder behind and do not count:
    ///
    /// * **A `FOR EACH` over `name`.** The loop variable refers to one element (or
    ///   one inner block) for one iteration, inside `name`'s scope. It is a second
    ///   holder only if it goes somewhere itself: bound, returned, captured, stored
    ///   into a collection, kept by a callee, read through `get`, or iterated by a
    ///   loop whose own variable does one of those.
    /// * **A borrowed-element read passed straight to a call that does not keep
    ///   it** (`fs::writeAll(collections::get(xs, 0), "x")`). That is recorded at
    ///   the walk: such a read never reaches `element_readers`.
    ///
    /// Both are the argument-position rule the floated element locals already live
    /// by. Anything else still answers "held".
    fn block_held_elsewhere(&self, name: &str) -> bool {
        self.escaping_beyond_iteration.contains(name)
            || self.kept.contains(name)
            || self.element_readers.contains(name)
            || self.iterated_reference_leaves(name, 64)
    }

    /// [`Self::block_held_elsewhere`] without bug-1000's two readings: named
    /// anywhere outside an argument position, a `FOR EACH` included, or read
    /// through a borrowed-element call at all. For a float target that has no
    /// bind in this body (a parameter), whose stores this walk cannot see.
    ///
    /// bug-1028: a collection that reaches a resource union used to take this
    /// rule too. It was written around a defect bug-1012 removed (an adopted
    /// list's drain freed 16 bytes at the address of each element's box, a span
    /// of the list's own block; a drain now frees no box at all, because an
    /// owned-list node holds a copy of the box's two words). Measured with the
    /// wider rule, such a collection frees its block and its records once
    /// (`tests/runtime/rt_resource_union_reclaim.rs`).
    fn block_held_elsewhere_strict(&self, name: &str) -> bool {
        self.escaping.contains(name)
            || self.kept.contains(name)
            || self.element_readers.contains(name)
            || self.transient_readers.contains(name)
    }

    /// Whether the loop variable of some `FOR EACH` over `name` leaves its loop.
    fn iterated_reference_leaves(&self, name: &str, fuel: usize) -> bool {
        self.iterations
            .iter()
            .filter(|(source, _)| source == name)
            .any(|(_, variable)| {
                fuel == 0
                    || self.escaping_beyond_iteration.contains(variable)
                    || self.kept.contains(variable)
                    || self.element_readers.contains(variable)
                    || self.iterated_reference_leaves(variable, fuel - 1)
            })
    }

    fn block_is_fresh(&self, name: &str) -> bool {
        self.fresh_block.get(name).copied().unwrap_or(false)
    }

    fn note_store(&mut self, name: &str, value: Option<&NirValue>) {
        let fresh = collection_store_is_fresh(name, value)
            || value.is_some_and(|value| self.user_call_returns_fresh_block(value))
            // bug-1045: a record raised into `name` by assignment is copied by
            // value, so the block installed is `name`'s own.
            || matches!(value, Some(value @ NirValue::Local(source))
                if self.is_store_operand(value) && self.store_record_roots.contains(source))
            // And a `WITH` over `name` itself that such a store raises a field
            // into builds `name`'s own block, as a self-receiving mutator does.
            || matches!(value, Some(NirValue::WithUpdate { target, updates, .. })
                if matches!(target.as_ref(), NirValue::Local(target) if target == name)
                    && updates.iter().any(|update| self.is_store_operand(&update.value)));
        let entry = self.fresh_block.entry(name.to_string()).or_insert(true);
        *entry &= fresh;
        let chunks = matches!(
            value,
            Some(NirValue::Call { target, .. } | NirValue::CallResult { target, .. })
                if INNER_OWNING_BUILTIN_BODIES.iter().any(|prefix| target.starts_with(prefix))
        );
        let entry = self.chunk_stores.entry(name.to_string()).or_insert(true);
        *entry &= chunks;
    }
}

impl CollectionUses<'_, '_, '_> {
    /// bug-753: whether `value` calls a function whose every `RETURN` hands back a
    /// block that function built — so the caller's binding is the block's one
    /// holder. A `RETURN` qualifies when its value is a collection literal, a call
    /// `collection_store_is_fresh` already trusts, a call to another such function,
    /// or a local whose every store is fresh, that no call keeps, and that is named
    /// nowhere but in `RETURN`s. A parameter never qualifies: it is the caller's own
    /// block (`FUNC same(xs) … RETURN xs`). Recursion answers "not fresh" until it
    /// resolves.
    fn user_call_returns_fresh_block(&mut self, value: &NirValue) -> bool {
        let (NirValue::Call { target, .. } | NirValue::CallResult { target, .. }) = value else {
            return false;
        };
        if let Some(known) = self.callees.fresh_returns.get(target) {
            return *known;
        }
        let Some(callee) = self.functions.get(target).copied() else {
            return false;
        };
        if !self.callees.fresh_visiting.insert(target.clone()) {
            return false;
        }
        let mut inner = CollectionUses::of(callee, self.functions, self.callees);
        let returns = std::mem::take(&mut inner.returns);
        let fresh = !returns.is_empty()
            && returns.iter().all(|returned| match returned {
                NirValue::ListLiteral { .. }
                | NirValue::SetLiteral { .. }
                | NirValue::MapLiteral { .. }
                // bug-1025: a record the function built is a block of its own.
                | NirValue::Constructor { .. } => true,
                NirValue::Local(name) => {
                    inner.block_is_fresh(name)
                        && !inner.kept.contains(name)
                        && !inner.escaping_outside_return.contains(name)
                }
                call => {
                    collection_store_is_fresh("", Some(call))
                        || inner.user_call_returns_fresh_block(call)
                }
            });
        self.callees.fresh_visiting.remove(target);
        self.callees.fresh_returns.insert(target.clone(), fresh);
        fresh
    }
}

impl CollectionUses<'_, '_, '_> {
    /// bug-1025: whether a binding of `type_` is a record that reaches a resource.
    fn is_record_container(&self, type_: &ParameterType) -> bool {
        self.callees
            .record_container
            .is_some_and(|record_container| record_container(type_))
    }

    /// Walk the one local a `FOR EACH` variable, a `MATCH` reference, a record
    /// copy or a field read is derived from: it lands in `escaping` (every strict
    /// verdict keeps reading it) but not in `escaping_beyond_iteration`.
    fn visit_derived_from(&mut self, source: &NirValue) {
        let previous = self.in_iterable;
        self.in_iterable = true;
        self.visit_value(source);
        self.in_iterable = previous;
    }
}

/// bug-1028: whether `name` is the temporary the `MATCH` desugar binds its
/// scrutinee to (`Bind $matchN = <scrutinee>`, `ir::lower`). No source program can
/// spell the name.
///
/// A `MATCH` reads its scrutinee in two places only: as the `MATCH`'s own value,
/// which reads the tag, and inside the `UnionExtract` each case binding is
/// initialized from. So the temporary is a reference to what it was bound from
/// that lasts for the `MATCH` and no longer, and a case binding is a reference to
/// the variant the scrutinee holds. Each is a holder only if it goes somewhere
/// itself, which is the rule a `FOR EACH` variable lives by (bug-1000), and
/// [`CollectionUses::iterated_reference_leaves`] asks it of all of them.
///
/// Before this, `MATCH h` named `h` outside an argument position, so a function
/// that only inspected a resource-union parameter counted as keeping it. A
/// resource union can be read no other way, so every `List`/`Map OF RES <Union>`
/// whose elements were looked at lost its owner and kept its block.
///
/// Only those readings are recognized. The temporary named anywhere else, or a
/// case binding bound, returned, captured, stored or kept by a callee, lands in
/// the ordinary sets and answers "held".
fn is_match_view(name: &str) -> bool {
    name.starts_with("$match")
}

impl NirVisitor for CollectionUses<'_, '_, '_> {
    fn visit_op(&mut self, op: &NirOp) {
        match op {
            NirOp::Return { value: Some(value) } => {
                self.returns.push(value.clone());
                let previous = self.in_return;
                self.in_return = true;
                self.visit_value(value);
                self.in_return = previous;
                return;
            }
            // bug-1028: the `MATCH` desugar's scrutinee temporary is a reference to
            // the local it reads, for the length of the `MATCH`.
            NirOp::Bind {
                name,
                value: Some(value @ NirValue::Local(source)),
                ..
            } if is_match_view(name) => {
                self.note_store(name, Some(value));
                self.iterations.push((source.clone(), name.clone()));
                self.visit_derived_from(value);
                return;
            }
            // bug-1028: and so is one bound from a borrowed-element read, to the
            // collection the element was read from. The read is transient exactly
            // as one written into an argument is: the temporary is read by the
            // `MATCH` and its case bindings and by nothing else.
            NirOp::Bind {
                name,
                value:
                    Some(
                        value @ (NirValue::Call { target, args, .. }
                        | NirValue::CallResult { target, args, .. }
                        | NirValue::RuntimeCall { target, args, .. }),
                    ),
                ..
            } if is_match_view(name) && CodeBuilder::target_returns_borrowed_resource(target) => {
                self.note_store(name, Some(value));
                if let Some(NirValue::Local(source)) = args.first() {
                    self.iterations.push((source.clone(), name.clone()));
                    self.transient_borrow = true;
                }
                self.visit_value(value);
                self.transient_borrow = false;
                return;
            }
            // bug-1028: a case binding is a reference to the variant its scrutinee
            // holds. The scrutinee is the temporary, or the local itself where the
            // optimizer forwarded an immutable one (a `FOR EACH` variable).
            NirOp::Bind {
                name,
                value:
                    Some(
                        value @ NirValue::UnionExtract {
                            value: scrutinee, ..
                        },
                    ),
                ..
            } if matches!(scrutinee.as_ref(), NirValue::Local(_)) => {
                self.note_store(name, Some(value));
                if let NirValue::Local(scrutinee) = scrutinee.as_ref() {
                    self.iterations.push((scrutinee.clone(), name.clone()));
                }
                self.visit_derived_from(scrutinee);
                return;
            }
            // bug-1028: the `MATCH` itself reads its scrutinee's tag and keeps
            // nothing.
            NirOp::Match {
                value: value @ NirValue::Local(_),
                cases,
            } => {
                self.visit_derived_from(value);
                for case in cases {
                    match &case.pattern {
                        NirMatchPattern::Else => {}
                        NirMatchPattern::Value(value) => self.visit_value(value),
                        NirMatchPattern::OneOf(values) => {
                            for value in values {
                                self.visit_value(value);
                            }
                        }
                    }
                    if let Some(guard) = &case.guard {
                        self.visit_value(guard);
                    }
                    self.visit_ops(&case.body);
                }
                return;
            }
            // bug-1025: a record that reaches a resource is copied by value, and the
            // copy holds the same handles. Bound from another such record (`LET k3 =
            // k2`, `LET k2 = WITH k { n := 9 }`), the copy lives inside the scope of
            // the record it was made from, so it is a holder only if it goes
            // somewhere itself: the `FOR EACH` variable's rule again. A collection
            // is not copied by a bind (the two names share one block), so this is
            // asked of records only; and only of a bind, never an assignment, whose
            // target can outlive the source.
            NirOp::Bind {
                name,
                type_,
                value: Some(value),
                ..
            } if match value {
                NirValue::Local(_) => true,
                NirValue::WithUpdate { target, .. } => {
                    matches!(target.as_ref(), NirValue::Local(_))
                }
                _ => false,
            } && self.is_record_container(type_) =>
            {
                self.note_store(name, Some(value));
                match value {
                    NirValue::WithUpdate {
                        target, updates, ..
                    } => {
                        if let NirValue::Local(source) = target.as_ref() {
                            self.iterations.push((source.clone(), name.clone()));
                        }
                        self.visit_derived_from(target);
                        for update in updates {
                            self.visit_value(&update.value);
                        }
                    }
                    source => {
                        if let NirValue::Local(source) = source {
                            self.iterations.push((source.clone(), name.clone()));
                        }
                        self.visit_derived_from(source);
                    }
                }
                return;
            }
            // bug-1025: a field read bound to a plain scalar (`LET n = k.n`) copies
            // a value that holds nothing.
            NirOp::Bind {
                name,
                type_,
                value: Some(value @ NirValue::MemberAccess { .. }),
                ..
            } if is_plain_scalar(type_) && field_read_root(value).is_some() => {
                self.note_store(name, Some(value));
                self.visit_derived_from(value);
                return;
            }
            NirOp::Bind {
                name, type_, value, ..
            } => {
                if matches!(type_, ParameterType::Func(..)) {
                    self.function_values.insert(name.clone());
                }
                self.note_store(name, value.as_ref());
            }
            // bug-1045: `r = WITH r { h := k.h }` where the store raises `k.h` into
            // `r`: the target names the binding being assigned, which the
            // statement replaces, so it is no second holder of `r`'s block.
            NirOp::Assign {
                name,
                value:
                    value @ NirValue::WithUpdate {
                        target, updates, ..
                    },
            } if matches!(target.as_ref(), NirValue::Local(target) if target == name)
                && updates
                    .iter()
                    .any(|update| self.is_store_operand(&update.value)) =>
            {
                self.note_store(name, Some(value));
                for update in updates {
                    self.visit_value(&update.value);
                }
                return;
            }
            NirOp::Assign { name, value } => self.note_store(name, Some(value)),
            // bug-1000: the iterable still lands in `escaping` (every other verdict
            // keeps reading it), but not in `escaping_beyond_iteration`.
            NirOp::ForEach {
                name,
                iterable: iterable @ NirValue::Local(source),
                body,
                ..
            } => {
                self.iterations.push((source.clone(), name.clone()));
                self.in_iterable = true;
                self.visit_value(iterable);
                self.in_iterable = false;
                self.visit_ops(body);
                return;
            }
            // `s.state.f = v` desugars to a whole-state `WITH` whose target is, BY
            // CONSTRUCTION, `MemberAccess{Local(<resource>), "state"}` — the op names the
            // resource itself. That target is a write THROUGH the handle to the block at
            // `RESOURCE_OFFSET_STATE`; it leaves no second holder of the record, so
            // skipping it is what lets a floated `RES s AS Stream STATE S` element still
            // qualify. Only the target is skipped: each update's own value walks
            // normally, and an op whose target is not that exact shape falls through to
            // the ordinary (escaping) walk.
            NirOp::StateAssign {
                resource,
                value:
                    NirValue::WithUpdate {
                        target, updates, ..
                    },
            } if matches!(
                target.as_ref(),
                NirValue::MemberAccess { target, member }
                    if member == "state"
                        && matches!(target.as_ref(), NirValue::Local(name) if name == resource)
            ) =>
            {
                for update in updates {
                    self.visit_value(&update.value);
                }
                return;
            }
            _ => {}
        }
        walk_op(self, op);
    }

    fn visit_value(&mut self, value: &NirValue) {
        // bug-1045: the operand a store raises into its container (see the field).
        if self.is_store_operand(value) {
            return;
        }
        match value {
            // Any other position that names a local can leave a second holder behind,
            // so the default is "escapes" and only the argument arm below narrows it —
            // a NIR variant added tomorrow lands here and answers fail-safe.
            NirValue::Local(name) => {
                self.escaping.insert(name.clone());
                if !self.in_return {
                    self.escaping_outside_return.insert(name.clone());
                }
                if !self.in_iterable {
                    self.escaping_beyond_iteration.insert(name.clone());
                }
            }
            NirValue::Call { target, args, .. }
            | NirValue::CallResult { target, args, .. }
            | NirValue::RuntimeCall { target, args, .. } => {
                // Whether THIS call's result is only used by a call that does not
                // keep it; consumed here so nothing nested inherits it by accident.
                let transient = std::mem::take(&mut self.transient_borrow);
                let borrowed = CodeBuilder::target_returns_borrowed_resource(target);
                if borrowed {
                    let readers = if transient {
                        &mut self.transient_readers
                    } else {
                        &mut self.element_readers
                    };
                    for arg in args {
                        // bug-1025: a field read names the record it reads through.
                        if let Some(name) = field_read_root(arg) {
                            readers.insert(name.to_string());
                        }
                    }
                }
                // An argument to a natively lowered builtin is a USE, not a second
                // owner: `len(c)`, the receiver of `collections::append(c, x)` and
                // `udp::close(s)` all read through the pointer and none of them keep it.
                //
                // bug-801: two argument positions DO keep a collection's block. A `collections` member's
                // non-receiver argument can be stored as an element —
                // `collections::append(out, piece)` puts `piece`'s block pointer inside
                // `out` — so it escapes. And a callee with a body (a user FUNC, or a
                // builtin's source body such as `chunks`'s `__collections_slice`) is
                // asked what its own parameter does: `FUNC box(xs) … append(out, xs)
                // … RETURN out` lets the caller's argument escape exactly as a direct
                // append would.
                let stores_elements = target.starts_with("collections.");
                let callee = self.callee_keeps(target);
                let callee_block = self.callee_block_keeps(target);
                // bug-1000: a consuming parameter takes over what its argument
                // reaches (`deactivate_consumed_cleanups`), which is keeping it.
                let consumes = crate::codegen::resource::builtin_consuming_parameter_index(target);
                // bug-1000: and so does code this walk cannot read.
                let unread = self.call_reaches_unread_code(target, args);
                for (index, arg) in args.iter().enumerate() {
                    let keeps = unread
                        || (stores_elements && index > 0)
                        || callee
                            .as_ref()
                            .is_some_and(|keeps| keeps.get(index).copied().unwrap_or(true));
                    let block_keeps = unread
                        || (stores_elements && index > 0)
                        || callee_block
                            .as_ref()
                            .is_some_and(|keeps| keeps.get(index).copied().unwrap_or(true));
                    // bug-1046: a local wrapped for a union-typed position
                    // (`append(hs, s)` with `s` a `fs::File` and `hs` a `List OF RES
                    // Handle`) names the local in that argument position exactly as
                    // the bare local would: the wrap builds a statement's box around
                    // the same record and is no holder of its own. Walked as an
                    // ordinary value it made every floated member "escape", so the
                    // list that owns it closed its records and freed none of them.
                    let arg = match arg {
                        NirValue::UnionWrap { value, .. }
                            if matches!(value.as_ref(), NirValue::Local(_)) =>
                        {
                            value.as_ref()
                        }
                        other => other,
                    };
                    // bug-1045: the operand a store raises into its container.
                    if self.is_store_operand(arg) {
                        continue;
                    }
                    let NirValue::Local(name) = arg else {
                        // bug-1025: a field read written straight into an argument
                        // position is a use of the record it reads through, exactly
                        // as naming the record there is: kept when the call keeps or
                        // consumes the argument, a use that ends with the call
                        // otherwise.
                        if let (NirValue::MemberAccess { .. }, Some(root)) =
                            (arg, field_read_root(arg))
                        {
                            if keeps || consumes == Some(index) {
                                self.kept.insert(root.to_string());
                                self.block_kept.insert(root.to_string());
                            }
                            if unread {
                                self.kept_by_unread_code.insert(root.to_string());
                            }
                            self.visit_derived_from(arg);
                            continue;
                        }
                        // bug-1000: a borrowed-element read written straight into an
                        // argument position is a use of the element exactly as naming
                        // a floated element local there is — provided this call does
                        // not keep the argument. When this call is itself a borrowed
                        // read (`get(get(nested, 0), 1)`), its receiver is as
                        // transient as its own result. Every other nested value walks
                        // normally.
                        let borrowed_read = matches!(
                            arg,
                            NirValue::Call { target, .. }
                                | NirValue::CallResult { target, .. }
                                | NirValue::RuntimeCall { target, .. }
                                if CodeBuilder::target_returns_borrowed_resource(target)
                        );
                        self.transient_borrow = borrowed_read
                            && !keeps
                            && consumes != Some(index)
                            && (!borrowed || (transient && index == 0));
                        // bug-999: a call written straight into an argument this
                        // call neither keeps nor can answer with.
                        if !borrowed_read
                            && !borrowed
                            && !keeps
                            && consumes != Some(index)
                            && self.call_takes_only_plain_scalars(arg)
                            && self.call_returns_plain_scalar(target)
                        {
                            let fresh_block = self.user_call_returns_fresh_block(arg);
                            self.exclusive_temps
                                .insert(arg as *const NirValue as usize, fresh_block);
                        }
                        self.visit_value(arg);
                        self.transient_borrow = false;
                        continue;
                    };
                    if keeps {
                        self.kept.insert(name.clone());
                    }
                    if block_keeps {
                        self.block_kept.insert(name.clone());
                    }
                    if unread {
                        self.kept_by_unread_code.insert(name.clone());
                    }
                }
                return;
            }
            // bug-1025: a comparison yields a `Boolean`, which holds nothing, so a
            // field read as its operand is a use that ends with the comparison
            // (`IF k.name = "x" AND k.n = 3`). Any other operand walks normally.
            NirValue::Binary {
                op, left, right, ..
            } if op.is_comparison() => {
                for operand in [left.as_ref(), right.as_ref()] {
                    if matches!(operand, NirValue::MemberAccess { .. })
                        && field_read_root(operand).is_some()
                    {
                        self.visit_derived_from(operand);
                    } else {
                        self.visit_value(operand);
                    }
                }
                return;
            }
            _ => {}
        }
        walk_value(self, value);
    }
}

/// bug-1025: the local a pure field read `x.a.b` reads through (`x` itself for a
/// bare local). `None` for anything else: a read through a call, an index or an
/// extract is not recognized and walks as an ordinary value.
fn field_read_root(value: &NirValue) -> Option<&str> {
    match value {
        NirValue::Local(name) => Some(name),
        NirValue::MemberAccess { target, .. } => field_read_root(target),
        _ => None,
    }
}

/// A type whose values hold no reference to anything: no collection, record,
/// union, resource or function value.
fn is_plain_scalar(type_: &ParameterType) -> bool {
    matches!(
        type_,
        ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Integer
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::String
    )
}

/// The `collections::` members that mutate a collection and are stored back into it.
/// Each either grows the block in place or allocates a replacement and frees the
/// original, so the destination stays the block's one owner.
const COLLECTION_SELF_MUTATORS: &[&str] = &[
    "collections.append",
    "collections.prepend",
    "collections.insert",
    "collections.set",
    "collections.add",
    "collections.remove",
    "collections.removeAt",
    "collections.removeKey",
    "collections.repeat",
];

/// Whether one store into `dest` leaves it holding a block nothing else owns.
fn collection_store_is_fresh(dest: &str, value: Option<&NirValue>) -> bool {
    let Some(value) = value else {
        // A bind with no initializer materializes its own default empty collection.
        return true;
    };
    match value {
        NirValue::ListLiteral { .. }
        | NirValue::SetLiteral { .. }
        | NirValue::MapLiteral { .. } => true,
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => {
            (COLLECTION_SELF_MUTATORS.contains(&target.as_str())
                && matches!(args.first(), Some(NirValue::Local(name)) if name == dest))
                || FRESH_NATIVE_MEMBERS.contains(&target.as_str())
                || FRESH_BUILTIN_BODIES
                    .iter()
                    .any(|prefix| target.starts_with(prefix))
        }
        _ => false,
    }
}

/// Built-in source bodies audited to return a block nothing else holds (bug-753),
/// matched by their instantiated-name prefix.
///
/// * `#collections_slice$` — `gen_slice.rs:try_inline_slice_op` has one path, which
///   allocates the result block and range-copies into it; its declining fallback
///   is the source FUNC, which builds a fresh list from `[]` by appends.
/// * `#collections_take$` / `#collections_drop$` — `RETURN __collections_slice(…)`,
///   no fast path of their own.
///
/// A body with a native fast path is listed only after that fast path is audited:
/// the fast path replaces the body at the call, so the body's returns alone prove
/// nothing about what the call hands back.
///
/// A user function's result is judged by its body instead
/// (`CollectionUses::user_call_returns_fresh_block`).
const FRESH_BUILTIN_BODIES: &[&str] = &[
    "#collections_slice$",
    "#collections_take$",
    "#collections_drop$",
    "#collections_chunks$",
    "#collections_window$",
];

/// Natively lowered `collections` members audited to return a block nothing else
/// holds, for every element type (bug-753). Each successful path ends by returning
/// a block it allocated itself; every failure raises:
///
/// * `filter` — `func_filter.rs:lower_filter` reserves its output and appends into
///   it; no early exit hands back the input, even when nothing is dropped.
/// * `append` / `prepend` / `insert` — `lower_collection_end_insert` /
///   `lower_insert` → `list_mutate.rs:lower_list_insert_collection`, which allocates
///   `count_A + count_B` and returns that block.
/// * `set` — a list: `lower_list_remove_at` then `lower_list_insert_collection`
///   (both fresh, the intermediate freed), or for a fixed-width element an in-place
///   write into a `copy_collection_tight` copy; a map: `lower_map_remove_key` and a
///   singleton concatenated by `lower_map_concat`, both intermediates freed.
/// * `removeAt` — `list_mutate.rs:lower_list_remove_at` allocates and copies.
/// * `mid` — `builder_search.rs:lower_list_mid` allocates and copies on both paths;
///   the no-copy form exists only as a self-update.
///
/// `chunks` and `window` (in [`FRESH_BUILTIN_BODIES`]) keep a native fast path only
/// for numeric and `String` elements; a `RES` element runs the source body, which
/// builds its result from `[]` and appends `__collections_slice` pieces.
///
/// A block-fresh result still holds references to the argument's resources; only
/// the block is the binding's. Self-updates (`xs = append(xs, …)`) are
/// [`COLLECTION_SELF_MUTATORS`], the only lowering that writes an existing block.
const FRESH_NATIVE_MEMBERS: &[&str] = &[
    "collections.filter",
    "collections.append",
    "collections.prepend",
    "collections.insert",
    "collections.set",
    "collections.removeAt",
    "collections.mid",
];

#[cfg(test)]
mod owned_list_ownership_tests {
    use super::*;
    use crate::target::shared::nir::{NirOp, NirSourceLoc};

    /// A function with `params` and `body`, for walking a body or naming a callee.
    fn function(params: &[&str], body: Vec<NirOp>) -> NirFunction {
        NirFunction {
            name: "f".to_string(),
            visibility: String::new(),
            kind: "function".to_string(),
            isolated: false,
            params: params
                .iter()
                .map(|name| crate::target::shared::nir::NirParam {
                    name: name.to_string(),
                    type_: ParameterType::parse("List OF RES fs.File"),
                    default: None,
                })
                .collect(),
            returns: ParameterType::parse("List OF List OF RES fs.File"),
            body,
            file: String::new(),
            resource_owners: HashMap::new(),
        }
    }

    fn call(target: &str, args: Vec<NirValue>) -> NirValue {
        NirValue::Call {
            target: target.to_string(),
            args,
            loc: NirSourceLoc::default(),
        }
    }

    fn local(name: &str) -> NirValue {
        NirValue::Local(name.to_string())
    }

    /// The two stores that leave a `List OF RES X` binding holding a block nothing
    /// else owns — which is what licenses freeing it at the drain (bug-645).
    #[test]
    fn a_literal_and_a_self_receiving_mutator_are_fresh_blocks() {
        let literal = NirValue::ListLiteral {
            type_: ParameterType::parse("List OF RES udp.Socket"),
            values: Vec::new(),
        };
        assert!(collection_store_is_fresh("chans", Some(&literal)));
        let append = call("collections.append", vec![local("chans"), local("c")]);
        assert!(collection_store_is_fresh("chans", Some(&append)));
        // A bind with no initializer materializes its own default empty collection.
        assert!(collection_store_is_fresh("chans", None));
    }

    /// The guard, and the reason it is a *self*-receiving test: a mutator of another
    /// collection that no audit covers may hand back a block that collection still
    /// names. Freeing both at scope exit is the double free this module exists to
    /// refuse. `a = append(b, x)` is not that shape: only a self-update writes an
    /// existing block (`is_self_update_call`), so the value path allocates (bug-753).
    #[test]
    fn a_mutator_of_another_collection_is_not_a_fresh_block() {
        let remove = call("collections.removeKey", vec![local("other"), local("c")]);
        assert!(!collection_store_is_fresh("chans", Some(&remove)));
        let append = call("collections.append", vec![local("other"), local("c")]);
        assert!(collection_store_is_fresh("chans", Some(&append)));
    }

    /// Every other store shape answers "not fresh": an alias of another binding, a
    /// call that hands back somebody else's list, a non-mutating `collections::`
    /// member. Fail-safe is the default, not an enumerated case.
    #[test]
    fn any_other_store_is_not_a_fresh_block() {
        assert!(!collection_store_is_fresh("chans", Some(&local("other"))));
        assert!(!collection_store_is_fresh(
            "chans",
            Some(&call("build", vec![]))
        ));
        assert!(!collection_store_is_fresh(
            "chans",
            Some(&call("collections.slice", vec![local("chans")]))
        ));
    }

    /// An argument is a USE of a collection, not a second owner — `len(c)` and
    /// `collections::append(c, x)` must not disqualify `c`, or the fix would never
    /// fire on the shape §15.6 is written for.
    #[test]
    fn an_argument_position_does_not_escape() {
        let no_functions = HashMap::new();
        let mut callees = CalleeParamUses::default();
        let mut uses = CollectionUses::of(&function(&[], Vec::new()), &no_functions, &mut callees);
        uses.visit_op(&NirOp::Assign {
            name: "chans".to_string(),
            value: call("collections.append", vec![local("chans"), local("c")]),
        });
        uses.visit_op(&NirOp::Assign {
            name: "total".to_string(),
            value: call("len", vec![local("chans")]),
        });
        assert!(!uses.escaping.contains("chans"));
        assert!(!uses.escaping.contains("c"));
        assert!(uses.block_is_fresh("chans"));
    }

    /// Every other position does escape: a bind from the collection, a `RETURN`, a
    /// `FOR EACH` over it. Each leaves a second name on the one block.
    #[test]
    fn a_bind_a_return_and_a_for_each_escape() {
        for op in [
            NirOp::Bind {
                mutable: false,
                name: "same".to_string(),
                type_: ParameterType::parse("List OF RES udp.Socket"),
                value: Some(local("chans")),
            },
            NirOp::Return {
                value: Some(local("chans")),
            },
            NirOp::ForEach {
                name: "h".to_string(),
                type_: ParameterType::parse("udp.Socket"),
                iterable: local("chans"),
                body: Vec::new(),
            },
        ] {
            let no_functions = HashMap::new();
            let mut callees = CalleeParamUses::default();
            let mut uses =
                CollectionUses::of(&function(&[], Vec::new()), &no_functions, &mut callees);
            uses.visit_op(&op);
            assert!(
                uses.escaping.contains("chans"),
                "a collection named outside an argument position must not be freed"
            );
        }
    }

    /// `collections::get`/`getOr` "yield a POINTER to the one resource" (§15.6), so
    /// a list they are applied to keeps the close-only drain: a freed record could
    /// still be live in the reader's hands.
    #[test]
    fn a_borrowed_element_read_disqualifies_its_collection() {
        let no_functions = HashMap::new();
        let mut callees = CalleeParamUses::default();
        let mut uses = CollectionUses::of(&function(&[], Vec::new()), &no_functions, &mut callees);
        uses.visit_op(&NirOp::Bind {
            mutable: false,
            name: "pick".to_string(),
            type_: ParameterType::parse("udp.Socket"),
            value: Some(call(
                "collections.get",
                vec![local("chans"), NirValue::Local("i".to_string())],
            )),
        });
        assert!(uses.element_readers.contains("chans"));
    }

    /// bug-801: a collection passed as a `collections` member's element argument is
    /// KEPT — `append(out, piece)` stores `piece`'s block inside `out` — so `piece`
    /// must not free its block at scope exit. The receiver and a resource element
    /// stay plain uses (the float the owning list exists for).
    #[test]
    fn a_collection_appended_into_another_is_kept() {
        let no_functions = HashMap::new();
        let mut callees = CalleeParamUses::default();
        let mut uses = CollectionUses::of(&function(&[], Vec::new()), &no_functions, &mut callees);
        uses.visit_op(&NirOp::Assign {
            name: "out".to_string(),
            value: call("collections.append", vec![local("out"), local("piece")]),
        });
        assert!(uses.block_held_elsewhere("piece"));
        assert!(!uses.block_held_elsewhere("out"));
        assert!(
            !uses.escaping.contains("piece"),
            "a floated element still qualifies"
        );
    }

    /// bug-801: a callee with a body is asked what its parameter does. `box` appends its
    /// parameter into the list it returns, so the caller's argument is kept; `count`
    /// only measures its parameter, so the caller's argument stays a use.
    #[test]
    fn a_callee_that_keeps_its_parameter_keeps_the_argument() {
        let boxer = function(
            &["xs"],
            vec![
                NirOp::Assign {
                    name: "out".to_string(),
                    value: call("collections.append", vec![local("out"), local("xs")]),
                },
                NirOp::Return {
                    value: Some(local("out")),
                },
            ],
        );
        let counter = function(
            &["xs"],
            vec![NirOp::Return {
                value: Some(call("len", vec![local("xs")])),
            }],
        );
        let functions: HashMap<String, &NirFunction> =
            HashMap::from([("box".to_string(), &boxer), ("count".to_string(), &counter)]);
        let mut callees = CalleeParamUses::default();
        let caller = function(
            &[],
            vec![
                NirOp::Return {
                    value: Some(call("box", vec![local("piece")])),
                },
                NirOp::Return {
                    value: Some(call("count", vec![local("other")])),
                },
            ],
        );
        let uses = CollectionUses::of(&caller, &functions, &mut callees);
        assert!(uses.block_held_elsewhere("piece"));
        assert!(!uses.block_held_elsewhere("other"));
    }

    /// A recursive callee answers fail-safe while it is being resolved.
    #[test]
    fn a_recursive_callee_keeps_its_argument() {
        let looping = function(
            &["xs"],
            vec![NirOp::Return {
                value: Some(call("loop", vec![local("xs")])),
            }],
        );
        let functions: HashMap<String, &NirFunction> =
            HashMap::from([("loop".to_string(), &looping)]);
        let mut callees = CalleeParamUses::default();
        let caller = function(
            &[],
            vec![NirOp::Return {
                value: Some(call("loop", vec![local("piece")])),
            }],
        );
        let uses = CollectionUses::of(&caller, &functions, &mut callees);
        assert!(uses.block_held_elsewhere("piece"));
    }

    /// `s.state.f = v` writes THROUGH the handle: the `WITH` target names the resource
    /// but leaves no second holder of its record, so a floated `RES s AS U STATE S`
    /// element still qualifies. The update's own value keeps walking.
    #[test]
    fn a_state_assign_does_not_escape_its_resource() {
        let no_functions = HashMap::new();
        let mut callees = CalleeParamUses::default();
        let mut uses = CollectionUses::of(&function(&[], Vec::new()), &no_functions, &mut callees);
        uses.visit_op(&NirOp::StateAssign {
            resource: "c".to_string(),
            value: NirValue::WithUpdate {
                type_: ParameterType::declared("Cursor"),
                target: Box::new(NirValue::MemberAccess {
                    target: Box::new(local("c")),
                    member: "state".to_string(),
                }),
                updates: vec![crate::target::shared::nir::NirRecordUpdate {
                    field: "pos".to_string(),
                    value: local("j"),
                }],
            },
        });
        assert!(!uses.escaping.contains("c"));
        assert!(uses.escaping.contains("j"), "the update value still walks");
    }

    /// The guard on that narrowing: a `WITH` target that is NOT this op's own
    /// `<resource>.state` falls through to the ordinary walk and escapes.
    #[test]
    fn a_state_assign_naming_another_resource_still_escapes() {
        let no_functions = HashMap::new();
        let mut callees = CalleeParamUses::default();
        let mut uses = CollectionUses::of(&function(&[], Vec::new()), &no_functions, &mut callees);
        uses.visit_op(&NirOp::StateAssign {
            resource: "c".to_string(),
            value: NirValue::WithUpdate {
                type_: ParameterType::declared("Cursor"),
                target: Box::new(NirValue::MemberAccess {
                    target: Box::new(local("other")),
                    member: "state".to_string(),
                }),
                updates: Vec::new(),
            },
        });
        assert!(uses.escaping.contains("other"));
    }
    fn for_each(variable: &str, iterable: &str, body: Vec<NirOp>) -> NirOp {
        NirOp::ForEach {
            name: variable.to_string(),
            type_: ParameterType::parse("fs.File"),
            iterable: local(iterable),
            body,
        }
    }

    fn eval(value: NirValue) -> NirOp {
        NirOp::Eval { value }
    }

    fn uses_of<'f>(
        body: Vec<NirOp>,
        functions: &'f HashMap<String, &'f NirFunction>,
        callees: &mut CalleeParamUses,
        check: impl FnOnce(&CollectionUses),
    ) {
        let caller = function(&[], body);
        let uses = CollectionUses::of(&caller, functions, callees);
        check(&uses);
    }

    /// bug-1000: iterating a list leaves no holder behind when the loop variable is
    /// only used as an argument, so the list still owns what it holds.
    #[test]
    fn a_for_each_whose_variable_stays_in_the_loop_does_not_hold_the_list() {
        let no_functions = HashMap::new();
        uses_of(
            vec![for_each(
                "h",
                "files",
                vec![eval(call("fs.readLine", vec![local("h")]))],
            )],
            &no_functions,
            &mut CalleeParamUses::default(),
            |uses| {
                assert!(!uses.block_held_elsewhere("files"));
                assert!(
                    uses.escaping.contains("files"),
                    "the reassign-free verdict still sees the iteration"
                );
            },
        );
    }

    /// The guard on that: a loop variable that is bound, stored into another
    /// collection, read through `get`, or iterated by a loop whose own variable
    /// leaves, is a second holder.
    #[test]
    fn a_for_each_whose_variable_leaves_holds_the_list() {
        let no_functions = HashMap::new();
        let leaving_bodies = [
            vec![NirOp::Bind {
                mutable: false,
                name: "keep".to_string(),
                type_: ParameterType::parse("fs.File"),
                value: Some(local("h")),
            }],
            vec![NirOp::Assign {
                name: "out".to_string(),
                value: call("collections.append", vec![local("out"), local("h")]),
            }],
            vec![NirOp::Bind {
                mutable: false,
                name: "pick".to_string(),
                type_: ParameterType::parse("fs.File"),
                value: Some(call("collections.get", vec![local("h"), local("i")])),
            }],
            vec![for_each(
                "inner",
                "h",
                vec![NirOp::Return {
                    value: Some(local("inner")),
                }],
            )],
        ];
        for body in leaving_bodies {
            uses_of(
                vec![for_each("h", "files", body)],
                &no_functions,
                &mut CalleeParamUses::default(),
                |uses| assert!(uses.block_held_elsewhere("files")),
            );
        }
        // A nested loop whose variable stays put holds nothing.
        uses_of(
            vec![for_each(
                "row",
                "rows",
                vec![for_each(
                    "h",
                    "row",
                    vec![eval(call("fs.readLine", vec![local("h")]))],
                )],
            )],
            &no_functions,
            &mut CalleeParamUses::default(),
            |uses| assert!(!uses.block_held_elsewhere("rows")),
        );
    }

    /// bug-1000: a borrowed-element read written straight into an argument of a call
    /// that does not keep it ends with the call.
    #[test]
    fn a_borrowed_read_used_as_an_argument_is_not_an_element_reader() {
        let no_functions = HashMap::new();
        let get = || call("collections.get", vec![local("files"), local("i")]);
        uses_of(
            vec![eval(call("fs.writeAll", vec![get(), local("text")]))],
            &no_functions,
            &mut CalleeParamUses::default(),
            |uses| {
                assert!(!uses.element_readers.contains("files"));
                assert!(!uses.block_held_elsewhere("files"));
            },
        );
        // Through a second borrowed read, its receiver is as transient as it is.
        uses_of(
            vec![eval(call(
                "fs.writeAll",
                vec![
                    call(
                        "collections.get",
                        vec![
                            call("collections.get", vec![local("rows"), local("i")]),
                            local("j"),
                        ],
                    ),
                    local("text"),
                ],
            ))],
            &no_functions,
            &mut CalleeParamUses::default(),
            |uses| assert!(!uses.element_readers.contains("rows")),
        );
    }

    /// The guard on that: stored as an element, returned, nested in a literal,
    /// passed through a function value, or bound through a second read, the pointer
    /// outlives the call and the list is an element reader again.
    #[test]
    fn a_borrowed_read_that_is_kept_is_an_element_reader() {
        let no_functions = HashMap::new();
        let get = || call("collections.get", vec![local("files"), local("i")]);
        let keeping = [
            NirOp::Assign {
                name: "out".to_string(),
                value: call("collections.append", vec![local("out"), get()]),
            },
            NirOp::Return { value: Some(get()) },
            eval(call(
                "use",
                vec![NirValue::ListLiteral {
                    type_: ParameterType::parse("List OF RES fs.File"),
                    values: vec![get()],
                }],
            )),
            // `put` is no function in the program and no builtin: a function value.
            eval(call("put", vec![get()])),
            NirOp::Bind {
                mutable: false,
                name: "pick".to_string(),
                type_: ParameterType::parse("fs.File"),
                value: Some(call(
                    "collections.get",
                    vec![
                        call("collections.get", vec![local("files"), local("i")]),
                        local("j"),
                    ],
                )),
            },
        ];
        for op in keeping {
            uses_of(
                vec![op],
                &no_functions,
                &mut CalleeParamUses::default(),
                |uses| assert!(uses.element_readers.contains("files")),
            );
        }
    }

    /// bug-1000: a callee whose body is read keeps a borrowed read exactly when it
    /// keeps the parameter the read lands in.
    #[test]
    fn a_borrowed_read_follows_the_callee_that_receives_it() {
        let reader = function(&["h"], vec![eval(call("fs.readLine", vec![local("h")]))]);
        let keeper = function(
            &["h"],
            vec![NirOp::Return {
                value: Some(local("h")),
            }],
        );
        let functions: HashMap<String, &NirFunction> = HashMap::from([
            ("reader".to_string(), &reader),
            ("keeper".to_string(), &keeper),
        ]);
        let get = |list: &str| call("collections.get", vec![local(list), local("i")]);
        uses_of(
            vec![
                eval(call("reader", vec![get("read")])),
                eval(call("keeper", vec![get("kept")])),
            ],
            &functions,
            &mut CalleeParamUses::default(),
            |uses| {
                assert!(!uses.element_readers.contains("read"));
                assert!(uses.element_readers.contains("kept"));
            },
        );
    }

    /// bug-1000: a call handed a callback reaches the callback's body. One that
    /// stores its parameter keeps every argument of the call; one that only reads
    /// through it keeps nothing; a callback held in a local cannot be read.
    #[test]
    fn a_callback_that_keeps_its_parameter_keeps_the_arguments() {
        let stash = function(
            &["h"],
            vec![NirOp::Assign {
                name: "keep".to_string(),
                value: call("collections.append", vec![local("keep"), local("h")]),
            }],
        );
        let read = function(&["h"], vec![eval(call("fs.readLine", vec![local("h")]))]);
        let functions: HashMap<String, &NirFunction> =
            HashMap::from([("stash".to_string(), &stash), ("read".to_string(), &read)]);
        let closure = |name: &str| NirValue::Closure {
            name: name.to_string(),
            type_: ParameterType::parse("FUNC(fs.File) AS Nothing"),
            captures: Vec::new(),
        };
        uses_of(
            vec![
                eval(call(
                    "collections.forEach",
                    vec![local("stashed"), closure("stash")],
                )),
                eval(call(
                    "collections.forEach",
                    vec![local("read"), closure("read")],
                )),
                NirOp::Bind {
                    mutable: false,
                    name: "cb".to_string(),
                    type_: ParameterType::parse("FUNC(fs.File) AS Nothing"),
                    value: Some(closure("read")),
                },
                eval(call(
                    "collections.forEach",
                    vec![local("unknown"), local("cb")],
                )),
            ],
            &functions,
            &mut CalleeParamUses::default(),
            |uses| {
                assert!(uses.block_held_elsewhere("stashed"));
                assert!(uses.kept_by_unread_code.contains("stashed"));
                assert!(!uses.block_held_elsewhere("read"));
                assert!(uses.block_held_elsewhere("unknown"));
            },
        );
    }

    /// The `MATCH` desugar over `scrutinee`: the temporary's bind, the `MATCH` on
    /// it, and one case that binds `f` from the temporary and runs `case_body`.
    fn match_on(scrutinee: NirValue, case_body: Vec<NirOp>) -> Vec<NirOp> {
        let mut body = vec![NirOp::Bind {
            mutable: false,
            name: "f".to_string(),
            type_: ParameterType::parse("fs.File"),
            value: Some(NirValue::UnionExtract {
                type_: ParameterType::parse("fs.File"),
                value: Box::new(local("$match0")),
            }),
        }];
        body.extend(case_body);
        vec![
            NirOp::Bind {
                mutable: false,
                name: "$match0".to_string(),
                type_: ParameterType::parse("Handle"),
                value: Some(scrutinee),
            },
            NirOp::Match {
                value: local("$match0"),
                cases: vec![crate::target::shared::nir::NirMatchCase {
                    pattern: NirMatchPattern::Value(local("fs.File")),
                    guard: None,
                    body,
                }],
            },
        ]
    }

    /// bug-1028: a function that reads a resource-union parameter with `MATCH`
    /// keeps nothing, so a list whose loop variable or borrowed element it is
    /// handed still owns its block. Reading it is all a `MATCH` does: the
    /// scrutinee temporary and the case binding stay inside it.
    #[test]
    fn a_match_that_only_reads_its_scrutinee_does_not_hold_the_list() {
        let which = function(
            &["h"],
            match_on(local("h"), vec![eval(call("fs.readAll", vec![local("f")]))]),
        );
        let functions: HashMap<String, &NirFunction> =
            HashMap::from([("which".to_string(), &which)]);
        uses_of(
            vec![
                for_each("h", "iterated", vec![eval(call("which", vec![local("h")]))]),
                eval(call(
                    "which",
                    vec![call("collections.get", vec![local("read"), local("i")])],
                )),
            ],
            &functions,
            &mut CalleeParamUses::default(),
            |uses| {
                assert!(!uses.block_held_elsewhere("iterated"));
                assert!(!uses.block_held_elsewhere("read"));
            },
        );
        // The `MATCH` written in the loop body, on the loop variable itself (the
        // optimizer forwards an immutable scrutinee, so no temporary is left) and
        // on a borrowed element.
        let on_variable = vec![NirOp::Match {
            value: local("h"),
            cases: vec![crate::target::shared::nir::NirMatchCase {
                pattern: NirMatchPattern::Value(local("fs.File")),
                guard: None,
                body: vec![
                    NirOp::Bind {
                        mutable: false,
                        name: "f".to_string(),
                        type_: ParameterType::parse("fs.File"),
                        value: Some(NirValue::UnionExtract {
                            type_: ParameterType::parse("fs.File"),
                            value: Box::new(local("h")),
                        }),
                    },
                    eval(call("fs.readAll", vec![local("f")])),
                ],
            }],
        }];
        let mut body = vec![for_each("h", "iterated", on_variable)];
        body.extend(match_on(
            call("collections.get", vec![local("read"), local("i")]),
            vec![eval(call("fs.readAll", vec![local("f")]))],
        ));
        uses_of(
            body,
            &HashMap::new(),
            &mut CalleeParamUses::default(),
            |uses| {
                assert!(!uses.block_held_elsewhere("iterated"));
                assert!(!uses.block_held_elsewhere("read"));
                assert!(
                    uses.escaping.contains("iterated") && uses.transient_readers.contains("read"),
                    "the strict verdicts still see both readings"
                );
            },
        );
    }

    /// The guard on that: a case binding that is returned, bound again, or stored
    /// into a collection is a second holder of the record, and so is the
    /// scrutinee temporary named anywhere but in its `MATCH`.
    #[test]
    fn a_match_whose_case_binding_leaves_holds_the_list() {
        let leaving: [Vec<NirOp>; 3] = [
            vec![NirOp::Return {
                value: Some(local("f")),
            }],
            vec![NirOp::Bind {
                mutable: false,
                name: "keep".to_string(),
                type_: ParameterType::parse("fs.File"),
                value: Some(local("f")),
            }],
            vec![NirOp::Assign {
                name: "out".to_string(),
                value: call("collections.append", vec![local("out"), local("f")]),
            }],
        ];
        for case_body in leaving {
            // In a callee the loop variable and a borrowed element are handed to.
            let pick = function(&["h"], match_on(local("h"), case_body.clone()));
            let functions: HashMap<String, &NirFunction> =
                HashMap::from([("pick".to_string(), &pick)]);
            uses_of(
                vec![
                    for_each("h", "iterated", vec![eval(call("pick", vec![local("h")]))]),
                    eval(call(
                        "pick",
                        vec![call("collections.get", vec![local("read"), local("i")])],
                    )),
                ],
                &functions,
                &mut CalleeParamUses::default(),
                |uses| {
                    assert!(uses.block_held_elsewhere("iterated"));
                    assert!(uses.block_held_elsewhere("read"));
                },
            );
            // And written where the element is read.
            let mut body = vec![for_each(
                "h",
                "iterated",
                match_on(local("h"), case_body.clone()),
            )];
            body.extend(match_on(
                call("collections.get", vec![local("read"), local("i")]),
                case_body,
            ));
            uses_of(
                body,
                &HashMap::new(),
                &mut CalleeParamUses::default(),
                |uses| {
                    assert!(uses.block_held_elsewhere("iterated"));
                    assert!(uses.block_held_elsewhere("read"));
                },
            );
        }
        // The temporary itself handed on.
        let mut body = match_on(local("h"), Vec::new());
        body.push(NirOp::Return {
            value: Some(local("$match0")),
        });
        uses_of(
            vec![for_each("h", "iterated", body)],
            &HashMap::new(),
            &mut CalleeParamUses::default(),
            |uses| assert!(uses.block_held_elsewhere("iterated")),
        );
    }

    fn field(root: &str, member: &str) -> NirValue {
        NirValue::MemberAccess {
            target: Box::new(local(root)),
            member: member.to_string(),
        }
    }

    fn bind(name: &str, type_: &str, value: NirValue) -> NirOp {
        NirOp::Bind {
            mutable: false,
            name: name.to_string(),
            type_: ParameterType::parse(type_),
            value: Some(value),
        }
    }

    /// [`uses_of`] with `Holder` known as a record that reaches a resource.
    fn record_uses_of<'f>(
        body: Vec<NirOp>,
        functions: &'f HashMap<String, &'f NirFunction>,
        check: impl FnOnce(&CollectionUses),
    ) {
        let holder = |type_: &ParameterType| *type_ == ParameterType::parse("Holder");
        let mut callees = CalleeParamUses {
            record_container: Some(&holder),
            ..CalleeParamUses::default()
        };
        uses_of(body, functions, &mut callees, check);
    }

    /// bug-1025: reading a record's field is a use of the record, not a second
    /// holder of the handle it carries, when the read is written into an argument
    /// of a call that does not keep it, is an operand of a comparison, or is bound
    /// to a plain scalar. A copy of the record that stays put holds nothing either.
    #[test]
    fn a_record_read_through_its_fields_and_copies_is_not_held() {
        let no_functions = HashMap::new();
        record_uses_of(
            vec![
                eval(call("fs.readAll", vec![field("k", "h")])),
                NirOp::If {
                    condition: NirValue::Binary {
                        op: crate::operators::BinaryOp::Equal,
                        left: Box::new(field("k", "n")),
                        right: Box::new(local("limit")),
                        loc: NirSourceLoc::default(),
                    },
                    then_body: Vec::new(),
                    else_body: Vec::new(),
                },
                bind("n", "Integer", field("k", "n")),
                bind("k2", "Holder", local("k")),
                bind(
                    "k3",
                    "Holder",
                    NirValue::WithUpdate {
                        type_: ParameterType::parse("Holder"),
                        target: Box::new(local("k2")),
                        updates: Vec::new(),
                    },
                ),
                eval(call("fs.readAll", vec![field("k3", "h")])),
            ],
            &no_functions,
            |uses| {
                assert!(!uses.block_held_elsewhere("k"));
                assert!(
                    uses.escaping.contains("k"),
                    "the strict verdicts still see every reading"
                );
            },
        );
    }

    /// The guard on that: the handle bound on its own, the field stored as an
    /// element, the field handed to a callee that keeps it, a copy that is
    /// returned, and a copy made by assignment (whose target can outlive the
    /// record) are each a second holder. So is any copy when the type is not known
    /// to be a record.
    #[test]
    fn a_record_whose_handle_or_copy_leaves_is_held() {
        let keeper = function(
            &["f"],
            vec![NirOp::Return {
                value: Some(local("f")),
            }],
        );
        let functions: HashMap<String, &NirFunction> =
            HashMap::from([("keeper".to_string(), &keeper)]);
        let leaving: [Vec<NirOp>; 5] = [
            vec![bind("f", "fs.File", field("k", "h"))],
            vec![NirOp::Assign {
                name: "out".to_string(),
                value: call("collections.append", vec![local("out"), field("k", "h")]),
            }],
            vec![eval(call("keeper", vec![field("k", "h")]))],
            vec![
                bind("k2", "Holder", local("k")),
                NirOp::Return {
                    value: Some(local("k2")),
                },
            ],
            vec![NirOp::Assign {
                name: "outer".to_string(),
                value: local("k"),
            }],
        ];
        for body in leaving {
            record_uses_of(body, &functions, |uses| {
                assert!(uses.block_held_elsewhere("k"));
            });
        }
        // The same copy that stays put, with no record type known.
        uses_of(
            vec![bind("k2", "Holder", local("k"))],
            &functions,
            &mut CalleeParamUses::default(),
            |uses| assert!(uses.block_held_elsewhere("k")),
        );
        // A collection bound to a second name shares its block; it is no copy.
        record_uses_of(
            vec![bind("same", "List OF RES fs.File", local("files"))],
            &functions,
            |uses| assert!(uses.block_held_elsewhere("files")),
        );
    }

    /// bug-1025: a function that returns a record it built hands back a block of
    /// its own, directly or through another such function.
    #[test]
    fn a_function_returning_a_record_it_built_returns_a_fresh_block() {
        let keep = function(
            &["h"],
            vec![NirOp::Return {
                value: Some(NirValue::Constructor {
                    type_: ParameterType::parse("Holder"),
                    args: vec![local("h")],
                }),
            }],
        );
        let make = function(
            &[],
            vec![NirOp::Return {
                value: Some(call("keep", vec![local("u")])),
            }],
        );
        let same = function(
            &["b"],
            vec![NirOp::Return {
                value: Some(local("b")),
            }],
        );
        let functions: HashMap<String, &NirFunction> = HashMap::from([
            ("keep".to_string(), &keep),
            ("make".to_string(), &make),
            ("same".to_string(), &same),
        ]);
        record_uses_of(
            vec![
                bind("k", "Holder", call("make", vec![])),
                bind("other", "Holder", call("same", vec![local("k")])),
            ],
            &functions,
            |uses| {
                assert!(uses.block_is_fresh("k"));
                assert!(!uses.block_is_fresh("other"));
            },
        );
    }

    /// bug-1053: a callee that keeps what it reads OUT of its parameter (an element
    /// appended to the list it returns) is a second reader of the elements and no
    /// holder of the parameter's block; one that returns the parameter is.
    #[test]
    fn a_callee_that_keeps_an_element_does_not_keep_the_block() {
        let pick = function(
            &["hs"],
            vec![
                for_each(
                    "h",
                    "hs",
                    vec![NirOp::Assign {
                        name: "out".to_string(),
                        value: call("collections.append", vec![local("out"), local("h")]),
                    }],
                ),
                NirOp::Return {
                    value: Some(local("out")),
                },
            ],
        );
        let same = function(
            &["xs"],
            vec![NirOp::Return {
                value: Some(local("xs")),
            }],
        );
        let functions: HashMap<String, &NirFunction> =
            HashMap::from([("pick".to_string(), &pick), ("same".to_string(), &same)]);
        let literal = || NirValue::ListLiteral {
            type_: ParameterType::parse("List OF RES fs.File"),
            values: Vec::new(),
        };
        let caller = function(
            &[],
            vec![
                bind("read", "List OF RES fs.File", literal()),
                bind("returned", "List OF RES fs.File", literal()),
                bind("aliased", "List OF RES fs.File", literal()),
                bind("alias", "List OF RES fs.File", local("aliased")),
                eval(call("pick", vec![local("read")])),
                eval(call("same", vec![local("returned")])),
            ],
        );
        let stores = collect(&caller, &functions);
        let mut callees = CalleeParamUses::default();
        let uses = CollectionUses::of(&caller, &functions, &mut callees);
        // The whole proof still refuses `read`: its elements have a second reader.
        assert!(uses.block_held_elsewhere("read"));
        let blocks = block_owning_collections(&stores, &uses, &HashSet::new());
        assert!(blocks.contains("read"));
        assert!(!blocks.contains("returned"));
        assert!(!blocks.contains("aliased"));
        assert!(!blocks.contains("alias"));
    }

    /// bug-1053: a list of lists is not judged by the block rule: what a `FOR EACH`
    /// over it yields is itself a block.
    #[test]
    fn a_nested_collection_is_not_judged_by_the_block_rule() {
        let no_functions = HashMap::new();
        let caller = function(
            &[],
            vec![bind(
                "nested",
                "List OF List OF RES fs.File",
                NirValue::ListLiteral {
                    type_: ParameterType::parse("List OF List OF RES fs.File"),
                    values: Vec::new(),
                },
            )],
        );
        let stores = collect(&caller, &no_functions);
        let mut callees = CalleeParamUses::default();
        let uses = CollectionUses::of(&caller, &no_functions, &mut callees);
        assert!(block_owning_collections(&stores, &uses, &HashSet::new()).is_empty());
    }

    fn capture(by_ref: bool) -> NirValue {
        NirValue::Capture {
            index: 0,
            type_: ParameterType::parse("List OF RES fs.File"),
            by_ref,
        }
    }

    /// bug-1053: the one store that carries a reference to a longer-lived binding
    /// from code the storing function never sees as a store is an assignment
    /// through a by-reference capture of a binding that reaches a resource.
    #[test]
    fn a_lambda_that_assigns_a_captured_container_is_found() {
        let reaches = |type_: &ParameterType| *type_ == ParameterType::parse("List OF RES fs.File");
        let append = || NirOp::Assign {
            name: "keep".to_string(),
            value: call("collections.append", vec![local("keep"), local("h")]),
        };
        let writes = function(
            &["h"],
            vec![bind("keep", "List OF RES fs.File", capture(true)), append()],
        );
        // A capture by value is the lambda's own copy of the pointer.
        let copies = function(
            &["h"],
            vec![
                bind("keep", "List OF RES fs.File", capture(false)),
                append(),
            ],
        );
        // A by-reference capture that is only read stores nothing.
        let reads = function(
            &["h"],
            vec![
                bind("keep", "List OF RES fs.File", capture(true)),
                eval(call("len", vec![local("keep")])),
            ],
        );
        // And a captured binding that reaches no resource is no concern here.
        let counts = function(
            &["h"],
            vec![
                NirOp::Bind {
                    mutable: true,
                    name: "keep".to_string(),
                    type_: ParameterType::parse("List OF Integer"),
                    value: Some(capture(true)),
                },
                append(),
            ],
        );
        let harmless: HashMap<String, &NirFunction> = HashMap::from([
            ("copies".to_string(), &copies),
            ("reads".to_string(), &reads),
            ("counts".to_string(), &counts),
        ]);
        assert!(!module_stores_through_a_captured_reference(
            &harmless, &reaches
        ));
        let mut with_writer = harmless.clone();
        with_writer.insert("writes".to_string(), &writes);
        assert!(module_stores_through_a_captured_reference(
            &with_writer,
            &reaches
        ));
    }

    /// bug-1053: an assignment that carries an inner scope's binding outward in a
    /// position the store-time raise does not cover is recorded; the stores it
    /// does cover, a same-scope store and an adopted call result are not.
    #[test]
    fn an_assignment_the_raise_does_not_cover_is_recorded() {
        let list = "List OF RES fs.File";
        let container = |type_: &ParameterType| *type_ == ParameterType::parse(list);
        let record = |_: &ParameterType| false;
        let literal = || NirValue::ListLiteral {
            type_: ParameterType::parse(list),
            values: Vec::new(),
        };
        let nested = |store: NirValue| {
            function(
                &[],
                vec![
                    bind("keep", list, literal()),
                    NirOp::If {
                        condition: NirValue::Const {
                            type_: ParameterType::Boolean,
                            value: "true".to_string(),
                        },
                        then_body: vec![
                            bind("xs", list, literal()),
                            NirOp::Assign {
                                name: "keep".to_string(),
                                value: store,
                            },
                        ],
                        else_body: Vec::new(),
                    },
                ],
            )
        };
        let unraised = |function: &NirFunction| {
            collect_store_floats(function, &container, &record).unraised_outward_store
        };
        // A member that re-packages its argument's handles: nothing raises them.
        assert!(unraised(&nested(call(
            "collections.filter",
            vec![local("xs")]
        ))));
        // The inserting members are the store-time raise's own.
        assert!(!unraised(&nested(call(
            "collections.append",
            vec![local("keep"), local("xs")]
        ))));
        // A call to a function with a body: the assignment adopts its result.
        assert!(!unraised(&nested(call("build", vec![local("xs")]))));
        // The same scope: ownership is unchanged.
        let same_scope = function(
            &[],
            vec![
                bind("keep", list, literal()),
                bind("xs", list, literal()),
                NirOp::Assign {
                    name: "keep".to_string(),
                    value: call("collections.filter", vec![local("xs")]),
                },
            ],
        );
        assert!(!unraised(&same_scope));
    }
}
