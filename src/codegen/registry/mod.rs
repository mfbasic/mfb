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

/// One field of a [`RegistryRecord`], e.g. `value AS Float`.
#[derive(Clone, Debug)]
pub(crate) struct RecordProp {
    /// The field name (`value`).
    pub(crate) name: &'static str,
    /// The field's type name (`Float`, `List OF Byte`, `net.Url`, …).
    pub(crate) ty: &'static str,
    /// One-line documentation of the field. Retained for doc generation; **not**
    /// rendered into the `TYPE` declaration [`RegistryPackage::get_mfb`] emits (the
    /// declaration is bare `name AS type`, as in a hand-written companion).
    pub(crate) description: &'static str,
}

/// A package value record — an `[EXPORT] TYPE Name … END TYPE` declaration, e.g.
///
/// ```text
/// EXPORT TYPE JsonNum
///   value AS Float
/// END TYPE
/// ```
///
/// Fields are private — construct via [`RegistryPackage::add_record`].
#[derive(Debug)]
pub(crate) struct RegistryRecord {
    name: &'static str,
    export: bool,
    props: Vec<RecordProp>,
}

impl RegistryRecord {
    /// The record's type name (`JsonNum`).
    pub(crate) fn name(&self) -> &'static str {
        self.name
    }
    /// Whether the record is `EXPORT`ed (visible to importers) or package-internal.
    pub(crate) fn is_exported(&self) -> bool {
        self.export
    }
    /// The record's fields, in declaration order.
    pub(crate) fn props(&self) -> &[RecordProp] {
        &self.props
    }

    /// Render the `[EXPORT] TYPE … END TYPE` declaration (no trailing newline).
    fn render(&self) -> String {
        let mut out = String::new();
        if self.export {
            out.push_str("EXPORT ");
        }
        out.push_str("TYPE ");
        out.push_str(self.name);
        for prop in &self.props {
            out.push_str("\n  ");
            out.push_str(prop.name);
            out.push_str(" AS ");
            out.push_str(prop.ty);
        }
        out.push_str("\nEND TYPE");
        out
    }
}

/// One variant of a [`RegistryUnion`] — a reference, by name, to a member type
/// (`JsonNum`, `JsonStr`, …).
#[derive(Clone, Debug)]
pub(crate) struct UnionVariant {
    /// The variant's type name (must name a record/type the package declares).
    pub(crate) name: &'static str,
    /// One-line documentation of the variant. Retained for doc generation; **not**
    /// rendered into the `UNION` declaration [`RegistryPackage::get_mfb`] emits (the
    /// declaration is a bare list of variant names, as in a hand-written companion).
    pub(crate) description: &'static str,
}

/// A package tagged-union type — an `[EXPORT] UNION Name … END UNION` declaration,
/// e.g.
///
/// ```text
/// EXPORT UNION Json
///   JsonNull
///   JsonBool
///   JsonNum
/// END UNION
/// ```
///
/// (The `UNION … INCLUDES Base` extension form is unused by builtin packages, so it
/// is intentionally not modeled; add an `Option` field if a builtin ever needs it.)
/// Fields are private — construct via [`RegistryPackage::add_union`].
#[derive(Debug)]
pub(crate) struct RegistryUnion {
    name: &'static str,
    export: bool,
    variants: Vec<UnionVariant>,
}

impl RegistryUnion {
    /// The union's type name (`Json`).
    pub(crate) fn name(&self) -> &'static str {
        self.name
    }
    /// Whether the union is `EXPORT`ed (visible to importers) or package-internal.
    pub(crate) fn is_exported(&self) -> bool {
        self.export
    }
    /// The union's variants, in declaration order.
    pub(crate) fn variants(&self) -> &[UnionVariant] {
        &self.variants
    }

    /// Render the `[EXPORT] UNION … END UNION` declaration (no trailing newline).
    fn render(&self) -> String {
        let mut out = String::new();
        if self.export {
            out.push_str("EXPORT ");
        }
        out.push_str("UNION ");
        out.push_str(self.name);
        for variant in &self.variants {
            out.push_str("\n  ");
            out.push_str(variant.name);
        }
        out.push_str("\nEND UNION");
        out
    }
}

/// One builtin package: its import name, documentation, the packages it imports, its
/// records, unions, and functions. Fields are private — construct via
/// [`Registry::add_package`] and fill with [`RegistryPackage::add_imports`] /
/// [`RegistryPackage::add_record`] / [`RegistryPackage::add_union`] /
/// [`RegistryPackage::add_function`].
#[derive(Debug)]
pub(crate) struct RegistryPackage {
    import_name: &'static str,
    intro: &'static str,
    desc: &'static str,
    imports: Vec<&'static str>,
    records: Vec<RegistryRecord>,
    unions: Vec<RegistryUnion>,
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

    /// The packages this one imports, in the order they were added.
    pub(crate) fn imports(&self) -> &[&'static str] {
        &self.imports
    }

    /// The package's records, in declaration order.
    pub(crate) fn records(&self) -> &[RegistryRecord] {
        &self.records
    }

    /// The package's unions, in declaration order.
    pub(crate) fn unions(&self) -> &[RegistryUnion] {
        &self.unions
    }

    /// Assemble this package's complete injectable MFBASIC source — the modern
    /// equivalent of a hand-written `package.mfb` (e.g.
    /// `codegen/builtins/csv/package.mfb`), reconstructed from the package's records
    /// and the members' own [`Body::Mfb`] bodies.
    ///
    /// The output is, in order:
    ///
    /// 1. the package's [`imports`](Self::imports), as a leading block of
    ///    `IMPORT <name>` lines (imports must precede all declarations in MFBASIC);
    /// 2. every [`RegistryRecord`] as an `[EXPORT] TYPE … END TYPE` block (in
    ///    `add_record` order);
    /// 3. every [`RegistryUnion`] as an `[EXPORT] UNION … END UNION` block (in
    ///    `add_union` order);
    /// 4. the optional `helper_functions` — the shared `__pkg_*` helper functions the
    ///    members call, which `get_mfb` does not synthesize;
    /// 5. every [`Body::Mfb`] body across all functions (each overload counts — a
    ///    same-named native-overload set like `encoding::utf8Encode` emits one `FUNC`
    ///    per overload), in registration order.
    ///
    /// Bodies keep their raw `__pkg_name` spelling; internalization to `#pkg_name`
    /// happens later when the assembled text is parsed.
    ///
    /// Returns the **empty string** only when the package has *nothing* to inject —
    /// no records, no unions, and no `Mfb` member — even if imports or
    /// `helper_functions` are given, because then the imports and helpers support
    /// nothing. (Records and unions are injectable source in their own right: a
    /// package whose functions are all `Native`/`Rewrite` still emits its
    /// `TYPE`/`UNION` declarations here.) Pieces are separated by a blank line and the
    /// result ends with a newline, so the output is directly parseable.
    pub(crate) fn get_mfb(&self, helper_functions: Option<&str>) -> String {
        let bodies: Vec<&str> = self
            .functions
            .iter()
            .flat_map(|f| f.implementations.iter())
            .filter_map(|imp| match &imp.body {
                Body::Mfb { body, .. } => Some(body.trim_end()),
                _ => None,
            })
            .collect();
        if self.records.is_empty() && self.unions.is_empty() && bodies.is_empty() {
            return String::new();
        }

        let mut pieces: Vec<String> =
            Vec::with_capacity(1 + self.records.len() + self.unions.len() + 1 + bodies.len());
        if !self.imports.is_empty() {
            let imports = self
                .imports
                .iter()
                .map(|name| format!("IMPORT {name}"))
                .collect::<Vec<_>>()
                .join("\n");
            pieces.push(imports);
        }
        pieces.extend(self.records.iter().map(RegistryRecord::render));
        pieces.extend(self.unions.iter().map(RegistryUnion::render));
        if let Some(helper_functions) = helper_functions {
            let helper_functions = helper_functions.trim_end();
            if !helper_functions.is_empty() {
                pieces.push(helper_functions.to_string());
            }
        }
        pieces.extend(bodies.iter().map(|body| body.to_string()));

        let mut out = pieces.join("\n\n");
        out.push('\n');
        out
    }

    /// Add packages to this package's import list. Additive — later calls append —
    /// so a package may accumulate its imports across several calls. They render into
    /// [`get_mfb`](Self::get_mfb) as leading `IMPORT <name>` lines, before everything
    /// else, in the order added.
    pub(crate) fn add_imports(&mut self, imports: Vec<&'static str>) -> &mut Self {
        self.imports.extend(imports);
        self
    }

    /// Add a value record (`[EXPORT] TYPE … END TYPE`) to this package. `props` must
    /// be non-empty — a `TYPE` needs at least one field. Records render into
    /// [`get_mfb`](Self::get_mfb) in the order they are added, before the functions.
    pub(crate) fn add_record(
        &mut self,
        name: &'static str,
        export: bool,
        props: Vec<RecordProp>,
    ) -> &mut Self {
        debug_assert!(
            !props.is_empty(),
            "{}::{name}: a record needs at least one field",
            self.import_name,
        );
        self.records.push(RegistryRecord {
            name,
            export,
            props,
        });
        self
    }

    /// Add a tagged union (`[EXPORT] UNION … END UNION`) to this package. `variants`
    /// must be non-empty — a `UNION` needs at least one variant. Unions render into
    /// [`get_mfb`](Self::get_mfb) in the order they are added, between the records and
    /// the functions.
    pub(crate) fn add_union(
        &mut self,
        name: &'static str,
        export: bool,
        variants: Vec<UnionVariant>,
    ) -> &mut Self {
        debug_assert!(
            !variants.is_empty(),
            "{}::{name}: a union needs at least one variant",
            self.import_name,
        );
        self.unions.push(RegistryUnion {
            name,
            export,
            variants,
        });
        self
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
            imports: Vec::new(),
            records: Vec::new(),
            unions: Vec::new(),
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

    // Build a throwaway package with two Mfb members plus one non-Mfb member, to
    // exercise get_mfb's collection/joining without touching the example package.
    fn mfb_impl(body: &'static str) -> Implementation {
        Implementation {
            params: vec![],
            return_type: "String",
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::mfb(body),
        }
    }

    #[test]
    fn get_mfb_appends_member_bodies_to_the_prefix() {
        let mut r = Registry::new();
        let pkg = r.add_package("demo", "intro", "desc");
        pkg.add_function(
            "a",
            "i",
            "d",
            "e",
            vec![mfb_impl(
                "FUNC __demo_a() AS String\n  RETURN \"a\"\nEND FUNC",
            )],
        );
        // A non-Mfb member contributes no source.
        pkg.add_function(
            "b",
            "i",
            "d",
            "e",
            vec![Implementation {
                params: vec![],
                return_type: "String",
                errors: vec![],
                lowering: Lowering::Helper,
                body: Body::Rewrite("__demo_b"),
            }],
        );
        pkg.add_function(
            "c",
            "i",
            "d",
            "e",
            vec![mfb_impl(
                "FUNC __demo_c() AS String\n  RETURN \"c\"\nEND FUNC",
            )],
        );

        let pkg = r.get_package("demo").expect("demo package");

        // With helper functions: the shared helper first, then both Mfb bodies (a, c).
        let src = pkg.get_mfb(Some("FUNC __demo_helper() AS Nothing\nEND FUNC"));
        assert_eq!(
            src,
            "FUNC __demo_helper() AS Nothing\nEND FUNC\n\n\
             FUNC __demo_a() AS String\n  RETURN \"a\"\nEND FUNC\n\n\
             FUNC __demo_c() AS String\n  RETURN \"c\"\nEND FUNC\n",
        );
        // The Rewrite member 'b' contributed nothing.
        assert!(!src.contains("__demo_b"));

        // Without helper functions: just the bodies.
        let src = pkg.get_mfb(None);
        assert!(src.starts_with("FUNC __demo_a"));
        assert!(src.contains("FUNC __demo_c"));
        assert!(src.ends_with("END FUNC\n"));
    }

    #[test]
    fn get_mfb_is_empty_when_the_package_has_no_mfb_member() {
        // The example package is all Intrinsic/Rewrite and has no records — nothing
        // to inject.
        let pkg = registry().get_package("example").expect("example package");
        assert_eq!(
            pkg.get_mfb(Some("FUNC __helper() AS Nothing\nEND FUNC")),
            ""
        );
        assert_eq!(pkg.get_mfb(None), "");

        // Imports and helper functions are scaffolding: with no records/unions/Mfb
        // member to support, they render nothing.
        let mut r = Registry::new();
        let pkg = r.add_package("bare", "i", "d");
        pkg.add_imports(vec!["strings"]);
        let pkg = r.get_package("bare").expect("bare package");
        assert_eq!(
            pkg.get_mfb(Some("FUNC __helper() AS Nothing\nEND FUNC")),
            ""
        );
    }

    fn prop(name: &'static str, ty: &'static str) -> RecordProp {
        RecordProp {
            name,
            ty,
            description: "field doc",
        }
    }

    #[test]
    fn add_record_renders_the_type_declaration() {
        let mut r = Registry::new();
        let pkg = r.add_package("json", "intro", "desc");
        pkg.add_record("JsonNum", true, vec![prop("value", "Float")]);
        pkg.add_record(
            "Pair",
            false,
            vec![prop("key", "String"), prop("val", "Integer")],
        );

        let pkg = r.get_package("json").expect("json package");
        assert_eq!(pkg.records().len(), 2);
        assert!(pkg.records()[0].is_exported());
        assert!(!pkg.records()[1].is_exported());

        assert_eq!(
            pkg.records()[0].render(),
            "EXPORT TYPE JsonNum\n  value AS Float\nEND TYPE"
        );
        assert_eq!(
            pkg.records()[1].render(),
            "TYPE Pair\n  key AS String\n  val AS Integer\nEND TYPE"
        );
    }

    fn variant(name: &'static str) -> UnionVariant {
        UnionVariant {
            name,
            description: "variant doc",
        }
    }

    #[test]
    fn add_union_renders_the_union_declaration() {
        let mut r = Registry::new();
        let pkg = r.add_package("json", "intro", "desc");
        pkg.add_union(
            "Json",
            true,
            vec![variant("JsonNull"), variant("JsonNum"), variant("JsonStr")],
        );
        pkg.add_union("Internal", false, vec![variant("A")]);

        let pkg = r.get_package("json").expect("json package");
        assert_eq!(pkg.unions().len(), 2);
        assert!(pkg.unions()[0].is_exported());
        assert!(!pkg.unions()[1].is_exported());
        assert_eq!(
            pkg.unions()[0].render(),
            "EXPORT UNION Json\n  JsonNull\n  JsonNum\n  JsonStr\nEND UNION"
        );
        assert_eq!(pkg.unions()[1].render(), "UNION Internal\n  A\nEND UNION");
    }

    #[test]
    fn get_mfb_orders_imports_records_unions_helpers_functions() {
        let mut r = Registry::new();
        let pkg = r.add_package("json", "intro", "desc");
        // add_imports accumulates across calls.
        pkg.add_imports(vec!["collections"]);
        pkg.add_imports(vec!["strings"]);
        pkg.add_record("JsonNum", true, vec![prop("value", "Float")]);
        pkg.add_record("JsonBool", true, vec![prop("flag", "Boolean")]);
        pkg.add_union("Json", true, vec![variant("JsonNum"), variant("JsonBool")]);
        pkg.add_function(
            "render",
            "i",
            "d",
            "e",
            vec![mfb_impl(
                "FUNC __json_render() AS String\n  RETURN \"\"\nEND FUNC",
            )],
        );

        let pkg = r.get_package("json").expect("json package");
        assert_eq!(pkg.imports(), &["collections", "strings"]);
        // Full order: imports, records, unions, helper functions, then member bodies.
        assert_eq!(
            pkg.get_mfb(Some("FUNC __json_helper() AS Nothing\nEND FUNC")),
            "IMPORT collections\nIMPORT strings\n\n\
             EXPORT TYPE JsonNum\n  value AS Float\nEND TYPE\n\n\
             EXPORT TYPE JsonBool\n  flag AS Boolean\nEND TYPE\n\n\
             EXPORT UNION Json\n  JsonNum\n  JsonBool\nEND UNION\n\n\
             FUNC __json_helper() AS Nothing\nEND FUNC\n\n\
             FUNC __json_render() AS String\n  RETURN \"\"\nEND FUNC\n",
        );
    }

    #[test]
    fn get_mfb_emits_records_even_with_no_mfb_functions() {
        // A package with a record but only a Native/Rewrite function still injects
        // its TYPE declaration.
        let mut r = Registry::new();
        let pkg = r.add_package("shape", "intro", "desc");
        pkg.add_record("Point", true, vec![prop("x", "Float"), prop("y", "Float")]);
        pkg.add_function(
            "origin",
            "i",
            "d",
            "e",
            vec![Implementation {
                params: vec![],
                return_type: "Point",
                errors: vec![],
                lowering: Lowering::Helper,
                body: Body::Rewrite("__shape_origin"),
            }],
        );

        let pkg = r.get_package("shape").expect("shape package");
        assert_eq!(
            pkg.get_mfb(None),
            "EXPORT TYPE Point\n  x AS Float\n  y AS Float\nEND TYPE\n",
        );
    }
}
