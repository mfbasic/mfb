//! Clean-room builtin registry (north star — see `planning/todo.md`).
//!
//! This is a **standalone** replacement for the descriptor vocabulary in
//! `target::shared::registry`, built in parallel and deliberately NOT wired into
//! the existing pipeline. Nothing here answers the old query surface
//! (`REGISTRY.function`, `DefaultResolver`, …); builtin packages migrate onto this
//! shape one at a time, and the pipeline flips to it only once enough have moved.
//!
//! The shape it commits to — the whole reason it exists — is:
//!
//! ```text
//! Registry ─┬─ RegistryPackage ─┬─ RegistryFunction ── [Implementation, …]  (>= 1)
//! ```
//!
//! A function is **a name + docs + one-or-more fully-specified implementations**.
//! Each [`Implementation`] carries its *own* signature (params + return type), its
//! lowering, its realization ([`Body`]), and its errors — so overloading is just
//! "more than one implementation" and needs no `Custom`/resolver side-channel. This
//! is the endpoint the `overloads`-carries-`Implementation` and unified-
//! `implementations`-array notes in `planning/todo.md` step toward.
//!
//! Unlike `target::shared::registry` (a tree of `const`/`static` struct literals),
//! this registry is **built imperatively** — `add_package` / `add_function` — then
//! frozen once behind a `OnceLock` ([`registry`]). The imperative builder is the
//! point: a package registers itself in one readable block instead of a nested
//! struct literal plus a sidecar resolver.

use std::sync::OnceLock;

/// One parameter of an [`Implementation`]'s signature.
#[derive(Clone, Debug)]
pub(crate) struct Parameter {
    /// The canonical parameter name (as written in the source signature).
    pub(crate) name: &'static str,
    /// Accepted alternate spellings at a keyword-argument call site.
    pub(crate) aliases: &'static [&'static str],
    /// The parameter's type name, e.g. `"Integer"` or `"List OF Byte"`.
    pub(crate) ty: &'static str,
}

/// How an implementation reaches its code: a `bl` into a runtime helper, or emitted
/// in place at the call site. Per-implementation on purpose — the standing "is
/// `Lowering` redundant" question (`planning/todo.md`) is answered by making it a
/// property of each version, not of the whole name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lowering {
    /// Lowered as a call into a named runtime helper.
    Helper,
    /// Emitted inline at the call site.
    Inline,
}

/// How one implementation is *realized* in codegen. Data-only kinds for now; the
/// target-generic native and per-OS lowering-fn kinds (the `Native(NativeLower)` /
/// `Os { posix, win, all }` shapes in `target::shared::registry`) land here when the
/// first package that needs them migrates — they carry function pointers, so they
/// are added with real callers, never as empty arms.
///
/// Note on what is *not* a variant here: an MFBASIC body's optional **native fast
/// path** (`Implementation::Mfb.fast_path` in the old enum) is not its own kind. A
/// fast path is an accelerator for the *same* implementation, selected at monomorph
/// time by whether the instantiation qualifies (a computed axis, not the call's
/// arg/return signature) — so it cannot be a second element of the signature-
/// selected `implementations` array either. When a fast-path package (`zip`,
/// `findLastIndex`) migrates, [`Body::Mfb`] widens from a bare body to
/// `Mfb { body, fast_path: Option<..> }`; it does not gain an `MfbFastPath` variant.
#[derive(Clone, Debug)]
pub(crate) enum Body {
    /// An MFBASIC source body (`FUNC __pkg_name(...) ... END FUNC`) injected before
    /// monomorphization and mangled per signature (the `encoding::utf8Encode`
    /// native-overload pattern). Widens to `{ body, fast_path: Option<..> }` when a
    /// member carrying a native accelerator migrates (see the type-level note).
    Mfb(&'static str),
    /// A fixed internal rewrite target: the call becomes a call to this `__`-symbol.
    Rewrite(&'static str),
    /// A by-name intrinsic: an inline op with no rewrite and no source body (the
    /// `bits`/`math` shape).
    Intrinsic,
}

/// One version/overload of a function: its signature, how it lowers, how it is
/// realized, and the errors it can raise. A [`RegistryFunction`] holds `>= 1`.
#[derive(Clone, Debug)]
pub(crate) struct Implementation {
    /// This overload's parameters, in signature order.
    pub(crate) params: Vec<Parameter>,
    /// This overload's return type name (`"Nothing"` for a statement-like call).
    pub(crate) return_type: &'static str,
    /// The error codes this overload can raise (rule ids), or empty.
    pub(crate) errors: Vec<&'static str>,
    /// Whether this overload lowers via a helper `bl` or inline.
    pub(crate) lowering: Lowering,
    /// How this overload is realized in codegen.
    pub(crate) body: Body,
}

/// One function of a package: a public name, its documentation, and its
/// implementations. Fields are private so the registry is an API, not an open
/// record — construct via [`RegistryPackage::add_function`].
#[derive(Debug)]
pub(crate) struct RegistryFunction {
    name: &'static str,
    intro: &'static str,
    desc: &'static str,
    example: &'static str,
    implementations: Vec<Implementation>,
}

impl RegistryFunction {
    /// The function's public (unqualified) name, e.g. `"utf8Encode"`.
    pub(crate) fn name(&self) -> &'static str {
        self.name
    }
    /// One-line documentation intro.
    pub(crate) fn intro(&self) -> &'static str {
        self.intro
    }
    /// Full documentation description.
    pub(crate) fn desc(&self) -> &'static str {
        self.desc
    }
    /// A runnable documentation example.
    pub(crate) fn example(&self) -> &'static str {
        self.example
    }
    /// The function's implementations (`>= 1`); more than one means an overload set.
    pub(crate) fn implementations(&self) -> &[Implementation] {
        &self.implementations
    }
}

/// One builtin package: its import name, documentation, and functions. Fields are
/// private — construct via [`Registry::add_package`] and fill with
/// [`RegistryPackage::add_function`].
#[derive(Debug)]
pub(crate) struct RegistryPackage {
    import_name: &'static str,
    intro: &'static str,
    desc: &'static str,
    functions: Vec<RegistryFunction>,
}

impl RegistryPackage {
    /// The package's import name, e.g. `"encoding"`.
    pub(crate) fn import_name(&self) -> &'static str {
        self.import_name
    }
    /// One-line documentation intro.
    pub(crate) fn intro(&self) -> &'static str {
        self.intro
    }
    /// Full documentation description.
    pub(crate) fn desc(&self) -> &'static str {
        self.desc
    }
    /// The package's functions, in registration order.
    pub(crate) fn functions(&self) -> &[RegistryFunction] {
        &self.functions
    }
    /// The function with this public name, or `None`.
    pub(crate) fn function(&self, name: &str) -> Option<&RegistryFunction> {
        self.functions.iter().find(|f| f.name == name)
    }

    /// Add a function to this package. `implementations` must be non-empty — a
    /// function is a name plus at least one fully-specified implementation.
    pub(crate) fn add_function(
        &mut self,
        name: &'static str,
        intro: &'static str,
        desc: &'static str,
        example: &'static str,
        implementations: Vec<Implementation>,
    ) -> &mut Self {
        debug_assert!(
            !implementations.is_empty(),
            "{}::{name}: a function needs at least one implementation",
            self.import_name,
        );
        self.functions.push(RegistryFunction {
            name,
            intro,
            desc,
            example,
            implementations,
        });
        self
    }
}

/// The clean-room registry: an ordered set of packages. Built imperatively, then
/// frozen (see [`registry`]).
#[derive(Debug, Default)]
pub(crate) struct Registry {
    packages: Vec<RegistryPackage>,
}

impl Registry {
    /// An empty registry.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Add a package with no functions; returns it so functions can be added by
    /// chaining [`RegistryPackage::add_function`].
    pub(crate) fn add_package(
        &mut self,
        import_name: &'static str,
        intro: &'static str,
        desc: &'static str,
    ) -> &mut RegistryPackage {
        self.packages.push(RegistryPackage {
            import_name,
            intro,
            desc,
            functions: Vec::new(),
        });
        self.packages.last_mut().expect("just pushed a package")
    }

    /// All packages, in registration order.
    pub(crate) fn packages(&self) -> &[RegistryPackage] {
        &self.packages
    }

    /// The package with this import name, or `None`.
    pub(crate) fn get_package(&self, import_name: &str) -> Option<&RegistryPackage> {
        self.packages.iter().find(|p| p.import_name == import_name)
    }
}

/// The process-wide clean-room registry, built once on first access.
///
/// Mirrors the `OnceLock` freeze idiom in `src/unicode/runtime_tables.rs`: the
/// imperative [`build`] runs once, and every caller thereafter shares the frozen
/// `&'static Registry`.
pub(crate) fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(build)
}

/// Construct the registry by registering every migrated package.
///
/// Empty of real packages today — they migrate over one at a time. The `example`
/// package below is an illustrative first entry that exercises the shape (a single-
/// implementation function and a two-implementation overload); it is deleted once a
/// real package lands here.
fn build() -> Registry {
    let mut r = Registry::new();
    register_example(&mut r);
    r
}

/// Illustrative first package — NOT a real builtin. Shows a single-implementation
/// function (`identity`) and a two-implementation parameter overload (`describe`),
/// the shape that makes a resolver unnecessary. Delete when a real package migrates.
fn register_example(r: &mut Registry) {
    let pkg = r.add_package(
        "example",
        "An illustrative clean-room package.",
        "Demonstrates the packages -> functions -> implementations shape; not a real builtin.",
    );

    pkg.add_function(
        "identity",
        "Return the argument unchanged.",
        "`example::identity(x)` returns `x`.",
        "example::identity(42)",
        vec![Implementation {
            params: vec![Parameter {
                name: "x",
                aliases: &[],
                ty: "Integer",
            }],
            return_type: "Integer",
            errors: vec![],
            lowering: Lowering::Inline,
            body: Body::Intrinsic,
        }],
    );

    pkg.add_function(
        "describe",
        "Describe a value as text.",
        "`example::describe(v)` renders an Integer or a String as text.",
        "example::describe(1)",
        vec![
            Implementation {
                params: vec![Parameter {
                    name: "v",
                    aliases: &[],
                    ty: "Integer",
                }],
                return_type: "String",
                errors: vec![],
                lowering: Lowering::Helper,
                body: Body::Rewrite("__example_describe_int"),
            },
            Implementation {
                params: vec![Parameter {
                    name: "v",
                    aliases: &[],
                    ty: "String",
                }],
                return_type: "String",
                errors: vec![],
                lowering: Lowering::Helper,
                body: Body::Rewrite("__example_describe_str"),
            },
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_registry_exposes_the_example_package() {
        let pkg = registry()
            .get_package("example")
            .expect("example package registered");
        assert_eq!(pkg.import_name(), "example");
        assert_eq!(pkg.functions().len(), 2);
        assert!(registry().get_package("nope").is_none());
    }

    #[test]
    fn a_single_implementation_function_has_one_implementation() {
        let identity = registry()
            .get_package("example")
            .and_then(|p| p.function("identity"))
            .expect("identity function");
        assert_eq!(identity.implementations().len(), 1);
        let only = &identity.implementations()[0];
        assert_eq!(only.return_type, "Integer");
        assert_eq!(only.lowering, Lowering::Inline);
        assert!(matches!(only.body, Body::Intrinsic));
    }

    #[test]
    fn an_overload_is_two_implementations_differing_by_parameter_type() {
        let describe = registry()
            .get_package("example")
            .and_then(|p| p.function("describe"))
            .expect("describe function");
        let impls = describe.implementations();
        assert_eq!(impls.len(), 2);
        // Same arity, one parameter, differing types — a parameter overload.
        assert_eq!(impls[0].params.len(), 1);
        assert_eq!(impls[0].params[0].ty, "Integer");
        assert_eq!(impls[1].params[0].ty, "String");
        assert!(matches!(
            impls[0].body,
            Body::Rewrite("__example_describe_int")
        ));
        assert!(matches!(
            impls[1].body,
            Body::Rewrite("__example_describe_str")
        ));
    }

    #[test]
    fn the_builder_grows_a_registry_from_empty() {
        let mut r = Registry::new();
        assert!(r.packages().is_empty());
        let pkg = r.add_package("t", "intro", "desc");
        pkg.add_function(
            "f",
            "intro",
            "desc",
            "example",
            vec![Implementation {
                params: vec![],
                return_type: "Nothing",
                errors: vec![],
                lowering: Lowering::Inline,
                body: Body::Intrinsic,
            }],
        );
        assert_eq!(r.packages().len(), 1);
        assert_eq!(r.get_package("t").unwrap().functions().len(), 1);
    }
}
