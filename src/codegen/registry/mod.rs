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

use std::borrow::Cow;
use std::fmt;
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

/// A [`Parameter`]'s type. An enum rather than a bare `&'static str` so future kinds
/// (argument unions, generic placeholders) can be added without touching every
/// parameter. Mirrors `target::shared::registry::ParameterType`.
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
    Named(&'static str),
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
        }
    }
}

impl fmt::Display for ParameterType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Whether a [`Parameter`] is required, or optional with a default — mirrors
/// `target::shared::registry::DefaultValue`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DefaultValue {
    /// A required parameter — no default.
    None,
    /// An optional parameter padded with `(type_name, expr)` when omitted — the
    /// caller injects the literal (csv's `delimiter`/`quote`/`newline`).
    Fill {
        type_name: ParameterType,
        expr: &'static str,
    },
    /// An optional parameter that widens arity but is NOT default-padded — the
    /// implementation selects a distinct body by argument count (datetime's trailing
    /// `zone`). Contributes to the arity range like `Fill`, but padding skips it.
    Optional,
}

/// One parameter of an [`Implementation`]'s signature.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Parameter {
    /// The canonical parameter name (as written in the source signature).
    pub(crate) name: &'static str,
    /// The description
    pub(crate) desc: &'static str,
    /// Accepted alternate spellings at a keyword-argument call site.
    pub(crate) aliases: &'static [&'static str],
    /// The parameter's type, e.g. `ParameterType::Integer`.
    pub(crate) ty: ParameterType,
    /// Whether the parameter is required or optional (with how it defaults).
    pub(crate) default: DefaultValue,
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
    /// monomorphization, plus the internal symbol a call to this member `rewrite`s to
    /// (the `FUNC` the body declares — e.g. `csv.readRow` → `__csv_next`, which the
    /// public name cannot derive) and an optional native `fast_path` that lowers a
    /// qualifying monomorph instantiation directly instead of instantiating `body`.
    /// Build with [`Body::mfb`] / [`Body::mfb_with_fast_path`].
    Mfb {
        body: &'static str,
        rewrite: &'static str,
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
    /// An `Mfb` body that a call `rewrite`s to, with no native fast path. `rewrite`
    /// is the internal symbol the body declares (`FUNC <rewrite>(…)`).
    pub(crate) fn mfb(body: &'static str, rewrite: &'static str) -> Self {
        debug_assert!(
            body.contains(rewrite),
            "Body::mfb: body does not declare its rewrite target `{rewrite}`",
        );
        Body::Mfb {
            body,
            rewrite,
            fast_path: None,
        }
    }

    /// An `Mfb` body carrying a native fast path (the `zip`/`findLastIndex` shape).
    pub(crate) fn mfb_with_fast_path(
        body: &'static str,
        rewrite: &'static str,
        fast_path: MfbFastPath,
    ) -> Self {
        debug_assert!(
            body.contains(rewrite),
            "Body::mfb_with_fast_path: body does not declare its rewrite target `{rewrite}`",
        );
        Body::Mfb {
            body,
            rewrite,
            fast_path: Some(fast_path),
        }
    }

    /// The internal symbol a call to this member rewrites to, or `None` when the
    /// member is not a rewrite (a `Native`/`Intrinsic` lowering). Unifies the two
    /// rewrite forms — `Rewrite`'s fixed symbol and `Mfb`'s body-declared one —
    /// replacing the old per-package `implementation_name`.
    pub(crate) fn rewrite_target(&self) -> Option<&'static str> {
        match self {
            Body::Rewrite(symbol) => Some(symbol),
            Body::Mfb { rewrite, .. } => Some(rewrite),
            Body::Native { .. } | Body::Intrinsic => None,
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
    pub(crate) return_type: ParameterType,
    /// The error codes this overload can raise (rule ids), or empty.
    pub(crate) errors: Vec<&'static str>,
    /// Whether this overload lowers via a helper `bl` or inline.
    pub(crate) lowering: Lowering,
    /// How this overload is realized in codegen.
    pub(crate) body: Body,
}

pub(crate) struct CallShape {
    /// This overload's parameters, in signature order.
    pub(crate) args: Vec<ParameterType>,
    /// This overload's return type name (`"Nothing"` for a statement-like call).
    pub(crate) return_type: ParameterType,
}

/// One function of a package: a public name, its documentation, and its
/// implementations. Fields are private so the registry is an API, not an open
/// record — construct via [`RegistryPackage::add_function`].
#[derive(Debug)]
pub(crate) struct RegistryFunction {
    /// The function's public (unqualified) name, e.g. `"utf8Encode"`.
    pub(crate) name: &'static str,
    /// One-line documentation intro.
    pub(crate) intro: &'static str,
    /// Full documentation description.
    pub(crate) desc: &'static str,
    /// A runnable documentation example.
    pub(crate) example: &'static str,
    /// The function's implementations (`>= 1`); more than one means an overload set.
    pub(crate) implementations: Vec<Implementation>,
}

impl RegistryFunction {
    /// All overloads.
    pub(crate) fn implementations(&self) -> &[Implementation] {
        &self.implementations
    }

    /// The overload matching this call shape, if exactly one does.
    pub(crate) fn select(&self, args: &CallShape) -> Option<&Implementation> {
        let mut matches = self.implementations.iter().filter(|implementation| {
            let required_params = implementation
                .params
                .iter()
                .filter(|param| matches!(param.default, DefaultValue::None))
                .count();

            let arg_count = args.args.len();
            if arg_count < required_params || arg_count > implementation.params.len() {
                return false;
            }

            implementation
                .params
                .iter()
                .zip(&args.args)
                .all(|(param, arg_ty)| param.ty == *arg_ty)
        });

        let implementation = matches.next()?;
        matches.next().is_none().then_some(implementation)
    }

    // Facts that ARE uniform across overloads live here:
    pub(crate) fn arity(&self) -> Option<(usize, usize)> {
        let mut min = usize::MAX;
        let mut max = 0usize;

        for implementation in &self.implementations {
            let required = implementation
                .params
                .iter()
                .filter(|param| matches!(param.default, DefaultValue::None))
                .count();

            min = min.min(required);
            max = max.max(implementation.params.len());
        }

        (min != usize::MAX).then_some((min, max))
    }

    pub(crate) fn declares_error(&self, name: &str) -> bool {
        self.implementations
            .iter()
            .any(|implementation| implementation.errors.iter().any(|error| *error == name))
    }
}

/// One field of a [`RegistryRecord`], e.g. `value AS Float`.
#[derive(Clone, Debug)]
pub(crate) struct RecordProp {
    /// The field name (`value`).
    pub(crate) name: &'static str,
    /// The field's type name (`Float`, `List OF Byte`, `net.Url`, …).
    pub(crate) ty: ParameterType,
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
/// Construct with a named struct literal; add via [`RegistryPackage::add_record`].
#[derive(Debug)]
pub(crate) struct RegistryRecord {
    /// The record's type name (`JsonNum`).
    pub(crate) name: &'static str,
    /// Whether the record is `EXPORT`ed (visible to importers) or package-internal.
    pub(crate) export: bool,
    /// The record's fields, in declaration order (`>= 1`).
    pub(crate) props: Vec<RecordProp>,
}

impl RegistryRecord {
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
            out.push_str(&prop.ty.name());
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
/// Construct with a named struct literal; add via [`RegistryPackage::add_union`].
#[derive(Debug)]
pub(crate) struct RegistryUnion {
    /// The union's type name (`Json`).
    pub(crate) name: &'static str,
    /// Whether the union is `EXPORT`ed (visible to importers) or package-internal.
    pub(crate) export: bool,
    /// The union's variants, in declaration order (`>= 1`).
    pub(crate) variants: Vec<UnionVariant>,
}

impl RegistryUnion {
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
    /// Shared `__pkg_*` helper functions the members call, as source chunks. Not
    /// callable members; rendered between the unions and the member bodies.
    helper_functions: Vec<&'static str>,
    functions: Vec<RegistryFunction>,
}

impl RegistryPackage {
    /// An empty package with the given import name and documentation. Fill it with
    /// [`add_imports`](Self::add_imports) / [`add_record`](Self::add_record) /
    /// [`add_union`](Self::add_union) / [`add_helper_functions`](Self::add_helper_functions)
    /// / [`add_function`](Self::add_function), then hand it to [`Registry::add_package`].
    pub(crate) fn new(import_name: &'static str, intro: &'static str, desc: &'static str) -> Self {
        Self {
            import_name,
            intro,
            desc,
            imports: Vec::new(),
            records: Vec::new(),
            unions: Vec::new(),
            helper_functions: Vec::new(),
            functions: Vec::new(),
        }
    }

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

    /// The package's shared helper-function source chunks, in the order added.
    pub(crate) fn helper_functions(&self) -> &[&'static str] {
        &self.helper_functions
    }

    /// Whether `ast` imports this package — the generic replacement for the old
    /// per-package `uses_package`. A property of the *program being compiled*, so it
    /// takes the AST rather than being a stored flag.
    pub(crate) fn is_imported_by(&self, ast: &crate::ast::AstProject) -> bool {
        ast.files.iter().any(|file| {
            file.imports
                .iter()
                .any(|import| import.package_name() == self.import_name)
        })
    }

    /// Assemble this package's complete injectable MFBASIC source — the modern
    /// equivalent of a hand-written `package.mfb` (e.g.
    /// `codegen/builtins/csv/package.mfb`), reconstructed from the package's own
    /// imports, records, unions, helper functions, and members' [`Body::Mfb`] bodies.
    ///
    /// The output is, in order:
    ///
    /// 1. the package's [`imports`](Self::imports), as a leading block of
    ///    `IMPORT <name>` lines (imports must precede all declarations in MFBASIC);
    /// 2. every [`RegistryRecord`] as an `[EXPORT] TYPE … END TYPE` block (in
    ///    `add_record` order);
    /// 3. every [`RegistryUnion`] as an `[EXPORT] UNION … END UNION` block (in
    ///    `add_union` order);
    /// 4. the [`helper_functions`](Self::helper_functions) — the shared `__pkg_*`
    ///    helpers the members call, in the order added;
    /// 5. every [`Body::Mfb`] body across all functions (each overload counts — a
    ///    same-named native-overload set like `encoding::utf8Encode` emits one `FUNC`
    ///    per overload), in registration order.
    ///
    /// Bodies keep their raw `__pkg_name` spelling; internalization to `#pkg_name`
    /// happens later when the assembled text is parsed.
    ///
    /// Returns the **empty string** only when the package has *nothing* to inject —
    /// no records, no unions, and no `Mfb` member — even if imports or helper
    /// functions are present, because then they support nothing. (Records and unions
    /// are injectable source in their own right: a package whose functions are all
    /// `Native`/`Rewrite` still emits its `TYPE`/`UNION` declarations here.) Pieces are
    /// separated by a blank line and the result ends with a newline, so the output is
    /// directly parseable.
    pub(crate) fn get_mfb(&self) -> String {
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

        let mut pieces: Vec<String> = Vec::with_capacity(
            1 + self.records.len() + self.unions.len() + self.helper_functions.len() + bodies.len(),
        );
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
        pieces.extend(
            self.helper_functions
                .iter()
                .map(|helper| helper.trim_end().to_string()),
        );
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

    /// Add shared helper-function source chunks (the `__pkg_*` helpers the members
    /// call). Additive — later calls append. They render into
    /// [`get_mfb`](Self::get_mfb) between the unions and the member bodies.
    pub(crate) fn add_helper_functions(&mut self, helpers: Vec<&'static str>) -> &mut Self {
        self.helper_functions.extend(helpers);
        self
    }

    /// Add a value record (a `RegistryRecord { … }`). Records render into
    /// [`get_mfb`](Self::get_mfb) in the order they are added, before the functions.
    pub(crate) fn add_record(&mut self, record: RegistryRecord) -> &mut Self {
        debug_assert!(
            !record.props.is_empty(),
            "record `{}` needs at least one field",
            record.name,
        );
        self.records.push(record);
        self
    }

    /// Add a tagged union (a `RegistryUnion { … }`). Unions render into
    /// [`get_mfb`](Self::get_mfb) in the order they are added, between the records and
    /// the functions.
    pub(crate) fn add_union(&mut self, union: RegistryUnion) -> &mut Self {
        debug_assert!(
            !union.variants.is_empty(),
            "union `{}` needs at least one variant",
            union.name,
        );
        self.unions.push(union);
        self
    }

    /// Add a function (a `RegistryFunction { … }`).
    pub(crate) fn add_function(&mut self, function: RegistryFunction) -> &mut Self {
        debug_assert!(
            !function.implementations.is_empty(),
            "function `{}` needs at least one implementation",
            function.name,
        );
        self.functions.push(function);
        self
    }
}

pub(crate) struct ResolvedFunc<'r> {
    pub(crate) package: &'r RegistryPackage,
    pub(crate) function: &'r RegistryFunction,
}

pub(crate) enum ResolvedType<'r> {
    Record(&'r RegistryRecord),
    Union(&'r RegistryUnion),
}

/// The registry: an ordered set of packages. Built imperatively, then
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

    /// Add a fully-built package (built with [`RegistryPackage::new`] and filled
    /// with its records / unions / helpers / functions).
    pub(crate) fn add_package(&mut self, package: RegistryPackage) -> &mut Self {
        self.packages.push(package);
        self
    }

    /// All packages, in registration order.
    pub(crate) fn packages(&self) -> &[RegistryPackage] {
        &self.packages
    }

    // Lookup functions
    pub(crate) fn resolve_package(&self, qualified: &str) -> Option<&RegistryPackage> {
        let (pkg_name, _) = qualified.split_once('.')?;
        self.packages.iter().find(|p| p.import_name == pkg_name)
    }

    pub(crate) fn resolve_func(&self, qualified: &str) -> Option<ResolvedFunc<'_>> {
        let (pkg_name, func_name) = qualified.split_once('.')?;
        let package = self.packages.iter().find(|p| p.import_name == pkg_name)?;
        let function = package.function(func_name)?;
        Some(ResolvedFunc { package, function })
    }

    pub(crate) fn resolve_implementation(
        &self,
        qualified: &str,
        args: &CallShape,
    ) -> Option<&Implementation> {
        let (pkg_name, func_name) = qualified.split_once('.')?;
        let package = self.packages.iter().find(|p| p.import_name == pkg_name)?;
        let function = package.function(func_name)?;

        function.select(args)
    }

    pub(crate) fn resolve_type(&self, qualified: &str) -> Option<ResolvedType<'_>> {
        let (pkg_name, type_name) = qualified.split_once('.')?;
        let package = self.packages.iter().find(|p| p.import_name == pkg_name)?;

        if let Some(record) = package.records().iter().find(|r| r.name == type_name) {
            return Some(ResolvedType::Record(record));
        }
        if let Some(union) = package.unions().iter().find(|u| u.name == type_name) {
            return Some(ResolvedType::Union(union));
        }
        None
    }

    /// Inject every package's reassembled source into `ast`, for
    /// each package the program imports — the registry-driven replacement for the
    /// per-package `augmented_project` functions. A package that is not imported, or has
    /// nothing to inject (empty [`get_mfb`](RegistryPackage::get_mfb)), contributes no
    /// file. The synthetic path/doc labels match the pre-migration convention
    /// (`<builtin-csv>` / `builtins/csv.mfb`).
    pub(crate) fn augment_project(
        &self,
        ast: &crate::ast::AstProject,
    ) -> Result<crate::ast::AstProject, ()> {
        let mut synthetic_files = Vec::new();

        for package in self.packages() {
            if !package.is_imported_by(ast) {
                continue;
            }
            let source = package.get_mfb();
            if source.is_empty() {
                continue;
            }
            let label = format!("<builtin-{}>", package.import_name());
            let doc = format!("builtins/{}.mfb", package.import_name());
            let file =
                crate::ast::parse_source_internal(std::path::Path::new(&label), &doc, &source)?;
            synthetic_files.push(file);
        }

        if synthetic_files.is_empty() {
            return Ok(ast.clone());
        }

        let mut augmented = ast.clone();
        augmented.files.extend(synthetic_files);
        Ok(augmented)
    }

    //
    // Blow this line exists for the old compiler
    // interface and should be removed when the compiler
    // directly uses the above 3 resovle_* functions
    //

    /// The package with this import name, or `None`.
    /// #[deprecated(note = "remove when other function are migrated")]
    pub(crate) fn get_package(&self, import_name: &str) -> Option<&RegistryPackage> {
        self.packages.iter().find(|p| p.import_name == import_name)
    }

    /// The function named `qualified` — a `<import_name>.<function>` call name such
    /// as `"csv.parse"` — or `None` if no migrated package declares it.
    /// #[deprecated(note = "remove when other function are migrated")]
    pub(crate) fn function_by_qualified(&self, qualified: &str) -> Option<&RegistryFunction> {
        let (package, function) = qualified.split_once('.')?;
        self.get_package(package)?.function(function)
    }

    /// The package that declares the function named `qualified` — a
    /// `<import_name>.<function>` call name such as `"csv.parse"` — or `None` if no
    /// migrated package declares it.
    ///
    /// This is the registry's single membership query, replacing the per-package
    /// `is_<pkg>_call` checks: a `csv.parse` call is a csv call iff this returns the
    /// csv package. Because it hands back the whole [`RegistryPackage`], the caller
    /// can reach the function's [`Implementation`] from the same lookup (via
    /// [`RegistryPackage::function`]) for the return type and rewrite target — the two
    /// other facts the old `resolve_call_return_type` / `implementation_name` hooks
    /// answered separately.
    /// #[deprecated(note = "remove when other function are migrated")]
    pub(crate) fn get_package_by_func_name(&self, qualified: &str) -> Option<&RegistryPackage> {
        let (package, function) = qualified.split_once('.')?;
        let package = self.get_package(package)?;
        package.function(function).map(|_| package)
    }
}

/// The process-wide registry, built once on first access.
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
/// The `example` package is an illustrative entry that exercises the shape (a
/// single-implementation function and a two-implementation overload); `csv` is the
/// first real package migrated off `target::shared::registry` — it registers itself
/// from its own module, `crate::codegen::builtins::csv`.
fn build() -> Registry {
    let mut r = Registry::new();
    crate::codegen::builtins::csv::register(&mut r);
    crate::codegen::builtins::json::register(&mut r);
    crate::codegen::builtins::regex::register(&mut r);
    r
}

//
// Everything below this should be depricated
//

/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn augment_project(ast: &crate::ast::AstProject) -> Result<crate::ast::AstProject, ()> {
    registry().augment_project(ast)
}

// The generic-dispatch query surface. Each answers, for a
// call, the fact the old `REGISTRY`-based generic dispatch answered — so a caller
// can dual-path `registry::X(name).or(old(name))`. `None`/`false` means "no
// package owns this call", i.e. fall through to the old path.

/// Whether a migrated package declares the call `qualified` (`"csv.parse"`).
/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn is_member(qualified: &str) -> bool {
    registry().resolve_func(qualified).is_some()
}

/// The import name of the migrated package that owns `qualified`, or `None`.
/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn owning_package(qualified: &str) -> Option<&'static str> {
    registry()
        .resolve_func(qualified)
        .map(|resolved| resolved.package.import_name)
}

/// The return type of the migrated call `qualified`, or `None`. (csv has one
/// implementation per member; the return type is uniform across a name's overloads.)
/// This leaks, once migration is complete it goes away
/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn call_return_type(qualified: &str) -> Option<&'static str> {
    let name = registry()
        .resolve_func(qualified)?
        .function
        .implementations
        .first()?
        .return_type
        .name();

    Some(match name {
        Cow::Borrowed(s) => s,
        Cow::Owned(s) => Box::leak(s.into_boxed_str()),
    })
}

/// Whether a scalar type name is a primitive (non-container, non-nominal) type.
fn is_scalar_type_name(name: &str) -> bool {
    matches!(
        name,
        "Boolean" | "Byte" | "Integer" | "Fixed" | "Float" | "Money" | "Nothing" | "String"
    )
}

/// Whether an actual argument type `arg` is compatible with a parameter type. An
/// `Unknown` argument (unresolved) is always accepted, exact names match, and two
/// *different known scalars* are the only definite incompatibility — container /
/// nominal / union types are accepted conservatively (the type checker never emits a
/// false rejection).
fn arg_matches_param(arg: &str, ty: &ParameterType) -> bool {
    if arg == "Unknown" {
        return true;
    }
    let expected = ty.name();
    if arg == expected.as_ref() {
        return true;
    }
    !(is_scalar_type_name(&expected) && is_scalar_type_name(arg))
}

/// Resolve the migrated call `qualified` against `arg_types`, returning its return
/// type only when the arguments are a valid arity and type match — the clean-room
/// equivalent of the old `DefaultResolver::resolve_call`. `None` means "no migrated
/// package accepts this call with these arguments" (a wrong arity or a scalar type
/// mismatch), which the type checker turns into an arity / argument-type error.
pub(crate) fn resolve_call(qualified: &str, arg_types: &[String]) -> Option<&'static str> {
    let function = registry().resolve_func(qualified)?.function;
    let implementation = function.implementations.first()?;
    let required = implementation
        .params
        .iter()
        .filter(|param| matches!(param.default, DefaultValue::None))
        .count();
    if arg_types.len() < required || arg_types.len() > implementation.params.len() {
        return None;
    }
    for (arg, param) in arg_types.iter().zip(implementation.params.iter()) {
        if !arg_matches_param(arg, &param.ty) {
            return None;
        }
    }
    Some(match implementation.return_type.name() {
        Cow::Borrowed(s) => s,
        Cow::Owned(s) => Box::leak(s.into_boxed_str()),
    })
}

/// The `(min, max)` argument arity of the migrated call `qualified`, or `None`.
/// `min` counts the required (non-defaulted) params; `max` is the widest overload.
/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn arity(qualified: &str) -> Option<(usize, usize)> {
    let resolved = registry().resolve_func(qualified)?;
    resolved.function.arity()
}

/// Whether `name` is a value type (`EXPORT TYPE`/`UNION`) declared by any migrated
/// package (`CsvReader`/`CsvRow`).
/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn is_builtin_type(name: &str) -> bool {
    registry().packages().iter().any(|package| {
        package.records().iter().any(|record| record.name == name)
            || package.unions().iter().any(|union| union.name == name)
    })
}

/// A `package.Type` reference (`"csv.CsvReader"`) resolve_funcd to its bare member type
/// id when the migrated package declares it, else `None`.
/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn qualified_builtin_type(qualified: &str) -> Option<String> {
    registry()
        .resolve_type(qualified)
        .map(|resolved| match resolved {
            ResolvedType::Record(record) => record.name.to_string(),
            ResolvedType::Union(union) => union.name.to_string(),
        })
}

/// Whether the migrated call `qualified` declares `error_name` among any of its
/// implementations' errors — the half of the `raise_error` "a builtin
/// must declare the errors it raises" check.
/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn declares_error(qualified: &str, error_name: &str) -> bool {
    registry()
        .resolve_func(qualified)
        .is_some_and(|resolved| resolved.function.declares_error(error_name))
}

/// The internal symbol the migrated call `qualified` rewrites to at IR lowering, or
/// `None`.
/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn rewrite_target(qualified: &str) -> Option<&'static str> {
    registry()
        .function_by_qualified(qualified)?
        .implementations
        .first()?
        .body
        .rewrite_target()
}

/// The primary expected argument type (first parameter) of the migrated call
/// `qualified`, or `None`.
/// This leaks, once migration is complete it goes away
/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn expected_arguments(qualified: &str) -> Option<&'static str> {
    let name = registry()
        .function_by_qualified(qualified)?
        .implementations
        .first()?
        .params
        .first()?
        .ty
        .name();

    Some(match name {
        Cow::Borrowed(s) => s,
        Cow::Owned(s) => Box::leak(s.into_boxed_str()),
    })
}

/// The per-position `[name, alias…]` keyword-matching lists for the migrated call
/// `qualified`, or `None`.
/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn call_param_names(qualified: &str) -> Option<Vec<Vec<&'static str>>> {
    let implementation = registry()
        .function_by_qualified(qualified)?
        .implementations
        .first()?;
    Some(
        implementation
            .params
            .iter()
            .map(|param| {
                let mut names = Vec::with_capacity(1 + param.aliases.len());
                names.push(param.name);
                names.extend_from_slice(param.aliases);
                names
            })
            .collect(),
    )
}

/// The `(type, expr)` constants to append after `provided` real arguments so a
/// migrated call's injected body receives its full arity — the `Fill` params past
/// `provided`. Empty when no migrated package owns `qualified`.
/// This leaks, once migration is complete it goes away
/// #[deprecated(note = "migrate registry().*")]
pub(crate) fn default_argument_padding(
    qualified: &str,
    provided: usize,
) -> Vec<(&'static str, &'static str)> {
    let Some(function) = registry().function_by_qualified(qualified) else {
        return Vec::new();
    };
    let Some(implementation) = function.implementations.first() else {
        return Vec::new();
    };
    implementation
        .params
        .iter()
        .skip(provided)
        .filter_map(|param| match &param.default {
            DefaultValue::Fill { type_name, expr } => {
                let name = match type_name.name() {
                    Cow::Borrowed(s) => s,
                    Cow::Owned(s) => Box::leak(s.into_boxed_str()),
                };
                Some((name, *expr))
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- test builders (named-literal wrappers with throwaway docs) ---

    fn func(name: &'static str, implementations: Vec<Implementation>) -> RegistryFunction {
        RegistryFunction {
            name,
            intro: "i",
            desc: "d",
            example: "e",
            implementations,
        }
    }

    fn prop(name: &'static str, ty: ParameterType) -> RecordProp {
        RecordProp {
            name,
            ty,
            description: "field doc",
        }
    }

    fn rec(name: &'static str, export: bool, props: Vec<RecordProp>) -> RegistryRecord {
        RegistryRecord {
            name,
            export,
            props,
        }
    }

    fn variant(name: &'static str) -> UnionVariant {
        UnionVariant {
            name,
            description: "variant doc",
        }
    }

    fn uni(name: &'static str, export: bool, variants: Vec<UnionVariant>) -> RegistryUnion {
        RegistryUnion {
            name,
            export,
            variants,
        }
    }

    // A source-backed member built from its `FUNC __name(...)` body; the rewrite
    // symbol is derived from the body's declared name.
    fn mfb_impl(body: &'static str) -> Implementation {
        let rewrite = body
            .strip_prefix("FUNC ")
            .and_then(|rest| rest.split(['(', ' ']).next())
            .expect("test body starts with `FUNC <name>`");
        Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::mfb(body, rewrite),
        }
    }

    fn intrinsic(return_type: ParameterType) -> Implementation {
        Implementation {
            params: vec![],
            return_type,
            errors: vec![],
            lowering: Lowering::Inline,
            body: Body::Intrinsic,
        }
    }

    fn rewrite_impl(symbol: &'static str) -> Implementation {
        Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::Rewrite(symbol),
        }
    }

    fn param(name: &'static str, ty: ParameterType) -> Parameter {
        Parameter {
            name,
            desc: "",
            aliases: &[],
            ty,
            default: DefaultValue::None,
        }
    }

    #[test]
    fn frozen_registry_exposes_the_csv_package() {
        let pkg = registry().get_package("csv").expect("csv registered");
        assert_eq!(pkg.import_name(), "csv");
        assert_eq!(pkg.functions().len(), 4);
        assert!(registry().get_package("nope").is_none());
    }

    #[test]
    fn an_overload_is_two_implementations_differing_by_parameter_type() {
        let describe = func(
            "describe",
            vec![
                Implementation {
                    params: vec![param("v", ParameterType::Integer)],
                    return_type: ParameterType::String,
                    errors: vec![],
                    lowering: Lowering::Helper,
                    body: Body::Rewrite("__describe_int"),
                },
                Implementation {
                    params: vec![param("v", ParameterType::String)],
                    return_type: ParameterType::String,
                    errors: vec![],
                    lowering: Lowering::Helper,
                    body: Body::Rewrite("__describe_str"),
                },
            ],
        );
        let impls = &describe.implementations;
        assert_eq!(impls.len(), 2);
        assert_eq!(impls[0].params[0].ty.name(), "Integer");
        assert_eq!(impls[1].params[0].ty.name(), "String");
    }

    #[test]
    fn add_function_takes_a_function_value() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("t", "intro", "desc");
        pkg.add_function(func("f", vec![intrinsic(ParameterType::Nothing)]));
        r.add_package(pkg);
        assert_eq!(r.packages().len(), 1);
        assert_eq!(r.get_package("t").unwrap().functions().len(), 1);
    }

    #[test]
    fn get_package_by_func_name_finds_the_owning_package() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("csv", "i", "d");
        pkg.add_function(func("parse", vec![rewrite_impl("__csv_parse")]));
        r.add_package(pkg);

        assert_eq!(
            r.get_package_by_func_name("csv.parse")
                .map(RegistryPackage::import_name),
            Some("csv"),
        );
        assert!(r.get_package_by_func_name("csv.nope").is_none());
        assert!(r.get_package_by_func_name("nope.parse").is_none());
        assert!(r.get_package_by_func_name("toString").is_none());
        // Works against the frozen registry too (a real migrated member).
        assert_eq!(
            registry()
                .get_package_by_func_name("csv.parse")
                .map(RegistryPackage::import_name),
            Some("csv"),
        );
    }

    fn sample_lower<'a>(
        _b: &mut crate::target::shared::code::CodeBuilder<'a>,
        _args: &[crate::target::shared::nir::NirValue],
    ) -> Result<crate::target::shared::code::ValueResult, String> {
        Err("sample lowering (test fixture, not invoked)".to_string())
    }

    fn sample_os_lower(
        _call: &str,
        _symbol: &str,
        _imports: &std::collections::HashMap<String, String>,
        _platform: &dyn crate::target::shared::code::CodegenPlatform,
    ) -> crate::target::shared::code::HelperResult {
        Err("sample OS lowering (test fixture, not invoked)".to_string())
    }

    fn sample_fast_path<'a>(
        _b: &mut crate::target::shared::code::CodeBuilder<'a>,
        _target: &str,
        _args: &[crate::target::shared::nir::NirValue],
    ) -> Result<Option<crate::target::shared::code::ValueResult>, String> {
        Ok(None)
    }

    #[test]
    fn mfb_body_carries_a_rewrite_target_and_optional_fast_path() {
        let plain = Body::mfb("FUNC __x() ... END FUNC", "__x");
        assert!(matches!(
            plain,
            Body::Mfb {
                fast_path: None,
                ..
            }
        ));
        assert_eq!(plain.rewrite_target(), Some("__x"));

        let accelerated =
            Body::mfb_with_fast_path("FUNC __x() ... END FUNC", "__x", sample_fast_path);
        assert!(matches!(
            accelerated,
            Body::Mfb {
                fast_path: Some(_),
                ..
            }
        ));

        assert_eq!(Body::Rewrite("__y").rewrite_target(), Some("__y"));
        assert_eq!(Body::Intrinsic.rewrite_target(), None);
    }

    #[test]
    fn native_holds_three_per_family_slots() {
        match Body::native(None, None, Some(sample_lower as NativeLower)) {
            Body::Native { posix, win, common } => {
                assert!(posix.is_none() && win.is_none() && common.is_some());
            }
            _ => panic!("expected Body::Native"),
        }
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

    #[test]
    fn get_mfb_renders_helper_functions_before_member_bodies() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("demo", "intro", "desc");
        pkg.add_helper_functions(vec!["FUNC __demo_helper() AS Nothing\nEND FUNC"]);
        pkg.add_function(func(
            "a",
            vec![mfb_impl(
                "FUNC __demo_a() AS String\n  RETURN \"a\"\nEND FUNC",
            )],
        ));
        pkg.add_function(func("b", vec![rewrite_impl("__demo_b")]));
        pkg.add_function(func(
            "c",
            vec![mfb_impl(
                "FUNC __demo_c() AS String\n  RETURN \"c\"\nEND FUNC",
            )],
        ));
        r.add_package(pkg);

        let src = r.get_package("demo").unwrap().get_mfb();
        assert_eq!(
            src,
            "FUNC __demo_helper() AS Nothing\nEND FUNC\n\n\
             FUNC __demo_a() AS String\n  RETURN \"a\"\nEND FUNC\n\n\
             FUNC __demo_c() AS String\n  RETURN \"c\"\nEND FUNC\n",
        );
        assert!(!src.contains("__demo_b"));
    }

    #[test]
    fn get_mfb_is_empty_when_the_package_has_no_mfb_member() {
        // A package with only Intrinsic/Rewrite members and no records/unions injects
        // nothing.
        let mut empty = Registry::new();
        let mut pkg = RegistryPackage::new("nomfb", "i", "d");
        pkg.add_function(func("a", vec![rewrite_impl("__a")]));
        empty.add_package(pkg);
        assert_eq!(empty.get_package("nomfb").unwrap().get_mfb(), "");

        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("bare", "i", "d");
        pkg.add_imports(vec!["strings"]);
        pkg.add_helper_functions(vec!["FUNC __helper() AS Nothing\nEND FUNC"]);
        r.add_package(pkg);
        assert_eq!(r.get_package("bare").unwrap().get_mfb(), "");
    }

    #[test]
    fn add_record_renders_the_type_declaration() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("json", "intro", "desc");
        pkg.add_record(rec(
            "JsonNum",
            true,
            vec![prop("value", ParameterType::Float)],
        ));
        pkg.add_record(rec(
            "Pair",
            false,
            vec![
                prop("key", ParameterType::String),
                prop("val", ParameterType::Integer),
            ],
        ));
        r.add_package(pkg);

        let pkg = r.get_package("json").unwrap();
        assert_eq!(pkg.records().len(), 2);
        assert!(pkg.records()[0].export);
        assert!(!pkg.records()[1].export);
        assert_eq!(
            pkg.records()[0].render(),
            "EXPORT TYPE JsonNum\n  value AS Float\nEND TYPE"
        );
        assert_eq!(
            pkg.records()[1].render(),
            "TYPE Pair\n  key AS String\n  val AS Integer\nEND TYPE"
        );
    }

    #[test]
    fn add_union_renders_the_union_declaration() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("json", "intro", "desc");
        pkg.add_union(uni(
            "Json",
            true,
            vec![variant("JsonNull"), variant("JsonNum"), variant("JsonStr")],
        ));
        pkg.add_union(uni("Internal", false, vec![variant("A")]));
        r.add_package(pkg);

        let pkg = r.get_package("json").unwrap();
        assert_eq!(pkg.unions().len(), 2);
        assert!(pkg.unions()[0].export);
        assert!(!pkg.unions()[1].export);
        assert_eq!(
            pkg.unions()[0].render(),
            "EXPORT UNION Json\n  JsonNull\n  JsonNum\n  JsonStr\nEND UNION"
        );
        assert_eq!(pkg.unions()[1].render(), "UNION Internal\n  A\nEND UNION");
    }

    #[test]
    fn get_mfb_orders_imports_records_unions_helpers_functions() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("json", "intro", "desc");
        pkg.add_imports(vec!["collections"]);
        pkg.add_imports(vec!["strings"]);
        pkg.add_record(rec(
            "JsonNum",
            true,
            vec![prop("value", ParameterType::Float)],
        ));
        pkg.add_record(rec(
            "JsonBool",
            true,
            vec![prop("flag", ParameterType::Boolean)],
        ));
        pkg.add_union(uni(
            "Json",
            true,
            vec![variant("JsonNum"), variant("JsonBool")],
        ));
        pkg.add_helper_functions(vec!["FUNC __json_helper() AS Nothing\nEND FUNC"]);
        pkg.add_function(func(
            "render",
            vec![mfb_impl(
                "FUNC __json_render() AS String\n  RETURN \"\"\nEND FUNC",
            )],
        ));
        r.add_package(pkg);

        let pkg = r.get_package("json").unwrap();
        assert_eq!(pkg.imports(), &["collections", "strings"]);
        assert_eq!(
            pkg.get_mfb(),
            "IMPORT collections\nIMPORT strings\n\n\
             EXPORT TYPE JsonNum\n  value AS Float\nEND TYPE\n\n\
             EXPORT TYPE JsonBool\n  flag AS Boolean\nEND TYPE\n\n\
             EXPORT UNION Json\n  JsonNum\n  JsonBool\nEND UNION\n\n\
             FUNC __json_helper() AS Nothing\nEND FUNC\n\n\
             FUNC __json_render() AS String\n  RETURN \"\"\nEND FUNC\n",
        );
    }

    #[test]
    fn descriptor_round_trips_the_full_surface() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("demo", "pkg intro", "pkg desc");
        pkg.add_imports(vec!["strings"]);
        pkg.add_helper_functions(vec!["FUNC __demo_helper()\nEND FUNC"]);
        pkg.add_record(rec("Rec", true, vec![prop("f", ParameterType::Integer)]));
        pkg.add_union(uni("Uni", false, vec![variant("V")]));
        pkg.add_function(RegistryFunction {
            name: "fn1",
            intro: "fn intro",
            desc: "fn desc",
            example: "fn example",
            implementations: vec![Implementation {
                params: vec![
                    Parameter {
                        name: "req",
                        desc: "",
                        aliases: &["r"],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "opt",
                        desc: "",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::Fill {
                            type_name: ParameterType::String,
                            expr: ",",
                        },
                    },
                    Parameter {
                        name: "zone",
                        desc: "",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::Optional,
                    },
                ],
                return_type: ParameterType::Nothing,
                errors: vec!["SOME_ERROR"],
                lowering: Lowering::Helper,
                body: Body::Rewrite("__demo_fn1"),
            }],
        });
        r.add_package(pkg);

        let pkg = r.get_package("demo").unwrap();
        assert_eq!(pkg.intro(), "pkg intro");
        assert_eq!(pkg.desc(), "pkg desc");
        assert_eq!(pkg.helper_functions(), &["FUNC __demo_helper()\nEND FUNC"]);

        let rec = &pkg.records()[0];
        assert_eq!(rec.name, "Rec");
        assert_eq!(rec.props[0].name, "f");
        assert_eq!(rec.props[0].description, "field doc");

        let uni = &pkg.unions()[0];
        assert_eq!(uni.name, "Uni");
        assert_eq!(uni.variants[0].name, "V");

        let f = pkg.function("fn1").expect("fn1");
        assert_eq!(f.name, "fn1");
        assert_eq!(f.intro, "fn intro");
        assert_eq!(f.example, "fn example");

        let imp = &f.implementations[0];
        assert_eq!(imp.errors, vec!["SOME_ERROR"]);
        assert_eq!(imp.params[0].aliases, &["r"]);
        assert!(matches!(imp.params[0].default, DefaultValue::None));
        assert!(matches!(
            imp.params[1].default,
            DefaultValue::Fill {
                type_name: ParameterType::String,
                expr: ","
            }
        ));
        assert!(matches!(imp.params[2].default, DefaultValue::Optional));
    }

    #[test]
    fn is_imported_by_checks_the_program_imports() {
        let mut r = Registry::new();
        r.add_package(RegistryPackage::new("csv", "i", "d"));
        let csv = r.get_package("csv").expect("csv package");

        let parse = |src: &str| {
            let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
                .expect("parse");
            crate::ast::AstProject {
                name: "test".to_string(),
                files: vec![file],
            }
        };
        assert!(csv.is_imported_by(&parse("IMPORT csv\nSUB main\nEND SUB\n")));
        assert!(!csv.is_imported_by(&parse("SUB main\nEND SUB\n")));
    }
}
