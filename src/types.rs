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
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// A concrete nominal type — a record, union, or user type named by the program.
    /// Matched by name (unlike [`Var`], which is bound). A descriptor names one with a
    /// static literal (`named("CsvReader")`); a concrete nominal argument is built at
    /// the boundary by [`parse`](Self::parse). The name is an interned [`Symbol`]
    /// (`Copy`, integer-compared) rather than a leaked `&'static str`.
    Named(Symbol),
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
        if let Some((key, value)) = name
            .strip_prefix("Map OF ")
            .and_then(|rest| rest.split_once(" TO "))
        {
            return ParameterType::map_of(ParameterType::parse(key), ParameterType::parse(value));
        }
        // `MapEntry OF K TO V` — the key/value pair shape, split on the first top-level
        // ` TO ` exactly as the `Map OF ` arm above (so the two spellings decompose
        // identically). `MapEntry OF …` does not start with `Map OF `, so the order
        // relative to that arm is immaterial.
        if let Some((key, value)) = name
            .strip_prefix("MapEntry OF ")
            .and_then(|rest| rest.split_once(" TO "))
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
            ParameterType::Named(elem) => Cow::Borrowed(elem.resolve()),
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
// syntaxcheck, codegen, binary_repr) speaks the same string vocabulary. This is the
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

/// Whether `name` spells a worker `ThreadWorker` handle type.
pub(crate) fn is_worker_thread_type(name: &str) -> bool {
    thread_parts(name).is_some_and(|(kind, _, _)| kind == THREAD_WORKER_TYPE)
}

/// The data-plane message type of a thread handle (`"Nothing"` for a resource-only
/// thread), or `None` for a non-thread type.
pub(crate) fn thread_message(name: &str) -> Option<&str> {
    thread_parts(name).map(|(_, message, _)| message)
}

/// The resource type carried on the thread's resource plane
/// (`thread::transfer`/`thread::accept`), or `None` for a data-only thread. A
/// data-only thread is spelled `Thread OF Msg TO Out`; the resource plane is the
/// optional `RES Res` clause: `Thread OF Msg RES Res TO Out` (or `Thread OF RES
/// Res TO Out` when there is no data channel).
pub(crate) fn thread_resource(name: &str) -> Option<&str> {
    thread_parts_full(name).and_then(|(_, _, resource, _)| resource)
}

/// Output type for `thread::waitFor`, which is only valid on a parent `Thread`
/// handle (not a `ThreadWorker`).
pub(crate) fn parent_thread_output(name: &str) -> Option<&str> {
    thread_parts(name).and_then(|(kind, _, output)| (kind == THREAD_TYPE).then_some(output))
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
        // The `STATE` clause stays inside the (opaque) nominal, the `RES` wraps it.
        assert_eq!(
            ParameterType::parse("List OF RES File STATE Cursor"),
            ParameterType::list_of(ParameterType::res(ParameterType::named(
                "File STATE Cursor"
            )))
        );
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
        ] {
            round_trip(spelling);
        }
    }
}
