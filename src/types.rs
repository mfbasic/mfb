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

/// The C ABI type vocabulary of a `LINK` binding's `ABI (...)` clause — the
/// names the marshaling backend acts on (`17_native-libraries.md`).
///
/// **Closed, and that is the point.** No user declaration can add one: the set
/// is exactly what `codegen::link::thunk` implements, so an exhaustive `match`
/// on this enum is a compile-time proof that every accepted name has a
/// marshaling arm. Before plan-113 that was a hand-maintained `&str` allow-list
/// kept in step with the backend by `ir::link::tests::ctype_list_is_exhaustive`.
/// It does **not** prove the arm *lowers*, which is a strictly stronger
/// property — `link_thunk::tests::every_known_ctype_lowers` still asserts that,
/// seeded from [`ALL`](Self::ALL).
///
/// **A CSTRUCT-named slot is NOT here.** `ABI (info SfFileInfo)` names a
/// `CSTRUCT` declared in the same `LINK` alias (`ir::verify::link`'s
/// `is_cstruct_slot`), and that namespace is open — it stays a
/// [`ParameterType::Named`]. That open nominal tail is why the vocabulary lives
/// as a [`ParameterType`] variant rather than as a standalone enum: a standalone
/// enum would need its own `Named` arm and become a second type grammar.
///
/// Three names are position-restricted, and the rules live with the predicates
/// that express them, not here: [`Void`](Self::Void) is return-only,
/// [`Str`](Self::Str) argument-only (`ir::link::abi_ctype_valid_as_argument` /
/// `abi_ctype_valid_as_return`), and [`Buffer`](Self::Buffer) is `OUT`-only with
/// a `BUFFER … SIZE` clause (`ir::link::check_buffer_slots`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CAbiType {
    Ptr,
    Str,
    Buffer,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Bool,
    Byte,
    Float,
    Double,
    Void,
}

impl CAbiType {
    /// The source spelling. Byte-exact: this renders into the `.ast`/`.ir`
    /// goldens and the IR binary's LINK section, so the strings do not move.
    pub(crate) fn name(self) -> &'static str {
        match self {
            CAbiType::Ptr => "CPtr",
            CAbiType::Str => "CString",
            CAbiType::Buffer => "CBuffer",
            CAbiType::Int8 => "CInt8",
            CAbiType::Int16 => "CInt16",
            CAbiType::Int32 => "CInt32",
            CAbiType::Int64 => "CInt64",
            CAbiType::UInt8 => "CUInt8",
            CAbiType::UInt16 => "CUInt16",
            CAbiType::UInt32 => "CUInt32",
            CAbiType::UInt64 => "CUInt64",
            CAbiType::Bool => "CBool",
            CAbiType::Byte => "CByte",
            CAbiType::Float => "CFloat",
            CAbiType::Double => "CDouble",
            CAbiType::Void => "CVoid",
        }
    }

    /// The inverse of [`name`](Self::name): `None` for anything that is not one
    /// of the 16 spellings, which is what makes a CSTRUCT name (or a typo) fall
    /// through to [`ParameterType::Named`] and be rejected by the checker that
    /// owns that position.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "CPtr" => CAbiType::Ptr,
            "CString" => CAbiType::Str,
            "CBuffer" => CAbiType::Buffer,
            "CInt8" => CAbiType::Int8,
            "CInt16" => CAbiType::Int16,
            "CInt32" => CAbiType::Int32,
            "CInt64" => CAbiType::Int64,
            "CUInt8" => CAbiType::UInt8,
            "CUInt16" => CAbiType::UInt16,
            "CUInt32" => CAbiType::UInt32,
            "CUInt64" => CAbiType::UInt64,
            "CBool" => CAbiType::Bool,
            "CByte" => CAbiType::Byte,
            "CFloat" => CAbiType::Float,
            "CDouble" => CAbiType::Double,
            "CVoid" => CAbiType::Void,
            _ => return None,
        })
    }

    /// Every variant, in declaration order — for tests that must enumerate the
    /// closed set (round-trip, exhaustiveness) without hand-copying it.
    #[cfg(test)]
    pub(crate) const ALL: [CAbiType; 16] = [
        CAbiType::Ptr,
        CAbiType::Str,
        CAbiType::Buffer,
        CAbiType::Int8,
        CAbiType::Int16,
        CAbiType::Int32,
        CAbiType::Int64,
        CAbiType::UInt8,
        CAbiType::UInt16,
        CAbiType::UInt32,
        CAbiType::UInt64,
        CAbiType::Bool,
        CAbiType::Byte,
        CAbiType::Float,
        CAbiType::Double,
        CAbiType::Void,
    ];
}

impl fmt::Display for CAbiType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

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
    /// A `#Result OF T` — a fallible call's success-typed result. A single-child
    /// structural type, mirroring `monomorph::helpers::unify_type`'s `#Result OF` arm,
    /// so the middle-end shape is representable without folding into an opaque
    /// [`Named`](Self::Named). Spelled [`RESULT_TYPE`] (`#Result`), a name no user
    /// identifier can spell (bug-945): a user `TYPE Result` is an ordinary type.
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
    /// A C ABI type from a `LINK` binding's `ABI (...)` clause — `CPtr`,
    /// `CInt32`, … (plan-113). Closed: see [`CAbiType`].
    ///
    /// **These spellings used to be [`Named`](Self::Named)**, and every
    /// `Named(_)` guard meaning "is this a nominal?" therefore changed answer
    /// when this variant landed — the same hazard
    /// [`Stateful`](Self::Stateful) cost a real bug for. The audit is recorded
    /// in `planning/completed/plan-113-ctype-in-parametertype.md` §Corrections.
    /// The two production guards that had to move are
    /// `resolver::is_c_abi_type` and `ir::verify::link`'s local copy, both of
    /// which enumerate a *narrower* list than the 16 and must NOT be rewritten
    /// as `matches!(t, C(_))`.
    ///
    /// A CSTRUCT-named ABI slot is deliberately **not** here — it is a
    /// [`Named`](Self::Named), because that namespace is open.
    C(CAbiType),
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

    /// This type with every NOMINAL renamed through `rename` — each
    /// [`Named`](Self::Named) and each [`UserOf`](Self::UserOf) head, at every
    /// depth (`List OF`, `Map OF`, `Result OF`, `RES`, `STATE`, `FUNC`, thread
    /// planes, user type arguments). `rename` answers `None` to keep a name.
    /// Type variables, C ABI types and scalars are never nominals here.
    ///
    /// bug-632: this is how an imported user package's own types get their
    /// package-qualified identity (`A` → `pkg.A`) wherever that package's
    /// declarations, signatures and IR are read.
    pub(crate) fn map_nominals(&self, rename: &impl Fn(&str) -> Option<String>) -> ParameterType {
        let map = |t: &ParameterType| Box::new(t.map_nominals(rename));
        match self {
            ParameterType::Named(sym) => match rename(sym.resolve()) {
                Some(renamed) => ParameterType::named(&renamed),
                None => self.clone(),
            },
            ParameterType::UserOf(head, args) => {
                let args = args.iter().map(|a| a.map_nominals(rename)).collect();
                match rename(head.resolve()) {
                    Some(renamed) => ParameterType::UserOf(Symbol::intern(&renamed), args),
                    None => ParameterType::UserOf(*head, args),
                }
            }
            ParameterType::ListOf(e) => ParameterType::ListOf(map(e)),
            ParameterType::SetOf(e) => ParameterType::SetOf(map(e)),
            ParameterType::ResultOf(e) => ParameterType::ResultOf(map(e)),
            ParameterType::Res(e) => ParameterType::Res(map(e)),
            ParameterType::MapOf(k, v) => ParameterType::MapOf(map(k), map(v)),
            ParameterType::MapEntryOf(k, v) => ParameterType::MapEntryOf(map(k), map(v)),
            ParameterType::Stateful { base, state } => ParameterType::Stateful {
                base: map(base),
                state: map(state),
            },
            ParameterType::Func(params, ret, isolated) => ParameterType::Func(
                params.iter().map(|p| p.map_nominals(rename)).collect(),
                map(ret),
                *isolated,
            ),
            ParameterType::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => ParameterType::ThreadHandle {
                worker: *worker,
                msg: map(msg),
                res: map(res),
                out: map(out),
            },
            ParameterType::AttributeString
            | ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Integer
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::String
            | ParameterType::C(_)
            | ParameterType::Var(_)
            | ParameterType::Arg(_)
            | ParameterType::Unknown => self.clone(),
        }
    }
    /// This type with every type variable ([`Var`](Self::Var)) renamed through
    /// `rename`, at every depth — the [`map_nominals`](Self::map_nominals) walk for
    /// variables. `rename` answers `None` to keep a variable.
    ///
    /// bug-913: two template declarations are the same overload when their
    /// shapes agree up to the names of their type parameters (`FUNC f OF T(x AS
    /// T)` and `FUNC f OF U(y AS U)`), so duplicate detection compares them with
    /// each parameter renamed to its position.
    pub(crate) fn rename_vars(&self, rename: &impl Fn(Symbol) -> Option<Symbol>) -> ParameterType {
        let map = |t: &ParameterType| Box::new(t.rename_vars(rename));
        match self {
            ParameterType::Var(sym) => ParameterType::Var(rename(*sym).unwrap_or(*sym)),
            ParameterType::UserOf(head, args) => {
                ParameterType::UserOf(*head, args.iter().map(|a| a.rename_vars(rename)).collect())
            }
            ParameterType::ListOf(e) => ParameterType::ListOf(map(e)),
            ParameterType::SetOf(e) => ParameterType::SetOf(map(e)),
            ParameterType::ResultOf(e) => ParameterType::ResultOf(map(e)),
            ParameterType::Res(e) => ParameterType::Res(map(e)),
            ParameterType::MapOf(k, v) => ParameterType::MapOf(map(k), map(v)),
            ParameterType::MapEntryOf(k, v) => ParameterType::MapEntryOf(map(k), map(v)),
            ParameterType::Stateful { base, state } => ParameterType::Stateful {
                base: map(base),
                state: map(state),
            },
            ParameterType::Func(params, ret, isolated) => ParameterType::Func(
                params.iter().map(|p| p.rename_vars(rename)).collect(),
                map(ret),
                *isolated,
            ),
            ParameterType::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => ParameterType::ThreadHandle {
                worker: *worker,
                msg: map(msg),
                res: map(res),
                out: map(out),
            },
            ParameterType::AttributeString
            | ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Integer
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::String
            | ParameterType::C(_)
            | ParameterType::Named(_)
            | ParameterType::Arg(_)
            | ParameterType::Unknown => self.clone(),
        }
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

    /// The [`CAbiType`] this type *is*, or `None` for everything else.
    ///
    /// The single containment accessor for the C vocabulary. There is
    /// deliberately no `is_c()` returning a bare `bool`: the two production
    /// reject-lists (`resolver::is_c_abi_type`, `ir::verify::link`'s local copy)
    /// are narrower than the 16 on purpose, and a broad predicate is exactly
    /// what would silently widen `NATIVE_CPTR_ESCAPE`. Ask for the variant and
    /// enumerate the ones you mean.
    pub(crate) fn c_abi(&self) -> Option<CAbiType> {
        match self {
            ParameterType::C(ctype) => Some(*ctype),
            _ => None,
        }
    }

    /// Whether this type is exactly the C ABI type `ctype` — the variant
    /// analogue of the `slot.ctype == "CInt32"` compares plan-113 replaced.
    /// Takes a specific variant, so it cannot be the mistake
    /// [`c_abi`](Self::c_abi)'s doc warns about.
    pub(crate) fn is_c_abi(&self, ctype: CAbiType) -> bool {
        self.c_abi() == Some(ctype)
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
            // plan-113: a C ABI spelling is a `C`, not a `Named`, but it is still
            // a bare token — so `TYPE Box OF CPtr` may declare a template
            // parameter that happens to spell one, and it must reclassify
            // exactly as any other leaf. Measured on the pre-plan-113 binary:
            // that program builds (`Wrote executable to ./build/p113probe.out`),
            // and without this arm the field `v AS CPtr` stops being a `Var`
            // and reports `SYMBOL_UNKNOWN_TYPE`. Pinned by
            // `with_vars_reclassifies_a_c_abi_spelling_used_as_a_template_param`.
            ParameterType::C(ctype) if type_params.iter().any(|param| param == ctype.name()) => {
                ParameterType::var(ctype.name())
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
        // parser emits verbatim (`ast/expr/type_names.rs`). Peel it HERE, so it is peeled at
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
        //
        // plan-156-E: a ` STATE X` after the output is the VIEW's — each view of a
        // thread is a resource record carrying its own STATE — so when the output
        // parses to its own top-level `Stateful`, the clause is lifted onto the
        // handle. (An output is data, and data carries no STATE.)
        if let Some((kind, message, resource, output)) = thread_parts_full(name) {
            let (output, state) = ParameterType::parse(output).split_state();
            let handle = ParameterType::thread_handle(
                kind == THREAD_WORKER_TYPE,
                ParameterType::parse(message),
                resource.map_or(ParameterType::Nothing, ParameterType::parse),
                output,
            );
            return match state {
                Some(state) => ParameterType::Stateful {
                    base: Box::new(handle),
                    state: Box::new(state),
                },
                None => handle,
            };
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
        // `#Result OF T` — a single success-typed child, mirroring `List OF `/`Set OF `.
        if let Some(success) = name
            .strip_prefix(RESULT_TYPE)
            .and_then(|rest| rest.strip_prefix(" OF "))
        {
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
        // A user generic (`Stack OF T`, `Pair OF (Integer, String)`). Ordered LAST
        // among the ` OF ` shapes so every built-in constructor above claims its own
        // spelling first; what reaches here is by definition a non-builtin head. A
        // multi-argument list is one parenthesized group (bug-833, which `name`
        // renders); the arguments split on its TOP-LEVEL commas — the one place
        // that rule lives. A single group with no comma is a grouped argument.
        if let Some((head, rest)) = Self::split_user_generic(name) {
            let mut args: Vec<String> = crate::codegen::builtins::split_top_level_commas(rest)
                .into_iter()
                .map(str::to_string)
                .collect();
            if let Some(group_len) = rest
                .starts_with('(')
                .then(|| type_prefix_len(rest))
                .flatten()
            {
                let parts =
                    crate::codegen::builtins::split_top_level_commas(&rest[1..group_len - 1]);
                if parts.len() > 1 {
                    // A trailing ` STATE T` abuts the final argument, exactly as
                    // `with_state` attaches it.
                    args = parts.into_iter().map(str::to_string).collect();
                    if let Some(last) = args.last_mut() {
                        last.push_str(&rest[group_len..]);
                    }
                }
            }
            return ParameterType::user_of(
                head,
                args.iter().map(|arg| ParameterType::parse(arg)).collect(),
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
        // The C ABI vocabulary (plan-113), tried in the bare-token tail BEFORE
        // the `Named` fallback so `parse("CInt32")` is `C(Int32)` while
        // `parse("SfFileInfo")` — a CSTRUCT name, an open namespace — is still
        // `Named`. Ordered after the scalar table below only in reading order:
        // the two sets are disjoint, so the relative order is immaterial.
        if let Some(ctype) = CAbiType::parse(name) {
            return ParameterType::C(ctype);
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
            "List" | "Set" | "Map" | "MapEntry" | RESULT_TYPE | THREAD_TYPE | THREAD_WORKER_TYPE
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
    /// `parse` builds a [`Stateful`](Self::Stateful) for a leaf whose own spelling
    /// is a single token (`File STATE Cursor`), and for a thread handle, whose
    /// trailing clause is its view's (plan-156-E). Inside a container the clause
    /// lands on the *element*: `parse("List OF File STATE Cursor")` is
    /// `ListOf(Stateful(File, Cursor))`, because the container arm strips its
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
            // plan-156-E: a thread type's clause is its view's, lifted onto the handle
            // exactly as `parse` lifts it.
            ParameterType::ThreadHandle { .. } => ParameterType::Stateful {
                base: Box::new(self.clone()),
                state: Box::new(state.clone()),
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

    /// Whether this type is the built-in value type `leaf` declared by
    /// `package`, under **either** spelling the compiler can be holding.
    ///
    /// bug-480 Phase 4b made a builtin value type's declared identity
    /// package-qualified — but only where the registry rewrites it.
    /// `Registry::qualify_value_type_references` rewrites *signature* types (a
    /// member's parameters and return), so `tcp::localAddress` returns
    /// `net.Address`. A record *field* type is qualified only when it names
    /// ANOTHER package's type (bug-484: `udp::Datagram`'s `from` is
    /// `net.Address`); a field naming its own package's type stays bare in the
    /// registry prop (`net::PingResult`'s `address` is `Address`). Both spellings
    /// therefore reach any nominal question asked about that record.
    ///
    /// A question about what SOURCE wrote is different: ask
    /// [`Self::is_builtin_qualified`], because a bare leaf there names a project
    /// type (plan-132 D1).
    ///
    /// Asking `is_named` with one spelling answers `false` for every value that
    /// arrived by the other route — silently, since a nominal miss is not a
    /// compile error. That is bug-483: the record-layout predicate stopped
    /// recognising `net.Address` and its readers dereferenced a pointer field as
    /// a block-relative offset, and the read-only-record rule stopped refusing a
    /// user-written `net::Address[…]`. Use this, not `is_named`, for any
    /// question about a built-in value type.
    pub(crate) fn is_builtin_named(&self, package: &str, leaf: &str) -> bool {
        let ParameterType::Named(sym) = self else {
            return false;
        };
        let name = sym.resolve();
        name == leaf
            || name
                .strip_prefix(package)
                .and_then(|rest| rest.strip_prefix('.'))
                == Some(leaf)
    }

    /// Whether this type is the built-in value type `leaf` declared by `package`
    /// under its package-qualified identity ONLY (`net.Address`, never `Address`).
    ///
    /// For a question about a type SOURCE named. Source cannot name an imported
    /// builtin type by its bare leaf — `AS Address` under `IMPORT net` is
    /// `SYMBOL_UNKNOWN_TYPE` — so a bare leaf at a constructor or a `WITH` is a
    /// project type that happens to share the name, and
    /// [`Self::is_builtin_named`] would capture it: a project's own
    /// `TYPE Address` was refused as compiler-owned (plan-132 D1).
    pub(crate) fn is_builtin_qualified(&self, package: &str, leaf: &str) -> bool {
        let ParameterType::Named(sym) = self else {
            return false;
        };
        sym.resolve()
            .strip_prefix(package)
            .and_then(|rest| rest.strip_prefix('.'))
            == Some(leaf)
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
            ParameterType::ResultOf(success) => {
                Cow::Owned(format!("{RESULT_TYPE} OF {}", success.name()))
            }
            ParameterType::Res(inner) => Cow::Owned(format!("RES {}", inner.name())),
            // Byte-identical to the spelling `parse` split, and to what
            // `with_state` produced as one opaque `Named` before the variant
            // existed — the `.mfp` type table, the IR encoding and every
            // diagnostic that embeds a type name all depend on this rendering
            // not moving (plan-111-A §Compatibility).
            ParameterType::Stateful { base, state } => {
                Cow::Owned(format!("{} STATE {}", base.name(), state.name()))
            }
            ParameterType::C(ctype) => Cow::Borrowed(ctype.name()),
            ParameterType::Named(elem) => Cow::Borrowed(elem.resolve()),
            // bug-833: more than one argument is a parenthesized list, the
            // spelling source writes, so a multi-argument template nests inside a
            // function type's parameters or another template's arguments with no
            // comma ambiguity.
            //
            // A ` STATE T` clause on the final argument is written after the
            // list, where source writes it (`Pair OF (Integer, File) STATE S`);
            // `parse` reads either placement as that same structure, the clause
            // abutting the final argument exactly as `with_state` attaches it.
            ParameterType::UserOf(name, args) if args.len() > 1 => {
                let (last, init) = args.split_last().expect("more than one argument");
                let (last, clause) = match last {
                    ParameterType::Stateful { base, state } => {
                        (base.name(), format!(" STATE {}", state.name()))
                    }
                    other => (other.name(), String::new()),
                };
                let mut rendered: Vec<Cow<'_, str>> = init.iter().map(|a| a.name()).collect();
                rendered.push(last);
                Cow::Owned(format!(
                    "{} OF ({}){clause}",
                    name.resolve(),
                    rendered.join(", ")
                ))
            }
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

    /// The type as MFBASIC source spells it: a package qualifier is `::`
    /// (`List OF RES canvas::Font`).
    ///
    /// [`name`](Self::name) is the internal spelling (`canvas.Font`) that
    /// [`parse`](Self::parse) round-trips and the `.ast`/`.ir` goldens carry; a dot
    /// in source is field access. Anything a user reads — a man page, injected
    /// companion source — uses `display()`, never `name()` (bug-605).
    pub(crate) fn display(&self) -> String {
        self.name().replace('.', "::")
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

/// The internal fallible-call result type's name ([`ParameterType::ResultOf`]
/// renders `#Result OF T`). The [`INTERNAL_SIGIL`](crate::internal_name::INTERNAL_SIGIL)
/// keeps it out of user space: no user identifier can spell it, so `Result` is an
/// ordinary user name (bug-945).
pub(crate) const RESULT_TYPE: &str = "#Result";
/// The internal result's success member, spelled with the same sigil so a user
/// `TYPE Ok` is an ordinary type (bug-945).
pub(crate) const RESULT_SUCCESS_MEMBER: &str = "#Ok";

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
///
/// A view's own `STATE` clause (`ThreadWorker OF … STATE W`, plan-156-E) wraps the
/// handle in `Stateful` and never changes which side it is, so it is peeled first
/// (bug-844: a stateful worker's `send`/`receive`/`transfer`/`accept` were lowered
/// as the parent's and every message went to the wrong queue).
pub(crate) fn is_worker_thread_handle(type_: &ParameterType) -> bool {
    matches!(
        type_.without_state(),
        ParameterType::ThreadHandle { worker: true, .. }
    )
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

/// Length consumed by a single type prefix at the start of `input`.
///
/// **This must recognise every constructor [`ParameterType::parse`] can decompose,
/// and its arms are deliberately kept in `parse`'s order.** It is the only thing
/// standing between a *measured* position — a thread message plane, a `Map` key, a
/// generic argument — and a silent collapse to an opaque [`Named`](ParameterType::Named):
/// a constructor missing here does not merely measure short, it makes the enclosing
/// type fail to split at all, and the whole spelling survives as one nominal through
/// the entire front end (bug-463). Nothing downstream reports that; the type simply
/// stops being a type.
///
/// The asymmetry that hides such a gap: a *trailing* position is not measured at all
/// (`split_thread_types` takes "everything after the last ` TO `"), so the same
/// spelling can work on a thread's output plane and fail on its message plane. Nor
/// can a `parse`↔`name` round trip see it, since `Named(s).name()` echoes `s` back
/// verbatim — the guard is `type_prefix_len_measures_every_parse_constructor`, which
/// asserts a complete type measures to its own length.
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

    // The `RES ` ownership marker (§15.6). `parse` wraps a `Res` around whatever
    // follows it at EVERY position, so the measurement has to consume it at every
    // position too — a `List`/`Map`/`Set` element, a `Map` key, a generic argument.
    // Delegating to `resource_element_len` is what picks up the optional trailing
    // ` STATE T` clause, and is the same measurement the thread resource plane
    // already used; that plane working while `List OF RES …` on the message plane
    // did not was the whole of bug-463.
    if let Some(after_res) = input.strip_prefix("RES ") {
        return resource_element_len(after_res).map(|len| "RES ".len() + len);
    }

    // `[ISOLATED ]FUNC(<params>) AS <return>`. The base-name scan below stops dead
    // at the `(`, so without this arm a function type measures as just `FUNC`.
    for prefix in ["ISOLATED FUNC(", "FUNC("] {
        if let Some(rest) = input.strip_prefix(prefix) {
            return func_body_len(rest).map(|len| prefix.len() + len);
        }
    }

    // `#Result OF T`: the internal result's sigil-spelled base is not an
    // identifier the base-name scan below accepts.
    if let Some(success) = input
        .strip_prefix(RESULT_TYPE)
        .and_then(|rest| rest.strip_prefix(" OF "))
    {
        return type_prefix_len(success).map(|len| RESULT_TYPE.len() + 4 + len);
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

    // `Set` shares this arm with `List` (and `#Result` above) because `parse` gives
    // all three a single element child. It was missing here, so `Thread OF Set OF Integer TO
    // Integer` — a VALID, thread-sendable program — was rejected outright
    // (`SYMBOL_UNKNOWN_TYPE`) rather than merely mis-diagnosed (bug-463).
    if matches!(base, "List" | "Set") {
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

    // A user generic (`Pair OF Integer, String`) — every builtin head is claimed
    // above, so a bare head with an ` OF ` body is one by definition, exactly as in
    // `ParameterType::split_user_generic`. Its arguments are the comma-separated
    // list `split_top_level_commas` splits, and the list ends at the first separator
    // that is not `", "` — which is what lets the enclosing `Thread OF Pair OF
    // Integer, String TO Integer` find its own ` TO `.
    let mut total = base_end + " OF ".len();
    let mut rest = after_of;
    loop {
        let arg_len = type_prefix_len(rest)?;
        total += arg_len;
        rest = rest.get(arg_len..)?;
        let Some(next) = rest.strip_prefix(", ") else {
            return Some(total);
        };
        total += ", ".len();
        rest = next;
    }
}

/// Length consumed by a function type's body — everything after the `FUNC(` prefix
/// of a `[ISOLATED ]FUNC(<params>) AS <return>` spelling, up to and including its
/// return type.
///
/// Mirrors `codegen::builtins::split_func_params_and_return` on the paren scan (the
/// close paren is the one at depth 0, so a parameter that is itself a function type
/// is kept whole) but differs on the return type deliberately: that helper takes
/// "everything after `) AS `", because `parse` is handed a string that is nothing
/// but the type. A *measurement* has to stop where the type stops, so the return is
/// measured, letting `FUNC(Integer) AS String TO Integer` on a thread message plane
/// yield the message `FUNC(Integer) AS String` and leave ` TO Integer` to the thread.
fn func_body_len(rest: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut close = None;
    for (index, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => {
                close = Some(index);
                break;
            }
            ')' => depth -= 1,
            _ => {}
        }
    }
    let close = close?;
    let returns = rest.get(close..)?.strip_prefix(") AS ")?;
    let return_len = type_prefix_len(returns)?;
    Some(close + ") AS ".len() + return_len)
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
mod tests;
