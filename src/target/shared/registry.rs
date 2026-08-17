//! Descriptor vocabulary for builtin package metadata (plan-72).
//!
//! One `BuiltinModule` value carries everything a builtin package exposes: its
//! functions, each function's overloads, every overload's parameters and return
//! type, the package's builtin types, and its source-companion injection rule.
//! `DefaultResolver` derives — from that data alone, with no diagnostic-prose
//! parsing — the same answers the hand-written per-package helper functions
//! return today: `is_<pkg>_call` (membership), `arity`, `call_param_names`,
//! `call_return_type_name`, `argument_types`, `expected_arguments`,
//! `implementation_name`, and `default_argument_padding`.
//!
//! plan-72 lands this vocabulary first (letter A) and migrates one real package
//! per subsequent letter (B..AA); the final letter (BB) deletes the duplicated
//! free functions. Nothing in production dispatch consults these descriptors in
//! letter A — the current consumers are the unit tests here and the per-package
//! parity tests (the migration gate, see `parity`). The
//! `#[cfg_attr(not(test), allow(dead_code))]` markers record exactly that: each
//! item is exercised by those tests today and by the aggregate dispatchers as
//! each package migrates. This mirrors the established precedent on
//! `syntaxcheck::builtins::BuiltinPackage::name`.

#![cfg_attr(not(test), allow(dead_code))]

/// A parameter's declared type.
///
/// The payload is a *normalized* type name (`"Integer"`, `"List OF Byte"`), the
/// same spelling the type checker produces for an inferred argument and compares
/// against — never diagnostic prose. `Open Decisions` in the overview keeps the
/// door open for a strongly typed `TypeId` behind this enum; the string variant
/// is the conservative starting point because no stable global `TypeId` exists
/// for static builtin metadata yet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ParameterType {
    /// A concrete normalized type name.
    Named(&'static str),
}

impl ParameterType {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            ParameterType::Named(name) => name,
        }
    }
}

/// A function overload's return type.
///
/// `Fixed` covers every data-only package (the return does not vary with
/// argument types). Packages whose return type is argument-dependent
/// (`encoding.utf8Encode`, `vector::` monomorphs) carry a `Custom` marker and
/// supply the actual choice through a [`BuiltinResolver`]; `DefaultResolver`
/// reports `None` for a fixed-return query on such a function, matching the way
/// their legacy `call_return_type_name` returns `None`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReturnType {
    /// The same type name regardless of arguments.
    Fixed(&'static str),
    /// Argument-dependent; a [`BuiltinResolver`] selects it.
    Custom,
}

/// A parameter's default, used to pad omitted trailing arguments during IR
/// lowering. `Fill` mirrors the legacy `default_argument_padding` element
/// `(type_name, expr)`: the type the injected literal takes and the source
/// expression to inject (an empty `expr` means "the empty literal of that type",
/// e.g. crypto's empty `List OF Byte` `aad`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DefaultValue {
    /// A required parameter — no default.
    None,
    /// An optional parameter padded with `(type_name, expr)` when omitted — IR
    /// lowering injects the literal (`time`'s `second`/`nanos` → `0`).
    Fill {
        type_name: &'static str,
        expr: &'static str,
    },
    /// An optional parameter that widens arity but is NOT default-padded — the
    /// implementation selects a distinct body by argument count instead
    /// (`datetime.parse`'s trailing `zone`). Contributes to the arity range like
    /// `Fill`, but `default_padding` skips it.
    Optional,
}

/// One parameter of one overload.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Parameter {
    /// The canonical parameter name (the one diagnostics render).
    pub(crate) name: &'static str,
    /// Accepted alias spellings for named-argument binding, canonical name
    /// excluded. Most parameters have none.
    pub(crate) aliases: &'static [&'static str],
    pub(crate) ty: ParameterType,
    pub(crate) default: DefaultValue,
}

impl Parameter {
    pub(crate) const fn required(name: &'static str, ty: &'static str) -> Parameter {
        Parameter {
            name,
            aliases: &[],
            ty: ParameterType::Named(ty),
            default: DefaultValue::None,
        }
    }

    /// The name spellings a named argument may use to bind this parameter:
    /// canonical name first, then any aliases. This is the per-position alias
    /// list a legacy `call_param_names` row holds.
    pub(crate) fn name_spellings(&self) -> Vec<&'static str> {
        std::iter::once(self.name)
            .chain(self.aliases.iter().copied())
            .collect()
    }
}

/// One overload of a builtin function: a fixed parameter list plus a return
/// type. Optional arguments are modelled as trailing [`DefaultValue::Fill`]
/// parameters rather than as separate overloads, matching how the legacy
/// `arity` ranges and `default_argument_padding` are derived.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BuiltinOverload {
    pub(crate) params: &'static [Parameter],
    pub(crate) return_type: ReturnType,
}

impl BuiltinOverload {
    /// Required argument count: parameters with no default. Optional
    /// (defaulted) parameters are trailing by construction, so this is the
    /// overload's minimum arity.
    pub(crate) fn min_args(&self) -> usize {
        self.params
            .iter()
            .filter(|param| matches!(param.default, DefaultValue::None))
            .count()
    }

    /// Maximum arity: every parameter supplied.
    pub(crate) fn max_args(&self) -> usize {
        self.params.len()
    }
}

/// A builtin's target-generic lowering: given the code builder and the call's
/// NIR arguments, emit the abstract (target-neutral) instructions and return the
/// result value. Carried by [`Implementation::Native`]; the codegen dual-path
/// dispatch prefers it over the legacy `src/target` ladder (plan-95). The
/// higher-ranked lifetime is required because `CodeBuilder<'a>` is lifetime-
/// parameterized; free functions and methods satisfy it.
pub(crate) type NativeLower =
    for<'a> fn(
        &mut crate::target::shared::code::CodeBuilder<'a>,
        &[crate::target::shared::nir::NirValue],
    ) -> Result<crate::target::shared::code::ValueResult, String>;

/// A source-generic member's optional native **fast path**: the native
/// alternative to its `.mfb` body, chosen when a specific monomorph instantiation
/// qualifies (fixed-width elements, String keys, a const-`TRUE` flag, …). Given
/// the code builder, the `#pkg_<name>$<TypeArgs>` monomorph target, and the call
/// args, it either lowers natively (`Ok(Some(_))`) or **declines** (`Ok(None)`),
/// in which case the caller monomorphizes the `.mfb` body instead. This is the
/// `try_*`/`Ok(None)`-to-fall-back shape the collections fast paths already use;
/// carried by [`Implementation::Mfb`] alongside the body.
pub(crate) type MfbFastPath =
    for<'a> fn(
        &mut crate::target::shared::code::CodeBuilder<'a>,
        &str,
        &[crate::target::shared::nir::NirValue],
    ) -> Result<Option<crate::target::shared::code::ValueResult>, String>;

/// An OS-seam member's per-platform native emission: the arch-neutral `abi::` code
/// that emits its `_mfb_rt_<pkg>_*` runtime helper body. Same shape as a runtime
/// helper lowering — given the helper symbol, the platform imports, and the target
/// platform, it emits the body. [`Implementation::Os`] carries one for POSIX
/// (libc: macOS/Linux) and one for Windows (kernel32); the dispatch picks by
/// `platform.family()`. Arch-neutrality is why both live in the member's
/// `func_*.rs` and produce per-target machine code from a single source.
pub(crate) type OsLower = fn(
    &str, // the runtime-call name (e.g. "process.sendTimeout") — lets a member that
    // covers several code-form symbols pick the form; single-symbol members ignore it
    &str, // the mangled `_mfb_rt_<pkg>_<call>_<target>` helper symbol
    &std::collections::HashMap<String, String>,
    &dyn crate::target::shared::code::CodegenPlatform,
) -> crate::target::shared::code::HelperResult;

/// How a public call name maps to its implementation symbol.
///
/// `Same` means no rewrite — the public name *is* the implementation (the
/// legacy `implementation_name` for such packages returns `None`). `Rewrite`
/// carries a fixed internal symbol (encoding/regex/json/strings/net/csv rewrite
/// to a `__pkg_*` source body or native entry). Argument-type-dependent
/// selection (crypto's `_bytes`/`_text`, datetime by arity, vector monomorphs)
/// is `Custom` and resolved through a [`BuiltinResolver`]. `Native` carries the
/// function's own target-generic lowering (plan-95 migration).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Implementation {
    /// No rewrite; the public name is the implementation.
    Same,
    /// A single fixed implementation symbol.
    Rewrite(&'static str),
    /// Argument-dependent; a [`BuiltinResolver`] selects it.
    Custom,
    /// The function owns its target-generic lowering (reached via the codegen
    /// dual-path seam). Present only for migrated functions.
    Native(NativeLower),
    /// The function owns its MFBASIC source body (a `FUNC __pkg_<name> ... END
    /// FUNC` fragment), assembled into the package's injected source by the
    /// dual-path source loader and monomorphized like the package's `.mfb`
    /// companion. The external `.mfb` file remains the fallback for members not
    /// yet carrying `Mfb`, so the two paths coexist during migration.
    ///
    /// `fast_path` is the member's optional native accelerator (see
    /// [`MfbFastPath`]): when a monomorph instantiation qualifies it lowers the
    /// call natively instead of instantiating `body`. `None` for the members that
    /// only ever run their `.mfb` body.
    Mfb {
        body: &'static str,
        fast_path: Option<MfbFastPath>,
    },
    /// An OS-seam intrinsic: the member owns its arch-neutral, OS-branching native
    /// emission (`posix`/`win`), which the runtime-call dispatch selects by
    /// `platform.family()` to emit the `_mfb_rt_<pkg>_*` helper body. `all` lists
    /// every runtime-helper call name this member's emission covers — usually just
    /// its own (`["process.close"]`), but a member selected by arity/flags in
    /// `builder_values` emits several (`spawn` → `["process.spawn", "process.spawnEnv"]`).
    /// The honest, self-describing form of the OS-seam members that were `Same`.
    Os {
        posix: OsLower,
        win: OsLower,
        all: &'static [&'static str],
    },
}

// Hand-written so the `Native` fn pointer is compared by address via
// `std::ptr::fn_addr_eq` (a derived `PartialEq` would raise the
// "fn pointer comparisons are not meaningful" lint). Every non-`Native` consumer
// compares against `Same`/`Rewrite`/`Custom` only.
impl PartialEq for Implementation {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Implementation::Same, Implementation::Same)
            | (Implementation::Custom, Implementation::Custom) => true,
            (Implementation::Rewrite(a), Implementation::Rewrite(b)) => a == b,
            (Implementation::Native(a), Implementation::Native(b)) => std::ptr::fn_addr_eq(*a, *b),
            (
                Implementation::Mfb {
                    body: a,
                    fast_path: fa,
                },
                Implementation::Mfb {
                    body: b,
                    fast_path: fb,
                },
            ) => {
                a == b
                    && match (fa, fb) {
                        (None, None) => true,
                        (Some(fa), Some(fb)) => std::ptr::fn_addr_eq(*fa, *fb),
                        _ => false,
                    }
            }
            (
                Implementation::Os {
                    posix: pa,
                    win: wa,
                    all: aa,
                },
                Implementation::Os {
                    posix: pb,
                    win: wb,
                    all: ab,
                },
            ) => std::ptr::fn_addr_eq(*pa, *pb) && std::ptr::fn_addr_eq(*wa, *wb) && aa == ab,
            _ => false,
        }
    }
}
impl Eq for Implementation {}

/// How a builtin call is lowered. Descriptor-level classification only; the
/// detailed lowering stays in the backend. `Helper` is a `bl` into a runtime
/// helper; `Inline` lowers to native instructions in place (bits, math).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Lowering {
    Helper,
    Inline,
}

/// Per-function boolean facets kept out of the type-resolution path.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct BuiltinFlags {
    /// The call is compiler-internal and never written in user source.
    pub(crate) internal_only: bool,
    /// The call participates in return-type overloading resolved with a
    /// contextual expected type (`encoding.utf8Encode`).
    pub(crate) return_type_overloaded: bool,
}

/// One builtin function: its public name, doc slug, overloads, implementation
/// mapping, lowering class, and flags.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BuiltinFunction {
    /// The public call name as written after import resolution, e.g.
    /// `"bits.band"`.
    pub(crate) name: &'static str,
    /// The man-page slug for this function (documentation cross-reference).
    pub(crate) doc_slug: &'static str,
    /// Short intro/summary line for this function, authored in the renderer's
    /// Markdown subset. Empty until authored. Capped at 1024 bytes; the cap is
    /// enforced across the whole registry by the `doc_intro_within_cap` test
    /// rather than the type, so it stays a plain `&'static str`.
    pub(crate) doc_intro: &'static str,
    /// Full reference description for this function, authored in the renderer's
    /// Markdown subset. Unbounded. Empty until authored.
    pub(crate) doc_desc: &'static str,
    /// Worked `## Examples` section for this function, authored in the renderer's
    /// Markdown subset (fenced code blocks + prose). Unbounded. Empty until
    /// authored; set via [`BuiltinFunction::with_example`].
    pub(crate) doc_example: &'static str,
    /// The `errorCode::Err*` names this function can raise at runtime (e.g.
    /// `"ErrIndexOutOfRange"`), for documentation. Empty for infallible functions
    /// and until authored.
    pub(crate) errors: &'static [&'static str],
    pub(crate) overloads: &'static [BuiltinOverload],
    pub(crate) implementation: Implementation,
    pub(crate) lowering: Lowering,
    pub(crate) flags: BuiltinFlags,
}

impl BuiltinFunction {
    /// The function's own target-generic lowering, if it has been migrated to
    /// carry one (`Implementation::Native`). The codegen dual-path seam calls
    /// this; `None` means fall through to the legacy `src/target` ladder.
    pub(crate) fn native_lower(&self) -> Option<NativeLower> {
        match self.implementation {
            Implementation::Native(lower) => Some(lower),
            _ => None,
        }
    }

    /// Attach a worked `## Examples` section, consumed in `const` context:
    /// `BuiltinFunction::native(...).with_example(EXAMPLE)`. Additive, so a member
    /// without examples simply omits it and keeps `doc_example: ""`.
    pub(crate) const fn with_example(mut self, doc_example: &'static str) -> BuiltinFunction {
        self.doc_example = doc_example;
        self
    }

    /// Attach the one-line summary (`doc_intro`) in `const` context. Additive, for
    /// packages that declare members with a compact constructor (`datetime`'s `df`)
    /// and layer docs on afterward.
    pub(crate) const fn with_intro(mut self, doc_intro: &'static str) -> BuiltinFunction {
        self.doc_intro = doc_intro;
        self
    }

    /// Attach the full `## Description` prose (`doc_desc`) in `const` context.
    /// Additive companion to [`Self::with_intro`].
    pub(crate) const fn with_desc(mut self, doc_desc: &'static str) -> BuiltinFunction {
        self.doc_desc = doc_desc;
        self
    }

    /// Declare a builtin whose implementation is argument-dependent and selected
    /// by the owning package's [`BuiltinResolver`] (`Implementation::Custom`) —
    /// e.g. `encoding::utf8Encode`'s return-type overload. The registry-wide
    /// counterpart to [`Self::mfb`]/[`Self::native`] for the resolver-driven case,
    /// so a package need not hand-write the struct literal.
    pub(crate) const fn custom(
        name: &'static str,
        doc_slug: &'static str,
        doc_intro: &'static str,
        doc_desc: &'static str,
        errors: &'static [&'static str],
        overloads: &'static [BuiltinOverload],
    ) -> BuiltinFunction {
        BuiltinFunction {
            name,
            doc_slug,
            doc_intro,
            doc_desc,
            errors,
            overloads,
            doc_example: "",
            implementation: Implementation::Custom,
            lowering: Lowering::Helper,
            flags: BuiltinFlags {
                internal_only: false,
                return_type_overloaded: false,
            },
        }
    }

    /// **Deprecated — do not author new members with this.** `Implementation::Same`
    /// is legacy by-name dispatch: the descriptor carries *no* lowering, so the call
    /// falls through to the hand-written `src/target` ladder keyed on the call name.
    /// Every new (or migrated) member should instead own its lowering on the
    /// descriptor:
    ///
    /// - [`Self::os`] — an OS-seam member (lowers to a `_mfb_rt_<pkg>_*` runtime
    ///   helper): supply its `posix`/`win` emission.
    /// - [`Self::native`] — an arch-neutral member that emits in place or via a
    ///   shared helper: supply its target-generic `NativeLower`.
    ///
    /// Kept only so the not-yet-migrated packages (bits/math/strings/net/…) still
    /// compile; reach for `::os`/`::native` and this constructor disappears.
    #[deprecated(
        note = "Implementation::Same is legacy by-name dispatch; author new members with \
                BuiltinFunction::os() (OS-seam runtime helper) or ::native() (arch-neutral \
                inline/helper lowering) so the member owns its lowering on the descriptor"
    )]
    pub(crate) const fn same(
        name: &'static str,
        doc_slug: &'static str,
        doc_intro: &'static str,
        doc_desc: &'static str,
        errors: &'static [&'static str],
        overloads: &'static [BuiltinOverload],
    ) -> BuiltinFunction {
        BuiltinFunction {
            name,
            doc_slug,
            doc_intro,
            doc_desc,
            errors,
            overloads,
            doc_example: "",
            implementation: Implementation::Same,
            lowering: Lowering::Helper,
            flags: BuiltinFlags {
                internal_only: false,
                return_type_overloaded: false,
            },
        }
    }

    /// Declare an OS-seam member that owns its per-platform native emission
    /// ([`Implementation::Os`]) — the honest, self-describing form of the `Same`
    /// OS-seam members. `posix`/`win` emit the `_mfb_rt_<pkg>_*` helper body;
    /// `all` names every runtime-helper call the member covers.
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn os(
        name: &'static str,
        doc_slug: &'static str,
        doc_intro: &'static str,
        doc_desc: &'static str,
        errors: &'static [&'static str],
        overloads: &'static [BuiltinOverload],
        posix: OsLower,
        win: OsLower,
        all: &'static [&'static str],
    ) -> BuiltinFunction {
        BuiltinFunction {
            name,
            doc_slug,
            doc_intro,
            doc_desc,
            errors,
            overloads,
            doc_example: "",
            implementation: Implementation::Os { posix, win, all },
            lowering: Lowering::Helper,
            flags: BuiltinFlags {
                internal_only: false,
                return_type_overloaded: false,
            },
        }
    }

    /// The member's native fast path, if it carries one (`Implementation::Mfb`
    /// with a `Some` `fast_path`). The codegen monomorph-dispatch seam
    /// (`try_mfb_fast_path`) consults this before instantiating the `.mfb` body.
    pub(crate) fn mfb_fast_path(&self) -> Option<MfbFastPath> {
        match self.implementation {
            Implementation::Mfb { fast_path, .. } => fast_path,
            _ => None,
        }
    }

    /// Declare a builtin function that owns its target-generic lowering
    /// (`Implementation::Native`, reached through the codegen dual-path seam).
    /// The registry-wide constructor every migrated builtin uses, so no package
    /// hand-writes the `Implementation::Native` wiring (plan-95).
    pub(crate) const fn native(
        name: &'static str,
        doc_slug: &'static str,
        doc_intro: &'static str,
        doc_desc: &'static str,
        errors: &'static [&'static str],
        overloads: &'static [BuiltinOverload],
        lower: NativeLower,
    ) -> BuiltinFunction {
        BuiltinFunction {
            name,
            doc_slug,
            doc_intro,
            doc_desc,
            errors,
            overloads,
            doc_example: "",
            implementation: Implementation::Native(lower),
            lowering: Lowering::Helper,
            flags: BuiltinFlags {
                internal_only: false,
                return_type_overloaded: false,
            },
        }
    }

    /// Declare a source-generic builtin function that owns its MFBASIC source
    /// body (`Implementation::Mfb`, assembled into the package's injected source
    /// by the dual-path loader). The registry-wide constructor a package uses to
    /// pull a member's `FUNC __pkg_<name> ... END FUNC` out of its `.mfb`
    /// companion and into the descriptor, so the body, docs, and errors live in
    /// one place. `overloads` are documentation only — the `.mfb` signature drives
    /// arity/default resolution, exactly as for the un-migrated companion members.
    pub(crate) const fn mfb(
        name: &'static str,
        doc_slug: &'static str,
        doc_intro: &'static str,
        doc_desc: &'static str,
        errors: &'static [&'static str],
        overloads: &'static [BuiltinOverload],
        body: &'static str,
    ) -> BuiltinFunction {
        Self::mfb_impl(
            name, doc_slug, doc_intro, doc_desc, errors, overloads, body, None,
        )
    }

    /// Like [`Self::mfb`], but the member also owns a native [`MfbFastPath`] — the
    /// accelerator chosen for qualifying monomorph instantiations, with the `.mfb`
    /// `body` as the fallback.
    pub(crate) const fn mfb_with_fast_path(
        name: &'static str,
        doc_slug: &'static str,
        doc_intro: &'static str,
        doc_desc: &'static str,
        errors: &'static [&'static str],
        overloads: &'static [BuiltinOverload],
        body: &'static str,
        fast_path: MfbFastPath,
    ) -> BuiltinFunction {
        Self::mfb_impl(
            name,
            doc_slug,
            doc_intro,
            doc_desc,
            errors,
            overloads,
            body,
            Some(fast_path),
        )
    }

    #[allow(clippy::too_many_arguments)]
    const fn mfb_impl(
        name: &'static str,
        doc_slug: &'static str,
        doc_intro: &'static str,
        doc_desc: &'static str,
        errors: &'static [&'static str],
        overloads: &'static [BuiltinOverload],
        body: &'static str,
        fast_path: Option<MfbFastPath>,
    ) -> BuiltinFunction {
        BuiltinFunction {
            name,
            doc_slug,
            doc_intro,
            doc_desc,
            errors,
            overloads,
            doc_example: "",
            implementation: Implementation::Mfb { body, fast_path },
            lowering: Lowering::Helper,
            flags: BuiltinFlags {
                internal_only: false,
                return_type_overloaded: false,
            },
        }
    }
}

/// The kind of a builtin type, so the registry can describe primitives, opaque
/// resource handles, records, and enums uniformly (`term` alone spans hard-coded
/// records `TermColor`/`TermSize` and source-companion enums).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TypeKind {
    Primitive,
    Opaque,
    Record,
    Enum,
}

/// A builtin type a package contributes to the type namespace.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BuiltinType {
    pub(crate) name: &'static str,
    pub(crate) kind: TypeKind,
    /// Record field `(name, type_name)` pairs; empty for non-records, matching
    /// the legacy `builtin_type_fields`.
    pub(crate) fields: &'static [(&'static str, &'static str)],
}

/// When a package's source companion is injected into the user project.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InjectionRule {
    /// Inject whenever the package is imported (the `package_source_glue!`
    /// default: `uses_package` gates on an import of the package name).
    WhenImported,
    /// Inject only when a package-specific predicate holds (`strings` gates on
    /// scalar-seam member usage). The predicate lives on the [`BuiltinResolver`].
    WhenUsed,
}

/// A package's source companion: the injection rule and the loader that parses
/// the companion `.mfb` source into an AST file.
#[derive(Clone, Copy)]
pub(crate) struct BuiltinSource {
    pub(crate) rule: InjectionRule,
    pub(crate) loader: fn() -> Result<crate::ast::AstFile, ()>,
}

impl std::fmt::Debug for BuiltinSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltinSource")
            .field("rule", &self.rule)
            .finish_non_exhaustive()
    }
}

/// Optional per-package resolution hooks for the behaviors `DefaultResolver`
/// cannot derive from static data: argument-dependent return types and
/// implementation selection, default padding that is not a plain trailing fill,
/// overload-target monomorphization (`encoding`, `vector`), and a custom
/// source-use predicate (`strings`).
///
/// Every method has a data-only default so a package supplies only the hooks it
/// needs. The custom-resolver letters (`H` datetime, `I` encoding, and any other
/// letter whose census `custom` column is nonzero) implement this; the parity
/// harness accepts a resolver so those letters reuse it.
pub(crate) trait BuiltinResolver: Sync {
    /// Argument-dependent return type. Default: not customised.
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        _name: &str,
        _arg_types: &[String],
    ) -> Option<String> {
        None
    }

    /// Argument-dependent implementation symbol. Default: not customised.
    fn implementation_name(
        &self,
        _module: &BuiltinModule,
        _name: &str,
        _arg_types: &[String],
    ) -> Option<String> {
        None
    }

    /// Argument-dependent default padding. Default: not customised, so the
    /// caller falls back to `DefaultResolver::default_padding`.
    fn default_padding(
        &self,
        _module: &BuiltinModule,
        _name: &str,
        _provided: usize,
    ) -> Option<Vec<(&'static str, &'static str)>> {
        None
    }

    /// Monomorph target for an overloaded call, using the argument types and the
    /// contextual expected type. `Ok(None)` when the callee is not this package's
    /// overloaded name; `Err(())` when a return-type overload cannot be resolved
    /// without an expected type (`encoding.utf8Encode` with no
    /// `List OF Byte`/`List OF Integer` context). Default: `Ok(None)`.
    fn resolve_overload_target(
        &self,
        _module: &BuiltinModule,
        _name: &str,
        _arg_types: &[String],
        _expected_type: Option<&str>,
    ) -> Result<Option<String>, ()> {
        Ok(None)
    }

    /// Custom source-companion use predicate for [`InjectionRule::WhenUsed`].
    /// Default: none, so `WhenImported` semantics apply.
    fn uses_source(
        &self,
        _module: &BuiltinModule,
        _project: &crate::ast::AstProject,
    ) -> Option<bool> {
        None
    }
}

/// One builtin package described as data.
#[derive(Clone, Copy)]
pub(crate) struct BuiltinModule {
    /// The package name as it appears in an `IMPORT`, e.g. `"bits"`.
    pub(crate) name: &'static str,
    /// Short one-line package summary (the package-level analogue of
    /// [`BuiltinFunction::doc_intro`]). Empty until authored.
    pub(crate) doc_intro: &'static str,
    /// Full package-overview description, authored in the renderer's Markdown
    /// subset (the package-level analogue of [`BuiltinFunction::doc_desc`]).
    /// Empty until authored.
    pub(crate) doc_desc: &'static str,
    pub(crate) functions: &'static [BuiltinFunction],
    pub(crate) types: &'static [BuiltinType],
    pub(crate) source: Option<BuiltinSource>,
    /// Optional custom-resolution hooks. `None` means the package is fully
    /// data-only and every question is answered by [`DefaultResolver`].
    pub(crate) resolver: Option<&'static (dyn BuiltinResolver + 'static)>,
}

impl std::fmt::Debug for BuiltinModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The resolver is a `dyn` trait object without `Debug`; render only that
        // it is present, and the data fields normally.
        f.debug_struct("BuiltinModule")
            .field("name", &self.name)
            .field("doc_intro", &self.doc_intro)
            .field("doc_desc", &self.doc_desc)
            .field("functions", &self.functions)
            .field("types", &self.types)
            .field("source", &self.source)
            .field("resolver", &self.resolver.map(|_| "<resolver>"))
            .finish()
    }
}

impl BuiltinModule {
    /// The function descriptor for a fully qualified call name, or `None`.
    pub(crate) fn function(&self, name: &str) -> Option<&BuiltinFunction> {
        self.functions.iter().find(|function| function.name == name)
    }
}

/// The data-only derivations shared by every package.
///
/// Each method takes the owning [`BuiltinModule`] and answers one question the
/// hand-written helpers answer today, deriving it from the descriptor with no
/// per-package code. A package with argument-dependent behavior overrides the
/// relevant question through its [`BuiltinResolver`]; these methods are the
/// fallback and the whole answer for data-only packages.
pub(crate) struct DefaultResolver;

impl DefaultResolver {
    /// Membership — the legacy `is_<pkg>_call`.
    pub(crate) fn contains(module: &BuiltinModule, name: &str) -> bool {
        module.function(name).is_some()
    }

    /// Arity range across the function's overloads — legacy `arity`.
    /// `min` is the smallest overload minimum, `max` the largest overload
    /// maximum. `None` for an unknown call.
    pub(crate) fn arity(module: &BuiltinModule, name: &str) -> Option<(usize, usize)> {
        let function = module.function(name)?;
        let min = function
            .overloads
            .iter()
            .map(BuiltinOverload::min_args)
            .min()?;
        let max = function
            .overloads
            .iter()
            .map(BuiltinOverload::max_args)
            .max()?;
        Some((min, max))
    }

    /// Per-position name spellings for named-argument binding — legacy
    /// `call_param_names`. A function with a single overload returns its
    /// per-position spellings; a function whose overloads disagree on positions
    /// returns `None` (its names live in `param_name_overloads`), matching the way
    /// legacy `call_param_names` returns `None` for such a call (`audio.openInput`).
    pub(crate) fn param_names(
        module: &BuiltinModule,
        name: &str,
    ) -> Option<Vec<Vec<&'static str>>> {
        let function = module.function(name)?;
        if function.overloads.len() != 1 {
            return None;
        }
        Some(
            function.overloads[0]
                .params
                .iter()
                .map(Parameter::name_spellings)
                .collect(),
        )
    }

    /// Per-overload canonical parameter-name lists — legacy
    /// `call_param_name_overloads`. Each entry is one overload's parameter names
    /// (canonical spelling only), so a call whose overloads place a name at
    /// different positions (`audio.openInput`, `net::connectTcp`) is described
    /// faithfully rather than merged. `None` for a single-overload or unknown call
    /// (its names live in `param_names`), matching legacy
    /// `call_param_name_overloads`.
    pub(crate) fn param_name_overloads(
        module: &BuiltinModule,
        name: &str,
    ) -> Option<Vec<Vec<&'static str>>> {
        let function = module.function(name)?;
        if function.overloads.len() <= 1 {
            return None;
        }
        Some(
            function
                .overloads
                .iter()
                .map(|overload| overload.params.iter().map(|param| param.name).collect())
                .collect(),
        )
    }

    /// The canonical overload's per-position expected type names — legacy
    /// `argument_types`. A zero-parameter call has nothing to type and returns
    /// `None`, matching the shared convention (`app.getMode`,
    /// `money.getRounding`): the machine table lists only functions with typed
    /// positional arguments.
    pub(crate) fn argument_types(module: &BuiltinModule, name: &str) -> Option<Vec<&'static str>> {
        let overload = module.function(name)?.overloads.first()?;
        if overload.params.is_empty() {
            return None;
        }
        Some(
            overload
                .params
                .iter()
                .map(|param| param.ty.name())
                .collect(),
        )
    }

    /// The fixed return type shared by every overload — legacy
    /// `call_return_type_name`. `None` if the return is argument-dependent
    /// (`ReturnType::Custom`) or the overloads disagree; such a call resolves
    /// through the package [`BuiltinResolver`] instead.
    pub(crate) fn return_type_name(module: &BuiltinModule, name: &str) -> Option<&'static str> {
        let function = module.function(name)?;
        let mut fixed: Option<&'static str> = None;
        for overload in function.overloads {
            let ReturnType::Fixed(type_name) = overload.return_type else {
                return None;
            };
            match fixed {
                None => fixed = Some(type_name),
                Some(seen) if seen == type_name => {}
                Some(_) => return None,
            }
        }
        fixed
    }

    /// The human-readable expected-argument rendering — legacy
    /// `expected_arguments`. Renders the canonical overload's parameter type
    /// names joined by `", "` (`"Integer, Integer"`), or `"()"` for a
    /// zero-parameter call, matching the shared convention (`app.getMode`,
    /// `money.getRounding`, `crypto.generateP256`, `datetime.now`). A function
    /// whose overloads need a bespoke phrasing supplies it through its resolver's
    /// error path.
    pub(crate) fn expected_arguments(module: &BuiltinModule, name: &str) -> Option<String> {
        let overload = module.function(name)?.overloads.first()?;
        if overload.params.is_empty() {
            return Some("()".to_string());
        }
        Some(
            overload
                .params
                .iter()
                .map(|param| param.ty.name())
                .collect::<Vec<_>>()
                .join(", "),
        )
    }

    /// The fixed implementation symbol — legacy `implementation_name` for
    /// packages with a single rewrite. `None` for `Implementation::Same` (no
    /// rewrite) or `Implementation::Custom` (argument-dependent, resolver-owned).
    pub(crate) fn implementation_name(module: &BuiltinModule, name: &str) -> Option<&'static str> {
        match module.function(name)?.implementation {
            Implementation::Rewrite(symbol) => Some(symbol),
            // `Mfb` members route to their `__pkg_<name>` monomorph via
            // `internal_name`, not a fixed rewrite symbol — like the `Same` they
            // replace, they carry no `implementation_name`.
            Implementation::Same
            | Implementation::Custom
            | Implementation::Native(_)
            | Implementation::Mfb { .. }
            | Implementation::Os { .. } => None,
        }
    }

    /// Default padding for omitted trailing arguments — legacy
    /// `default_argument_padding`. Returns the `(type_name, expr)` fills for the
    /// parameters past `provided`; empty when the call is already full or has no
    /// defaulted parameters.
    pub(crate) fn default_padding(
        module: &BuiltinModule,
        name: &str,
        provided: usize,
    ) -> Vec<(&'static str, &'static str)> {
        let Some(overload) = module.function(name).and_then(|f| f.overloads.first()) else {
            return Vec::new();
        };
        overload
            .params
            .iter()
            .skip(provided)
            .filter_map(|param| match param.default {
                DefaultValue::Fill { type_name, expr } => Some((type_name, expr)),
                DefaultValue::None | DefaultValue::Optional => None,
            })
            .collect()
    }

    /// Resolve a call against its `Fixed`-return overloads by exact argument-type
    /// match — the data-only equivalent of a package's `resolve_call`. Returns
    /// the matched overload's fixed return type, or `None` if no overload accepts
    /// these argument types. A call whose return is argument-dependent
    /// (`ReturnType::Custom`) is resolver-owned and is not answered here.
    pub(crate) fn resolve_call(
        module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<&'static str> {
        let function = module.function(name)?;
        for overload in function.overloads {
            if arg_types.len() < overload.min_args() || arg_types.len() > overload.max_args() {
                continue;
            }
            let matches = overload
                .params
                .iter()
                .zip(arg_types.iter())
                .all(|(param, actual)| param.ty.name() == actual);
            if matches {
                if let ReturnType::Fixed(return_type) = overload.return_type {
                    return Some(return_type);
                }
            }
        }
        None
    }
}

/// A deterministic, ordered collection of builtin package descriptors.
///
/// Backed by a static slice rather than a `HashMap` so iteration and lookup are
/// order-preserving and allocation-free; the overview keeps a `HashMap` on the
/// table only if measurement ever proves lookup cost matters. Lookup is by
/// module name and by fully qualified function name (`"bits.band"`).
pub(crate) struct BuiltinRegistry {
    modules: &'static [&'static BuiltinModule],
}

impl BuiltinRegistry {
    pub(crate) const fn new(modules: &'static [&'static BuiltinModule]) -> BuiltinRegistry {
        BuiltinRegistry { modules }
    }

    /// The descriptor modules in registration order.
    pub(crate) fn modules(&self) -> &'static [&'static BuiltinModule] {
        self.modules
    }

    /// The module with this package name, consulting modules in order (the
    /// first match wins, mirroring the dispatcher's package order).
    pub(crate) fn module(&self, name: &str) -> Option<&'static BuiltinModule> {
        self.modules
            .iter()
            .copied()
            .find(|module| module.name == name)
    }

    /// The `(module, function)` owning a fully qualified call name, or `None`.
    pub(crate) fn function(
        &self,
        qualified: &str,
    ) -> Option<(&'static BuiltinModule, &'static BuiltinFunction)> {
        self.modules.iter().copied().find_map(|module| {
            module
                .function(qualified)
                .map(|function| (module, function))
        })
    }

    /// The first duplicated package name, if any — two modules sharing a name
    /// make `module()` order-dependent and are an authoring error. `None` means
    /// the registry's module names are unique.
    pub(crate) fn duplicate_module_name(&self) -> Option<&'static str> {
        for (index, module) in self.modules.iter().enumerate() {
            if self.modules[..index]
                .iter()
                .any(|earlier| earlier.name == module.name)
            {
                return Some(module.name);
            }
        }
        None
    }

    /// The first fully qualified function name owned by more than one module —
    /// which would make `function()` order-dependent. `None` means every
    /// function name is claimed by at most one module.
    pub(crate) fn duplicate_function_name(&self) -> Option<&'static str> {
        let mut seen: Vec<&'static str> = Vec::new();
        for module in self.modules {
            for function in module.functions {
                if seen.contains(&function.name) {
                    return Some(function.name);
                }
                seen.push(function.name);
            }
        }
        None
    }
}

/// The production registry. Each letter B..AA appends its migrated package's
/// `&<PKG>` descriptor here; a package not yet listed is still served by its
/// legacy per-package helper (the `mod.rs` adapters fall back on a registry
/// miss). BB then deletes the legacy helpers the adapters fall back to.
///
/// Migrated so far: `app` (B), `crypto` (F), `audio` (C),
/// `fs` (K), `general` (L), `http` (M), `io` (N), `math` (P),
/// `money` (Q), `net` (R), `os` (S), `resource` (U), `strings` (V), `term` (W),
/// `testing` (X). (`bits` D, `collections` E, `csv` G, `datetime` H, `encoding` I,
/// `errorCode` J, `json` O, `regex` T, and `process` have since moved to the
/// clean-room registry `crate::codegen::registry` and are no longer held here.)
pub(crate) static REGISTRY: BuiltinRegistry = BuiltinRegistry::new(&[
    &crate::builtins::app::APP,
    &crate::builtins::astrings::ASTRINGS,
    // bits migrated to the clean-room registry (crate::codegen::registry).
    // collections migrated to the clean-room registry (crate::codegen::registry).
    // csv migrated to the clean-room registry (crate::codegen::registry).
    // crypto migrated to the clean-room registry (crate::codegen::registry).
    &crate::builtins::audio::AUDIO,
    // datetime migrated to the clean-room registry (crate::codegen::registry).
    // encoding migrated to the clean-room registry (crate::codegen::registry).
    // errorCode migrated to the clean-room registry (crate::codegen::registry).
    // io migrated to the clean-room registry (crate::codegen::registry).
    &crate::builtins::general::GENERAL,
    // fs migrated to the clean-room registry (crate::codegen::registry).
    // http migrated to the clean-room registry (crate::codegen::registry).
    &crate::builtins::resource::RESOURCE,
    &crate::builtins::strings::STRINGS,
    &crate::builtins::term::TERM,
    &crate::builtins::testing::TESTING,
    // math migrated to the clean-room registry (crate::codegen::registry).
    // money migrated to the clean-room registry (crate::codegen::registry).
    // net migrated to the clean-room registry (crate::codegen::registry).
    // os migrated to the clean-room registry (crate::codegen::registry).
    // process migrated to the clean-room registry (crate::codegen::registry).
    // thread migrated to the clean-room registry (crate::codegen::registry).
    // tls migrated to the clean-room registry (crate::codegen::registry).
    // vector migrated to the clean-room registry (crate::codegen::registry).
]);

/// The migration parity harness (plan-72).
///
/// A descriptor migration is correct only if the descriptor answers every
/// metadata question exactly as the hand-written legacy helpers do. This module
/// is the gate that proves it, per package: each letter B..AA authors its
/// package's `BuiltinModule`, wires a [`LegacySet`] to that package's real
/// legacy helper functions, and calls [`assert_parity`] over every call the
/// package owns. A resolver-backed package additionally supplies
/// [`ResolverSample`]s so the custom hooks (`H` datetime, `I` encoding, and any
/// letter whose census `custom` column is nonzero) are checked against the same
/// legacy answers.
///
/// **These parity tests are the migration gate. Do not delete them until the
/// legacy helpers themselves are gone in letter BB** — while both the descriptor
/// and the legacy free function exist, this harness is the only thing pinning
/// them equal.
#[cfg(test)]
pub(crate) mod parity {
    use super::*;

    /// A package's legacy helper functions, normalized to comparable return
    /// shapes. The required rows exist for every package; the optional rows are
    /// present only when the package has that facet (`argument_types`,
    /// `implementation_name`, `default_argument_padding`, per-overload param
    /// names, builtin type fields). A caller wraps its real helpers in closures
    /// that adapt their native signatures (e.g. `&'static str` →
    /// `String`) to these.
    pub(crate) struct LegacySet<'a> {
        pub is_call: &'a dyn Fn(&str) -> bool,
        pub arity: &'a dyn Fn(&str) -> Option<(usize, usize)>,
        pub param_names: &'a dyn Fn(&str) -> Option<Vec<Vec<&'static str>>>,
        pub return_type_name: &'a dyn Fn(&str) -> Option<&'static str>,
        /// Optional: a package whose `expected_arguments` uses a bespoke phrasing
        /// the descriptor's per-position types cannot render (`collections`'
        /// `"List OF T, Integer or Map OF K TO V, K"`) sets this to `None` and
        /// keeps its hand-authored strings.
        pub expected_arguments: Option<&'a dyn Fn(&str) -> Option<String>>,
        pub param_name_overloads: Option<&'a dyn Fn(&str) -> Option<Vec<Vec<&'static str>>>>,
        pub argument_types: Option<&'a dyn Fn(&str) -> Option<Vec<&'static str>>>,
        pub implementation_name: Option<&'a dyn Fn(&str) -> Option<&'static str>>,
        pub default_padding: Option<&'a dyn Fn(&str, usize) -> Vec<(&'static str, &'static str)>>,
        pub builtin_type_fields:
            Option<&'a dyn Fn(&str) -> Option<&'static [(&'static str, &'static str)]>>,
    }

    /// One argument-dependent probe for a resolver-backed package: a call, a
    /// concrete argument-type list, and the answers the legacy resolver gives
    /// for those arguments. `assert_parity` drives the module's
    /// [`BuiltinResolver`] with these and asserts equality.
    pub(crate) struct ResolverSample<'a> {
        pub call: &'a str,
        pub arg_types: &'a [&'a str],
        pub expected_return: Option<&'a str>,
        pub expected_impl: Option<&'a str>,
        pub expected_padding: Option<Vec<(&'static str, &'static str)>>,
        /// The contextual expected type driving a return-type overload
        /// (`encoding.utf8Encode`). Passed to `resolve_overload_target`.
        pub expected_type: Option<&'a str>,
        pub expected_overload_target: Option<&'a str>,
    }

    fn arg_type_vec(arg_types: &[&str]) -> Vec<String> {
        arg_types.iter().map(|s| s.to_string()).collect()
    }

    /// Assert the descriptor `module` answers the same as the `legacy` helper
    /// set for every call in `calls`, and — for a resolver-backed module — the
    /// same as the legacy resolver for every `resolver_samples` probe.
    pub(crate) fn assert_parity(
        module: &BuiltinModule,
        calls: &[&str],
        legacy: &LegacySet,
        resolver_samples: &[ResolverSample],
    ) {
        for &call in calls {
            assert_eq!(
                DefaultResolver::contains(module, call),
                (legacy.is_call)(call),
                "membership parity for {call}"
            );
            assert_eq!(
                DefaultResolver::arity(module, call),
                (legacy.arity)(call),
                "arity parity for {call}"
            );
            assert_eq!(
                DefaultResolver::param_names(module, call),
                (legacy.param_names)(call),
                "param-name parity for {call}"
            );
            // Data-only return type. A resolver-backed call's return is
            // argument-dependent and is checked through `resolver_samples`.
            if module.resolver.is_none() {
                assert_eq!(
                    DefaultResolver::return_type_name(module, call),
                    (legacy.return_type_name)(call),
                    "return-type parity for {call}"
                );
            }
            if let Some(expected_arguments) = legacy.expected_arguments {
                assert_eq!(
                    DefaultResolver::expected_arguments(module, call),
                    expected_arguments(call),
                    "expected-arguments parity for {call}"
                );
            }
            if let Some(overloads) = legacy.param_name_overloads {
                assert_eq!(
                    DefaultResolver::param_name_overloads(module, call),
                    overloads(call),
                    "param-name-overload parity for {call}"
                );
            }
            if let Some(argument_types) = legacy.argument_types {
                assert_eq!(
                    DefaultResolver::argument_types(module, call),
                    argument_types(call),
                    "argument-type parity for {call}"
                );
            }
            if let Some(implementation_name) = legacy.implementation_name {
                if module.resolver.is_none() {
                    assert_eq!(
                        DefaultResolver::implementation_name(module, call),
                        implementation_name(call),
                        "implementation-name parity for {call}"
                    );
                }
            }
            if let Some(default_padding) = legacy.default_padding {
                if module.resolver.is_none() {
                    let (_, max) = DefaultResolver::arity(module, call).unwrap_or((0, 0));
                    for provided in 0..=max {
                        assert_eq!(
                            DefaultResolver::default_padding(module, call, provided),
                            default_padding(call, provided),
                            "default-padding parity for {call} at {provided} args"
                        );
                    }
                }
            }
        }

        if let Some(builtin_type_fields) = legacy.builtin_type_fields {
            for ty in module.types {
                // An opaque/enum type carries no record fields; the descriptor
                // models that as an empty slice, the legacy helper as `None`.
                let descriptor_fields = (!ty.fields.is_empty()).then_some(ty.fields);
                assert_eq!(
                    descriptor_fields,
                    builtin_type_fields(ty.name),
                    "builtin-type-field parity for {}",
                    ty.name
                );
            }
        }

        // Resolver-backed argument-dependent answers.
        let resolver = module.resolver;
        for sample in resolver_samples {
            let resolver = resolver.expect("resolver samples require a module resolver");
            let arg_types = arg_type_vec(sample.arg_types);
            if let Some(expected) = sample.expected_return {
                assert_eq!(
                    resolver
                        .resolve_return_type(module, sample.call, &arg_types)
                        .as_deref(),
                    Some(expected),
                    "resolver return parity for {} {:?}",
                    sample.call,
                    sample.arg_types
                );
            }
            if let Some(expected) = sample.expected_impl {
                assert_eq!(
                    resolver
                        .implementation_name(module, sample.call, &arg_types)
                        .as_deref(),
                    Some(expected),
                    "resolver implementation parity for {} {:?}",
                    sample.call,
                    sample.arg_types
                );
            }
            if let Some(expected) = &sample.expected_padding {
                assert_eq!(
                    resolver
                        .default_padding(module, sample.call, sample.arg_types.len())
                        .as_ref(),
                    Some(expected),
                    "resolver padding parity for {} {:?}",
                    sample.call,
                    sample.arg_types
                );
            }
            if let Some(expected) = sample.expected_overload_target {
                assert_eq!(
                    resolver.resolve_overload_target(
                        module,
                        sample.call,
                        &arg_types,
                        sample.expected_type
                    ),
                    Ok(Some(expected.to_string())),
                    "resolver overload-target parity for {} {:?}",
                    sample.call,
                    sample.arg_types
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small data-only test module standing in for a real package, with two
    // functions: a fixed-arity `add(a, b)` and an `emit(value, opts?)` that has
    // an alias, an optional defaulted trailing argument, and a rewrite.
    const ADD: BuiltinFunction = BuiltinFunction {
        name: "t.add",
        doc_slug: "add",
        doc_intro: "",
        doc_desc: "",
        doc_example: "",
        errors: &[],
        overloads: &[BuiltinOverload {
            params: &[
                Parameter::required("a", "Integer"),
                Parameter::required("b", "Integer"),
            ],
            return_type: ReturnType::Fixed("Integer"),
        }],
        implementation: Implementation::Same,
        lowering: Lowering::Inline,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    };

    const EMIT: BuiltinFunction = BuiltinFunction {
        name: "t.emit",
        doc_slug: "emit",
        doc_intro: "",
        doc_desc: "",
        doc_example: "",
        errors: &[],
        overloads: &[BuiltinOverload {
            params: &[
                Parameter {
                    name: "value",
                    aliases: &["val"],
                    ty: ParameterType::Named("String"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "opts",
                    aliases: &[],
                    ty: ParameterType::Named("List OF Byte"),
                    default: DefaultValue::Fill {
                        type_name: "List OF Byte",
                        expr: "",
                    },
                },
            ],
            return_type: ReturnType::Fixed("String"),
        }],
        implementation: Implementation::Rewrite("__t_emit"),
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    };

    // A function whose return varies with arguments: no fixed return type.
    const PICK: BuiltinFunction = BuiltinFunction {
        name: "t.pick",
        doc_slug: "pick",
        doc_intro: "",
        doc_desc: "",
        doc_example: "",
        errors: &[],
        overloads: &[BuiltinOverload {
            params: &[Parameter::required("x", "Integer")],
            return_type: ReturnType::Custom,
        }],
        implementation: Implementation::Custom,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    };

    // One builtin type of each kind, so the registry describes primitives,
    // opaque handles, records (with fields), and enums uniformly.
    const TEST_TYPES: &[BuiltinType] = &[
        BuiltinType {
            name: "TCount",
            kind: TypeKind::Primitive,
            fields: &[],
        },
        BuiltinType {
            name: "THandle",
            kind: TypeKind::Opaque,
            fields: &[],
        },
        BuiltinType {
            name: "TPoint",
            kind: TypeKind::Record,
            fields: &[("x", "Integer"), ("y", "Integer")],
        },
        BuiltinType {
            name: "TMode",
            kind: TypeKind::Enum,
            fields: &[],
        },
    ];

    // A zero-argument function, to pin the shared zero-arg conventions
    // (`expected_arguments` → "()", `argument_types` → None).
    const NOW: BuiltinFunction = BuiltinFunction {
        name: "t.now",
        doc_slug: "now",
        doc_intro: "",
        doc_desc: "",
        doc_example: "",
        errors: &[],
        overloads: &[BuiltinOverload {
            params: &[],
            return_type: ReturnType::Fixed("Integer"),
        }],
        implementation: Implementation::Same,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    };

    const TEST_MODULE: BuiltinModule = BuiltinModule {
        name: "t",
        doc_intro: "",
        doc_desc: "",

        functions: &[ADD, EMIT, PICK, NOW],
        types: TEST_TYPES,
        // A real source loader (borrowed from `app`) so the source rule and
        // loader fields are exercised end to end without a bespoke stub.
        source: Some(BuiltinSource {
            rule: InjectionRule::WhenImported,
            loader: crate::builtins::app::source_file,
        }),
        resolver: None,
    };

    // A second data-only module, so registry lookup order and multi-module
    // behavior can be exercised.
    const OTHER_ADD: BuiltinFunction = BuiltinFunction {
        name: "u.add",
        doc_slug: "add",
        doc_intro: "",
        doc_desc: "",
        doc_example: "",
        errors: &[],
        overloads: &[BuiltinOverload {
            params: &[Parameter::required("a", "Integer")],
            return_type: ReturnType::Fixed("Integer"),
        }],
        implementation: Implementation::Same,
        lowering: Lowering::Inline,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    };

    const OTHER_MODULE: BuiltinModule = BuiltinModule {
        name: "u",
        doc_intro: "",
        doc_desc: "",

        functions: &[OTHER_ADD],
        types: &[],
        source: None,
        resolver: None,
    };

    #[test]
    fn contains_membership() {
        assert!(DefaultResolver::contains(&TEST_MODULE, "t.add"));
        assert!(DefaultResolver::contains(&TEST_MODULE, "t.emit"));
        assert!(!DefaultResolver::contains(&TEST_MODULE, "t.missing"));
        assert!(!DefaultResolver::contains(&TEST_MODULE, "other.add"));
    }

    #[test]
    fn arity_fixed_and_defaulted() {
        // Two required params: (2, 2).
        assert_eq!(DefaultResolver::arity(&TEST_MODULE, "t.add"), Some((2, 2)));
        // One required, one optional: (1, 2).
        assert_eq!(DefaultResolver::arity(&TEST_MODULE, "t.emit"), Some((1, 2)));
        assert_eq!(DefaultResolver::arity(&TEST_MODULE, "t.missing"), None);
    }

    #[test]
    fn param_names_with_aliases() {
        assert_eq!(
            DefaultResolver::param_names(&TEST_MODULE, "t.add"),
            Some(vec![vec!["a"], vec!["b"]])
        );
        // `value` carries an alias `val`; `opts` has none.
        assert_eq!(
            DefaultResolver::param_names(&TEST_MODULE, "t.emit"),
            Some(vec![vec!["value", "val"], vec!["opts"]])
        );
        assert_eq!(
            DefaultResolver::param_names(&TEST_MODULE, "t.missing"),
            None
        );
    }

    #[test]
    fn argument_type_list() {
        assert_eq!(
            DefaultResolver::argument_types(&TEST_MODULE, "t.add"),
            Some(vec!["Integer", "Integer"])
        );
        assert_eq!(
            DefaultResolver::argument_types(&TEST_MODULE, "t.emit"),
            Some(vec!["String", "List OF Byte"])
        );
        // Zero-argument call: nothing to type -> None (shared convention).
        assert_eq!(DefaultResolver::argument_types(&TEST_MODULE, "t.now"), None);
        assert_eq!(
            DefaultResolver::argument_types(&TEST_MODULE, "t.missing"),
            None
        );
    }

    #[test]
    fn fixed_return_resolution() {
        assert_eq!(
            DefaultResolver::return_type_name(&TEST_MODULE, "t.add"),
            Some("Integer")
        );
        assert_eq!(
            DefaultResolver::return_type_name(&TEST_MODULE, "t.emit"),
            Some("String")
        );
        // Argument-dependent return: no fixed answer, resolver-owned.
        assert_eq!(
            DefaultResolver::return_type_name(&TEST_MODULE, "t.pick"),
            None
        );
        assert_eq!(
            DefaultResolver::return_type_name(&TEST_MODULE, "t.missing"),
            None
        );
    }

    #[test]
    fn expected_arguments_rendering() {
        assert_eq!(
            DefaultResolver::expected_arguments(&TEST_MODULE, "t.add").as_deref(),
            Some("Integer, Integer")
        );
        assert_eq!(
            DefaultResolver::expected_arguments(&TEST_MODULE, "t.emit").as_deref(),
            Some("String, List OF Byte")
        );
        // Zero-argument call renders as "()" (shared convention).
        assert_eq!(
            DefaultResolver::expected_arguments(&TEST_MODULE, "t.now").as_deref(),
            Some("()")
        );
        assert_eq!(
            DefaultResolver::expected_arguments(&TEST_MODULE, "t.missing"),
            None
        );
    }

    #[test]
    fn implementation_name_rewrite_and_same() {
        // No rewrite → None (public name is the implementation).
        assert_eq!(
            DefaultResolver::implementation_name(&TEST_MODULE, "t.add"),
            None
        );
        // Fixed rewrite.
        assert_eq!(
            DefaultResolver::implementation_name(&TEST_MODULE, "t.emit"),
            Some("__t_emit")
        );
        // Custom (argument-dependent) → None, resolver-owned.
        assert_eq!(
            DefaultResolver::implementation_name(&TEST_MODULE, "t.pick"),
            None
        );
        assert_eq!(
            DefaultResolver::implementation_name(&TEST_MODULE, "t.missing"),
            None
        );
    }

    #[test]
    fn default_padding_by_provided_count() {
        // `add` has no defaults: never any padding.
        assert!(DefaultResolver::default_padding(&TEST_MODULE, "t.add", 2).is_empty());
        // `emit` with only `value` provided → pad the trailing `opts` default.
        assert_eq!(
            DefaultResolver::default_padding(&TEST_MODULE, "t.emit", 1),
            vec![("List OF Byte", "")]
        );
        // `emit` fully supplied → no padding.
        assert!(DefaultResolver::default_padding(&TEST_MODULE, "t.emit", 2).is_empty());
        // Unknown call → no padding.
        assert!(DefaultResolver::default_padding(&TEST_MODULE, "t.missing", 0).is_empty());
    }

    fn types(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolve_call_exact_argument_match() {
        // Exact-match resolution returns the matched overload's fixed return.
        assert_eq!(
            DefaultResolver::resolve_call(&TEST_MODULE, "t.add", &types(&["Integer", "Integer"])),
            Some("Integer")
        );
        // Zero-arg function resolves with no arguments.
        assert_eq!(
            DefaultResolver::resolve_call(&TEST_MODULE, "t.now", &[]),
            Some("Integer")
        );
        // `emit` accepts its required arg alone (the optional trailing arg omitted).
        assert_eq!(
            DefaultResolver::resolve_call(&TEST_MODULE, "t.emit", &types(&["String"])),
            Some("String")
        );
        // Wrong argument type / arity / name -> None.
        assert_eq!(
            DefaultResolver::resolve_call(&TEST_MODULE, "t.add", &types(&["Integer", "String"])),
            None
        );
        assert_eq!(
            DefaultResolver::resolve_call(&TEST_MODULE, "t.add", &types(&["Integer"])),
            None
        );
        assert_eq!(
            DefaultResolver::resolve_call(&TEST_MODULE, "t.now", &types(&["Integer"])),
            None
        );
        assert_eq!(
            DefaultResolver::resolve_call(&TEST_MODULE, "t.missing", &[]),
            None
        );
        // A `Custom`-return call is resolver-owned, not answered here.
        assert_eq!(
            DefaultResolver::resolve_call(&TEST_MODULE, "t.pick", &types(&["Integer"])),
            None
        );
    }

    #[test]
    fn unresolved_calls_are_none() {
        // Every derivation returns None/empty for a name the module does not own.
        assert!(!DefaultResolver::contains(&TEST_MODULE, "t.nope"));
        assert_eq!(DefaultResolver::arity(&TEST_MODULE, "t.nope"), None);
        assert_eq!(DefaultResolver::param_names(&TEST_MODULE, "t.nope"), None);
        assert_eq!(
            DefaultResolver::argument_types(&TEST_MODULE, "t.nope"),
            None
        );
        assert_eq!(
            DefaultResolver::return_type_name(&TEST_MODULE, "t.nope"),
            None
        );
        assert_eq!(
            DefaultResolver::expected_arguments(&TEST_MODULE, "t.nope"),
            None
        );
        assert_eq!(
            DefaultResolver::implementation_name(&TEST_MODULE, "t.nope"),
            None
        );
        assert!(DefaultResolver::default_padding(&TEST_MODULE, "t.nope", 0).is_empty());
    }

    // ---- A2: registry shell -------------------------------------------------

    static TEST_REGISTRY: BuiltinRegistry = BuiltinRegistry::new(&[&TEST_MODULE, &OTHER_MODULE]);

    #[test]
    fn registry_module_lookup_in_order() {
        assert_eq!(TEST_REGISTRY.modules().len(), 2);
        // Registration order is preserved.
        assert_eq!(TEST_REGISTRY.modules()[0].name, "t");
        assert_eq!(TEST_REGISTRY.modules()[1].name, "u");
        assert!(TEST_REGISTRY.module("t").is_some());
        assert!(TEST_REGISTRY.module("u").is_some());
    }

    #[test]
    fn registry_unknown_module_is_none() {
        assert!(TEST_REGISTRY.module("missing").is_none());
        assert!(TEST_REGISTRY.module("").is_none());
    }

    #[test]
    fn registry_function_lookup_by_qualified_name() {
        let (module, function) = TEST_REGISTRY
            .function("t.emit")
            .expect("t.emit is registered");
        assert_eq!(module.name, "t");
        assert_eq!(function.name, "t.emit");
        // A function owned by the second module resolves to it.
        let (module, function) = TEST_REGISTRY
            .function("u.add")
            .expect("u.add is registered");
        assert_eq!(module.name, "u");
        assert_eq!(function.name, "u.add");
    }

    #[test]
    fn registry_unknown_function_is_none() {
        assert!(TEST_REGISTRY.function("t.missing").is_none());
        assert!(TEST_REGISTRY.function("missing.add").is_none());
        assert!(TEST_REGISTRY.function("add").is_none());
    }

    #[test]
    fn registry_names_are_unique() {
        // The well-formed test registry has no duplicate module or function names.
        assert_eq!(TEST_REGISTRY.duplicate_module_name(), None);
        assert_eq!(TEST_REGISTRY.duplicate_function_name(), None);

        // A registry that lists a module name twice is flagged.
        static DUP_MODULES: BuiltinRegistry = BuiltinRegistry::new(&[&TEST_MODULE, &TEST_MODULE]);
        assert_eq!(DUP_MODULES.duplicate_module_name(), Some("t"));

        // Two distinct modules sharing a fully qualified function name are
        // flagged (constructed so the module names differ but a function collides).
        assert_eq!(COLLIDING_REGISTRY.duplicate_function_name(), Some("t.add"));
    }

    // A module whose name differs from `t` but which re-declares `t.add`,
    // producing a fully qualified function-name collision across modules.
    const COLLIDING_MODULE: BuiltinModule = BuiltinModule {
        name: "t2",
        doc_intro: "",
        doc_desc: "",

        functions: &[ADD],
        types: &[],
        source: None,
        resolver: None,
    };
    static COLLIDING_REGISTRY: BuiltinRegistry =
        BuiltinRegistry::new(&[&TEST_MODULE, &COLLIDING_MODULE]);

    #[test]
    fn production_registry_holds_migrated_packages() {
        // Migrated packages are registered and resolvable by module name and by
        // qualified function name. As of plan-72-Y/Z/AA (thread, tls, vector) the
        // LAST three packages are migrated, so the registry is now COMPLETE — every
        // builtin package is present (28 as of plan-90-A). (This test tracked a
        // still-unmigrated example — `math` until plan-72-P, `regex` until -T,
        // `tls` until -Z — but none remains, so it now asserts completeness.)
        assert!(REGISTRY.module("app").is_some());
        assert!(REGISTRY.function("app.setMode").is_some());
        // `vector` migrated to the clean-room registry (`crate::codegen::registry`) and
        // is no longer held here.
        assert!(REGISTRY.module("vector").is_none());
        assert!(REGISTRY.function("vector.length").is_none());
        // plan-89-A: the `astrings` package (opaque AttributedString + fromString).
        assert!(REGISTRY.module("astrings").is_some());
        assert!(REGISTRY.function("astrings.fromString").is_some());
        // bits / csv / json / regex / process / datetime / encoding / collections /
        // money / os have migrated onto the clean-room registry (`crate::codegen::registry`)
        // and are no longer held here.
        assert!(REGISTRY.module("bits").is_none());
        assert!(REGISTRY.module("csv").is_none());
        assert!(REGISTRY.module("json").is_none());
        assert!(REGISTRY.module("regex").is_none());
        assert!(REGISTRY.module("process").is_none());
        assert!(REGISTRY.module("datetime").is_none());
        assert!(REGISTRY.module("encoding").is_none());
        assert!(REGISTRY.module("collections").is_none());
        assert!(REGISTRY.module("money").is_none());
        assert!(REGISTRY.module("os").is_none());
        // `fs` / `io` / `errorCode` / `crypto` / `tls` / `thread` have migrated onto the
        // clean-room registry too.
        assert!(REGISTRY.module("fs").is_none());
        assert!(REGISTRY.module("io").is_none());
        assert!(REGISTRY.module("errorCode").is_none());
        assert!(REGISTRY.module("crypto").is_none());
        assert!(REGISTRY.module("tls").is_none());
        assert!(REGISTRY.module("thread").is_none());
        // `math` / `vector` migrated to the clean-room registry too.
        assert!(REGISTRY.module("math").is_none());
        assert!(REGISTRY.module("vector").is_none());
        // `net` / `http` migrated to the clean-room registry too.
        assert!(REGISTRY.module("net").is_none());
        assert!(REGISTRY.module("http").is_none());
        // The 28 builtin packages minus the migrated ones.
        assert_eq!(REGISTRY.modules().len(), 8);
        // The registry's names stay unique across every appended package.
        assert_eq!(REGISTRY.duplicate_module_name(), None);
        assert_eq!(REGISTRY.duplicate_function_name(), None);
    }

    #[test]
    fn doc_intro_within_cap() {
        // `doc_intro` is a short intro line, capped at 1024 bytes. The bound is a
        // registry-wide invariant enforced here rather than by the type, so the
        // field can stay a plain `&'static str`. Empty (unauthored) entries pass.
        for module in REGISTRY.modules() {
            for function in module.functions {
                assert!(
                    function.doc_intro.len() <= 1024,
                    "{} doc_intro is {} bytes, exceeds the 1024-byte cap",
                    function.name,
                    function.doc_intro.len()
                );
            }
        }
    }

    #[test]
    fn descriptor_fields_are_well_formed() {
        // Read the facets not on the resolution path (doc_slug, lowering, flags,
        // builtin types, source) so their invariants are asserted and they are
        // live in the test build.
        for module in TEST_REGISTRY.modules() {
            for function in module.functions {
                assert!(!function.doc_slug.is_empty(), "{}", function.name);
                assert!(matches!(
                    function.lowering,
                    Lowering::Helper | Lowering::Inline
                ));
                assert!(!function.flags.internal_only);
                assert!(!function.flags.return_type_overloaded);
                assert!(!function.overloads.is_empty(), "{}", function.name);
                // Documentation/contract facets (doc surface + plan-88 `errors`):
                // off the resolution path, so read them here to keep them live.
                let _ = (function.doc_intro, function.doc_desc, function.errors);
            }
        }

        // Builtin types of each kind, with record fields populated only on the
        // record.
        let point = TEST_MODULE
            .types
            .iter()
            .find(|ty| ty.name == "TPoint")
            .expect("TPoint present");
        assert_eq!(point.kind, TypeKind::Record);
        assert_eq!(point.fields, &[("x", "Integer"), ("y", "Integer")]);
        assert!(TEST_MODULE
            .types
            .iter()
            .any(|ty| ty.kind == TypeKind::Primitive));
        assert!(TEST_MODULE
            .types
            .iter()
            .any(|ty| ty.kind == TypeKind::Opaque));
        assert!(TEST_MODULE.types.iter().any(|ty| ty.kind == TypeKind::Enum));

        // The source rule and loader are reachable and the loader parses.
        let source = TEST_MODULE.source.expect("test module has a source");
        assert_eq!(source.rule, InjectionRule::WhenImported);
        assert!((source.loader)().is_ok());
    }

    // ---- A3: parity harness exercising resolver callbacks ------------------
    //
    // A synthetic module with a two-overload function, an argument-type-dependent
    // return/implementation/padding, an overload-target monomorph, a record
    // builtin type, and a `WhenUsed` source — the shape the custom-resolver
    // letters (`H` datetime, `I` encoding) present. This proves the harness
    // drives a package's `BuiltinResolver` and asserts its answers against the
    // legacy resolver, and exercises the optional `LegacySet` facets `bits` lacks.

    struct SResolver;
    impl BuiltinResolver for SResolver {
        fn resolve_return_type(
            &self,
            _module: &BuiltinModule,
            name: &str,
            arg_types: &[String],
        ) -> Option<String> {
            if name != "s.pick" {
                return None;
            }
            Some(
                if arg_types.first().map(String::as_str) == Some("String") {
                    "String"
                } else {
                    "Integer"
                }
                .to_string(),
            )
        }

        fn implementation_name(
            &self,
            _module: &BuiltinModule,
            name: &str,
            arg_types: &[String],
        ) -> Option<String> {
            if name != "s.pick" {
                return None;
            }
            Some(if arg_types.len() >= 2 {
                "__s_pick2".to_string()
            } else {
                "__s_pick1".to_string()
            })
        }

        fn default_padding(
            &self,
            _module: &BuiltinModule,
            name: &str,
            provided: usize,
        ) -> Option<Vec<(&'static str, &'static str)>> {
            if name != "s.pick" {
                return None;
            }
            Some(if provided < 2 {
                vec![("Integer", "0")]
            } else {
                vec![]
            })
        }

        fn resolve_overload_target(
            &self,
            _module: &BuiltinModule,
            name: &str,
            arg_types: &[String],
            _expected_type: Option<&str>,
        ) -> Result<Option<String>, ()> {
            if name != "s.pick" {
                return Ok(None);
            }
            Ok(Some(if arg_types.len() >= 2 {
                "s.pick#1".to_string()
            } else {
                "s.pick#0".to_string()
            }))
        }

        fn uses_source(
            &self,
            _module: &BuiltinModule,
            _project: &crate::ast::AstProject,
        ) -> Option<bool> {
            Some(true)
        }
    }
    static S_RESOLVER: SResolver = SResolver;

    const S_OV0: &[Parameter] = &[Parameter::required("a", "Integer")];
    const S_OV1: &[Parameter] = &[
        Parameter::required("a", "Integer"),
        Parameter::required("b", "Integer"),
    ];
    const S_OVERLOADS: &[BuiltinOverload] = &[
        BuiltinOverload {
            params: S_OV0,
            return_type: ReturnType::Custom,
        },
        BuiltinOverload {
            params: S_OV1,
            return_type: ReturnType::Custom,
        },
    ];
    const S_PICK: BuiltinFunction = BuiltinFunction {
        name: "s.pick",
        doc_slug: "pick",
        doc_intro: "",
        doc_desc: "",
        doc_example: "",
        errors: &[],
        overloads: S_OVERLOADS,
        implementation: Implementation::Custom,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    };
    const S_TYPES: &[BuiltinType] = &[BuiltinType {
        name: "SPoint",
        kind: TypeKind::Record,
        fields: &[("x", "Integer"), ("y", "Integer")],
    }];
    const S_MODULE: BuiltinModule = BuiltinModule {
        name: "s",
        doc_intro: "",
        doc_desc: "",

        functions: &[S_PICK],
        types: S_TYPES,
        source: Some(BuiltinSource {
            rule: InjectionRule::WhenUsed,
            loader: crate::builtins::app::source_file,
        }),
        resolver: Some(&S_RESOLVER),
    };

    #[test]
    fn resolver_backed_descriptor_matches_legacy_resolver() {
        let legacy = parity::LegacySet {
            is_call: &|name| name == "s.pick",
            arity: &|name| (name == "s.pick").then_some((1, 2)),
            // `s.pick` is multi-overload, so its names live in
            // `param_name_overloads`; `param_names` is None (single-overload only).
            param_names: &|_| None,
            // Return type is resolver-owned, so this row is not asserted for a
            // resolver-backed module; supply the data-only default anyway.
            return_type_name: &|_| None,
            expected_arguments: Some(&|name| (name == "s.pick").then(|| "Integer".to_string())),
            param_name_overloads: Some(&|name| {
                (name == "s.pick").then(|| vec![vec!["a"], vec!["a", "b"]])
            }),
            argument_types: Some(&|name| (name == "s.pick").then(|| vec!["Integer"])),
            implementation_name: Some(&|_| None),
            default_padding: Some(&|_, _| Vec::new()),
            builtin_type_fields: Some(&|name| match name {
                "SPoint" => Some(&[("x", "Integer"), ("y", "Integer")][..]),
                _ => None,
            }),
        };
        let samples = [
            parity::ResolverSample {
                call: "s.pick",
                arg_types: &["Integer"],
                expected_return: Some("Integer"),
                expected_impl: Some("__s_pick1"),
                expected_padding: Some(vec![("Integer", "0")]),
                expected_type: None,
                expected_overload_target: Some("s.pick#0"),
            },
            parity::ResolverSample {
                call: "s.pick",
                arg_types: &["Integer", "Integer"],
                expected_return: Some("Integer"),
                expected_impl: Some("__s_pick2"),
                expected_padding: Some(vec![]),
                expected_type: None,
                expected_overload_target: Some("s.pick#1"),
            },
        ];
        parity::assert_parity(&S_MODULE, &["s.pick"], &legacy, &samples);

        // The `WhenUsed` source rule and its custom use predicate are reachable.
        let project = crate::ast::AstProject {
            name: String::new(),
            files: Vec::new(),
        };
        assert_eq!(
            S_MODULE.source.expect("s has source").rule,
            InjectionRule::WhenUsed
        );
        assert_eq!(S_RESOLVER.uses_source(&S_MODULE, &project), Some(true));
    }

    // ---- Coverage: const constructors invoked at runtime -------------------

    #[test]
    fn parameter_required_constructor_runtime() {
        // `Parameter::required` is a `const fn` used only in const contexts by the
        // module fixtures; exercise it at runtime so its body is covered.
        let p = Parameter::required("x", "Integer");
        assert_eq!(p.name, "x");
        assert!(p.aliases.is_empty());
        // `ParameterType`/`DefaultValue` derive `PartialEq` — assert_eq on them.
        assert_eq!(p.ty, ParameterType::Named("Integer"));
        assert_eq!(p.default, DefaultValue::None);
        // `ParameterType::name` accessor and the aliasless `name_spellings`.
        assert_eq!(p.ty.name(), "Integer");
        assert_eq!(p.name_spellings(), vec!["x"]);
    }

    #[test]
    fn builtin_registry_new_at_runtime() {
        // `BuiltinRegistry::new` is a `const fn` used only by static REGISTRY /
        // TEST_REGISTRY initializers; call it at runtime to cover its body.
        let modules: &'static [&'static BuiltinModule] = &[&TEST_MODULE, &OTHER_MODULE];
        let reg = BuiltinRegistry::new(modules);
        assert_eq!(reg.modules().len(), 2);
        assert_eq!(reg.modules()[0].name, "t");
        assert_eq!(reg.modules()[1].name, "u");
    }

    // ---- Coverage: the two hand-written Debug impls ------------------------

    #[test]
    fn builtin_source_debug_is_non_exhaustive() {
        let source = TEST_MODULE.source.expect("test module has a source");
        let rendered = format!("{source:?}");
        assert!(rendered.contains("BuiltinSource"));
        assert!(rendered.contains("WhenImported"));
        // `finish_non_exhaustive` renders the trailing `..` (the loader fn is
        // deliberately omitted).
        assert!(rendered.contains(".."));
    }

    #[test]
    fn builtin_module_debug_renders_resolver_presence() {
        // A resolver-backed module renders the placeholder, exercising the
        // `.map(|_| "<resolver>")` closure and the Some arm.
        let with_resolver = format!("{S_MODULE:?}");
        assert!(with_resolver.contains("BuiltinModule"));
        assert!(with_resolver.contains("<resolver>"));
        assert!(with_resolver.contains("\"s\""));
        // A data-only module renders `None` for the absent resolver.
        let without = format!("{OTHER_MODULE:?}");
        assert!(without.contains("BuiltinModule"));
        assert!(without.contains("\"u\""));
        assert!(without.contains("None"));
    }

    // ---- Coverage: BuiltinResolver default (data-only) methods -------------

    struct BareResolver;
    impl BuiltinResolver for BareResolver {}

    #[test]
    fn resolver_default_methods_are_data_only() {
        // A resolver that overrides nothing exercises every default trait-method
        // body (the not-customised fallbacks), which `SResolver` never reaches
        // because it overrides them all.
        let r = BareResolver;
        let args = vec!["Integer".to_string()];
        assert_eq!(r.resolve_return_type(&TEST_MODULE, "t.add", &args), None);
        assert_eq!(r.implementation_name(&TEST_MODULE, "t.add", &args), None);
        assert_eq!(r.default_padding(&TEST_MODULE, "t.add", 0), None);
        assert_eq!(
            r.resolve_overload_target(&TEST_MODULE, "t.add", &args, Some("Integer")),
            Ok(None)
        );
        let project = crate::ast::AstProject {
            name: String::new(),
            files: Vec::new(),
        };
        assert_eq!(r.uses_source(&TEST_MODULE, &project), None);
    }

    // ---- Coverage: arity across overloads, param-name-overloads paths ------

    #[test]
    fn arity_spans_overload_extremes() {
        // `s.pick` has a 1-param and a 2-param overload; arity min/max span both,
        // covering the `max()?` fold across overloads.
        assert_eq!(DefaultResolver::arity(&S_MODULE, "s.pick"), Some((1, 2)));
        assert_eq!(DefaultResolver::arity(&S_MODULE, "s.missing"), None);
    }

    #[test]
    fn param_name_overloads_single_multi_and_missing() {
        // Multi-overload: per-overload canonical names, faithfully un-merged.
        assert_eq!(
            DefaultResolver::param_name_overloads(&S_MODULE, "s.pick"),
            Some(vec![vec!["a"], vec!["a", "b"]])
        );
        // Single-overload call → None (its names live in `param_names`).
        assert_eq!(
            DefaultResolver::param_name_overloads(&TEST_MODULE, "t.add"),
            None
        );
        // Unknown call → None via the `?` miss.
        assert_eq!(
            DefaultResolver::param_name_overloads(&TEST_MODULE, "t.missing"),
            None
        );
    }

    // ---- Coverage: return-type disagreement + multi-overload resolve_call --

    const MIXED_RET: BuiltinFunction = BuiltinFunction {
        name: "t.mixed",
        doc_slug: "mixed",
        doc_intro: "",
        doc_desc: "",
        doc_example: "",
        errors: &[],
        overloads: &[
            BuiltinOverload {
                params: &[Parameter::required("a", "Integer")],
                return_type: ReturnType::Fixed("Integer"),
            },
            BuiltinOverload {
                params: &[Parameter::required("a", "String")],
                return_type: ReturnType::Fixed("String"),
            },
        ],
        implementation: Implementation::Same,
        lowering: Lowering::Inline,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    };

    const MIXED_MODULE: BuiltinModule = BuiltinModule {
        name: "m",
        doc_intro: "",
        doc_desc: "",

        functions: &[MIXED_RET],
        types: &[],
        source: None,
        resolver: None,
    };

    #[test]
    fn return_type_name_disagreeing_overloads_is_none() {
        // Two Fixed overloads with DIFFERENT return types → no single fixed
        // answer (the `Some(_) => return None` arm).
        assert_eq!(
            DefaultResolver::return_type_name(&MIXED_MODULE, "t.mixed"),
            None
        );
        // A single Custom-return overload → None (the `else { return None }` arm).
        assert_eq!(
            DefaultResolver::return_type_name(&TEST_MODULE, "t.pick"),
            None
        );
        // Same-return overloads still resolve to the shared type.
        assert_eq!(
            DefaultResolver::return_type_name(&TEST_MODULE, "t.add"),
            Some("Integer")
        );
    }

    #[test]
    fn resolve_call_selects_the_matching_overload() {
        // Multi-overload dispatch by exact argument-type match.
        assert_eq!(
            DefaultResolver::resolve_call(&MIXED_MODULE, "t.mixed", &types(&["Integer"])),
            Some("Integer")
        );
        assert_eq!(
            DefaultResolver::resolve_call(&MIXED_MODULE, "t.mixed", &types(&["String"])),
            Some("String")
        );
        // No overload accepts a mismatched type.
        assert_eq!(
            DefaultResolver::resolve_call(&MIXED_MODULE, "t.mixed", &types(&["List OF Byte"])),
            None
        );
    }

    // ---- Coverage: argument_types / expected_arguments first-overload paths -

    #[test]
    fn argument_types_and_expected_arguments_paths() {
        assert_eq!(
            DefaultResolver::argument_types(&S_MODULE, "s.pick"),
            Some(vec!["Integer"])
        );
        // Zero-parameter overload → None (nothing to type).
        assert_eq!(DefaultResolver::argument_types(&TEST_MODULE, "t.now"), None);
        assert_eq!(
            DefaultResolver::argument_types(&TEST_MODULE, "t.missing"),
            None
        );
        assert_eq!(
            DefaultResolver::expected_arguments(&S_MODULE, "s.pick").as_deref(),
            Some("Integer")
        );
        assert_eq!(
            DefaultResolver::expected_arguments(&TEST_MODULE, "t.now").as_deref(),
            Some("()")
        );
        assert_eq!(
            DefaultResolver::expected_arguments(&TEST_MODULE, "t.missing"),
            None
        );
    }

    // ---- Coverage: SResolver's non-matching-name and String-arg branches ---

    #[test]
    fn sresolver_non_matching_name_falls_through_to_defaults() {
        let args = vec!["Integer".to_string()];
        assert_eq!(
            S_RESOLVER.resolve_return_type(&S_MODULE, "s.other", &args),
            None
        );
        assert_eq!(
            S_RESOLVER.implementation_name(&S_MODULE, "s.other", &args),
            None
        );
        assert_eq!(S_RESOLVER.default_padding(&S_MODULE, "s.other", 0), None);
        assert_eq!(
            S_RESOLVER.resolve_overload_target(&S_MODULE, "s.other", &args, None),
            Ok(None)
        );
    }

    #[test]
    fn sresolver_string_first_arg_returns_string() {
        // Drives the `"String"` return arm the parity samples never take.
        let args = vec!["String".to_string()];
        assert_eq!(
            S_RESOLVER
                .resolve_return_type(&S_MODULE, "s.pick", &args)
                .as_deref(),
            Some("String")
        );
    }

    // ---- Coverage: assert_parity data-only branches (resolver.is_none()) ----

    #[test]
    fn assert_parity_data_only_module_exercises_all_facets() {
        // A data-only (resolver-less) module drives the `module.resolver.is_none()`
        // TRUE branches of `assert_parity` (return-type, implementation-name, and
        // default-padding parity), which the resolver-backed test skips. Each
        // legacy closure delegates to `DefaultResolver` on the same module, so
        // every assertion is trivially satisfied while the parity code runs.
        let legacy = parity::LegacySet {
            is_call: &|name| DefaultResolver::contains(&TEST_MODULE, name),
            arity: &|name| DefaultResolver::arity(&TEST_MODULE, name),
            param_names: &|name| DefaultResolver::param_names(&TEST_MODULE, name),
            return_type_name: &|name| DefaultResolver::return_type_name(&TEST_MODULE, name),
            expected_arguments: Some(&|name| {
                DefaultResolver::expected_arguments(&TEST_MODULE, name)
            }),
            param_name_overloads: Some(&|name| {
                DefaultResolver::param_name_overloads(&TEST_MODULE, name)
            }),
            argument_types: Some(&|name| DefaultResolver::argument_types(&TEST_MODULE, name)),
            implementation_name: Some(&|name| {
                DefaultResolver::implementation_name(&TEST_MODULE, name)
            }),
            default_padding: Some(&|name, provided| {
                DefaultResolver::default_padding(&TEST_MODULE, name, provided)
            }),
            builtin_type_fields: Some(&|name| {
                TEST_MODULE
                    .types
                    .iter()
                    .find(|ty| ty.name == name)
                    .and_then(|ty| (!ty.fields.is_empty()).then_some(ty.fields))
            }),
        };
        parity::assert_parity(
            &TEST_MODULE,
            &["t.add", "t.emit", "t.now", "t.pick"],
            &legacy,
            &[],
        );
    }

    #[test]
    fn assert_parity_drives_every_resolver_sample_facet() {
        // A resolver-backed module with a sample carrying all four Some facets
        // drives the resolver-sample asserts (return / implementation / padding /
        // overload-target) inside `assert_parity`.
        let legacy = parity::LegacySet {
            is_call: &|name| DefaultResolver::contains(&S_MODULE, name),
            arity: &|name| DefaultResolver::arity(&S_MODULE, name),
            param_names: &|name| DefaultResolver::param_names(&S_MODULE, name),
            return_type_name: &|_| None,
            expected_arguments: None,
            param_name_overloads: None,
            argument_types: None,
            implementation_name: None,
            default_padding: None,
            builtin_type_fields: None,
        };
        let samples = [parity::ResolverSample {
            call: "s.pick",
            arg_types: &["Integer"],
            expected_return: Some("Integer"),
            expected_impl: Some("__s_pick1"),
            expected_padding: Some(vec![("Integer", "0")]),
            expected_type: None,
            expected_overload_target: Some("s.pick#0"),
        }];
        parity::assert_parity(&S_MODULE, &["s.pick"], &legacy, &samples);
    }
}
