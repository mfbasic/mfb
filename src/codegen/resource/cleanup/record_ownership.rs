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
    /// bug-645: owner-collection locals whose owned-list drain provably owns every
    /// block it can reach — see [`owning_collections`].
    pub(crate) owning_collections: HashSet<String>,
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
    let owning_collections = owning_collections(
        function,
        functions,
        record_type,
        &CodeBuilder::is_res_marked_resource_collection,
        &stores,
        &params,
    );
    RecordOwnership {
        owning_locals,
        alias_sources,
        owning_collections,
    }
}

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
///   record field, a union wrap, a closure capture or a `FOR EACH` iterable does.
/// * **No element is read back out.** No `collections::get`/`getOr` — nor a borrowed
///   `tcp`/`udp`/`tls::poll` — names `c`. Those "yield a POINTER to the one resource"
///   (§15.6), so a freed record could still be live in the caller's hands.
/// * **Every floated element is a fresh record that does not escape.** Fresh by this
///   module's existing rules (a producer call, a closed `TRAP` default, a wrap of one
///   of those); a wrap of a LOCAL is an alias whose record another binding still frees,
///   and is refused. And the element local itself never appears outside an argument
///   position, so no `RETURN c` / `LET keep = c` hands the record on.
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
) -> HashSet<String> {
    let mut floats: HashMap<String, Vec<String>> = HashMap::new();
    for (name, owner) in &function.resource_owners {
        if let crate::ir::resource_escape::ResOwner::Float(collection) = owner {
            floats
                .entry(collection.clone())
                .or_default()
                .push(name.clone());
        }
    }
    let mut callees = CalleeParamUses::default();
    let uses = CollectionUses::of(function, functions, &mut callees);
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
    if floats.is_empty() {
        return owning;
    }
    for (collection, elements) in &floats {
        if uses.block_held_elsewhere(collection) {
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
    owning
}

/// Where each local is NAMED, for [`owning_collections`]: the positions that can create
/// a second holder of a block, and the calls that hand a resource pointer back out of a
/// collection.
struct CollectionUses<'m, 'f> {
    /// Every function in the program, to ask what a callee does with an argument.
    functions: &'f HashMap<String, &'f NirFunction>,
    /// What each callee's parameters are known to do, shared across the walk.
    callees: &'m mut CalleeParamUses,
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
    /// bug-753: every `RETURN`'s value, for asking whether the function hands back
    /// only blocks it built.
    returns: Vec<NirValue>,
    /// Set while the walk is inside a `RETURN`'s value.
    in_return: bool,
}

/// Per callee, per parameter position: whether the callee keeps that argument's block
/// past the call (bug-801). Memoized; a function still being resolved answers
/// fail-safe.
#[derive(Default)]
struct CalleeParamUses {
    known: HashMap<String, Vec<bool>>,
    visiting: HashSet<String>,
    /// bug-753: per function, whether every `RETURN` hands back a block it built.
    fresh_returns: HashMap<String, bool>,
    fresh_visiting: HashSet<String>,
}

impl<'m, 'f> CollectionUses<'m, 'f> {
    fn of(
        function: &NirFunction,
        functions: &'f HashMap<String, &'f NirFunction>,
        callees: &'m mut CalleeParamUses,
    ) -> Self {
        let mut collector = CollectionUses {
            functions,
            callees,
            escaping: HashSet::new(),
            kept: HashSet::new(),
            element_readers: HashSet::new(),
            fresh_block: HashMap::new(),
            escaping_outside_return: HashSet::new(),
            returns: Vec::new(),
            in_return: false,
        };
        collector.visit_ops(&function.body);
        collector
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
            .map(|p| inner.escaping.contains(&p.name) || inner.kept.contains(&p.name))
            .collect();
        self.callees.visiting.remove(target);
        self.callees.known.insert(target.to_string(), keeps.clone());
        Some(keeps)
    }

    /// Whether a holder other than `name` itself can still reach its block or read a
    /// resource pointer out of it after `name`'s scope exits.
    fn block_held_elsewhere(&self, name: &str) -> bool {
        self.escaping.contains(name)
            || self.kept.contains(name)
            || self.element_readers.contains(name)
    }

    fn block_is_fresh(&self, name: &str) -> bool {
        self.fresh_block.get(name).copied().unwrap_or(false)
    }

    fn note_store(&mut self, name: &str, value: Option<&NirValue>) {
        let fresh = collection_store_is_fresh(name, value)
            || value.is_some_and(|value| self.user_call_returns_fresh_block(value));
        let entry = self.fresh_block.entry(name.to_string()).or_insert(true);
        *entry &= fresh;
    }
}

impl CollectionUses<'_, '_> {
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
                | NirValue::MapLiteral { .. } => true,
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

impl NirVisitor for CollectionUses<'_, '_> {
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
            NirOp::Bind { name, value, .. } => self.note_store(name, value.as_ref()),
            NirOp::Assign { name, value } => self.note_store(name, Some(value)),
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
        match value {
            // Any other position that names a local can leave a second holder behind,
            // so the default is "escapes" and only the argument arm below narrows it —
            // a NIR variant added tomorrow lands here and answers fail-safe.
            NirValue::Local(name) => {
                self.escaping.insert(name.clone());
                if !self.in_return {
                    self.escaping_outside_return.insert(name.clone());
                }
            }
            NirValue::Call { target, args, .. }
            | NirValue::CallResult { target, args, .. }
            | NirValue::RuntimeCall { target, args, .. } => {
                if CodeBuilder::value_aliases_live_resource(value) {
                    for arg in args {
                        if let NirValue::Local(name) = arg {
                            self.element_readers.insert(name.clone());
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
                for (index, arg) in args.iter().enumerate() {
                    let NirValue::Local(name) = arg else {
                        // Nested values inside an argument still walk normally.
                        self.visit_value(arg);
                        continue;
                    };
                    if stores_elements && index > 0 {
                        self.kept.insert(name.clone());
                    }
                    if callee
                        .as_ref()
                        .is_some_and(|keeps| keeps.get(index).copied().unwrap_or(true))
                    {
                        self.kept.insert(name.clone());
                    }
                }
                return;
            }
            _ => {}
        }
        walk_value(self, value);
    }
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

    /// The guard, and the reason it is a *self*-receiving test: `a = append(b, x)`
    /// leaves `a` holding a block grown from `b`'s, which `b` still names. Freeing
    /// both at scope exit is the double free this module exists to refuse.
    #[test]
    fn a_mutator_of_another_collection_is_not_a_fresh_block() {
        let append = call("collections.append", vec![local("other"), local("c")]);
        assert!(!collection_store_is_fresh("chans", Some(&append)));
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
}
