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

use std::borrow::Cow;
use std::fmt;

/// A [`crate::codegen::registry::Parameter`]'s type. An enum rather than a bare
/// `&'static str` so future kinds (argument unions, generic placeholders) can be
/// added without touching every parameter. Mirrors
/// `target::shared::registry::ParameterType`.
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
    /// A concrete nominal type — a record, union, or user type named by the program.
    /// Matched by name (unlike [`Var`], which is bound). A descriptor names one with a
    /// static literal (`Named("CsvReader")`); a concrete nominal argument is built at
    /// the boundary by [`parse`](Self::parse).
    Named(&'static str),
    /// A bindable type variable in a generic signature (`T`, `K`, `V`) — always
    /// `&'static`, because variables are only ever *declared* in a static descriptor.
    /// It is *unified* against the concrete argument type at a call site (binding the
    /// variable) and then *substituted* into the return type: this is how the registry
    /// expresses `collections::get(List OF T, Integer) AS T` with no per-package
    /// resolver. Renders as its bare name in documentation.
    Var(&'static str),
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
    /// or function type) becomes a [`Named`] whose runtime name is interned to
    /// `&'static` — a deliberate leak, but only at this boundary. A leading `RES `
    /// ownership marker is stripped (a collection element stores the bare type).
    pub(crate) fn parse(name: &str) -> ParameterType {
        let name = name.strip_prefix("RES ").unwrap_or(name);
        if name == "Unknown" {
            return ParameterType::Unknown;
        }
        // A parametric thread type (`Thread OF Msg [RES Res] TO Out` /
        // `ThreadWorker OF ...`) decomposes structurally into a `ThreadHandle`, its
        // three slots recursively parsed — mirroring the `List`/`Map`/`FUNC` arms.
        // The resource plane defaults to `Nothing` when absent (elided by `name`).
        // A bare opaque `Thread` / `ThreadWorker` (no ` OF ` body) is not a handle
        // and falls through to `Named`.
        if let Some((kind, message, resource, output)) =
            crate::builtins::thread::thread_parts_full(name)
        {
            return ParameterType::thread_handle(
                kind == crate::builtins::thread::THREAD_WORKER_TYPE,
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
            // `FUNC(...)`). Mirrors `builtins::general::function_parts`.
            if let Some((params, ret)) = crate::builtins::split_func_params_and_return(rest) {
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
            other => ParameterType::Named(Box::leak(other.to_string().into_boxed_str())),
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
            ParameterType::Named(elem) => Cow::Borrowed(elem),
            ParameterType::Var(name) => Cow::Borrowed(name),
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
                    crate::builtins::thread::THREAD_WORKER_TYPE
                } else {
                    crate::builtins::thread::THREAD_TYPE
                };
                // The resource plane is elided when `Nothing`, exactly reproducing
                // `format_thread_type` (a data-only `Thread OF Msg TO Out`).
                let res_name = match res.as_ref() {
                    ParameterType::Nothing => None,
                    other => Some(other.name()),
                };
                Cow::Owned(crate::builtins::thread::format_thread_type(
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
                ParameterType::Named("fs.File"),
                ParameterType::String,
            )
        );
        assert_eq!(
            ParameterType::parse("Thread"),
            ParameterType::Named("Thread")
        );
        assert_eq!(
            ParameterType::parse("ThreadWorker"),
            ParameterType::Named("ThreadWorker")
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
                        ParameterType::Named("fs.File"),
                        ParameterType::String,
                    )
                );
            }
            other => panic!("expected Func, got {other:?}"),
        }
    }
}
