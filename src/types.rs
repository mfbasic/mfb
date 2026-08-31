//! The compiler's structural type vocabulary.
//!
//! [`ParameterType`] is the neutral, dependency-light representation of a type
//! shared across the front/middle of the compiler and the builtin registry. It
//! lives here — not under `codegen` — because the type checker, resolver, IR, and
//! registry all speak it, and `codegen` sits *below* those consumers: a type they
//! depend on should not live in the back end. `codegen::registry` re-exports it
//! (`pub(crate) use crate::types::ParameterType`) so existing `registry::…` import
//! paths keep resolving.
//!
//! The only outward dependency is `builtins::split_func_params_and_return` (used by
//! [`ParameterType::parse`] to split a `FUNC(...) AS R` type string); a module cycle
//! with `builtins` is fine — modules are not separate compilation units.

use crate::intern::Symbol;
use std::borrow::Cow;
use std::fmt;

/// A [`crate::codegen::registry::Parameter`]'s type. An enum rather than a bare
/// `&'static str` so future kinds (argument unions, generic placeholders) can be
/// added without touching every parameter.
/// `Hash` is derived alongside `Eq` so a type can be a map *key* — plan-111-C
/// re-keys `TypeModel`'s nine tables (and the registry's) from a rendered
/// `String` to the type itself, which is the whole point of the migration: two
/// spellings of one type miss each other in a `HashMap<String, _>`, where
/// `ParameterType` equality cannot. Every payload was already `Hash` —
/// `Symbol` is `#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]`
/// (`src/intern.rs:26`), and the rest are `Box`/`Vec`/`usize`/`bool` — so the
/// derive is the entire change. `hash` agrees with `eq` by construction and
/// `parameter_type_hash_agrees_with_eq` pins it over the round-trip corpus.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ParameterType {
    AttributeString,
    Boolean,
    Byte,
    Integer,
    Fixed,
    Float,
    Money,
    Nothing,
    String,
    ListOf(Box<ParameterType>),
    MapOf(Box<ParameterType>, Box<ParameterType>),
    SetOf(Box<ParameterType>),
    /// A `MapEntry OF K TO V` — a key/value pair type (the element type of a map
    /// iteration). Structurally the same key-`TO`-value shape as [`MapOf`](Self::MapOf),
    /// but a distinct nominal so `MapEntry OF …` round-trips and unifies as itself.
    /// `monomorph::helpers::unify_type` already handled this shape on strings; the
    /// variant lets a `ParameterType` represent it with no gap.
    MapEntryOf(Box<ParameterType>, Box<ParameterType>),
    /// A `Result OF T` — a success-typed result. A single-child structural type,
    /// mirroring `monomorph::helpers::unify_type`'s `Result OF` arm, so the middle-end
    /// shape is representable without folding into an opaque [`Named`](Self::Named).
    ResultOf(Box<ParameterType>),
    /// A `RES`-marked collection element (`List OF RES fs.File`) — the mandatory
    /// ownership-axis marker for a resource stored as a collection element (§15.6).
    /// Matching is RES-*transparent*: [`unify`](crate::codegen::registry) and
    /// `leaf_matches` unwrap it on the concrete side before matching, exactly
    /// reproducing the historical strip-on-[`parse`](Self::parse). The marker exists
    /// only so a type NAME round-trips byte-exact (`parse("List OF RES File").name()
    /// == "List OF RES File"`) once the type checker carries `ParameterType` instead
    /// of the raw string — a `Var` binding still drops it (bound to the unwrapped
    /// inner), and only [`Arg`](Self::Arg) echoes it verbatim.
    Res(Box<ParameterType>),
    /// A resource carrying its own top-level ` STATE T` clause —
    /// `File STATE Cursor`, the state-machine plane of §15 / plan-52 / plan-54.
    ///
    /// plan-111-A: before this variant `parse` had no arm for `STATE`, so
    /// `File STATE Cursor` was one opaque [`Named`](Self::Named) and the clause
    /// was readable only by re-splitting the spelling. That gap is why the split
    /// existed **twice**, hand-rolled — `types::split_state_clause` and
    /// `codegen::resource::split_state_clause`, byte-identical and pinned
    /// against each other by a parity test. A second grammar edited out of
    /// lockstep is exactly the failure plan-106-E's Correction 3 recorded (the
    /// `split_once(" TO ")` mis-split), and `planning/Compiler Pipeline.md:25`
    /// flagged as an architectural hazard.
    ///
    /// **Top-level only, and that is load-bearing.** [`parse`](Self::parse)
    /// builds this ONLY when the text before ` STATE ` is a single bare token
    /// (`File`, `fs.File`). A base containing a space means the clause belongs
    /// to something *nested* — a thread plane's `RES` element
    /// (`Thread OF … RES File STATE Cursor TO …`, plan-54) or a collection's
    /// element (`List OF RES File STATE Cursor`) — and the enclosing `parse` arm
    /// has already claimed it, so the clause lands on the **inner** type where
    /// it belongs. Splitting it at the composite instead is the asymmetry
    /// bug-429 fixed. That rule is unchanged: it is still `split_state_clause`,
    /// which `parse` now calls instead of every consumer re-deriving it.
    ///
    /// Because the variant only ever appears where the clause is genuinely
    /// top-level, [`split_state`](Self::split_state) is a plain structural match
    /// on it — no spelling, no guard — and `ListOf(Res(Stateful { .. }))`
    /// correctly reports *no* top-level clause of its own.
    Stateful {
        base: Box<ParameterType>,
        state: Box<ParameterType>,
    },
    /// A concrete nominal type — a record, union, or user type named by the program.
    /// Matched by name (unlike [`Var`], which is bound). A descriptor names one with a
    /// static literal (`named("CsvReader")`); a concrete nominal argument is built at
    /// the boundary by [`parse`](Self::parse). The name is an interned [`Symbol`]
    /// (`Copy`, integer-compared) rather than a leaked `&'static str`.
    Named(Symbol),
    /// A **user** generic instantiation — `Pair OF Integer, String`, `Stack OF T`
    /// — decomposed into its template name and its type arguments.
    ///
    /// The head is any name that is not one of the built-in type constructors
    /// (`List`/`Set`/`Map`/`MapEntry`/`Result`/`Thread`/`ThreadWorker`/`FUNC`); the
    /// [`parse`](Self::parse) arm is ordered after all of those, which is what
    /// enforces it. `Thread OF …` deliberately keeps its own
    /// [`ThreadHandle`](Self::ThreadHandle) variant — it carries the RES/STATE
    /// planes — and is never a `UserOf`.
    ///
    /// Before plan-105-B this shape folded into an opaque [`Named`](Self::Named)
    /// whose *name string* monomorph then re-split with a private grammar copy
    /// (`user_template_parts` + `split_top_level_commas`). The variant is what lets
    /// `unify`/`substitute`/`contains_var` recurse into the arguments structurally,
    /// the same way they already do for `ListOf`/`MapOf`/`Func`.
    ///
    /// The argument list is separated by TOP-LEVEL commas, and a type argument may
    /// itself be comma-bearing (`Holder OF Pair OF A, B`), so both the split and the
    /// render are depth-aware. `name()` reproduces the exact source spelling, which
    /// the monomorph mangler and every name-keyed type lookup depend on.
    UserOf(Symbol, Vec<ParameterType>),
    /// A bindable type variable in a generic signature (`T`, `K`, `V`). The name is an
    /// interned [`Symbol`] — variables are declared in a descriptor via [`var`](Self::var)
    /// and, post-elaboration, minted by the front end.
    /// It is *unified* against the concrete argument type at a call site (binding the
    /// variable) and then *substituted* into the return type: this is how the registry
    /// expresses `collections::get(List OF T, Integer) AS T` with no per-package
    /// resolver. Renders as its bare name in documentation.
    Var(Symbol),
    /// A function-value type — `FUNC(<params>) AS <return>`. The parameter and return
    /// types may themselves contain [`Var`](Self::Var)s, so a higher-order member like
    /// `collections::transform(List OF T, FUNC(T) AS U) AS List OF U` unifies the
    /// callback's shape structurally (binding `T`/`U`) instead of matching an opaque
    /// `Named("FUNC(Integer) AS String")` blob. Built by [`parse`](Self::parse) from a
    /// concrete `FUNC(...)` argument, and written in a descriptor with [`func`](Self::func).
    ///
    /// The trailing `bool` is `isolated`: an `ISOLATED FUNC(...)` (a capture-free
    /// worker entry — `thread::start`'s callback) renders and round-trips with the
    /// `ISOLATED ` prefix, and only unifies against a matching-isolation concrete, so a
    /// plain `FUNC` is not accepted where `thread::start` demands an isolated worker.
    Func(Vec<ParameterType>, Box<ParameterType>, bool),
    /// A return-type marker meaning "the concrete type of argument `n`, echoed
    /// **verbatim**". Only valid in a return position. Unlike a substituted [`Var`],
    /// which is reconstructed from parsed pieces (and so drops a `RES ` ownership
    /// marker), this hands back the caller's original argument-type string unchanged —
    /// so `collections::append(List OF RES File STATE Cursor, x)` returns exactly
    /// `List OF RES File STATE Cursor`. Resolved by `resolve_call`, which alone holds
    /// the raw argument strings.
    Arg(usize),
    /// An unresolved concrete argument type. Only appears on the concrete side (from
    /// [`parse`](Self::parse)); it unifies with any pattern as a wildcard.
    Unknown,
    /// A thread handle type — the structured decomposition of `Thread OF Msg TO Out`
    /// / `ThreadWorker OF Msg TO Out`, with an optional resource plane
    /// (`Thread OF Msg RES Res TO Out`). `worker` distinguishes a parent `Thread`
    /// handle from a `ThreadWorker` handle (the two never unify with each other).
    /// `msg`/`out` are the data-plane message and output types; `res` is the
    /// resource-plane type, defaulting to [`Nothing`](Self::Nothing) when the handle
    /// carries no `RES Res` clause (elided by [`name`](Self::name)). The three slots
    /// may themselves be generic ([`Var`](Self::Var)), so `start` can return a fresh
    /// parent handle and `waitFor`/`receive`/`accept` can return `out`/`msg`/`res`
    /// with no per-package resolver — the variant rides the same
    /// [`unify`](crate::codegen::registry)/`substitute` recursion as `ListOf`/`Func`.
    ThreadHandle {
        worker: bool,
        msg: Box<ParameterType>,
        res: Box<ParameterType>,
        out: Box<ParameterType>,
    },
}

impl ParameterType {
    pub(crate) fn list_of(elem: ParameterType) -> Self {
        ParameterType::ListOf(Box::new(elem))
    }
    pub(crate) fn map_of(key: ParameterType, val: ParameterType) -> Self {
        ParameterType::MapOf(Box::new(key), Box::new(val))
    }
    pub(crate) fn set_of(elem: ParameterType) -> Self {
        ParameterType::SetOf(Box::new(elem))
    }
    /// A `MapEntry OF key TO val` pair type.
    pub(crate) fn map_entry_of(key: ParameterType, val: ParameterType) -> Self {
        ParameterType::MapEntryOf(Box::new(key), Box::new(val))
    }
    /// A `Result OF success` type.
    pub(crate) fn result_of(success: ParameterType) -> Self {
        ParameterType::ResultOf(Box::new(success))
    }
    /// A `RES`-marked element (`RES fs.File`) wrapping `inner`.
    pub(crate) fn res(inner: ParameterType) -> Self {
        ParameterType::Res(Box::new(inner))
    }
    /// The type a SPELLING denotes — a declaration's own name, or a value's
    /// recorded type where codegen still stores one as text.
    ///
    /// **Not [`named`](Self::named), and the difference is load-bearing.** A
    /// declared type may shadow a built-in spelling — `TYPE Integer` is legal
    /// and compiles — and when it does it must denote the same type an
    /// `AS Integer` annotation denotes, because that is what the name-keyed
    /// tables did before plan-111 keyed them by type. `named("Integer")` would
    /// mint a distinct `Named` nominal and SPLIT a key the string world merged.
    ///
    /// Measured, against a pre-plan-111 binary: keying `ir::verify`'s tables
    /// with `named` instead of this made `TYPE Integer` with a field
    /// `a AS Integer` stop reporting `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`,
    /// because the record's key (`Named("Integer")`) no longer equalled its
    /// field's type (`Integer`). For every name that is not a built-in spelling
    /// the two are identical, which is why the divergence hides so well.
    ///
    /// This is `parse`, and it lives here so the conversion stays inside the
    /// grammar's own file (plan-111-A §2, boundary 1) rather than becoming a
    /// `ParameterType::parse` call in every table's constructor.
    pub(crate) fn declared(name: &str) -> Self {
        ParameterType::parse(name)
    }

    /// A resource carrying its own top-level ` STATE T` clause.
    ///
    /// **Build a stateful type with this, never with
    /// `named("Stream STATE PendingState")`.** An opaque nominal that merely
    /// *spells* the clause is not a [`Stateful`](Self::Stateful), so since
    /// plan-111-A made [`split_state`](Self::split_state) a structural match it
    /// would report no state at all — and it compares unequal to the same
    /// spelling arriving through [`parse`](Self::parse), which is how a source
    /// annotation gets here. That mismatch is a real bug this constructor
    /// exists to prevent: `http.startRead`'s descriptor returned
    /// `named("Stream STATE PendingState")`, so
    /// `RES s AS http::Stream STATE PendingState = http::startRead(u)` failed
    /// with `TYPE_BINDING_MISMATCH` — the declared side had a state to strip
    /// and the initializer side no longer did.
    ///
    /// `codegen::registry`'s `descriptor_named_types_are_bare_nominals` fails on
    /// any descriptor that spells a structure it does not build, which is what
    /// caught that one.
    pub(crate) fn stateful(base: ParameterType, state: ParameterType) -> Self {
        ParameterType::Stateful {
            base: Box::new(base),
            state: Box::new(state),
        }
    }
    /// A user generic instantiation `name OF args[0], args[1], …`, interning the
    /// template name to a [`Symbol`].
    pub(crate) fn user_of(name: &str, args: Vec<ParameterType>) -> Self {
        ParameterType::UserOf(Symbol::intern(name), args)
    }
    /// A concrete nominal type named `name`, interning the name to a [`Symbol`]. This
    /// is the sole constructor for [`Named`](Self::Named): descriptors and the
    /// `parse` fallback both route through it, so the leaked `&'static str` is gone.
    pub(crate) fn named(name: &str) -> Self {
        ParameterType::Named(Symbol::intern(name))
    }

    /// Reclassify every [`Named`](Self::Named) leaf whose name is one of
    /// `type_params` as a [`Var`](Self::Var) type variable, recursing through the
    /// container/function/thread structure. On an empty `type_params` this is an
    /// identity rebuild. `elaborate` uses this to mark generic type variables from
    /// the enclosing decl's `template_params` — the classification `parse` cannot do
    /// alone because it has no scope (plan-102-D). Because `monomorph` clears
    /// `template_params` on every instantiated decl, this is a no-op on concrete
    /// (post-monomorph) input, so it stays byte-identical there.
    pub(crate) fn with_vars(&self, type_params: &[String]) -> ParameterType {
        match self {
            ParameterType::Named(sym) => {
                if type_params.iter().any(|param| param == sym.resolve()) {
                    ParameterType::Var(*sym)
                } else {
                    ParameterType::Named(*sym)
                }
            }
            ParameterType::ListOf(elem) => ParameterType::list_of(elem.with_vars(type_params)),
            ParameterType::SetOf(elem) => ParameterType::set_of(elem.with_vars(type_params)),
            ParameterType::MapOf(key, value) => {
                ParameterType::map_of(key.with_vars(type_params), value.with_vars(type_params))
            }
            ParameterType::MapEntryOf(key, value) => ParameterType::map_entry_of(
                key.with_vars(type_params),
                value.with_vars(type_params),
            ),
            ParameterType::ResultOf(success) => {
                ParameterType::result_of(success.with_vars(type_params))
            }
            ParameterType::Res(inner) => ParameterType::res(inner.with_vars(type_params)),
            // The template HEAD is a nominal, never a type variable (`T OF Integer`
            // is not expressible); only the arguments can name a declared param.
            ParameterType::UserOf(name, args) => ParameterType::UserOf(
                *name,
                args.iter().map(|a| a.with_vars(type_params)).collect(),
            ),
            ParameterType::Func(params, ret, isolated) => {
                let params = params.iter().map(|p| p.with_vars(type_params)).collect();
                let ret = ret.with_vars(type_params);
                if *isolated {
                    ParameterType::func_isolated(params, ret)
                } else {
                    ParameterType::func(params, ret)
                }
            }
            ParameterType::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => ParameterType::thread_handle(
                *worker,
                msg.with_vars(type_params),
                res.with_vars(type_params),
                out.with_vars(type_params),
            ),
            // Scalars, an existing `Var`, `Unknown`, and `Arg` carry no nominal leaf
            // to reclassify.
            other => other.clone(),
        }
    }
    /// A bindable type variable named `name`, interning the name to a [`Symbol`].
    pub(crate) fn var(name: &str) -> Self {
        ParameterType::Var(Symbol::intern(name))
    }
    pub(crate) fn func(params: Vec<ParameterType>, ret: ParameterType) -> Self {
        ParameterType::Func(params, Box::new(ret), false)
    }
    /// An `ISOLATED FUNC(...)` value type — a capture-free worker entry
    /// (`thread::start`'s callback).
    pub(crate) fn func_isolated(params: Vec<ParameterType>, ret: ParameterType) -> Self {
        ParameterType::Func(params, Box::new(ret), true)
    }
    /// A thread-handle type from its parts; `res` defaults to
    /// [`Nothing`](Self::Nothing) at the call site when the handle carries no
    /// resource plane.
    pub(crate) fn thread_handle(
        worker: bool,
        msg: ParameterType,
        res: ParameterType,
        out: ParameterType,
    ) -> Self {
        ParameterType::ThreadHandle {
            worker,
            msg: Box::new(msg),
            res: Box::new(res),
            out: Box::new(out),
        }
    }

    /// Parse a concrete type *name* — the currency the type checker still speaks at
    /// the registry boundary (`"List OF Integer"`, `"Instant"`, `"Unknown"`) — into a
    /// `ParameterType`. This is the *only* place a string becomes a `ParameterType`;
    /// inside the registry everything is already a `ParameterType`. Scalars and
    /// `List`/`Map`/`Set` are recognized structurally; anything else (a record, union,
    /// or function type) becomes a [`Named`] whose runtime name is interned to a
    /// `Copy` [`Symbol`] (deduplicated, not leaked per occurrence). A leading `RES `
    /// ownership marker is stripped (a collection element stores the bare type).
    pub(crate) fn parse(name: &str) -> ParameterType {
        // A grouped type name (`(T)`) is the source-level parenthesization the
        // parser emits verbatim (`ast/expr.rs`). Peel it HERE, so it is peeled at
        // every level of the grammar — that is what makes
        // `List OF (Map OF String TO Integer)` decompose into
        // `ListOf(MapOf(String, Integer))` (bug-105) instead of falling to a junk
        // `UserOf("(Map", [Named("String TO Integer)")])` whose only virtue was
        // that `name()` rendered the garbage back verbatim.
        //
        // This is the one place `parse` is deliberately NORMALIZING rather than
        // byte-exact: `parse("(Integer)").name()` is `"Integer"`. That is the
        // form every consumer already computed for itself before plan-106-D —
        // `resolver::resolve_type_name`, the former source checker's `parse_type`, and
        // `monomorph::lower` (×2) each called `strip_type_group` at their own
        // position — so nothing downstream sees a type it did not see before.
        // Once the type is structural, the group carries no information, and a
        // consumer walking variants (rather than re-parsing a string) has no
        // place left to strip it.
        if name.starts_with('(') && name.ends_with(')') {
            let stripped = strip_type_group(name);
            if stripped != name {
                return ParameterType::parse(stripped);
            }
        }
        // A `RES ` ownership marker wraps a [`Res`](Self::Res) around the inner type
        // (rather than being stripped), so a collection element like
        // `List OF RES File` round-trips byte-exact through `parse`/`name`. Matching
        // stays RES-transparent (unify/leaf_matches unwrap it), so overload selection
        // is unchanged from the historical strip-on-parse.
        if let Some(rest) = name.strip_prefix("RES ") {
            return ParameterType::res(ParameterType::parse(rest));
        }
        if name == "Unknown" {
            return ParameterType::Unknown;
        }
        // A parametric thread type (`Thread OF Msg [RES Res] TO Out` /
        // `ThreadWorker OF ...`) decomposes structurally into a `ThreadHandle`, its
        // three slots recursively parsed — mirroring the `List`/`Map`/`FUNC` arms.
        // The resource plane defaults to `Nothing` when absent (elided by `name`).
        // A bare opaque `Thread` / `ThreadWorker` (no ` OF ` body) is not a handle
        // and falls through to `Named`.
        if let Some((kind, message, resource, output)) = thread_parts_full(name) {
            return ParameterType::thread_handle(
                kind == THREAD_WORKER_TYPE,
                ParameterType::parse(message),
                resource.map_or(ParameterType::Nothing, ParameterType::parse),
                ParameterType::parse(output),
            );
        }
        if let Some(elem) = name.strip_prefix("List OF ") {
            return ParameterType::list_of(ParameterType::parse(elem));
        }
        if let Some(elem) = name.strip_prefix("Set OF ") {
            return ParameterType::set_of(ParameterType::parse(elem));
        }
        if let Some((key, value)) = name.strip_prefix("Map OF ").and_then(split_top_level_to) {
            return ParameterType::map_of(ParameterType::parse(key), ParameterType::parse(value));
        }
        // `MapEntry OF K TO V` — the key/value pair shape, split on the first top-level
        // ` TO ` exactly as the `Map OF ` arm above (so the two spellings decompose
        // identically), via the depth-aware [`split_top_level_to`]. `MapEntry OF …`
        // does not start with `Map OF `, so the order relative to that arm is
        // immaterial.
        if let Some((key, value)) = name
            .strip_prefix("MapEntry OF ")
            .and_then(split_top_level_to)
        {
            return ParameterType::map_entry_of(
                ParameterType::parse(key),
                ParameterType::parse(value),
            );
        }
        // `Result OF T` — a single success-typed child, mirroring `List OF `/`Set OF `.
        if let Some(success) = name.strip_prefix("Result OF ") {
            return ParameterType::result_of(ParameterType::parse(success));
        }
        // `ISOLATED FUNC(...)` (a capture-free worker entry — `thread::start`'s
        // callback) parses to an isolated [`Func`](Self::Func), so the `Func` arm can
        // reach the nested `ThreadWorker` handle in its first parameter; the
        // `ISOLATED ` marker is preserved for a byte-exact `name()` round-trip.
        let (isolated, func_rest) = match name.strip_prefix("ISOLATED FUNC(") {
            Some(rest) => (true, Some(rest)),
            None => (false, name.strip_prefix("FUNC(")),
        };
        if let Some(rest) = func_rest {
            // Split `<params>) AS <return>` at paren depth 0: the closing paren and the
            // separating commas are the ones at depth 0 (a parameter may itself be a
            // `FUNC(...)`). Mirrors `codegen::builtins::general::function_parts`.
            if let Some((params, ret)) =
                crate::codegen::builtins::split_func_params_and_return(rest)
            {
                let params = params.into_iter().map(ParameterType::parse).collect();
                let ret = ParameterType::parse(ret);
                return if isolated {
                    ParameterType::func_isolated(params, ret)
                } else {
                    ParameterType::func(params, ret)
                };
            }
        }
        // A user generic (`Pair OF Integer, String`). Ordered LAST among the ` OF `
        // shapes so every built-in constructor above claims its own spelling first;
        // what reaches here is by definition a non-builtin head. The argument list
        // splits on TOP-LEVEL commas — the one place that rule now lives (it used to
        // be duplicated in `monomorph::helpers::split_top_level_commas`).
        if let Some((head, rest)) = Self::split_user_generic(name) {
            return ParameterType::user_of(
                head,
                crate::codegen::builtins::split_top_level_commas(rest)
                    .into_iter()
                    .map(ParameterType::parse)
                    .collect(),
            );
        }
        // A resource's own top-level ` STATE T` clause (plan-111-A). Ordered
        // LAST, after every container arm above has claimed its own spelling —
        // so a ` STATE ` reaching here belongs to *this* leaf phrase and not to
        // a `List`/`Map`/`Thread` element that an arm above already re-parsed.
        // The decision is `split_state_clause`'s and only its: the base must be
        // a single bare token, or the clause is a nested type's and this stays
        // one opaque `Named`, exactly as before the variant existed.
        if let Some((base, state)) = split_state_clause(name) {
            return ParameterType::Stateful {
                base: Box::new(ParameterType::parse(base)),
                state: Box::new(ParameterType::parse(state)),
            };
        }
        match name {
            "AttributeString" => ParameterType::AttributeString,
            "Boolean" => ParameterType::Boolean,
            "Byte" => ParameterType::Byte,
            "Integer" => ParameterType::Integer,
            "Fixed" => ParameterType::Fixed,
            "Float" => ParameterType::Float,
            "Money" => ParameterType::Money,
            "Nothing" => ParameterType::Nothing,
            "String" => ParameterType::String,
            other => ParameterType::named(other),
        }
    }

    /// Split a user-generic spelling `Name OF a, b` into its head and its raw
    /// argument text, or `None` when `name` is not one.
    ///
    /// This is the ONE definition of "is a user generic" in the compiler. It used
    /// to be duplicated in `monomorph::helpers::user_template_parts`, which listed
    /// the same built-in prefixes and had to be edited in lockstep whenever the
    /// grammar grew (`planning/Compiler Pipeline.md:25`).
    ///
    /// The `OF` that separates head from arguments is the FIRST one, and the head
    /// must be a single bare identifier: `List OF Integer` is excluded by the
    /// built-in check, while `Holder OF Pair OF A, B` splits as
    /// (`Holder`, `Pair OF A, B`) and recurses.
    fn split_user_generic(name: &str) -> Option<(&str, &str)> {
        let (head, rest) = name.split_once(" OF ")?;
        // Every built-in constructor spelled with ` OF ` is claimed by its own
        // `parse` arm above; reaching here with one would mean that arm failed to
        // match (a malformed `Map OF K` with no ` TO `), and re-reading it as a user
        // generic named `Map` would be wrong.
        if matches!(
            head,
            "List" | "Set" | "Map" | "MapEntry" | "Result" | THREAD_TYPE | THREAD_WORKER_TYPE
        ) {
            return None;
        }
        // A head is a nominal type name, never a phrase: `RES fs.File` and
        // `ISOLATED FUNC(...)` are handled above, but a malformed input could still
        // leave a space here, and such a name is not a template.
        if head.is_empty() || head.contains(' ') || rest.is_empty() {
            return None;
        }
        Some((head, rest))
    }

    /// Attach a resource's ` STATE <T>` clause, producing exactly what
    /// [`parse`](Self::parse) produces for the concatenated spelling
    /// (`"{self} STATE {state}"`) — structurally, with no round trip.
    ///
    /// `STATE` is **not** a variant. Outside a thread plane `parse` has no arm for
    /// it, so `File STATE Cursor` falls through to the nominal tail as one opaque
    /// [`Named`](Self::Named). Inside a container the clause therefore lands on the
    /// *element*: `parse("List OF File STATE Cursor")` is
    /// `ListOf(Named("File STATE Cursor"))`, because the container arm strips its
    /// prefix and re-parses the whole remainder.
    ///
    /// So the fold below recurses into the **last child each shape renders** — the
    /// one the trailing text abuts — and wraps the leaf it reaches. `parse` is
    /// left-to-right over `name()`'s output, so "last rendered child" is exactly
    /// the position the appended clause would be re-read into.
    ///
    /// Guarded by `with_state_matches_parse_of_the_concatenated_spelling`, which
    /// asserts `t.with_state(s) == parse(&format!("{} STATE {}", t.name(),
    /// s.name()))` over every shape.
    pub(crate) fn with_state(&self, state: &ParameterType) -> ParameterType {
        let state_name = state.name();
        match self {
            ParameterType::ListOf(element) => ParameterType::list_of(element.with_state(state)),
            ParameterType::SetOf(element) => ParameterType::set_of(element.with_state(state)),
            ParameterType::ResultOf(success) => ParameterType::result_of(success.with_state(state)),
            ParameterType::Res(inner) => ParameterType::Res(Box::new(inner.with_state(state))),
            ParameterType::MapOf(key, value) => {
                ParameterType::map_of((**key).clone(), value.with_state(state))
            }
            ParameterType::MapEntryOf(key, value) => {
                ParameterType::map_entry_of((**key).clone(), value.with_state(state))
            }
            ParameterType::Func(params, ret, isolated) => {
                ParameterType::Func(params.clone(), Box::new(ret.with_state(state)), *isolated)
            }
            ParameterType::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => ParameterType::ThreadHandle {
                worker: *worker,
                msg: msg.clone(),
                res: res.clone(),
                out: Box::new(out.with_state(state)),
            },
            // A user generic renders its arguments last, comma-joined; the clause
            // abuts the final one.
            ParameterType::UserOf(name, args) if !args.is_empty() => {
                let mut args = args.clone();
                let last = args.len() - 1;
                args[last] = args[last].with_state(state);
                ParameterType::user_of(name.resolve(), args)
            }
            // Every leaf — scalar, nominal, `Var`, `Unknown`, `Arg`, an
            // argument-less user generic — takes the clause the way `parse`
            // reads the whole `"<leaf> STATE <T>"` phrase: a
            // [`Stateful`](Self::Stateful) when the leaf's own spelling is a
            // single token, one opaque `Named` when it is not.
            //
            // Routed THROUGH `parse` rather than built structurally, because
            // that is precisely the contract this method promises and
            // `with_state_matches_parse_of_the_concatenated_spelling` asserts
            // over every shape. A `Var` or an `Arg` is never something `parse`
            // produces, so a structural `Stateful { base: leaf.clone(), .. }`
            // would hold a `Var("T")` where the concatenated spelling reads
            // back a `Named("T")` — a divergence that did not exist while both
            // sides collapsed into one opaque nominal. `parse` is the grammar's
            // own recursion here (plan-111-A §2, boundary 1); the input is a
            // name this method just rendered, never a spelling from outside.
            leaf => ParameterType::parse(&format!("{} STATE {state_name}", leaf.name())),
        }
    }

    /// Split a resource type into its base and its own **top-level** ` STATE T`
    /// clause, if it carries one.
    ///
    /// plan-111-A: a pure structural match. `STATE` now HAS a variant
    /// ([`Stateful`](Self::Stateful)), and [`parse`](Self::parse) builds one
    /// only where the clause is genuinely top-level — so the question "does
    /// this type carry its own STATE?" is answered by which variant it is,
    /// with no spelling and no guard. Before the variant this method rendered
    /// `self` and re-split the text, which is how the grammar came to exist
    /// twice (plan-106-B §Phase 2 census, plan-106-C Phase 1).
    ///
    /// **Top-level only, and that is load-bearing.** The rule is unchanged,
    /// it just moved into `parse`: a base containing a space is a composite, so
    /// `List OF RES File STATE Cursor` and `Result OF Stream STATE Pending`
    /// split to *nothing* — their clause sits on the inner
    /// `Stateful`, and the outer `ListOf`/`ResultOf` falls to the `other` arm
    /// here. That no-op is what keeps both sides of a comparison
    /// normalizing identically — peeling the element's clause on one side while
    /// the other has none to peel is exactly the asymmetry bug-429 fixed
    /// (`ir::verify::values.rs`, `check_result_value_type`). It does **not**
    /// descend, and so it is NOT a general inverse of
    /// [`with_state`](Self::with_state), which attaches to the innermost
    /// rendered child; the two agree precisely on leaf bases, which is every
    /// place a resource's STATE is actually read.
    ///
    /// Guarded by `split_state_is_top_level_only` and
    /// `split_state_matches_the_name_domain_helpers`.
    pub(crate) fn split_state(&self) -> (ParameterType, Option<ParameterType>) {
        match self {
            ParameterType::Stateful { base, state } => ((**base).clone(), Some((**state).clone())),
            other => (other.clone(), None),
        }
    }

    /// The top-level ` STATE T` clause this type carries, if any — the structural
    /// twin of `codegen::resource::state_type_name`, and the read half of
    /// [`split_state`](Self::split_state).
    pub(crate) fn state(&self) -> Option<ParameterType> {
        self.split_state().1
    }

    /// This type with its top-level ` STATE T` clause removed — the structural
    /// twin of `codegen::resource::base_resource_name`.
    pub(crate) fn without_state(&self) -> ParameterType {
        self.split_state().0
    }

    /// Whether a ` STATE T` clause appears ANYWHERE in this type's rendered
    /// spelling — its own top-level one, or one riding a nested payload
    /// (`MapEntry OF K TO File STATE Cursor`, a thread's resource plane, a user
    /// generic's last argument).
    ///
    /// Exactly `self.name().contains(" STATE ")`, computed structurally;
    /// `contains_state_agrees_with_the_rendered_name` pins that equality over
    /// the round-trip corpus. [`split_state`](Self::split_state) answers a
    /// different and narrower question — whether the clause is *this* type's own
    /// — and a consumer that wants "is there a state anywhere in here" must ask
    /// this one instead (plan-111-B; `ir::verify::resources`'s defaultability
    /// rule is the caller that needs the broad form).
    ///
    /// The [`Named`](Self::Named) arm is not redundant. `parse` declines to
    /// split a ` STATE ` whose base is a composite, so such a spelling survives
    /// as one opaque nominal that still *reads* as stateful — and decoded
    /// package IR is attacker-controlled and need not be well formed (PKG-02),
    /// so the rule this serves must see it.
    pub(crate) fn contains_state(&self) -> bool {
        match self {
            ParameterType::Stateful { .. } => true,
            ParameterType::Named(sym) => sym.resolve().contains(" STATE "),
            ParameterType::ListOf(inner)
            | ParameterType::SetOf(inner)
            | ParameterType::ResultOf(inner)
            | ParameterType::Res(inner) => inner.contains_state(),
            ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
                key.contains_state() || value.contains_state()
            }
            ParameterType::UserOf(_, args) => args.iter().any(ParameterType::contains_state),
            ParameterType::Func(params, ret, _) => {
                params.iter().any(ParameterType::contains_state) || ret.contains_state()
            }
            ParameterType::ThreadHandle { msg, res, out, .. } => {
                msg.contains_state() || res.contains_state() || out.contains_state()
            }
            _ => false,
        }
    }

    /// Whether this type is the nominal named `name`.
    ///
    /// A NOMINAL only: a scalar variant answers `false` even when its rendered
    /// name matches, because the question this answers is "is this the named
    /// user/builtin type?", not "does this render as?". Added by plan-106-C when
    /// the former source checker's four built-in nominal variants became `Named`.
    pub(crate) fn is_named(&self, name: &str) -> bool {
        matches!(self, ParameterType::Named(sym) if sym.resolve() == name)
    }

    /// The parameter type's formatted name.
    pub(crate) fn name(&self) -> Cow<'static, str> {
        match self {
            ParameterType::AttributeString => Cow::Borrowed("AttributeString"),
            ParameterType::Boolean => Cow::Borrowed("Boolean"),
            ParameterType::Byte => Cow::Borrowed("Byte"),
            ParameterType::Integer => Cow::Borrowed("Integer"),
            ParameterType::Fixed => Cow::Borrowed("Fixed"),
            ParameterType::Float => Cow::Borrowed("Float"),
            ParameterType::Money => Cow::Borrowed("Money"),
            ParameterType::Nothing => Cow::Borrowed("Nothing"),
            ParameterType::String => Cow::Borrowed("String"),
            ParameterType::ListOf(elem) => Cow::Owned(format!("List OF {}", elem.name())),
            ParameterType::MapOf(elem_a, elem_b) => {
                Cow::Owned(format!("Map OF {} TO {}", elem_a.name(), elem_b.name()))
            }
            ParameterType::SetOf(elem) => Cow::Owned(format!("Set OF {}", elem.name())),
            ParameterType::MapEntryOf(key, value) => {
                Cow::Owned(format!("MapEntry OF {} TO {}", key.name(), value.name()))
            }
            ParameterType::ResultOf(success) => Cow::Owned(format!("Result OF {}", success.name())),
            ParameterType::Res(inner) => Cow::Owned(format!("RES {}", inner.name())),
            // Byte-identical to the spelling `parse` split, and to what
            // `with_state` produced as one opaque `Named` before the variant
            // existed — the `.mfp` type table, the IR encoding and every
            // diagnostic that embeds a type name all depend on this rendering
            // not moving (plan-111-A §Compatibility).
            ParameterType::Stateful { base, state } => {
                Cow::Owned(format!("{} STATE {}", base.name(), state.name()))
            }
            ParameterType::Named(elem) => Cow::Borrowed(elem.resolve()),
            ParameterType::UserOf(name, args) => Cow::Owned(format!(
                "{} OF {}",
                name.resolve(),
                args.iter().map(|a| a.name()).collect::<Vec<_>>().join(", ")
            )),
            ParameterType::Var(name) => Cow::Borrowed(name.resolve()),
            ParameterType::Func(params, ret, isolated) => Cow::Owned(format!(
                "{}FUNC({}) AS {}",
                if *isolated { "ISOLATED " } else { "" },
                params
                    .iter()
                    .map(|p| p.name())
                    .collect::<Vec<_>>()
                    .join(", "),
                ret.name()
            )),
            ParameterType::Arg(n) => Cow::Owned(format!("Arg{n}")),
            ParameterType::Unknown => Cow::Borrowed("Unknown"),
            ParameterType::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => {
                let kind = if *worker {
                    THREAD_WORKER_TYPE
                } else {
                    THREAD_TYPE
                };
                // The resource plane is elided when `Nothing`, exactly reproducing
                // `format_thread_type` (a data-only `Thread OF Msg TO Out`).
                let res_name = match res.as_ref() {
                    ParameterType::Nothing => None,
                    other => Some(other.name()),
                };
                Cow::Owned(format_thread_type(
                    kind,
                    &msg.name(),
                    res_name.as_deref(),
                    &out.name(),
                ))
            }
        }
    }

    /// Whether this is a scalar primitive (non-container, non-nominal).
    pub(crate) fn is_scalar(&self) -> bool {
        matches!(
            self,
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
}

impl fmt::Display for ParameterType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

// --- Thread-type string vocabulary --------------------------------------------
//
// The parent `Thread` and worker `ThreadWorker` handle types are spelled as
// parametric strings (`Thread OF Msg [RES Res] TO Out`). Their structural
// splitters/renderers live here — not under `builtins` — because
// [`ParameterType::parse`]/[`name`](ParameterType::name) decompose/render them into
// the [`ParameterType::ThreadHandle`] variant, and every other consumer (monomorph,
// the former source checker, codegen, binary_repr) speaks the same string vocabulary. This is the
// former `builtins::thread` splitter set, kept byte-identical so the migrated
// `thread` package and all its codegen citations agree on the spelling.

/// The parent thread handle's type name.
pub(crate) const THREAD_TYPE: &str = "Thread";
/// The worker thread handle's type name.
pub(crate) const THREAD_WORKER_TYPE: &str = "ThreadWorker";

/// Render a thread type string from its parts, emitting the optional `RES Res`
/// clause and the resource-only spelling (`message == "Nothing"`) symmetrically
/// with [`split_thread_types`].
pub(crate) fn format_thread_type(
    kind: &str,
    message: &str,
    resource: Option<&str>,
    output: &str,
) -> String {
    match resource {
        Some(resource) if message == "Nothing" => {
            format!("{kind} OF RES {resource} TO {output}")
        }
        Some(resource) => format!("{kind} OF {message} RES {resource} TO {output}"),
        None => format!("{kind} OF {message} TO {output}"),
    }
}

/// Whether `name` spells a parent `Thread` handle type. Part of the thread-type
/// vocabulary the man pages cite; the parent/worker split is now enforced structurally
/// by the `thread` descriptor's kind-split overloads, so this predicate has no
/// remaining code caller.
#[allow(dead_code)]
pub(crate) fn is_parent_thread_type(name: &str) -> bool {
    thread_parts(name).is_some_and(|(kind, _, _)| kind == THREAD_TYPE)
}

/// A thread handle's `(kind, message, output)`, dropping the resource plane.
pub(crate) fn thread_parts(name: &str) -> Option<(&str, &str, &str)> {
    thread_parts_full(name).map(|(kind, message, _, output)| (kind, message, output))
}

/// Full structural view of a thread type: `(kind, message, resource, output)`.
/// `message` is the data-plane message type (`"Nothing"` for a resource-only
/// thread); `resource` is the resource-plane type, present only when the type
/// carries a `RES Res` clause.
pub(crate) fn thread_parts_full(name: &str) -> Option<(&'static str, &str, Option<&str>, &str)> {
    let (kind, rest) = if let Some(rest) = name.strip_prefix("Thread OF ") {
        (THREAD_TYPE, rest)
    } else if let Some(rest) = name.strip_prefix("ThreadWorker OF ") {
        (THREAD_WORKER_TYPE, rest)
    } else {
        return None;
    };
    let (message, resource, output) = split_thread_types(rest)?;
    Some((
        kind,
        strip_type_group(message),
        resource.map(strip_type_group),
        strip_type_group(output),
    ))
}

/// Parse the body after `Thread OF ` / `ThreadWorker OF ` into
/// `(message, resource, output)`. Accepts three shapes:
///   `<msg> TO <out>`              (data-only)
///   `<msg> RES <res> TO <out>`    (data + resource planes)
///   `RES <res> TO <out>`          (resource-only; message defaults to Nothing)
fn split_thread_types(rest: &str) -> Option<(&str, Option<&str>, &str)> {
    let rest = rest.trim();

    // Resource-only: no data message before the `RES` clause.
    if let Some(after_res) = rest.strip_prefix("RES ") {
        let res_end = resource_element_len(after_res)?;
        let resource = after_res[..res_end].trim();
        let output = after_res.get(res_end..)?.strip_prefix(" TO ")?.trim();
        return Some(("Nothing", Some(resource), output));
    }

    let message_end = type_prefix_len(rest)?;
    let message = rest[..message_end].trim();
    let tail = rest.get(message_end..)?;

    // Optional ` RES <res>` clause between the message and ` TO `.
    if let Some(after_res) = tail.strip_prefix(" RES ") {
        let res_end = resource_element_len(after_res)?;
        let resource = after_res[..res_end].trim();
        let output = after_res.get(res_end..)?.strip_prefix(" TO ")?.trim();
        return Some((message, Some(resource), output));
    }

    let output = tail.strip_prefix(" TO ")?.trim();
    Some((message, None, output))
}

/// Strip a fully-parenthesized outer group (`(Integer)` → `Integer`), leaving a
/// non-span group (`(a), b`) unchanged.
/// Split a `Map`/`MapEntry` body `K TO V` on the ` TO ` that separates the outer
/// key from its value.
///
/// A leftmost `split_once(" TO ")` mis-parses a key that itself carries a top-level
/// ` TO ` (`Map OF Map OF String TO Integer TO Boolean`, bug-108.2): separators
/// inside parenthesized / `FUNC(...)` groups must be skipped, and so must the ` TO `
/// owned by each nested `Map`/`MapEntry`/`Thread`/`ThreadWorker` sub-type. Returns
/// `None` when there is no top-level ` TO `.
///
/// plan-105-B moved this here, into the canonical grammar, and pointed
/// [`ParameterType::parse`] at it. Before that, `parse` used the naive leftmost
/// split while `monomorph::helpers` kept this correct copy — so the "one grammar"
/// consolidation could not simply route through `parse` without regressing
/// bug-108.2. The two other copies (`monomorph`, the former source checker's `types::split_map_body`)
/// are the lockstep-edit hazard the architectural review flagged
/// (`planning/Compiler Pipeline.md:25`).
/// Split a type spelling into its base and its OWN top-level ` STATE T` clause.
///
/// **The compiler's one `STATE` grammar** (plan-111-A). It has exactly two
/// callers: [`ParameterType::parse`], which turns the split into a
/// [`Stateful`](ParameterType::Stateful) variant, and the `&str` adapters
/// `codegen::resource::{base_resource_name, state_type_name}`, which need to
/// return a slice *borrowed from their input* and so cannot go through the
/// owned typed accessors until letter E retypes them. Everything else asks
/// [`ParameterType::split_state`], which is now a structural match and touches
/// no spelling at all.
///
/// It used to exist twice — here and, byte-identical, in
/// `codegen::resource::split_state_clause` — with a parity test pinning the two
/// copies together. That is the lockstep-edit hazard
/// `planning/Compiler Pipeline.md:25` flagged and plan-106-E's Correction 3
/// caught in the wild.
///
/// The guard is the load-bearing part: a base containing a space is a composite
/// whose ` STATE ` belongs to something *nested*
/// (`Thread OF … RES File STATE Cursor TO …`, plan-54), not to this type.
pub(crate) fn split_state_clause(type_name: &str) -> Option<(&str, &str)> {
    let (base, state) = type_name.split_once(" STATE ")?;
    if base.contains(' ') {
        return None; // nested STATE inside a composite type — not this type's own.
    }
    Some((base, state))
}

pub(crate) fn split_top_level_to(body: &str) -> Option<(&str, &str)> {
    let bytes = body.as_bytes();
    let mut depth: usize = 0;
    // Nested `OF`-constructs seen at depth 0 whose ` TO ` has not yet appeared.
    let mut pending: usize = 0;
    let mut index = 0;
    while index < body.len() {
        match bytes[index] {
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            // `is_char_boundary` guards the slice: `.mfp`-decoded type strings are
            // not guaranteed ASCII, so `index` can land on a UTF-8 continuation
            // byte where `body[index..]` would panic (bug-169). A non-boundary
            // byte never begins ` TO ` nor a keyword, so skipping it is correct.
            _ if depth == 0
                && body.is_char_boundary(index)
                && body[index..].starts_with(" TO ") =>
            {
                if pending > 0 {
                    pending -= 1;
                    index += 4;
                } else {
                    return Some((&body[..index], &body[index + 4..]));
                }
            }
            _ if depth == 0
                && body.is_char_boundary(index)
                && type_owns_a_to_separator(body, index) =>
            {
                pending += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

/// Whether a `Map`/`MapEntry`/`Thread`/`ThreadWorker` `OF`-construct — each of
/// which owns exactly one top-level ` TO ` — begins at byte `at` of `body`. The
/// keyword must sit on a word boundary so a template whose name merely ends in
/// `Map` is not counted.
fn type_owns_a_to_separator(body: &str, at: usize) -> bool {
    let bytes = body.as_bytes();
    if at > 0 {
        let prev = bytes[at - 1];
        if prev.is_ascii_alphanumeric()
            || prev == b'_'
            || prev == b'.'
            || prev == b':'
            || prev >= 0x80
        {
            return false;
        }
    }
    ["MapEntry OF ", "ThreadWorker OF ", "Map OF ", "Thread OF "]
        .iter()
        .any(|keyword| body[at..].starts_with(keyword))
}

/// Whether `type_` is a WORKER-side thread handle (`ThreadWorker OF …`).
///
/// plan-106-E: the typed twin of [`is_worker_thread_type`], for the codegen sites
/// that pick the worker vs parent runtime helper off a value's static type. It is
/// the `worker` flag of the variant — no spelling involved.
pub(crate) fn is_worker_thread_handle(type_: &ParameterType) -> bool {
    matches!(type_, ParameterType::ThreadHandle { worker: true, .. })
}

pub(crate) fn strip_type_group(type_: &str) -> &str {
    let trimmed = type_.trim();
    if !(trimmed.starts_with('(') && trimmed.ends_with(')')) {
        return trimmed;
    }
    let mut depth = 0usize;
    for (index, ch) in trimmed.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && index + ch.len_utf8() != trimmed.len() {
                    return trimmed;
                }
            }
            _ => {}
        }
    }
    &trimmed[1..trimmed.len() - 1]
}

/// Length consumed by a thread plane's `RES` element: the resource base type plus
/// an optional ` STATE <T>` clause (plan-54).
fn resource_element_len(after_res: &str) -> Option<usize> {
    let base = type_prefix_len(after_res)?;
    match after_res
        .get(base..)
        .and_then(|tail| tail.strip_prefix(" STATE "))
    {
        Some(after_state) => {
            let state_len = type_prefix_len(after_state)?;
            Some(base + " STATE ".len() + state_len)
        }
        None => Some(base),
    }
}

/// Length consumed by a single type prefix at the start of `input`, descending
/// through `List`/`Result`/`Map`/`MapEntry`/`Thread`/`ThreadWorker` nesting.
fn type_prefix_len(input: &str) -> Option<usize> {
    let input = input.trim_start();
    if input.starts_with('(') {
        let mut depth = 0usize;
        for (index, ch) in input.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(index + ch.len_utf8());
                    }
                }
                _ => {}
            }
        }
        return None;
    }

    let base_end = input
        .char_indices()
        .find_map(|(index, ch)| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == ':' || ch == '.' {
                None
            } else {
                Some(index)
            }
        })
        .unwrap_or(input.len());
    if base_end == 0 {
        return None;
    }
    let base = &input[..base_end];
    let Some(after_of) = input[base_end..].strip_prefix(" OF ") else {
        return Some(base_end);
    };

    if matches!(base, "List" | "Result") {
        return type_prefix_len(after_of).map(|len| base_end + 4 + len);
    }

    if matches!(base, "Map" | "MapEntry") {
        let first_len = type_prefix_len(after_of)?;
        let after_first = after_of.get(first_len..)?;
        let second_input = after_first.strip_prefix(" TO ")?;
        let second_len = type_prefix_len(second_input)?;
        return Some(base_end + 4 + first_len + 4 + second_len);
    }

    if matches!(base, "Thread" | "ThreadWorker") {
        // `[msg] [RES res] TO out` — mirror split_thread_types' three shapes.
        return thread_body_len(after_of).map(|len| base_end + 4 + len);
    }

    Some(base_end)
}

/// Length consumed by a thread type body (`[msg] [RES res] TO out`) starting at
/// `rest`. Used by [`type_prefix_len`] to measure a nested thread type.
fn thread_body_len(rest: &str) -> Option<usize> {
    if let Some(after_res) = rest.strip_prefix("RES ") {
        let res_len = resource_element_len(after_res)?;
        let to = after_res.get(res_len..)?.strip_prefix(" TO ")?;
        let out_len = type_prefix_len(to)?;
        // "RES " (4) + res + " TO " (4) + out
        return Some(4 + res_len + 4 + out_len);
    }

    let msg_len = type_prefix_len(rest)?;
    let tail = rest.get(msg_len..)?;

    if let Some(after_res) = tail.strip_prefix(" RES ") {
        let res_len = resource_element_len(after_res)?;
        let to = after_res.get(res_len..)?.strip_prefix(" TO ")?;
        let out_len = type_prefix_len(to)?;
        // msg + " RES " (5) + res + " TO " (4) + out
        return Some(msg_len + 5 + res_len + 4 + out_len);
    }

    let to = tail.strip_prefix(" TO ")?;
    let out_len = type_prefix_len(to)?;
    // msg + " TO " (4) + out
    Some(msg_len + 4 + out_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip a thread-type spelling through `parse` → `name` and assert it is
    /// byte-identical — the core `ThreadHandle` guarantee (nested threads,
    /// parenthesized groups, `RES … STATE …` planes all preserved).
    fn round_trip(spelling: &str) {
        assert_eq!(
            ParameterType::parse(spelling).name(),
            spelling,
            "round-trip mismatch for `{spelling}`"
        );
    }

    /// plan-106-D: a grouped type name decomposes STRUCTURALLY at every level,
    /// instead of surviving as a junk `Named`/`UserOf` that only round-tripped
    /// because `name()` echoed the garbage back verbatim.
    ///
    /// This is the deliberate exception to `parse`↔`name` byte-exactness: the
    /// group is source-level parenthesization carrying no type information, and
    /// it is the form every consumer already computed by calling
    /// `strip_type_group` at its own position.
    #[test]
    fn a_grouped_type_name_decomposes_at_every_level() {
        assert_eq!(ParameterType::parse("(Integer)"), ParameterType::Integer);
        assert_eq!(
            ParameterType::parse("List OF (Map OF String TO Integer)"),
            ParameterType::list_of(ParameterType::map_of(
                ParameterType::String,
                ParameterType::Integer,
            ))
        );
        assert_eq!(
            ParameterType::parse("(((Integer)))"),
            ParameterType::Integer
        );
        // The group is normalized away, so the render is the bare spelling.
        assert_eq!(
            ParameterType::parse("List OF (Map OF String TO Integer)").name(),
            "List OF Map OF String TO Integer"
        );

        // A leading `(` that is NOT a whole-name group must not be peeled: the
        // depth check in `strip_type_group` is what keeps `(A) TO (B)` — and any
        // other shape whose first `(` closes before the end — intact.
        assert_eq!(
            ParameterType::parse("(A) TO (B)"),
            ParameterType::named("(A) TO (B)")
        );
        // A `FUNC(...)` type does not start with `(`, so the peel never sees it.
        assert_eq!(
            ParameterType::parse("FUNC(Integer) AS String").name(),
            "FUNC(Integer) AS String"
        );
    }

    #[test]
    fn thread_handle_parses_into_variant() {
        assert_eq!(
            ParameterType::parse("Thread OF Integer TO String"),
            ParameterType::thread_handle(
                false,
                ParameterType::Integer,
                ParameterType::Nothing,
                ParameterType::String,
            )
        );
        assert_eq!(
            ParameterType::parse("ThreadWorker OF Integer TO String"),
            ParameterType::thread_handle(
                true,
                ParameterType::Integer,
                ParameterType::Nothing,
                ParameterType::String,
            )
        );
        assert_eq!(
            ParameterType::parse("Thread OF RES fs.File TO String"),
            ParameterType::thread_handle(
                false,
                ParameterType::Nothing,
                ParameterType::named("fs.File"),
                ParameterType::String,
            )
        );
        assert_eq!(
            ParameterType::parse("Thread"),
            ParameterType::named("Thread")
        );
        assert_eq!(
            ParameterType::parse("ThreadWorker"),
            ParameterType::named("ThreadWorker")
        );
    }

    #[test]
    fn thread_handle_round_trips_byte_exact() {
        for spelling in [
            "Thread OF Integer TO String",
            "ThreadWorker OF Integer TO String",
            "Thread OF Integer RES fs.File TO String",
            "Thread OF RES fs.File TO String",
            "Thread OF RES fs.File STATE Cursor TO Integer",
            "Thread OF Integer RES fs.File STATE Cursor TO String",
            "Thread OF List OF Integer TO Map OF String TO Integer",
            "Thread OF Thread OF Integer TO String TO Boolean",
            "Thread OF Thread OF Integer RES fs.File TO String TO Boolean",
            "Thread OF Unknown TO String",
            "Thread OF Nothing TO String",
            // bug-463: the crossing cases — a message plane whose type has to be
            // MEASURED (`type_prefix_len`) past a `RES` marker, a `Set`, a
            // `FUNC(...)` or a user generic. Byte-exactness alone does NOT guard
            // these: with the bug present every one of them still round-tripped,
            // because the whole spelling collapsed to a `Named` and `Named(s).name()`
            // echoes `s` back verbatim. The tests that actually see it are
            // `res_collection_on_thread_plane_parses_structurally` and
            // `type_prefix_len_measures_every_parse_constructor`.
            "Thread OF List OF RES fs.File TO Integer",
            "Thread OF Map OF String TO RES fs.File TO Integer",
            "ThreadWorker OF List OF RES fs.File TO Integer",
            "Thread OF List OF RES fs.File STATE Cursor TO Integer",
            "Thread OF List OF RES fs.File RES fs.File TO Integer",
            "Thread OF Map OF RES fs.File TO Integer TO Integer",
            "Thread OF Set OF Integer TO Integer",
            "Thread OF Set OF RES fs.File TO Integer",
            "Thread OF FUNC(Integer) AS String TO Integer",
            "Thread OF ISOLATED FUNC(Integer) AS String TO Integer",
            "Thread OF Pair OF Integer, String TO Integer",
            "Thread OF List OF Set OF Integer TO Integer",
            "Thread OF Map OF Set OF Integer TO Integer TO Integer",
        ] {
            round_trip(spelling);
        }
    }

    #[test]
    fn isolated_func_round_trips_and_decomposes() {
        // `thread::start`'s worker callback: the `ISOLATED ` marker round-trips, and the
        // nested `ThreadWorker` handle is reachable through the `Func` params.
        let spelling =
            "ISOLATED FUNC(ThreadWorker OF Integer RES fs.File TO String, Integer) AS String";
        assert_eq!(ParameterType::parse(spelling).name(), spelling);
        assert_eq!(
            ParameterType::parse("FUNC(Integer) AS String").name(),
            "FUNC(Integer) AS String"
        );
        match ParameterType::parse(spelling) {
            ParameterType::Func(params, ret, isolated) => {
                assert!(isolated);
                assert_eq!(ret.name(), "String");
                assert_eq!(
                    params[0],
                    ParameterType::thread_handle(
                        true,
                        ParameterType::Integer,
                        ParameterType::named("fs.File"),
                        ParameterType::String,
                    )
                );
            }
            other => panic!("expected Func, got {other:?}"),
        }
    }

    #[test]
    fn map_entry_and_result_parse_into_variants_and_round_trip() {
        // The two shapes `monomorph::helpers::unify_type` handled on strings but `parse`
        // previously folded into `Named`. They now decompose structurally and round-trip
        // byte-exact.
        assert_eq!(
            ParameterType::parse("MapEntry OF String TO Integer"),
            ParameterType::map_entry_of(ParameterType::String, ParameterType::Integer),
        );
        assert_eq!(
            ParameterType::parse("Result OF Nothing"),
            ParameterType::result_of(ParameterType::Nothing),
        );
        assert_eq!(
            ParameterType::parse("Result OF List OF Integer"),
            ParameterType::result_of(ParameterType::list_of(ParameterType::Integer)),
        );
        for spelling in [
            "MapEntry OF String TO Integer",
            "MapEntry OF String TO List OF Integer",
            "Result OF Nothing",
            "Result OF Integer",
            "Result OF List OF String",
            "List OF MapEntry OF String TO Integer",
        ] {
            round_trip(spelling);
        }
    }

    #[test]
    fn res_collection_element_parses_into_variant() {
        // A `RES`-marked collection element becomes a `Res` wrapping the inner type
        // (historically the marker was stripped, losing it on a `name()` round-trip).
        assert_eq!(
            ParameterType::parse("RES fs.File"),
            ParameterType::res(ParameterType::named("fs.File"))
        );
        assert_eq!(
            ParameterType::parse("List OF RES fs.File"),
            ParameterType::list_of(ParameterType::res(ParameterType::named("fs.File")))
        );
        assert_eq!(
            ParameterType::parse("Map OF String TO RES fs.File"),
            ParameterType::map_of(
                ParameterType::String,
                ParameterType::res(ParameterType::named("fs.File")),
            )
        );
        // The `STATE` clause stays on the ELEMENT and the `RES` wraps it — the
        // contract this line has always asserted. plan-111-A gave that element a
        // variant, so it is a `Stateful` rather than an opaque nominal; the
        // clause's *position* is what matters here and it has not moved.
        // `stateful_round_trips_byte_exact` pins the rendering, and
        // `structural_split_state_agrees_with_the_string_grammar` pins that the
        // container still reports no top-level clause of its own (bug-429).
        assert_eq!(
            ParameterType::parse("List OF RES File STATE Cursor"),
            ParameterType::list_of(ParameterType::res(ParameterType::Stateful {
                base: Box::new(ParameterType::named("File")),
                state: Box::new(ParameterType::named("Cursor")),
            }))
        );
    }

    /// plan-106-A: `with_state` must be *exactly* the structural equivalent of
    /// parsing the concatenated spelling — that identity is what lets `ir::lower`
    /// build a stateful resource type without a render→parse round trip.
    #[test]
    fn with_state_matches_parse_of_the_concatenated_spelling() {
        let states = [
            ParameterType::named("Cursor"),
            ParameterType::Integer,
            ParameterType::named("pkg.FileInfo"),
        ];
        let bases = [
            // Leaves: nominal, qualified nominal, every scalar shape, and the
            // non-nominal leaves that must still fold into one opaque `Named`.
            ParameterType::named("File"),
            ParameterType::named("fs.File"),
            ParameterType::Integer,
            ParameterType::String,
            ParameterType::Boolean,
            ParameterType::Nothing,
            ParameterType::Unknown,
            ParameterType::var("T"),
            // Containers: the clause lands on the element / the VALUE half.
            ParameterType::list_of(ParameterType::named("File")),
            ParameterType::set_of(ParameterType::named("File")),
            ParameterType::result_of(ParameterType::named("File")),
            ParameterType::Res(Box::new(ParameterType::named("File"))),
            ParameterType::list_of(ParameterType::Res(Box::new(ParameterType::named("File")))),
            ParameterType::map_of(ParameterType::String, ParameterType::named("File")),
            ParameterType::map_entry_of(ParameterType::String, ParameterType::named("File")),
            ParameterType::list_of(ParameterType::list_of(ParameterType::named("File"))),
            // Func / thread handle / user generic: the return, the `out` plane,
            // and the final type argument respectively.
            ParameterType::Func(
                vec![ParameterType::Integer],
                Box::new(ParameterType::named("File")),
                false,
            ),
            ParameterType::Func(
                vec![ParameterType::Integer],
                Box::new(ParameterType::named("File")),
                true,
            ),
            ParameterType::ThreadHandle {
                worker: false,
                msg: Box::new(ParameterType::Integer),
                res: Box::new(ParameterType::Nothing),
                out: Box::new(ParameterType::named("File")),
            },
            ParameterType::user_of("Pair", vec![ParameterType::Integer, ParameterType::String]),
        ];
        let mut checked = 0usize;
        for base in &bases {
            for state in &states {
                let spelling = format!("{} STATE {}", base.name(), state.name());
                assert_eq!(
                    base.with_state(state),
                    ParameterType::parse(&spelling),
                    "with_state diverged from parse for `{spelling}`"
                );
                // …and the result still renders back to that same spelling.
                assert_eq!(base.with_state(state).name(), spelling);
                checked += 1;
            }
        }
        assert_eq!(checked, bases.len() * states.len());
    }

    /// plan-106-C: on a LEAF base — every place a resource's STATE is actually
    /// read — `split_state` undoes `with_state` exactly.
    #[test]
    fn split_state_is_the_inverse_of_with_state_on_leaves() {
        let states = [
            ParameterType::named("Cursor"),
            ParameterType::Integer,
            ParameterType::named("pkg.FileInfo"),
            // A STRUCTURED state — `fs.File STATE List OF Choice` is legal, and
            // wrapping it with `named` instead of parsing it is the exact bug
            // plan-106-B Correction 4 fixed.
            ParameterType::list_of(ParameterType::named("Choice")),
        ];
        let leaves = [
            ParameterType::named("File"),
            ParameterType::named("fs.File"),
            ParameterType::Integer,
            ParameterType::String,
            ParameterType::Unknown,
        ];
        let mut checked = 0usize;
        for base in &leaves {
            assert_eq!(
                base.split_state(),
                (base.clone(), None),
                "stateless leaf reported a STATE: {base:?}"
            );
            for state in &states {
                let attached = base.with_state(state);
                assert_eq!(
                    attached.split_state(),
                    (base.clone(), Some(state.clone())),
                    "split_state did not invert with_state for `{}` STATE `{}`",
                    base.name(),
                    state.name()
                );
                assert_eq!(attached.without_state(), *base);
                assert_eq!(attached.state().as_ref(), Some(state));
                checked += 1;
            }
        }
        assert_eq!(checked, leaves.len() * states.len());

        // A `Var` is the one leaf that cannot round-trip, for the same sanctioned
        // reason the descriptor guard records: a type variable renders as its bare
        // name and `parse` cannot know a name is a variable without the declaring
        // scope, so it comes back a `Named`. Harmless — a STATE is attached to a
        // concrete resource, never to a type variable — but asserted so the limit
        // is explicit rather than discovered later.
        let var_with_state = ParameterType::var("T").with_state(&ParameterType::named("Cursor"));
        assert_eq!(
            var_with_state.split_state(),
            (
                ParameterType::named("T"),
                Some(ParameterType::named("Cursor"))
            )
        );
    }

    /// `split_state` is **top-level only**, and this pins that it does NOT
    /// descend — the property bug-429 depends on.
    ///
    /// `with_state` attaches to the innermost rendered child, so on a composite
    /// the two are deliberately not inverses: `List OF RES File STATE Cursor`
    /// carries its clause on the *element*, and the top-level split must report
    /// `None` for it. Peeling it here would strip one side of a comparison while
    /// the other has nothing to strip, which is precisely the asymmetry that
    /// rejected a correct STATE-carrying resource union (bug-429) — and, when
    /// this accessor was first written descending, broke
    /// `bug427_list_union_state_rt` and `bug429_owned_list_union_drain_rt` with
    /// "expected List OF RES Handle, got List OF RES Handle STATE Cursor".
    #[test]
    fn split_state_is_top_level_only() {
        for spelling in [
            "List OF RES File STATE Cursor",
            "Set OF RES File STATE Cursor",
            "Result OF File STATE Cursor",
            "Map OF String TO RES File STATE Cursor",
            "RES File STATE Cursor",
        ] {
            let type_ = ParameterType::parse(spelling);
            assert_eq!(
                type_.split_state(),
                (type_.clone(), None),
                "`{spelling}` must not split — its STATE belongs to a nested type"
            );
            // Exactly what the name-domain helper this replaces answers.
            assert_eq!(crate::codegen::resource::state_type_name(spelling), None);
        }
    }

    /// `split_state` must agree with the name-domain helpers it replaces
    /// (`codegen::resource::base_resource_name` / `state_type_name`) on every
    /// spelling, including the composites above and the thread planes whose
    /// ` STATE ` belongs to the plane, not the handle (plan-54).
    #[test]
    fn split_state_matches_the_name_domain_helpers() {
        for spelling in [
            "File",
            "File STATE Cursor",
            "fs.File STATE Cursor",
            "Integer",
            "RES File STATE Cursor",
            "List OF RES File STATE Cursor",
            "Map OF String TO RES File STATE Cursor",
            "Thread OF RES fs.File STATE Cursor TO Integer",
            "Thread OF Integer RES fs.File STATE Cursor TO String",
        ] {
            let type_ = ParameterType::parse(spelling);
            let (base, state) = type_.split_state();
            assert_eq!(
                state.as_ref().map(|s| s.name()).as_deref(),
                crate::codegen::resource::state_type_name(spelling),
                "STATE disagreement on `{spelling}`"
            );
            assert_eq!(
                base.name(),
                crate::codegen::resource::base_resource_name(spelling),
                "base disagreement on `{spelling}`"
            );
            assert_eq!(type_.name(), spelling);
        }
    }

    /// plan-111-A Phase 2: from letter C onward a `ParameterType` is a map
    /// *key*, so `hash` must agree with `eq` — the `HashMap`/`HashSet`
    /// contract. The derive gives that by construction; this pins it, and pins
    /// that variants sharing a payload shape (`ListOf`/`SetOf`,
    /// `MapOf`/`MapEntryOf`, `Named`/`Var`, parent/worker `ThreadHandle`) stay
    /// *distinct* keys — a collision there would silently merge two types'
    /// entries in exactly the tables letter C re-keys.
    #[test]
    fn parameter_type_hash_agrees_with_eq() {
        use std::collections::hash_map::DefaultHasher;
        use std::collections::HashSet;
        use std::hash::{Hash, Hasher};

        fn digest(t: &ParameterType) -> u64 {
            let mut h = DefaultHasher::new();
            t.hash(&mut h);
            h.finish()
        }

        // One spelling per parseable variant, drawn from the round-trip corpora
        // above (`thread_handle_round_trips_byte_exact`,
        // `map_entry_and_result_parse_into_variants_and_round_trip`,
        // `res_collection_round_trips_byte_exact`).
        const CORPUS: &[&str] = &[
            "AttributeString",
            "Boolean",
            "Byte",
            "Integer",
            "Fixed",
            "Float",
            "Money",
            "Nothing",
            "String",
            "Unknown",
            "List OF Integer",
            "Set OF Integer",
            "Map OF String TO Integer",
            "MapEntry OF String TO Integer",
            "Result OF Integer",
            "Result OF List OF String",
            "RES fs.File",
            "RES File STATE Cursor",
            "List OF RES fs.File",
            "List OF RES File STATE Cursor",
            "Map OF RES fs.File TO Integer",
            "CsvReader",
            "Pair OF Integer, String",
            "FUNC(Integer) AS String",
            "ISOLATED FUNC(Integer) AS String",
            "Thread OF Integer TO String",
            "ThreadWorker OF Integer TO String",
            "Thread OF Integer RES fs.File STATE Cursor TO String",
            "List OF List OF RES fs.File",
        ];

        // 1. Equal types hash equal. Two independent parses of one spelling are
        //    `eq`, so they must be indistinguishable as keys.
        for spelling in CORPUS {
            let a = ParameterType::parse(spelling);
            let b = ParameterType::parse(spelling);
            assert_eq!(a, b, "parse is not deterministic for `{spelling}`");
            assert_eq!(
                digest(&a),
                digest(&b),
                "hash disagrees with eq for `{spelling}`"
            );
        }

        // 2. Distinct types are distinct keys, across every container variant.
        let mut set: HashSet<ParameterType> = HashSet::new();
        for spelling in CORPUS {
            assert!(
                set.insert(ParameterType::parse(spelling)),
                "`{spelling}` is not a distinct key — it collided with an \
                 earlier corpus entry"
            );
        }
        assert_eq!(set.len(), CORPUS.len());

        // 3. A key built independently still finds its entry — the property
        //    every re-keyed table in letter C relies on.
        for spelling in CORPUS {
            assert!(
                set.contains(&ParameterType::parse(spelling)),
                "lookup missed for `{spelling}`"
            );
        }

        // 4. Same payload, different variant: these must NOT be one key.
        //    `Var`/`Arg` are never produced by `parse`, so they are built here.
        let same_payload_distinct = [
            ParameterType::list_of(ParameterType::Integer),
            ParameterType::set_of(ParameterType::Integer),
            ParameterType::result_of(ParameterType::Integer),
            ParameterType::map_of(ParameterType::String, ParameterType::Integer),
            ParameterType::map_entry_of(ParameterType::String, ParameterType::Integer),
            ParameterType::named("T"),
            ParameterType::var("T"),
            ParameterType::Arg(0),
            ParameterType::Arg(1),
            ParameterType::func(vec![ParameterType::Integer], ParameterType::String),
            ParameterType::func_isolated(vec![ParameterType::Integer], ParameterType::String),
            ParameterType::thread_handle(
                false,
                ParameterType::Integer,
                ParameterType::Nothing,
                ParameterType::String,
            ),
            ParameterType::thread_handle(
                true,
                ParameterType::Integer,
                ParameterType::Nothing,
                ParameterType::String,
            ),
        ];
        let mut lookalikes: HashSet<ParameterType> = HashSet::new();
        for t in &same_payload_distinct {
            assert!(
                lookalikes.insert(t.clone()),
                "`{}` collided with a same-payload lookalike",
                t.name()
            );
        }
        assert_eq!(lookalikes.len(), same_payload_distinct.len());
    }

    /// plan-111-A Phase 3: `parse` builds a [`ParameterType::Stateful`] for a
    /// resource's OWN top-level ` STATE T`, and for nothing else. These are the
    /// nested-plane cases the deleted `split_state_clause` copies protected with
    /// their `base.contains(' ')` guard; the guard did not go away, it moved
    /// into `parse`.
    #[test]
    fn stateful_parses_top_level_only() {
        // The top-level case: a bare resource with its own clause.
        assert_eq!(
            ParameterType::parse("File STATE Cursor"),
            ParameterType::Stateful {
                base: Box::new(ParameterType::named("File")),
                state: Box::new(ParameterType::named("Cursor")),
            }
        );
        // A qualified base is still one token.
        assert_eq!(
            ParameterType::parse("fs.File STATE Cursor"),
            ParameterType::Stateful {
                base: Box::new(ParameterType::named("fs.File")),
                state: Box::new(ParameterType::named("Cursor")),
            }
        );
        // A composite STATE payload decomposes structurally too.
        assert_eq!(
            ParameterType::parse("File STATE List OF Integer"),
            ParameterType::Stateful {
                base: Box::new(ParameterType::named("File")),
                state: Box::new(ParameterType::list_of(ParameterType::Integer)),
            }
        );

        // Nested: the clause lands on the ELEMENT / the plane, never on the
        // container. Splitting these at the composite is the truncation bug-429
        // fixed (`ThreadWorker OF RES File STATE Cursor TO Integer` became
        // `ThreadWorker OF RES File`).
        let stateful_file = ParameterType::Stateful {
            base: Box::new(ParameterType::named("File")),
            state: Box::new(ParameterType::named("Cursor")),
        };
        assert_eq!(
            ParameterType::parse("RES File STATE Cursor"),
            ParameterType::res(stateful_file.clone())
        );
        assert_eq!(
            ParameterType::parse("List OF RES File STATE Cursor"),
            ParameterType::list_of(ParameterType::res(stateful_file.clone()))
        );
        assert_eq!(
            ParameterType::parse("Set OF RES File STATE Cursor"),
            ParameterType::set_of(ParameterType::res(stateful_file.clone()))
        );
        assert_eq!(
            ParameterType::parse("Map OF String TO RES File STATE Cursor"),
            ParameterType::map_of(
                ParameterType::String,
                ParameterType::res(stateful_file.clone())
            )
        );
        assert_eq!(
            ParameterType::parse("Result OF Stream STATE Pending"),
            ParameterType::result_of(ParameterType::Stateful {
                base: Box::new(ParameterType::named("Stream")),
                state: Box::new(ParameterType::named("Pending")),
            })
        );
        assert_eq!(
            ParameterType::parse("Thread OF RES fs.File STATE Cursor TO Integer"),
            ParameterType::thread_handle(
                false,
                ParameterType::Nothing,
                // A thread's resource plane stores the inner type directly;
                // `name()` re-adds the `RES ` marker when it renders the plane.
                ParameterType::Stateful {
                    base: Box::new(ParameterType::named("fs.File")),
                    state: Box::new(ParameterType::named("Cursor")),
                },
                ParameterType::Integer,
            )
        );
        // A user generic carries the clause on its LAST argument, which is where
        // the trailing text abuts (`with_state`'s `UserOf` arm agrees).
        assert_eq!(
            ParameterType::parse("Pair OF Integer, String STATE Cursor"),
            ParameterType::user_of(
                "Pair",
                vec![
                    ParameterType::Integer,
                    ParameterType::Stateful {
                        base: Box::new(ParameterType::String),
                        state: Box::new(ParameterType::named("Cursor")),
                    },
                ]
            )
        );

        // A base that is itself a phrase is NOT this type's own clause, so the
        // whole spelling stays one opaque nominal — byte-for-byte what the
        // pre-variant `parse` did with it.
        assert_eq!(
            ParameterType::parse("A B STATE C"),
            ParameterType::named("A B STATE C")
        );
        assert_eq!(ParameterType::parse("A B STATE C").split_state().1, None);
    }

    /// The structural [`ParameterType::split_state`] must answer exactly what
    /// the string rule answered, for every shape — that equivalence is the
    /// entire safety argument for replacing the render-and-re-split
    /// implementation with a variant match.
    #[test]
    fn structural_split_state_agrees_with_the_string_grammar() {
        for spelling in [
            "File",
            "Integer",
            "File STATE Cursor",
            "fs.File STATE Cursor",
            "File STATE List OF Integer",
            "RES File STATE Cursor",
            "List OF RES File STATE Cursor",
            "Set OF RES File STATE Cursor",
            "Map OF String TO RES File STATE Cursor",
            "Result OF Stream STATE Pending",
            "Thread OF RES fs.File STATE Cursor TO Integer",
            "ThreadWorker OF RES File STATE Cursor TO Integer",
            "Thread OF Integer RES fs.File STATE Cursor TO String",
            "Pair OF Integer, String STATE Cursor",
            "A B STATE C",
        ] {
            let type_ = ParameterType::parse(spelling);
            let (base, state) = type_.split_state();
            // The string rule, applied to the same spelling.
            let expected = split_state_clause(spelling);
            assert_eq!(
                state.as_ref().map(|s| s.name().into_owned()),
                expected.map(|(_, s)| s.to_string()),
                "STATE disagreement on `{spelling}`"
            );
            assert_eq!(
                base.name(),
                expected.map_or(spelling, |(b, _)| b),
                "base disagreement on `{spelling}`"
            );
        }
    }

    /// plan-111-A Phase 3: every stateful spelling in the tree survives
    /// `parse` → `name` byte-exact. The variant renders what the opaque `Named`
    /// rendered, which is what keeps the `.mfp` type table, the IR encoding and
    /// every type-naming diagnostic unchanged (plan-111-A §Compatibility).
    #[test]
    fn stateful_round_trips_byte_exact() {
        for spelling in [
            "File STATE Cursor",
            "fs.File STATE Cursor",
            "pkg.Name STATE S",
            "File STATE List OF Integer",
            "RES File STATE Cursor",
            "List OF RES File STATE Cursor",
            "Set OF RES File STATE Cursor",
            "Map OF String TO RES File STATE Cursor",
            "List OF List OF RES File STATE Cursor",
            "Result OF Stream STATE Pending",
            "Thread OF RES fs.File STATE Cursor TO Integer",
            "ThreadWorker OF RES File STATE Cursor TO Integer",
            "Thread OF Integer RES fs.File STATE Cursor TO String",
            "Pair OF Integer, String STATE Cursor",
            "FUNC(Integer) AS File STATE Cursor",
            // Not a top-level clause: still one opaque nominal, and it must
            // round-trip as one.
            "A B STATE C",
        ] {
            round_trip(spelling);
        }
    }

    /// plan-111-B: `contains_state` is defined as
    /// `name().contains(" STATE ")` and computed structurally. Pin the equality
    /// over every shape, including the ones where the clause is NESTED (which is
    /// where a naive `matches!(t, Stateful { .. })` would silently disagree) and
    /// the composite-base spelling `parse` declines to split.
    #[test]
    fn contains_state_agrees_with_the_rendered_name() {
        let mut checked = 0usize;
        for spelling in [
            "File",
            "Integer",
            "Unknown",
            "File STATE Cursor",
            "fs.File STATE Cursor",
            "File STATE List OF Integer",
            "RES File STATE Cursor",
            "List OF RES File STATE Cursor",
            "Set OF RES File STATE Cursor",
            "Map OF String TO RES File STATE Cursor",
            "Map OF RES File STATE Cursor TO Integer",
            "MapEntry OF String TO File STATE Cursor",
            "List OF List OF RES File STATE Cursor",
            "Result OF Stream STATE Pending",
            "Thread OF RES fs.File STATE Cursor TO Integer",
            "ThreadWorker OF RES File STATE Cursor TO Integer",
            "Thread OF Integer RES fs.File STATE Cursor TO String",
            "Pair OF Integer, String STATE Cursor",
            "FUNC(Integer) AS File STATE Cursor",
            "FUNC(File STATE Cursor) AS Integer",
            // `parse` declines to split a composite base, so this stays one
            // opaque `Named` — and must still read as stateful.
            "A B STATE C",
            "List OF A B STATE C",
            // Negatives that merely look close.
            "StateMachine",
            "List OF StateMachine",
            "Thread OF Integer TO String",
        ] {
            let type_ = ParameterType::parse(spelling);
            assert_eq!(
                type_.contains_state(),
                type_.name().contains(" STATE "),
                "contains_state disagrees with the rendered name for `{spelling}`"
            );
            checked += 1;
        }
        assert_eq!(checked, 25);
        // Var and Arg are never produced by `parse`; neither can carry a clause.
        assert!(!ParameterType::var("T").contains_state());
        assert!(!ParameterType::Arg(0).contains_state());
        assert!(!ParameterType::Unknown.contains_state());
    }

    #[test]
    fn res_collection_round_trips_byte_exact() {
        // The whole point of the `Res` variant: these spellings round-tripped LOSSILY
        // before (the `RES ` marker was dropped), which is why `resolve_call` had to
        // echo the raw argument string. Now `parse`→`name` is byte-exact.
        for spelling in [
            "RES fs.File",
            "RES File STATE Cursor",
            "List OF RES fs.File",
            "List OF RES File STATE Cursor",
            "Set OF RES fs.File",
            "Map OF String TO RES fs.File",
            "Map OF RES fs.File TO Integer",
            "List OF List OF RES fs.File",
            // bug-463: a `RES` collection nested inside a thread plane. Same caveat
            // as in `thread_handle_round_trips_byte_exact` — round-tripping is
            // necessary but NOT sufficient; these passed with the bug present.
            "Thread OF List OF RES fs.File TO Integer",
            "Map OF Thread OF List OF RES fs.File TO Integer TO Integer",
        ] {
            round_trip(spelling);
        }
    }

    /// bug-463: a `RES`-marked collection on a thread's MESSAGE plane must decompose
    /// structurally, exactly as it already does on the OUTPUT plane.
    ///
    /// The output plane is not measured — `split_thread_types` takes "everything after
    /// the last ` TO `" — but the message plane must be measured to find where it ends,
    /// and `type_prefix_len` had no `RES ` arm. It therefore stopped inside the element
    /// (`List OF RES` = 11 bytes), the ` TO ` split failed, and the ENTIRE spelling
    /// survived as one opaque `Named` through the whole front end: the
    /// thread-sendability rule that governs the program (`ir::verify::resources`,
    /// `ParameterType::Res(_) => false`) never ran, and the resolver rejected the junk
    /// name for an unrelated reason instead.
    ///
    /// This asserts the parsed VARIANT, not the round trip, because a round trip cannot
    /// see this class of bug at all: `Named(s).name() == s` echoes the garbage back.
    #[test]
    fn res_collection_on_thread_plane_parses_structurally() {
        for spelling in [
            "Thread OF List OF RES fs.File TO Integer",
            "Thread OF Map OF String TO RES fs.File TO Integer",
            "ThreadWorker OF List OF RES fs.File TO Integer",
            "Thread OF List OF RES fs.File STATE Cursor TO Integer",
            "Thread OF List OF RES fs.File RES fs.File TO Integer",
            "Thread OF Map OF RES fs.File TO Integer TO Integer",
            "Thread OF Set OF RES fs.File TO Integer",
            // The contrast cases, which already worked: they must keep working.
            "Thread OF List OF Integer TO Integer",
            "Thread OF Integer RES fs.File TO Integer",
            "Thread OF Integer TO List OF RES fs.File",
        ] {
            assert!(
                matches!(
                    ParameterType::parse(spelling),
                    ParameterType::ThreadHandle { .. }
                ),
                "`{spelling}` did not parse into ThreadHandle; got {:?}",
                ParameterType::parse(spelling),
            );
        }
        // The message plane is intact, not merely non-`Named`.
        assert_eq!(
            ParameterType::parse("Thread OF List OF RES fs.File TO Integer"),
            ParameterType::thread_handle(
                false,
                ParameterType::list_of(ParameterType::res(ParameterType::named("fs.File"))),
                ParameterType::Nothing,
                ParameterType::Integer,
            ),
        );
        // A nested thread type inside a `Map` key is measured by the same helper.
        assert_eq!(
            ParameterType::parse("Map OF Thread OF List OF RES fs.File TO Integer TO Integer"),
            ParameterType::map_of(
                ParameterType::thread_handle(
                    false,
                    ParameterType::list_of(ParameterType::res(ParameterType::named("fs.File"))),
                    ParameterType::Nothing,
                    ParameterType::Integer,
                ),
                ParameterType::Integer,
            ),
        );
    }

    /// bug-463 (widened): `type_prefix_len` must measure every constructor
    /// [`ParameterType::parse`] can decompose. It is the ONLY thing standing between a
    /// nested position — a thread message plane, a `Map` key, a generic argument — and
    /// a silent collapse to an opaque `Named`.
    ///
    /// Beyond the `RES` marker the bug report documented, the measurement was also
    /// missing `Set`, `FUNC(...)`/`ISOLATED FUNC(...)` and user generics. Those gaps
    /// were worse than the `RES` one: `Thread OF Set OF Integer TO Integer` is a
    /// perfectly sendable, VALID program, and it was rejected outright with
    /// `SYMBOL_UNKNOWN_TYPE`.
    ///
    /// The property asserted is the one that makes a measurement correct: a spelling
    /// that IS one complete type measures to its own full length.
    #[test]
    fn type_prefix_len_measures_every_parse_constructor() {
        for spelling in [
            // Leaves and the `(T)` group.
            "Integer",
            "fs.File",
            "(Integer)",
            "(Map OF String TO Integer)",
            // The `RES` ownership marker, at top level and nested, with `STATE`.
            "RES fs.File",
            "RES File STATE Cursor",
            "List OF RES fs.File",
            "List OF RES File STATE Cursor",
            "Set OF RES fs.File",
            "Map OF String TO RES fs.File",
            "Map OF RES fs.File TO Integer",
            "Result OF RES fs.File",
            "MapEntry OF String TO RES fs.File",
            // `Set`, which the measurement never descended into at all.
            "Set OF Integer",
            "Set OF List OF Integer",
            "List OF Set OF Integer",
            "Map OF Set OF Integer TO Integer",
            // Function types.
            "FUNC(Integer) AS String",
            "FUNC() AS Nothing",
            "FUNC(FUNC(Integer) AS String, Integer) AS Boolean",
            "ISOLATED FUNC(Integer) AS String",
            "List OF FUNC(Integer) AS String",
            // User generics, including a nested one and a generic argument list.
            "Pair OF Integer, String",
            "Holder OF Integer",
            "Holder OF Pair OF Integer, String",
            "Pair OF Integer, Map OF String TO Integer",
            "List OF Pair OF Integer, String",
            // The builtin containers and threads, which already worked.
            "List OF Integer",
            "Map OF String TO Integer",
            "MapEntry OF String TO Integer",
            "Result OF Integer",
            "Thread OF Integer TO String",
            "Thread OF RES fs.File STATE Cursor TO Integer",
            "Thread OF List OF RES fs.File TO Integer",
            "ThreadWorker OF Set OF Integer RES fs.File TO Integer",
        ] {
            assert_eq!(
                type_prefix_len(spelling),
                Some(spelling.len()),
                "`{spelling}` is one complete type but did not measure to its own length"
            );
        }

        // A measured prefix stops exactly where the type ends — the property the thread
        // message plane and the `Map` key depend on. Each pair is (input, its prefix).
        for (input, prefix) in [
            ("List OF RES fs.File TO Integer", "List OF RES fs.File"),
            ("Set OF Integer TO Integer", "Set OF Integer"),
            ("FUNC(Integer) AS String TO Integer", "FUNC(Integer) AS String"),
            (
                "Pair OF Integer, String TO Integer",
                "Pair OF Integer, String",
            ),
            ("RES File STATE Cursor TO Integer", "RES File STATE Cursor"),
            (
                "Map OF RES fs.File TO Integer TO Integer",
                "Map OF RES fs.File TO Integer",
            ),
        ] {
            assert_eq!(
                type_prefix_len(input),
                Some(prefix.len()),
                "measuring `{input}` should stop after `{prefix}`"
            );
        }
    }

    /// bug-463: the thread message plane decomposes for every constructor, so the rule
    /// that governs the program is the one that gets to reject (or accept) it — not an
    /// accident of spelling in the resolver.
    #[test]
    fn thread_message_plane_measures_every_parse_constructor() {
        for spelling in [
            "Thread OF Set OF Integer TO Integer",
            "Thread OF List OF Set OF Integer TO Integer",
            "Thread OF Map OF Set OF Integer TO Integer TO Integer",
            "Thread OF FUNC(Integer) AS String TO Integer",
            "Thread OF ISOLATED FUNC(Integer) AS String TO Integer",
            "Thread OF Pair OF Integer, String TO Integer",
        ] {
            assert!(
                matches!(
                    ParameterType::parse(spelling),
                    ParameterType::ThreadHandle { .. }
                ),
                "`{spelling}` did not parse into ThreadHandle; got {:?}",
                ParameterType::parse(spelling),
            );
        }
        assert_eq!(
            ParameterType::parse("Thread OF Set OF Integer TO Integer"),
            ParameterType::thread_handle(
                false,
                ParameterType::set_of(ParameterType::Integer),
                ParameterType::Nothing,
                ParameterType::Integer,
            ),
        );
        assert_eq!(
            ParameterType::parse("Thread OF FUNC(Integer) AS String TO Integer"),
            ParameterType::thread_handle(
                false,
                ParameterType::func(vec![ParameterType::Integer], ParameterType::String),
                ParameterType::Nothing,
                ParameterType::Integer,
            ),
        );
        assert_eq!(
            ParameterType::parse("Thread OF Pair OF Integer, String TO Integer"),
            ParameterType::thread_handle(
                false,
                ParameterType::user_of(
                    "Pair",
                    vec![ParameterType::Integer, ParameterType::String],
                ),
                ParameterType::Nothing,
                ParameterType::Integer,
            ),
        );
    }
}
