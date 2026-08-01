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
    /// An optional parameter padded with `(type_name, expr)`.
    Fill {
        type_name: &'static str,
        expr: &'static str,
    },
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

/// How a public call name maps to its implementation symbol.
///
/// `Same` means no rewrite — the public name *is* the implementation (the
/// legacy `implementation_name` for such packages returns `None`). `Rewrite`
/// carries a fixed internal symbol (encoding/regex/json/strings/net/csv rewrite
/// to a `__pkg_*` source body or native entry). Argument-type-dependent
/// selection (crypto's `_bytes`/`_text`, datetime by arity, vector monomorphs)
/// is `Custom` and resolved through a [`BuiltinResolver`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Implementation {
    /// No rewrite; the public name is the implementation.
    Same,
    /// A single fixed implementation symbol.
    Rewrite(&'static str),
    /// Argument-dependent; a [`BuiltinResolver`] selects it.
    Custom,
}

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
    pub(crate) overloads: &'static [BuiltinOverload],
    pub(crate) implementation: Implementation,
    pub(crate) lowering: Lowering,
    pub(crate) flags: BuiltinFlags,
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

    /// Monomorph/override target for an overloaded call. Default: none.
    fn resolve_overload_target(
        &self,
        _module: &BuiltinModule,
        _name: &str,
        _arg_types: &[String],
    ) -> Option<String> {
        None
    }

    /// Custom source-companion use predicate for [`InjectionRule::WhenUsed`].
    /// Default: none, so `WhenImported` semantics apply.
    fn uses_source(&self, _module: &BuiltinModule, _project: &crate::ast::AstProject) -> Option<bool> {
        None
    }
}

/// One builtin package described as data.
#[derive(Clone, Copy)]
pub(crate) struct BuiltinModule {
    /// The package name as it appears in an `IMPORT`, e.g. `"bits"`.
    pub(crate) name: &'static str,
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
    /// `call_param_names`. Uses the function's canonical (first) overload; a
    /// function whose overloads place a name at different positions is a
    /// `call_param_name_overloads` case and is resolved elsewhere.
    pub(crate) fn param_names(module: &BuiltinModule, name: &str) -> Option<Vec<Vec<&'static str>>> {
        let overload = module.function(name)?.overloads.first()?;
        Some(
            overload
                .params
                .iter()
                .map(Parameter::name_spellings)
                .collect(),
        )
    }

    /// The canonical overload's per-position expected type names — legacy
    /// `argument_types`.
    pub(crate) fn argument_types(module: &BuiltinModule, name: &str) -> Option<Vec<&'static str>> {
        let overload = module.function(name)?.overloads.first()?;
        Some(overload.params.iter().map(|param| param.ty.name()).collect())
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
    /// names joined by `", "` (`"Integer, Integer"`). A function whose overloads
    /// need a bespoke phrasing supplies it through its resolver's error path.
    pub(crate) fn expected_arguments(module: &BuiltinModule, name: &str) -> Option<String> {
        let overload = module.function(name)?.overloads.first()?;
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
            Implementation::Same | Implementation::Custom => None,
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
                DefaultValue::None => None,
            })
            .collect()
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

/// The production registry. Empty in plan-72-A: no package is migrated yet, so
/// every adapter in `mod.rs` that consults it falls back to the legacy
/// per-package helper. Each letter B..AA appends its `&<PKG>` here; BB then
/// deletes the legacy helpers the adapters fall back to.
pub(crate) static REGISTRY: BuiltinRegistry = BuiltinRegistry::new(&[]);

#[cfg(test)]
mod tests {
    use super::*;

    // A small data-only test module standing in for a real package, with two
    // functions: a fixed-arity `add(a, b)` and an `emit(value, opts?)` that has
    // an alias, an optional defaulted trailing argument, and a rewrite.
    const ADD: BuiltinFunction = BuiltinFunction {
        name: "t.add",
        doc_slug: "add",
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

    const TEST_MODULE: BuiltinModule = BuiltinModule {
        name: "t",
        functions: &[ADD, EMIT, PICK],
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
        assert_eq!(DefaultResolver::param_names(&TEST_MODULE, "t.missing"), None);
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
        assert_eq!(DefaultResolver::argument_types(&TEST_MODULE, "t.missing"), None);
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
        assert_eq!(DefaultResolver::return_type_name(&TEST_MODULE, "t.pick"), None);
        assert_eq!(DefaultResolver::return_type_name(&TEST_MODULE, "t.missing"), None);
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
        assert_eq!(DefaultResolver::expected_arguments(&TEST_MODULE, "t.missing"), None);
    }

    #[test]
    fn implementation_name_rewrite_and_same() {
        // No rewrite → None (public name is the implementation).
        assert_eq!(DefaultResolver::implementation_name(&TEST_MODULE, "t.add"), None);
        // Fixed rewrite.
        assert_eq!(
            DefaultResolver::implementation_name(&TEST_MODULE, "t.emit"),
            Some("__t_emit")
        );
        // Custom (argument-dependent) → None, resolver-owned.
        assert_eq!(DefaultResolver::implementation_name(&TEST_MODULE, "t.pick"), None);
        assert_eq!(DefaultResolver::implementation_name(&TEST_MODULE, "t.missing"), None);
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

    #[test]
    fn unresolved_calls_are_none() {
        // Every derivation returns None/empty for a name the module does not own.
        assert!(!DefaultResolver::contains(&TEST_MODULE, "t.nope"));
        assert_eq!(DefaultResolver::arity(&TEST_MODULE, "t.nope"), None);
        assert_eq!(DefaultResolver::param_names(&TEST_MODULE, "t.nope"), None);
        assert_eq!(DefaultResolver::argument_types(&TEST_MODULE, "t.nope"), None);
        assert_eq!(DefaultResolver::return_type_name(&TEST_MODULE, "t.nope"), None);
        assert_eq!(DefaultResolver::expected_arguments(&TEST_MODULE, "t.nope"), None);
        assert_eq!(DefaultResolver::implementation_name(&TEST_MODULE, "t.nope"), None);
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
        let (module, function) = TEST_REGISTRY.function("t.emit").expect("t.emit is registered");
        assert_eq!(module.name, "t");
        assert_eq!(function.name, "t.emit");
        // A function owned by the second module resolves to it.
        let (module, function) = TEST_REGISTRY.function("u.add").expect("u.add is registered");
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
        static DUP_MODULES: BuiltinRegistry =
            BuiltinRegistry::new(&[&TEST_MODULE, &TEST_MODULE]);
        assert_eq!(DUP_MODULES.duplicate_module_name(), Some("t"));

        // Two distinct modules sharing a fully qualified function name are
        // flagged (constructed so the module names differ but a function collides).
        assert_eq!(COLLIDING_REGISTRY.duplicate_function_name(), Some("t.add"));
    }

    // A module whose name differs from `t` but which re-declares `t.add`,
    // producing a fully qualified function-name collision across modules.
    const COLLIDING_MODULE: BuiltinModule = BuiltinModule {
        name: "t2",
        functions: &[ADD],
        types: &[],
        source: None,
        resolver: None,
    };
    static COLLIDING_REGISTRY: BuiltinRegistry =
        BuiltinRegistry::new(&[&TEST_MODULE, &COLLIDING_MODULE]);

    #[test]
    fn production_registry_is_empty_in_letter_a() {
        // No package is migrated in A, so the production registry is empty and
        // the mod.rs adapters always fall back to legacy helpers.
        assert!(REGISTRY.modules().is_empty());
        assert!(REGISTRY.module("bits").is_none());
        assert!(REGISTRY.function("bits.band").is_none());
    }

    #[test]
    fn descriptor_fields_are_well_formed() {
        // Read the facets not on the resolution path (doc_slug, lowering, flags,
        // builtin types, source) so their invariants are asserted and they are
        // live in the test build.
        for module in TEST_REGISTRY.modules() {
            for function in module.functions {
                assert!(!function.doc_slug.is_empty(), "{}", function.name);
                assert!(matches!(function.lowering, Lowering::Helper | Lowering::Inline));
                assert!(!function.flags.internal_only);
                assert!(!function.flags.return_type_overloaded);
                assert!(!function.overloads.is_empty(), "{}", function.name);
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
        assert!(TEST_MODULE.types.iter().any(|ty| ty.kind == TypeKind::Primitive));
        assert!(TEST_MODULE.types.iter().any(|ty| ty.kind == TypeKind::Opaque));
        assert!(TEST_MODULE.types.iter().any(|ty| ty.kind == TypeKind::Enum));

        // The source rule and loader are reachable and the loader parses.
        let source = TEST_MODULE.source.expect("test module has a source");
        assert_eq!(source.rule, InjectionRule::WhenImported);
        assert!((source.loader)().is_ok());
    }
}
