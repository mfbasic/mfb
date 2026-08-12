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

/// A member's target-generic native lowering — the plan-95
/// `target::shared::registry::NativeLower` shape: given the code builder and the
/// call's NIR args, emit the **call-site** sequence and return its result value.
/// Held by the `common` slot of [`Body::Native`] (the all-targets lowering).
pub(crate) type NativeLower =
    for<'a> fn(
        &mut crate::target::shared::code::CodeBuilder<'a>,
        &[crate::target::shared::nir::NirValue],
    ) -> Result<crate::target::shared::code::ValueResult, String>;

/// An OS-seam member's per-platform native emission — the
/// `target::shared::registry::OsLower` shape: given the runtime-call name, the
/// mangled `_mfb_rt_<pkg>_<call>_<target>` helper symbol, the platform imports, and
/// the target platform, emit that **runtime-helper body**. Held by the `posix` and
/// `win` slots of [`Body::Native`]. This is a genuinely different codegen shape from
/// [`NativeLower`] (a helper-body emitter, not a call-site value emitter) — which is
/// why the two slots keep distinct types rather than being force-unified.
pub(crate) type OsLower = fn(
    &str,
    &str,
    &std::collections::HashMap<String, String>,
    &dyn crate::target::shared::code::CodegenPlatform,
) -> crate::target::shared::code::HelperResult;

/// A [`Body::Mfb`] member's optional native **fast path** — the plan-95
/// `target::shared::registry::MfbFastPath` shape. Given the builder, the
/// `#pkg_<name>$<TypeArgs>` monomorph target, and the call args, it either lowers
/// natively (`Ok(Some(_))`) or **declines** (`Ok(None)`), in which case the caller
/// instantiates the `.mfb` `body` instead. Selected by whether the monomorph
/// instantiation qualifies (a computed axis), so it rides on the `Mfb` body rather
/// than being its own realization kind or a second overload.
pub(crate) type MfbFastPath =
    for<'a> fn(
        &mut crate::target::shared::code::CodeBuilder<'a>,
        &str,
        &[crate::target::shared::nir::NirValue],
    ) -> Result<Option<crate::target::shared::code::ValueResult>, String>;

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

/// How one implementation is *realized* in codegen.
///
/// The old enum's separate `Native(NativeLower)` (target-generic) and
/// `Os { posix, win, all }` (per-platform) kinds are **merged** into one
/// [`Body::Native`] carrying three optional per-target-family lowering slots. This
/// is the design the merge commits to: a member without OS differences fills only
/// `common`; an OS-seam member fills `posix`/`win`; a member with a shared fallback
/// plus overrides can fill all three.
///
/// A fast path is deliberately *not* its own variant: it is an accelerator for the
/// *same* [`Body::Mfb`] implementation, selected at monomorph time by whether the
/// instantiation qualifies (a computed axis, not the call's arg/return signature) —
/// so it cannot be a second element of the signature-selected `implementations`
/// array either. It rides on `Mfb` as [`MfbFastPath`].
#[derive(Clone, Debug)]
pub(crate) enum Body {
    /// An MFBASIC source body (`FUNC __pkg_name(...) ... END FUNC`) injected before
    /// monomorphization and mangled per signature (the `encoding::utf8Encode`
    /// native-overload pattern), plus an optional native `fast_path` that lowers a
    /// qualifying monomorph instantiation directly instead of instantiating `body`.
    /// Build with [`Body::mfb`] / [`Body::mfb_with_fast_path`].
    Mfb {
        body: &'static str,
        fast_path: Option<MfbFastPath>,
    },
    /// The member owns its native lowering, split by target family. `posix` is
    /// emitted only on POSIX targets and `win` only on Windows — both as per-platform
    /// runtime-helper bodies ([`OsLower`]) — while `common` is emitted on **all**
    /// targets as a call-site lowering ([`NativeLower`]); a target picks its family
    /// slot, falling back to `common`. **At least one slot must be `Some`** — enforced
    /// by [`Body::native`], the blessed constructor. (Merges the old `Native` + `Os`
    /// kinds, keeping each slot's original signature.)
    Native {
        posix: Option<OsLower>,
        win: Option<OsLower>,
        common: Option<NativeLower>,
    },
    /// A fixed internal rewrite target: the call becomes a call to this `__`-symbol.
    Rewrite(&'static str),
    /// A by-name intrinsic: an inline op with no rewrite and no source body (the
    /// `bits`/`math` shape).
    Intrinsic,
}

impl Body {
    /// An `Mfb` body with no native fast path.
    pub(crate) fn mfb(body: &'static str) -> Self {
        Body::Mfb {
            body,
            fast_path: None,
        }
    }

    /// An `Mfb` body carrying a native fast path (the `zip`/`findLastIndex` shape).
    pub(crate) fn mfb_with_fast_path(body: &'static str, fast_path: MfbFastPath) -> Self {
        Body::Mfb {
            body,
            fast_path: Some(fast_path),
        }
    }

    /// A `Native` lowering, split by target family (`posix`/`win` as [`OsLower`]
    /// helper bodies, `common` as a [`NativeLower`] call-site lowering). At least one
    /// of the three must be `Some` — a `Native` that lowers on no target is meaningless.
    pub(crate) fn native(
        posix: Option<OsLower>,
        win: Option<OsLower>,
        common: Option<NativeLower>,
    ) -> Self {
        debug_assert!(
            posix.is_some() || win.is_some() || common.is_some(),
            "Body::native requires at least one of posix/win/common to be Some",
        );
        Body::Native { posix, win, common }
    }
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

    // A lowering fn of the `NativeLower` signature (the `common` slot). Never invoked
    // here — it exists only to give the slot a real function pointer to hold.
    fn sample_lower<'a>(
        _b: &mut crate::target::shared::code::CodeBuilder<'a>,
        _args: &[crate::target::shared::nir::NirValue],
    ) -> Result<crate::target::shared::code::ValueResult, String> {
        Err("sample lowering (test fixture, not invoked)".to_string())
    }

    // A fn of the `OsLower` signature (the `posix`/`win` slots). Declines with an
    // Err so the fixture need not construct a HelperBody.
    fn sample_os_lower(
        _call: &str,
        _symbol: &str,
        _imports: &std::collections::HashMap<String, String>,
        _platform: &dyn crate::target::shared::code::CodegenPlatform,
    ) -> crate::target::shared::code::HelperResult {
        Err("sample OS lowering (test fixture, not invoked)".to_string())
    }

    // A fast-path fn of the `MfbFastPath` signature (declines by returning Ok(None)).
    fn sample_fast_path<'a>(
        _b: &mut crate::target::shared::code::CodeBuilder<'a>,
        _target: &str,
        _args: &[crate::target::shared::nir::NirValue],
    ) -> Result<Option<crate::target::shared::code::ValueResult>, String> {
        Ok(None)
    }

    #[test]
    fn mfb_body_carries_an_optional_fast_path() {
        assert!(matches!(
            Body::mfb("FUNC __x() ... END FUNC"),
            Body::Mfb {
                fast_path: None,
                ..
            }
        ));
        assert!(matches!(
            Body::mfb_with_fast_path("FUNC __x() ... END FUNC", sample_fast_path),
            Body::Mfb {
                fast_path: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn native_holds_three_per_family_slots() {
        // A common-only member (no OS differences).
        match Body::native(None, None, Some(sample_lower as NativeLower)) {
            Body::Native { posix, win, common } => {
                assert!(posix.is_none() && win.is_none() && common.is_some());
            }
            _ => panic!("expected Body::Native"),
        }
        // A per-OS member (posix + win as OsLower helper bodies, no common fallback).
        match Body::native(
            Some(sample_os_lower as OsLower),
            Some(sample_os_lower as OsLower),
            None,
        ) {
            Body::Native { posix, win, common } => {
                assert!(posix.is_some() && win.is_some() && common.is_none());
            }
            _ => panic!("expected Body::Native"),
        }
    }

    #[test]
    #[should_panic(expected = "at least one of posix/win/common")]
    fn native_with_no_slots_is_rejected() {
        let _ = Body::native(None, None, None);
    }
}
