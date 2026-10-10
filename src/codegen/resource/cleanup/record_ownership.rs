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
    /// bug-753: `RES`-block collections whose copying reassignment may free the
    /// block it replaces — see [`block_collections`].
    pub(crate) reassign_free_collections: HashSet<String>,
    /// bug-753: owning nested collections whose inner blocks are theirs alone, so
    /// the scope-exit free releases each inner block, then the outer one.
    pub(crate) inner_owning_collections: HashSet<String>,
    /// bug-999: the call nodes (by address) whose result nothing but their own
    /// statement can hold, mapped to whether the result's block is one the callee
    /// built — see [`CollectionUses::exclusive_temps`].
    pub(crate) exclusive_temps: HashMap<usize, bool>,
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
    union_collection_type: &dyn Fn(&ParameterType) -> bool,
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
    let (owning_collections, exclusive_temps) = owning_collections(
        function,
        functions,
        record_type,
        res_collection_type,
        union_collection_type,
        &stores,
        &params,
    );
    let (reassign_free_collections, inner_owning_collections) =
        block_collections(function, functions, &stores, &owning_collections);
    RecordOwnership {
        owning_locals,
        alias_sources,
        owning_collections,
        reassign_free_collections,
        inner_owning_collections,
        exclusive_temps,
    }
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
) -> (HashSet<String>, HashSet<String>) {
    let mut callees = CalleeParamUses::default();
    let uses = CollectionUses::of(function, functions, &mut callees);
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
    union_collection_type: &dyn Fn(&ParameterType) -> bool,
    stores: &Stores,
    params: &HashSet<String>,
) -> (HashSet<String>, HashMap<usize, bool>) {
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
    let mut uses = CollectionUses::of(function, functions, &mut callees);
    let exclusive_temps = std::mem::take(&mut uses.exclusive_temps);
    // bug-1000's wider rule, except for a collection of resource-union boxes
    // (`block_held_elsewhere_strict`).
    let held_elsewhere = |name: &str, type_: &ParameterType| {
        if union_collection_type(type_) {
            uses.block_held_elsewhere_strict(name)
        } else {
            uses.block_held_elsewhere(name)
        }
    };
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
        if held_elsewhere(name, type_) {
            continue;
        }
        if !uses.block_is_fresh(name) {
            continue;
        }
        owning.insert(name.clone());
    }
    if floats.is_empty() {
        return (owning, exclusive_temps);
    }
    for (collection, elements) in &floats {
        // A float target with no bind in this body (a parameter) takes the strict
        // rule: its type is not known here.
        let held = match stores.types.get(collection) {
            Some(type_) => held_elsewhere(collection, type_),
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
    (owning, exclusive_temps)
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
    iterations: Vec<(String, String)>,
    /// Set while the walk is on a `FOR EACH`'s iterable.
    in_iterable: bool,
    /// bug-1000: set, for the walk of one call argument, when that argument is a
    /// borrowed-element read whose result the receiving call does not keep.
    transient_borrow: bool,
    /// bug-1000: collections a borrowed-element read names only transiently (see
    /// `transient_borrow`). Kept apart from `element_readers` for the collections
    /// that still take the strict rule ([`Self::block_held_elsewhere_strict`]).
    transient_readers: HashSet<String>,
    /// bug-999: call nodes (by address) whose result is the statement's alone, so
    /// the statement may FREE what it closes, as a binding's scope would
    /// (`LET t = mkList()` then `len(t)`). Three facts make it so:
    ///
    /// * the call is written straight into an argument of a call that does not
    ///   keep the argument (a natively lowered builtin, or a body read and found
    ///   not to) and takes no callback;
    /// * that receiving call can only answer with a plain scalar, so nothing of the
    ///   argument comes back out of it;
    /// * the call itself runs a body whose parameters are all plain scalars
    ///   ([`Self::call_takes_only_plain_scalars`]), so what its result reaches
    ///   unowned came from its own frame.
    ///
    /// The value says whether the result's own block is one the callee built
    /// ([`Self::user_call_returns_fresh_block`]). Any other aggregate temporary keeps
    /// the close-only rule: its records stay as closed tombstones, because a
    /// reference stored without a float (`keep = collections::append(keep,
    /// collections::get(mkList(), 0))`) must go on reading a closed handle.
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
    /// bug-1000: per call target, whether it is a builtin.
    builtins: HashMap<String, bool>,
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
            // bug-1000: a parameter the callee only iterates is not kept, by the
            // same rule as a local (`block_held_elsewhere`).
            .map(|p| {
                inner.escaping_beyond_iteration.contains(&p.name)
                    || inner.kept.contains(&p.name)
                    || inner.iterated_reference_leaves(&p.name, 64)
            })
            .collect();
        self.callees.visiting.remove(target);
        self.callees.known.insert(target.to_string(), keeps.clone());
        Some(keeps)
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
    /// produced and released: no argument's holder, and no closed tombstone some
    /// longer-lived list still names, can be among them.
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
    /// through a borrowed-element call at all. For a collection whose elements are
    /// resource-union boxes, whose drain cannot yet be widened: a box a callee's
    /// binding wrapped is freed by that binding as the callee returns (bug-1012),
    /// so an adopted list's drain that frees boxes frees them twice. The rule
    /// before bug-1000 already had that defect for a list that is only measured;
    /// this keeps it from reaching the lists that are read or iterated as well.
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
            || value.is_some_and(|value| self.user_call_returns_fresh_block(value));
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
            NirOp::Bind {
                name, type_, value, ..
            } => {
                if matches!(type_, ParameterType::Func(..)) {
                    self.function_values.insert(name.clone());
                }
                self.note_store(name, value.as_ref());
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
                        if let NirValue::Local(name) = arg {
                            readers.insert(name.clone());
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
                    let NirValue::Local(name) = arg else {
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
                    if unread {
                        self.kept_by_unread_code.insert(name.clone());
                    }
                }
                return;
            }
            _ => {}
        }
        walk_value(self, value);
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
}
