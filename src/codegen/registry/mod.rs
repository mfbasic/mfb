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
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// The context handed to a builder-driven `AbiLower` body (`AbiInline`/`AbiFunction`).
/// It bundles the OS-seam capabilities such a lowering may need — the DL/import
/// table, the target platform (per-OS emission + symbol/DL resolution), and the
/// build mode — so a body can reach them without the two distinct legacy
/// signatures. (Future: agnostic `dlopen`/`dlsym`/`load_const` helper methods hang
/// off this.)
pub(crate) struct AbiCtx<'a> {
    pub(crate) platform_imports: &'a std::collections::HashMap<String, String>,
    pub(crate) platform: &'a dyn crate::codegen::engine::types::CodegenPlatform,
    pub(crate) build_mode: crate::target::NativeBuildMode,
    /// The module (project) name — the build identity `os.resourcePath` bakes into
    /// its bundle/AppDir resource-base path. Threaded from the dispatch; empty (`""`)
    /// on the inline (`abi_inline`) path, which no resource-path member takes. Most
    /// abi bodies ignore it.
    pub(crate) module_name: &'a str,
    /// The runtime-call name being lowered (`audio.openInputDevice`,
    /// `datetime.nowNanos`) for an `abi_function` member. A member serving several IR-level
    /// overload-split code forms through one `abi_function` body (audio's
    /// `openInput`/`openInputDevice`, `read`/`readTimeout`, `close`/`closeInput`/
    /// `closeOutput`) selects its arm off this. Empty (`""`) on the inline
    /// (`abi_inline`) path, which lowers per call site and receives
    /// its target directly.
    pub(crate) call: &'a str,
    /// The arena offset of the TUI term-state slot, or `None` when the program
    /// uses no `term::` — the plan-35-B shadow-grid routing on `io.print`/`io.write`
    /// and the bug-149 cooked-mode restore on `io.readLine`/`io.input` consume it.
    /// Threaded from the dispatch so an `abi_function` OS-seam body (the migrated `io`
    /// members, plan-101) reaches this context. Carries the `ArenaLayout` value
    /// byte-for-byte (`Option<usize>`). Most abi bodies (crypto/bits) ignore it.
    pub(crate) term_state_offset: Option<usize>,
    /// The arena offset of the app presentation-mode slot, or `None` when the
    /// program is not an `--app` build. Read by the `app`/`term` `abi_function`
    /// bodies (presentation-mode load/store + the app-mode `ErrWrongMode` gate);
    /// carries the `ArenaLayout` value byte-for-byte. Most abi bodies ignore it.
    pub(crate) presentation_mode_offset: Option<usize>,
    /// The arena offset of the `canvas::` retained-scene region, or `None` when the
    /// program uses no `canvas::`. Read by the `canvas` `abi_function` bodies
    /// (`present`/`presentLayers`); carries the `ArenaLayout` value byte-for-byte.
    /// Every other abi body ignores it.
    /// The count of writable global slots (program globals + `LINK`/`FREE` pointer
    /// slots + `term::` state) the program uses — from `ArenaLayout::global_slots`.
    /// `thread.start` alone consumes it to size a spawned worker's arena block so its
    /// global-slot offsets match the main thread's (bug-369); every other abi body
    /// ignores it. `0` on the inline (`abi_inline`) path.
    pub(crate) arena_global_slots: usize,
    /// Whether the program uses the RNG (so a spawned worker must seed its per-thread
    /// RNG state). `thread.start` alone consumes it; every other abi body ignores it.
    /// `false` on the inline (`abi_inline`) path.
    pub(crate) uses_rng: bool,
}

/// A builder-driven **inline** lowering — the single sanctioned inline shape:
/// given the caller's `CodeBuilder`, the
/// call's **pre-lowered** `ValueResult` args (each carrying its
/// source `NirValue` for NIR-structural
/// analyses like bounds-check elision), and an [`AbiCtx`], emit the call inline and
/// return where the result value lives. The body reads its `ValueResult` args and
/// never re-lowers or frees them — the dispatch owns arg acquisition/lifetime.
pub(crate) type AbiInline =
    for<'a> fn(
        &mut crate::codegen::engine::builder::CodeBuilder<'a>,
        &[crate::codegen::engine::builder::ValueResult],
        &AbiCtx<'a>,
    ) -> Result<crate::codegen::engine::builder::ValueResult, String>;

/// A builder-driven **shared-function** lowering: the *same body shape* as
/// [`AbiInline`], but the wrapper binds its
/// args to the incoming ABI param registers and emits it once as a shared
/// `_mfb_rt_*` helper (emit-once, called by `bl` from every call site).
pub(crate) type AbiFunction =
    for<'a> fn(
        &mut crate::codegen::engine::builder::CodeBuilder<'a>,
        &[crate::codegen::engine::builder::ValueResult],
        &AbiCtx<'a>,
    ) -> Result<crate::codegen::engine::builder::ValueResult, String>;

/// A [`Body::Mfb`] member's optional native **fast path** — the plan-95
/// `target::shared::registry::MfbFastPath` shape. Given the builder, the
/// `#pkg_<name>$<TypeArgs>` monomorph target, and the call args, it either lowers
/// natively (`Ok(Some(_))`) or **declines** (`Ok(None)`), in which case the caller
/// instantiates the `.mfb` `body` instead. Selected by whether the monomorph
/// instantiation qualifies (a computed axis), so it rides on the `Mfb` body rather
/// than being its own realization kind or a second overload.
pub(crate) type MfbFastPath =
    for<'a> fn(
        &mut crate::codegen::engine::builder::CodeBuilder<'a>,
        &str,
        &[crate::target::shared::nir::NirValue],
    ) -> Result<Option<crate::codegen::engine::builder::ValueResult>, String>;

// A [`Parameter`]'s type is [`crate::types::ParameterType`] — the compiler-wide type
// vocabulary (see that module for why it lives outside `codegen`). Imported for the
// registry's own use; not re-exported, so callers name it as `crate::types::ParameterType`.
use crate::intern::Symbol;
use crate::types::ParameterType;

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

/// How one implementation is *realized* in codegen.
///
/// A member either carries an MFBASIC source body ([`Body::Mfb`]), a builder-driven
/// **inline** call-site lowering ([`Body::AbiInline`]), a builder-driven
/// **runtime-helper** lowering ([`Body::AbiFunction`]), a fixed internal rewrite
/// ([`Body::Rewrite`]), or a by-name [`Body::Intrinsic`].
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
    /// A builder-driven **inline** call-site lowering with **pre-lowered** args — the
    /// single sanctioned inline shape (the `bits`/collections/strings/math shape). The
    /// dispatch acquires each arg as a [`ValueResult`]
    /// for NIR-structural analyses like bounds-check elision) and the body combines them.
    AbiInline(AbiInline),
    /// A builder-driven **runtime-helper** lowering — the single sanctioned OS-seam /
    /// heavy shape (crypto/io/fs/datetime/audio/net). Wrapped once into a shared
    /// `_mfb_rt_*` helper and `bl`'d from every call site. `os_aliases` are the
    /// IR-level overload-split code forms (`connectTcpAddr`/`pollList`/audio's
    /// `openInputDevice`…) that share this member's body, selected off [`AbiCtx::call`];
    /// the aux→primary map is registry **data**, not a per-package dispatch branch.
    AbiFunction {
        lower: AbiFunction,
        os_aliases: &'static [&'static str],
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
    /// member is not a rewrite (an `AbiInline`/`AbiFunction`/`Intrinsic` lowering).
    /// Unifies the two rewrite forms — `Rewrite`'s fixed symbol and `Mfb`'s
    /// body-declared one — replacing the old per-package `implementation_name`.
    pub(crate) fn rewrite_target(&self) -> Option<&'static str> {
        match self {
            Body::Rewrite(symbol) => Some(symbol),
            Body::Mfb { rewrite, .. } => Some(rewrite),
            Body::AbiInline(_) | Body::AbiFunction { .. } | Body::Intrinsic => None,
        }
    }

    /// The single sanctioned **inline** lowering ([`Body::AbiInline`]): a builder-driven
    /// call-site lowering with **pre-lowered** args (the `bits`/collections/strings/math
    /// shape). Lowers on all targets; the call is emitted inline at each site.
    pub(crate) fn abi_inline(lower: AbiInline) -> Self {
        Body::AbiInline(lower)
    }

    /// The single sanctioned **runtime-helper** lowering ([`Body::AbiFunction`]): a
    /// builder-driven body wrapped once into a shared `_mfb_rt_*` helper and `bl`'d
    /// from each call site (crypto/io/fs/datetime shape). No overload-split aliases.
    pub(crate) fn abi_function(lower: AbiFunction) -> Self {
        Body::AbiFunction {
            lower,
            os_aliases: &[],
        }
    }

    /// An [`Body::AbiFunction`] lowering that ALSO serves the auxiliary runtime-call
    /// code forms named in `os_aliases` — the IR-level overload-split forms
    /// (`connectTcpAddr`/`pollList`; audio's `openInputDevice`/`readTimeout`/…) that
    /// share one member body, selected off [`AbiCtx::call`]. The aux→primary routing is
    /// registry data ([`abi_function_lower`]), not a per-package dispatch branch.
    pub(crate) fn abi_function_aliased(
        lower: AbiFunction,
        os_aliases: &'static [&'static str],
    ) -> Self {
        Body::AbiFunction { lower, os_aliases }
    }
}

/// When a [`RegistryHelper`] source chunk is injected into a program (plan-99). The
/// gated-injection facility that replaced the flat, always-on `helper_functions`:
/// most helpers are [`Always`](HelperGate::Always) (unconditional on import — the
/// pre-plan-99 behavior), but two shapes need a usage/cross-package gate the generic
/// on-import [`Registry::augment_project`] could not express — the `strings`
/// scalar-seam Unicode table and the `term`↔`astrings` bridge.
#[derive(Clone, Copy, Debug)]
pub(crate) enum HelperGate {
    /// Inject whenever the owning package's source is injected (i.e. the package is
    /// imported). Byte-identical to the pre-plan-99 unconditional `add_helper_functions`.
    Always,
    /// Inject only when the program references at least one of these **local**
    /// function names — the `strings` scalar-seam gate (`toScalars`/`isLetter`/…),
    /// which keeps the heavy Unicode general-category table out of a program that
    /// imports `strings` but calls no seam member (plan-41-D).
    WhenUsed(&'static [&'static str]),
    /// Inject only when the named package is **also** imported by the program,
    /// regardless of whether the OWNING package is imported — a cross-package bridge.
    /// The `strings` scalar seam an injected `astrings` companion calls needs it (an
    /// `astrings`-only program never imports `strings` in user source, yet its
    /// companion `IMPORT strings` + `strings::toScalars`). `other` is matched by raw
    /// import name, so it applies to a not-yet-migrated package that is absent from
    /// this registry.
    WhenImported(&'static str),
    /// Inject only when **both** named packages are imported by the program — a
    /// cross-package bridge whose body references the surface of both. The
    /// `term`↔`astrings` `drawText(AttributedString)` bridge needs it: its body
    /// references `term::` (so `term` must be imported) AND
    /// `AttributedString`/`astrings::` (so `astrings` must be imported). A plain
    /// [`WhenImported`](HelperGate::WhenImported) on either alone would over-inject the
    /// bridge into a program importing only one of the two, dragging in the other
    /// package's surface as dead code (the legacy `term::bridge_uses_package`
    /// required BOTH). Both names are matched by raw import name.
    WhenBothImported(&'static str, &'static str),
}

/// A source chunk (or ordering edge) the owning package contributes to an injected
/// program, gated by a [`HelperGate`] (plan-99). Deduplicated by [`name`](Self::name)
/// across all packages/gates, so a chunk reached through several packages is injected
/// exactly once. Exactly one of [`body`](Self::body) / [`import_name`](Self::import_name)
/// is `Some`:
///
/// - `body` — the injectable MFBASIC source chunk (`FUNC`/`TYPE`/`ENUM` text). An
///   [`Always`](HelperGate::Always) body renders **inline** in the owning package's
///   [`get_mfb`](RegistryPackage::get_mfb) (position preserved from the old
///   `helper_functions`); a gated body is injected as its own synthetic file.
/// - `import_name` — an ordering-dependency edge (no source): the named package's
///   source must be injected **before** this package's (the http-before-net /
///   crypto-before-encoding topological order that the hand-sequenced
///   `augment_project` chains used to encode).
#[derive(Clone, Debug)]
pub(crate) struct RegistryHelper {
    /// The dedup key — a helper with this `name` is injected at most once, no matter
    /// how many packages/gates reach it.
    pub(crate) name: &'static str,
    /// When to inject (see [`HelperGate`]).
    pub(crate) gate: HelperGate,
    /// The injectable source chunk, or `None` when this is an `import_name` ordering
    /// edge. XOR with [`import_name`](Self::import_name).
    pub(crate) body: Option<&'static str>,
    /// An ordering-dependency edge: the named package must be injected first. `None`
    /// for a source-bearing helper. XOR with [`body`](Self::body).
    // Kept for symmetry with `body` (the descriptor's two variants); no current
    // helper declares an ordering edge, so nothing reads it yet.
    #[allow(dead_code)]
    pub(crate) import_name: Option<&'static str>,
}

impl RegistryHelper {
    /// An [`Always`](HelperGate::Always) source chunk — the plan-99 replacement for a
    /// flat `add_helper_functions` entry (`__pkg_*` helper bodies). Renders inline in
    /// [`get_mfb`](RegistryPackage::get_mfb).
    pub(crate) fn always(name: &'static str, body: &'static str) -> Self {
        RegistryHelper {
            name,
            gate: HelperGate::Always,
            body: Some(body),
            import_name: None,
        }
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
    /// How this overload is realized in codegen.
    pub(crate) body: Body,
}

/// The concrete shape of a call site: the resolved argument types, in order. A
/// descriptor parameter's (possibly generic) [`ParameterType`] pattern is unified
/// against these concrete `ParameterType`s by [`RegistryFunction::select`]. The
/// compiler still hands the boundary type *names*; [`ParameterType::parse`] turns them
/// into `ParameterType`s so nothing inside the registry is a string.
pub(crate) struct CallShape {
    /// The call's concrete argument types, in order.
    pub(crate) args: Vec<ParameterType>,
}

/// The outcome of [`RegistryFunction::select`]: the chosen overload paired with the
/// concrete return type, formed by substituting the type variables bound while
/// unifying the call's arguments into that overload's (possibly generic) return type.
pub(crate) struct Selection<'a> {
    /// The selected overload.
    pub(crate) implementation: &'a Implementation,
    /// The overload's return type with all type variables resolved to concrete types.
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
    /// An optional hand-authored expected-argument phrasing for the
    /// argument-mismatch diagnostic. `Some` only when the per-position render the
    /// generic [`expected_arguments`] derives from the parameters cannot reproduce
    /// the intended phrasing — an argument union (`"List OF Byte or List OF
    /// Integer"`), a range (`"1 to 5 Integer"`), an optional-tail bracket
    /// (`"String, String[, Zone]"`), a zero-argument `"()"`, or a generic
    /// `"or"`-joined signature (`"List OF T, Integer or Map OF K TO V, K"`). `None`
    /// for the packages whose diagnostic equals the parameter-derived render
    /// (csv/json/regex), which keeps them byte-identical.
    pub(crate) expected_arguments: Option<&'static str>,
    /// Whether the member resolves **only** from toolchain-provided (`internal`)
    /// source — a native primitive the package's own injected companion calls that
    /// user source must never reach (`astrings`' `readSpans`/`writeSpans`/`scalarLen`
    /// opaque overlay bridge). Honored by `builtins::is_internal_only_call` (which
    /// gates it in `resolver::resolution` to non-`internal` files) and, implicitly, by
    /// the man docs (an internal member ships no man page, so no listing shows it).
    /// Default `false`.
    pub(crate) internal_only: bool,
    /// The function's implementations (`>= 1`); more than one means an overload set.
    pub(crate) implementations: Vec<Implementation>,
}

impl RegistryFunction {
    /// All overloads.
    pub(crate) fn implementations(&self) -> &[Implementation] {
        &self.implementations
    }

    /// STRICT overload resolution for **argument validation**: the first implementation
    /// whose arity and parameters [`unify`] *strictly* with the call's argument types —
    /// a scalar argument does NOT satisfy a nominal parameter (`String` ≠ `Named("Json")`).
    /// `None` when no overload accepts the call, which the type checker turns into an
    /// arity / argument-type error. Use this to answer "do these concrete args match?".
    pub(crate) fn resolve(&self, call: &CallShape) -> Option<Selection<'_>> {
        self.match_overload(call, true)
    }

    /// LENIENT overload resolution for **dispatch / return-type inference**: like
    /// [`resolve`](Self::resolve) but a scalar argument coarsely satisfies a nominal
    /// parameter. Used where a not-yet-resolved or nominally-spelled argument must not be
    /// rejected — overload dispatch (rewrite targets) and the return-type oracle that
    /// feeds IR lowering / codegen — because rejecting it there perturbs type propagation
    /// on valid programs. Validation strictness belongs on [`resolve`](Self::resolve).
    pub(crate) fn dispatch(&self, call: &CallShape) -> Option<Selection<'_>> {
        self.match_overload(call, false)
    }

    /// The overload this call resolves to: the first implementation whose arity and
    /// parameters [`unify`] with the call's argument types (binding any
    /// [`ParameterType::Var`] type variables), paired with the concrete return type from
    /// [`substitute`]-ing those bindings into the overload's return type. `strict` selects
    /// the argument-matching mode ([`resolve`](Self::resolve) vs [`dispatch`](Self::dispatch)).
    ///
    /// The registry's single overload-and-return resolver — exact-type matching is just
    /// the case with no type variables. `get(List OF T, Integer) AS T` called with
    /// `["List OF Integer", "Integer"]` selects the list overload, binds `T = Integer`,
    /// and reports the return type `Integer`, with no per-package resolver.
    fn match_overload(&self, call: &CallShape, strict: bool) -> Option<Selection<'_>> {
        for implementation in &self.implementations {
            let required = implementation
                .params
                .iter()
                .filter(|param| matches!(param.default, DefaultValue::None))
                .count();
            if call.args.len() < required || call.args.len() > implementation.params.len() {
                continue;
            }

            let mut bindings = BTreeMap::new();
            let unifies = implementation
                .params
                .iter()
                .zip(call.args.iter())
                .all(|(param, arg)| unify(&param.ty, arg, &mut bindings, strict));
            if !unifies {
                continue;
            }

            if let Some(return_type) = substitute(&implementation.return_type, &bindings) {
                return Some(Selection {
                    implementation,
                    return_type,
                });
            }
        }
        None
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
    /// Record-level documentation. When non-empty, [`render`](Self::render) emits a
    /// `DOC … END DOC` block (the record `DESC` plus each prop's
    /// [`description`](RecordProp::description) as a `PROP` line) before the `TYPE`,
    /// so a documented source record (e.g. crypto's `Sealed`/`KeyPair`) round-trips
    /// through `add_record` instead of living verbatim in a companion `.mfb`. Empty
    /// (the default for undocumented records) renders a bare `TYPE`, byte-identical
    /// to before this field existed.
    pub(crate) description: &'static str,
    /// The record's fields, in declaration order (`>= 1`).
    pub(crate) props: Vec<RecordProp>,
}

impl RegistryRecord {
    /// Render the `[EXPORT] TYPE … END TYPE` declaration (no trailing newline).
    fn render(&self) -> String {
        let mut out = String::new();
        // A documented record round-trips its `DOC` block; an undocumented one
        // (empty `description`) renders the bare `TYPE` exactly as before.
        if !self.description.is_empty() {
            out.push_str("DOC\n  TYPE ");
            out.push_str(self.name);
            out.push_str("\n  DESC ");
            out.push_str(self.description);
            for prop in &self.props {
                out.push_str("\n  PROP ");
                out.push_str(prop.name);
                if !prop.description.is_empty() {
                    out.push(' ');
                    out.push_str(prop.description);
                }
            }
            out.push_str("\nEND DOC\n");
        }
        if self.export {
            out.push_str("EXPORT ");
        }
        out.push_str("TYPE ");
        out.push_str(self.name);
        for prop in &self.props {
            out.push_str("\n  ");
            out.push_str(prop.name);
            out.push_str(" AS ");
            out.push_str(&source_spelling(&prop.ty));
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

/// One variant of a [`RegistryEnum`] — a bare value name (`StdOut`, `Kill`). Unlike a
/// [`UnionVariant`] (which names another declared type), an enum variant is a scalar
/// value whose ordinal is its declaration index.
#[derive(Clone, Debug)]
pub(crate) struct EnumVariant {
    /// The variant's value name (`StdOut`).
    pub(crate) name: &'static str,
    /// One-line documentation of the variant. Retained for doc generation; **not**
    /// rendered into the `ENUM` declaration [`RegistryPackage::get_mfb`] emits (the
    /// declaration is a bare list of variant names, as in a hand-written companion).
    pub(crate) description: &'static str,
    /// A compile-time advisory attached to the value: every user-authored source
    /// occurrence of `Enum.Variant` (an expression or a `MATCH` literal) reports the
    /// named `warn`-severity rule once, without rejecting the program. `None` for
    /// the ordinary variant. Injected builtin source (`HirFile::internal`) is
    /// exempt, so a package's own dispatch helpers never trip it.
    pub(crate) advisory: Option<EnumAdvisory>,
}

/// The advisory an [`EnumVariant`] carries — a `warn`-severity rule from
/// `crate::rules::RULES` plus the detail line the diagnostic renders under it.
/// The rule table owns the code and severity; this only names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EnumAdvisory {
    /// The rule name (`CRYPTO_SHA1_INSECURE`); must be a `Severity::Warn` row of
    /// `crate::rules::RULES`.
    pub(crate) rule: &'static str,
    /// The detail line rendered under the rule's one-line message.
    pub(crate) detail: &'static str,
}

/// A package value enum — an `[EXPORT] ENUM Name … END ENUM` declaration, e.g.
///
/// ```text
/// EXPORT ENUM Stream
///   StdOut
///   StdErr
/// END ENUM
/// ```
///
/// The twin of [`RegistryRecord`] / [`RegistryUnion`] for the third package value-type
/// kind. A variant's ordinal is its position, so [`variants`](Self::variants) order is
/// significant. Construct with a named struct literal; add via
/// [`RegistryPackage::add_enum`].
#[derive(Debug)]
pub(crate) struct RegistryEnum {
    /// The enum's type name (`Stream`).
    pub(crate) name: &'static str,
    /// Whether the enum is `EXPORT`ed (visible to importers) or package-internal.
    pub(crate) export: bool,
    /// The enum's variants, in declaration order (`>= 1`); order fixes each variant's
    /// ordinal value.
    pub(crate) variants: Vec<EnumVariant>,
}

impl RegistryEnum {
    /// Render the `[EXPORT] ENUM … END ENUM` declaration (no trailing newline).
    fn render(&self) -> String {
        let mut out = String::new();
        if self.export {
            out.push_str("EXPORT ");
        }
        out.push_str("ENUM ");
        out.push_str(self.name);
        for variant in &self.variants {
            out.push_str("\n  ");
            out.push_str(variant.name);
        }
        out.push_str("\nEND ENUM");
        out
    }
}

/// How one of a resource record's type-specific slots must move when the handle
/// is transferred to another thread (bug-464).
///
/// The distinction that matters is **who owns the memory the word points at**.
/// A pointer into a foreign heap (libssl's `malloc`, a refcounted
/// Core Foundation / dispatch object) is process-wide, so moving the word is
/// sound. A pointer into the *sender's arena* is not: arena state is per-thread
/// and no thread may free another's block, so the receiver would either free
/// into a foreign arena or read memory the sender's teardown already released.
/// Those must be copied into the receiver's arena and the fresh pointer stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotTransfer {
    /// Move the word verbatim: a foreign-heap pointer, a refcounted handle, or
    /// an inert scalar. Nothing in the receiver's arena depends on it.
    Verbatim,
    /// The word points at a fixed-size block in the **sender's arena**. Allocate
    /// `size` bytes in the receiver's arena, byte-copy, and store the new
    /// pointer. A null source pointer stays null.
    ///
    /// A byte copy is sufficient *because a transfer is a move*: the sender is
    /// tombstoned `moved|closed` and its cleanup deactivated, so any OS handles
    /// duplicated inside the block are released exactly once, by the receiver.
    ArenaBlock { size: usize },
    /// The word points at a NUL-terminated C string in the **sender's arena**.
    /// Measure it, allocate, copy including the terminator. A null stays null.
    ArenaCString,
}

/// Which backend a [`ResourceLiveSlot`] exists on.
///
/// A resource's record tail is **not** backend-uniform, and the same offset can
/// mean different things with different ownership: `tls::Socket`+40 is libssl's
/// `SSL*` (foreign heap) on OpenSSL, an SSPI block **in the arena** on Schannel,
/// and a dispatch queue on Network.framework. A flat offset list could not
/// express that, so each slot names the backend it belongs to and
/// `copy_resource_to_current_arena` selects by the target it is emitting for.
/// The three variants map 1:1 onto [`PlatformFamily`], which is how
/// `copy_resource_to_current_arena` selects. There is deliberately no "every
/// backend" variant: no resource needs one today, and every live slot that
/// exists is backend-specific. A resource with a platform-uniform tail should
/// add that variant when it arrives, rather than carrying an unused one now.
///
/// [`PlatformFamily`]: crate::codegen::engine::types::PlatformFamily
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotBackend {
    /// Linux (and any non-macOS, non-Windows target): the OpenSSL TLS backend.
    OpenSsl,
    /// Windows: the Schannel/SSPI TLS backend.
    Schannel,
    /// macOS: the Network.framework TLS backend.
    NetworkFramework,
}

/// One live word in a resource record **past the canonical header**
/// (tag @0, handle @8, closed @16, STATE @24), declared so the thread-transfer
/// copy can carry it (bug-464).
///
/// Before this existed, `copy_resource_to_current_arena` copied the header, then
/// unconditionally stored ZERO over every slot from 32 to 80 — those stores
/// reset `fs::File`'s write buffer and read cache on a move, but the routine is
/// type-agnostic and applied them to every resource. Any resource with live
/// state in its tail was therefore silently truncated to nulls on transfer,
/// which is the real reason `tls::Socket`/`tls::Listener` were fenced off behind
/// `sendable: false` rather than a product decision alone.
///
/// Declaring the slots here rather than special-casing types in the copy routine
/// is what makes that truncation structurally impossible for a resource added
/// later: a new handle with live tail state either declares its slots or is not
/// sendable, and `sendable` goes back to being an honest product decision.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResourceLiveSlot {
    /// Byte offset into the resource record. Must be >= 32 (past the header) and
    /// leave a whole word inside `RESOURCE_RECORD_SIZE_BYTES`.
    pub(crate) offset: usize,
    /// How the word moves.
    pub(crate) transfer: SlotTransfer,
    /// The backend whose record layout puts a live word here.
    pub(crate) backend: SlotBackend,
    /// What lives in the slot, for the reader of a layout table that otherwise
    /// reads as bare integers.
    pub(crate) what: &'static str,
}

/// A package resource type — an opaque handle (`File`, `Socket`, `Process`) whose
/// lifetime the RES ownership system tracks. Unlike a [`RegistryRecord`] /
/// [`RegistryUnion`] / [`RegistryEnum`], a resource has **no injectable source
/// declaration** (the handle is native), so it is not rendered by
/// [`RegistryPackage::get_mfb`]; it is a set of semantic facts the type checker and
/// codegen consult. Construct with a named struct literal; add via
/// [`RegistryPackage::add_resource`].
#[derive(Clone, Debug)]
pub(crate) struct RegistryResource {
    /// The resource type name (`File`, `Process`).
    pub(crate) name: &'static str,
    /// Whether the resource type is `EXPORT`ed (visible to importers) or
    /// package-internal.
    pub(crate) export: bool,
    /// One-line documentation of the handle. Shown on the man2 `types` page (a
    /// resource is opaque, so its page is just this description).
    pub(crate) description: &'static str,
    /// The qualified close op that releases the handle — the value the per-package
    /// `resource_close_function` returns (`fs.close`, `process.__drop`).
    pub(crate) close_function: &'static str,
    /// Whether the handle may cross a thread boundary (the RES "sendable to thread"
    /// bit — mirrors [`crate::codegen::resource::ResourceInfo::sendable`]).
    ///
    /// This is a **product decision** about whether moving the handle is
    /// meaningful, not a guard over the transfer copy — [`live_slots`] carries
    /// the record (bug-464). A resource that is not sendable must still declare
    /// its live slots, so that opting it in later is a one-line change that
    /// cannot silently truncate.
    ///
    /// [`live_slots`]: Self::live_slots
    pub(crate) sendable: bool,
    /// The live words in this resource's record **past the canonical header**,
    /// which the thread-transfer copy must carry (bug-464). Empty means the
    /// record is the header alone — an assertion, not an omission: an empty list
    /// says "everything live about this handle is tag/handle/closed/STATE", which
    /// is exactly why `fs::File`, `tcp::Socket` and `udp::Socket` were safely
    /// sendable all along.
    ///
    /// `fs::File` is the one deliberate exception: its tail *is* used (the
    /// plan-14-B write buffer and plan-14-C read cache), but those are a buffer
    /// and a cache that a move intentionally resets, so it declares no slots and
    /// keeps the zeroing behaviour it has always had.
    pub(crate) live_slots: &'static [ResourceLiveSlot],
    /// Whether the close op can fail (mirrors
    /// the drop-time cleanup derives the same fact from the close wrapper's
    /// `SUCCESS ON`).
    pub(crate) close_may_fail: bool,
    /// Provenance of the registration (`Builtin` for a native package resource).
    pub(crate) kind: crate::codegen::resource::ResourceKind,
}

/// A compile-time package **constant** that folds to a literal at every use site —
/// the registry twin of the hand tables `math::{is_math_constant,…}`,
/// `errorcode::{…}`, and `vector::{is_vector_constant,…}`. Two shapes:
///
/// * a **scalar** constant (`math.pi`, `errorCode.ErrNotFound`) sets [`value`](Self::value)
///   to its literal (`"3.14159…"`, `"77050004"`) and leaves [`components`](Self::components) `None`;
/// * a **record** constant (`vector.zeroFloat3`) leaves `value` `None` and sets
///   `components` to the ordered per-field literals (`["0.0", "0.0", "0.0"]`) that a
///   constructor of [`type_name`](Self::type_name) (`Float3`) inlines from — each
///   field's element type coming from the package's [`RegistryRecord`].
///
/// An `errorCode` scalar constant additionally carries the two error-**emission**
/// columns the codegen error path consults ([`message`](Self::message) /
/// [`symbol`](Self::symbol)): the human-readable message and the interned message
/// data-object symbol (`_mfb_str_error_*`). These are `Some` only for the `errorCode`
/// rows — the single authority the [`runtime_error`] / [`runtime_error_emission`] /
/// [`runtime_error_triple`] free fns scan by bare error name — and `None` for every
/// value constant (`math.pi`, `vector.zeroFloat3`), which never enters the emission
/// path.
///
/// Construct with a named struct literal; add via [`RegistryPackage::add_constant`].
#[derive(Clone, Debug)]
pub(crate) struct RegistryConstant {
    /// The constant's member name (unqualified), e.g. `"pi"` / `"zeroFloat3"`.
    pub(crate) name: &'static str,
    /// The type the constant evaluates to — a scalar type name (`"Float"`,
    /// `"Integer"`) for a scalar constant, or the record type (`"Float3"`) a record
    /// constant constructs.
    pub(crate) type_name: &'static str,
    /// The literal a **scalar** constant folds to (`"3.14159…"`); `None` for a record
    /// constant.
    pub(crate) value: Option<&'static str>,
    /// The ordered per-field literals a **record** constant inlines into a
    /// constructor of [`type_name`](Self::type_name); `None` for a scalar constant.
    pub(crate) components: Option<&'static [&'static str]>,
    /// The human-readable error **message** the codegen error-emission path emits for
    /// an `errorCode` constant (`"Requested item, key, file, or resource was not
    /// found."`); `None` for a value constant. Feeds [`runtime_error`] /
    /// [`runtime_error_triple`].
    pub(crate) message: Option<&'static str>,
    /// The interned message data-object **symbol** (`_mfb_str_error_not_found`) the
    /// fixed-runtime-helper error path references for an `errorCode` constant; `None`
    /// for a value constant. Feeds [`runtime_error_emission`] / [`runtime_error_triple`].
    pub(crate) symbol: Option<&'static str>,
}

/// A migrated package's **override** of an overridable general builtin (`toString`,
/// …) for one of its value types — the registry twin of the hand rows in
/// `builtins::general_override_target` (`toString(net.Url)` → `"__net_urlToString"`,
/// `toString(Float3)` → `"__vector_toString_float3"`). A general call `f(x)` whose
/// sole argument has `arg_type` routes to `helper` instead of the scalar builtin.
///
/// Construct with a named struct literal; add via [`RegistryPackage::add_override`].
#[derive(Clone, Debug)]
pub(crate) struct RegistryOverride {
    /// The overridable general builtin (`"toString"`).
    pub(crate) builtin: &'static str,
    /// The argument value type this override fires on (`"Float3"`, a `net.Url` id).
    pub(crate) arg_type: &'static str,
    /// The internal `__pkg_*` helper the call routes to (`"__net_urlToString"`).
    pub(crate) helper: &'static str,
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
    enums: Vec<RegistryEnum>,
    /// Opaque resource handle types. Semantic-only (not injectable source), so they are
    /// not rendered by [`get_mfb`](Self::get_mfb).
    resources: Vec<RegistryResource>,
    /// Shared source chunks (or ordering edges) the package contributes to an injected
    /// program, each gated by a [`HelperGate`] (plan-99). An [`Always`](HelperGate::Always)
    /// body renders inline in [`get_mfb`](Self::get_mfb) between the unions and the member
    /// bodies (the old `helper_functions` position); gated bodies are injected as their
    /// own synthetic files by [`Registry::augment_project`].
    helpers: Vec<RegistryHelper>,
    functions: Vec<RegistryFunction>,
    /// Value-type names (`EXPORT TYPE`/`ENUM`) a package declares **only** in its
    /// injected companion source (`package.mfb`) rather than as a modeled
    /// [`RegistryRecord`]/[`RegistryEnum`] — `datetime`'s `Instant`/`Date`/…/`ZoneKind`,
    /// whose `DOC`-block-carrying declarations and byte-exact formatting cannot be
    /// reproduced by [`get_mfb`](Self::get_mfb)'s renderers. Recorded as semantic-only
    /// facts so [`is_builtin_type`] / [`qualified_builtin_type`] recognize them without
    /// a per-package predicate; they are NOT rendered (the companion already declares
    /// them).
    source_types: Vec<&'static str>,
    /// Compile-time package constants (scalar folds + record-constructor inlines) the
    /// package owns — the registry home of the `math`/`errorcode`/`vector` constant
    /// hand tables. Queried by the [`is_package_constant`] / [`constant_type`] /
    /// [`constant_value`] / [`constant_components`] boundary fns.
    constants: Vec<RegistryConstant>,
    /// General-builtin overrides (`toString`, …) this package provides for its value
    /// types — the registry home of the `builtins::general_override_target` rows.
    /// Queried by [`general_override_target`].
    overrides: Vec<RegistryOverride>,
    /// Whether this package's members are **unqualified global** builtins — bare
    /// names (`expectEqual`, never `testing::expectEqual`) that carry no writable
    /// `IMPORT <pkg>` spelling. The package is registered under a real name only so it
    /// has a home in the registry; the member calls stay bare end-to-end. Set for the
    /// `testing` (and, later, `general`) migrations. Its one behavioral effect is
    /// documentation: `mfb man2 --all` skips such a package, because rendering a
    /// `# testing` / `testing::expect` page would advertise a spelling users cannot
    /// write. Defaults `false`; set via [`mark_unqualified_global`](Self::mark_unqualified_global).
    unqualified_global: bool,
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
            enums: Vec::new(),
            resources: Vec::new(),
            helpers: Vec::new(),
            functions: Vec::new(),
            source_types: Vec::new(),
            constants: Vec::new(),
            overrides: Vec::new(),
            unqualified_global: false,
        }
    }

    /// The package's import name, e.g. `"encoding"`.
    pub(crate) fn import_name(&self) -> &'static str {
        self.import_name
    }
    /// Whether this package is an **unqualified global** builtin package (see the
    /// [`unqualified_global`](Self::unqualified_global) field). `mfb man2 --all` skips
    /// packages for which this is true.
    pub(crate) fn is_unqualified_global(&self) -> bool {
        self.unqualified_global
    }
    /// Mark this package as an **unqualified global** builtin package — its members are
    /// bare names with no writable `IMPORT <pkg>` spelling, so `mfb man2 --all` skips
    /// its documentation page (see the field docs). Used by the `testing`/`general`
    /// migrations.
    pub(crate) fn mark_unqualified_global(&mut self) -> &mut Self {
        self.unqualified_global = true;
        self
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

    /// The package's enums, in declaration order.
    pub(crate) fn enums(&self) -> &[RegistryEnum] {
        &self.enums
    }

    /// The package's resource types, in declaration order.
    pub(crate) fn resources(&self) -> &[RegistryResource] {
        &self.resources
    }

    /// The package's gated helper chunks/edges, in the order added.
    pub(crate) fn helpers(&self) -> &[RegistryHelper] {
        &self.helpers
    }

    /// The bodies of this package's [`Always`](HelperGate::Always) helpers, in add
    /// order — the chunks that render inline in [`get_mfb`](Self::get_mfb) (the old
    /// `helper_functions`). Gated helpers (`WhenUsed`/`WhenImported`) are injected as
    /// separate files by [`Registry::augment_project`] and are excluded here.
    fn always_helper_bodies(&self) -> Vec<&'static str> {
        self.helpers
            .iter()
            .filter(|h| matches!(h.gate, HelperGate::Always))
            .filter_map(|h| h.body)
            .collect()
    }

    /// The package's source-declared value-type names (see [`source_types`](Self::source_types)).
    pub(crate) fn source_types(&self) -> &[&'static str] {
        &self.source_types
    }

    /// Whether `ast` imports this package — the generic replacement for the old
    /// per-package `uses_package`. A property of the *program being compiled*, so it
    /// takes the AST rather than being a stored flag.
    pub(crate) fn is_imported_by(&self, view: &ProjectView) -> bool {
        view.imports(self.import_name)
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
    /// 4. every [`RegistryEnum`] as an `[EXPORT] ENUM … END ENUM` block (in `add_enum`
    ///    order);
    /// 5. the [`helper_functions`](Self::helper_functions) — the shared `__pkg_*`
    ///    helpers the members call, in the order added;
    /// 6. every [`Body::Mfb`] body across all functions (each overload counts — a
    ///    same-named native-overload set like `encoding::utf8Encode` emits one `FUNC`
    ///    per overload), in registration order.
    ///
    /// Bodies keep their raw `__pkg_name` spelling; internalization to `#pkg_name`
    /// happens later when the assembled text is parsed.
    ///
    /// Returns the **empty string** only when the package has *nothing* to inject —
    /// no records, no unions, no enums, no `Mfb` member, and no helper functions.
    /// Records, unions, and enums are injectable source in their own right (a package
    /// whose functions are all `Native`/`Rewrite` still emits its `TYPE`/`UNION`/`ENUM`
    /// declarations here), and a
    /// helper-function chunk is likewise standalone injectable source — the native
    /// `process` package carries only its `Stream`/`Signal` `EXPORT ENUM` companion as
    /// a helper chunk, with no records, unions, or `Mfb` bodies. Pieces are
    /// separated by a blank line and the result ends with a newline, so the output is
    /// directly parseable.
    pub(crate) fn get_mfb(&self) -> String {
        let helper_bodies = self.always_helper_bodies();
        let bodies: Vec<&str> = self
            .functions
            .iter()
            .flat_map(|f| f.implementations.iter())
            .filter_map(|imp| match &imp.body {
                Body::Mfb { body, .. } => Some(body.trim_end()),
                _ => None,
            })
            .collect();
        if self.records.is_empty()
            && self.unions.is_empty()
            && self.enums.is_empty()
            && bodies.is_empty()
            && helper_bodies.is_empty()
        {
            return String::new();
        }

        let mut pieces: Vec<String> = Vec::with_capacity(
            1 + self.records.len()
                + self.unions.len()
                + self.enums.len()
                + helper_bodies.len()
                + bodies.len(),
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
        pieces.extend(self.enums.iter().map(RegistryEnum::render));
        pieces.extend(
            helper_bodies
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

    /// Add a gated helper chunk/edge (plan-99). Additive — later calls append. An
    /// [`Always`](HelperGate::Always) body renders into [`get_mfb`](Self::get_mfb)
    /// between the unions and the member bodies (the old `add_helper_functions`
    /// position); a gated body (`WhenUsed`/`WhenImported`) and any `import_name`
    /// ordering edge are consumed by [`Registry::augment_project`].
    pub(crate) fn add_helper(&mut self, helper: RegistryHelper) -> &mut Self {
        self.helpers.push(helper);
        self
    }

    /// Record the package's **source-declared value-type** names (see the
    /// [`source_types`](Self::source_types) field). Additive; the names are semantic-only
    /// (recognized by [`is_builtin_type`] / [`qualified_builtin_type`]) and are NOT
    /// rendered by [`get_mfb`](Self::get_mfb) — the companion source already declares them.
    pub(crate) fn add_source_types(&mut self, names: &[&'static str]) -> &mut Self {
        self.source_types.extend_from_slice(names);
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

    /// Add a value enum (a `RegistryEnum { … }`). Enums render into
    /// [`get_mfb`](Self::get_mfb) in the order they are added, between the unions and
    /// the helper functions.
    pub(crate) fn add_enum(&mut self, r#enum: RegistryEnum) -> &mut Self {
        debug_assert!(
            !r#enum.variants.is_empty(),
            "enum `{}` needs at least one variant",
            r#enum.name,
        );
        self.enums.push(r#enum);
        self
    }

    /// Add an opaque resource type (a `RegistryResource { … }`). Resources carry no
    /// injectable source, so they are recorded as semantic facts only and are not
    /// rendered by [`get_mfb`](Self::get_mfb).
    pub(crate) fn add_resource(&mut self, resource: RegistryResource) -> &mut Self {
        self.resources.push(resource);
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

    /// The package's compile-time constants, in registration order.
    pub(crate) fn constants(&self) -> &[RegistryConstant] {
        &self.constants
    }

    /// Add a compile-time package constant (a `RegistryConstant { … }`). Scalar
    /// constants set `value`; record constants set `components`.
    pub(crate) fn add_constant(&mut self, constant: RegistryConstant) -> &mut Self {
        debug_assert!(
            constant.value.is_some() != constant.components.is_some(),
            "constant `{}` must set exactly one of `value` (scalar) / `components` (record)",
            constant.name,
        );
        self.constants.push(constant);
        self
    }

    /// The package's general-builtin overrides, in registration order.
    pub(crate) fn overrides(&self) -> &[RegistryOverride] {
        &self.overrides
    }

    /// Add a general-builtin override (a `RegistryOverride { … }`).
    pub(crate) fn add_override(&mut self, r#override: RegistryOverride) -> &mut Self {
        self.overrides.push(r#override);
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
    Enum(&'r RegistryEnum),
    Resource(&'r RegistryResource),
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

    /// The package addressed by `name` — either a bare import name (`"csv"`) or the
    /// package half of a qualified call/type (`"csv.parse"` → `csv`). Membership of
    /// the function/type after the dot is not checked here; use [`resolve_func`] /
    /// [`resolve_type`] for that.
    pub(crate) fn resolve_package(&self, name: &str) -> Option<&RegistryPackage> {
        let pkg_name = name.split_once('.').map_or(name, |(pkg, _)| pkg);
        self.packages.iter().find(|p| p.import_name == pkg_name)
    }

    /// The advisory carried by the `enum_name.member` value of the package whose
    /// injected source is `builtins/<import_name>.mfb` — `None` when the package
    /// or enum is unknown, the member is not a variant, or the variant carries no
    /// advisory. Keyed by the owning package (not a bare enum-name scan) so a user
    /// enum that happens to share a builtin enum's name can never inherit its
    /// advisory. Consulted by `ir::verify` when it resolves a user-source enum
    /// member access (`check_enum_member_advisory`).
    pub(crate) fn enum_variant_advisory(
        &self,
        import_name: &str,
        enum_name: &str,
        member: &str,
    ) -> Option<EnumAdvisory> {
        let package = self
            .packages
            .iter()
            .find(|p| p.import_name == import_name)?;
        let r#enum = package.enums().iter().find(|e| e.name == enum_name)?;
        r#enum
            .variants
            .iter()
            .find(|variant| variant.name == member)?
            .advisory
    }

    pub(crate) fn resolve_func(&self, qualified: &str) -> Option<ResolvedFunc<'_>> {
        let (pkg_name, func_name) = qualified.split_once('.')?;
        let package = self.packages.iter().find(|p| p.import_name == pkg_name)?;
        let function = package.function(func_name)?;
        Some(ResolvedFunc { package, function })
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
        if let Some(r#enum) = package.enums().iter().find(|e| e.name == type_name) {
            return Some(ResolvedType::Enum(r#enum));
        }
        if let Some(resource) = package.resources().iter().find(|r| r.name == type_name) {
            return Some(ResolvedType::Resource(resource));
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
        let synthetic_files = self.synthetic_files(&ProjectView::of_ast(ast))?;
        if synthetic_files.is_empty() {
            return Ok(ast.clone());
        }
        let mut augmented = ast.clone();
        augmented.files.extend(synthetic_files);
        Ok(augmented)
    }

    /// The same injection, onto the elaborated project the former source checker consumes
    /// (plan-106-D). One decision procedure, two thin adapters — the synthetic
    /// files are parsed from source and elaborated, exactly as the AST pipeline's
    /// are parsed and then elaborated downstream.
    pub(crate) fn augment_hir_project(
        &self,
        hir: &crate::hir::HirProject,
    ) -> Result<crate::hir::HirProject, ()> {
        let synthetic_files = self.synthetic_files(&ProjectView::of_hir(hir))?;
        if synthetic_files.is_empty() {
            return Ok(hir.clone());
        }
        let mut augmented = hir.clone();
        augmented
            .files
            .extend(synthetic_files.iter().map(crate::hir::elaborate_file));
        Ok(augmented)
    }

    /// Every builtin-package source file whose injection gate `view` opens, in
    /// dependency order.
    fn synthetic_files(&self, view: &ProjectView) -> Result<Vec<crate::ast::AstFile>, ()> {
        let mut synthetic_files = Vec::new();

        for package in self.packages() {
            // `encoding` is a transitive dependency of the non-migrated `crypto` and
            // `strings` packages, whose source is injected *after* this generic pass
            // and contributes its own `IMPORT encoding` (and calls
            // `encoding::hexDecode`/`utf32Encode`). A single pass over the
            // pre-injection AST cannot see that transitive import, so `encoding` is
            // injected by its own dedicated late pass (`encoding::augmented_project`,
            // run after crypto/strings in the lowering pipeline). That late pass now
            // injects the *identical* generic `RegistryPackage::get_mfb` assembly this
            // pass would produce — only the injection position differs. Skipping it
            // here also prevents a double injection when a program imports `encoding`
            // directly.
            if package.import_name() == "encoding" {
                continue;
            }
            // `net` (and `http`) are injected by their own dedicated late passes
            // (`codegen::builtins::{net,http}::augmented_project`), for the same
            // transitivity reason as `encoding`: `http`'s injected source `IMPORT
            // net`s, and this single pass over the pre-injection AST cannot see that
            // transitive import. Skipping them here also prevents a double injection
            // when a program imports `net`/`http` directly.
            if matches!(package.import_name(), "net" | "http") {
                continue;
            }
            // `color` is injected by its own dedicated late pass
            // (`codegen::builtins::color::augmented_project`), for the same
            // transitivity reason: since plan-122-B `canvas`'s injected companion
            // carries `IMPORT color` and calls `color::toLinear`/`fromLinear` from
            // its blend and gradient helpers, and this single pass over the
            // pre-injection AST — where a canvas program has written only
            // `IMPORT canvas` — cannot see that. Skipping it here also prevents a
            // double injection when a program imports `color` directly.
            if package.import_name() == "color" {
                continue;
            }
            // `collections` is injected by its own dedicated pass at PARSE time
            // (`codegen::builtins::collections::augmented_project`, run by
            // `parse_project`): its members are source GENERICS the monomorphizer
            // must see to instantiate, long before this ir-lower-time pass runs.
            // That pass injects the identical `get_mfb` assembly; skipping it here
            // prevents a double injection for a program that imports it directly.
            if package.import_name() == "collections" {
                continue;
            }
            if !package.is_imported_by(view) {
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

        // plan-99: gated helper chunks — the `strings` scalar-seam Unicode table
        // (`WhenUsed`) and the `term`↔`astrings` bridge (`WhenImported`) — inject as
        // their own synthetic files, deduped by helper `name` so a chunk reached
        // through several packages is injected once. `Always` bodies are *not* here
        // (they already rendered inline in each package's `get_mfb` above);
        // `import_name` edges carry no body (ordering-only). Evaluated in package
        // registration order, then helper add order.
        let mut injected_helpers: std::collections::HashSet<&'static str> =
            std::collections::HashSet::new();
        for package in self.packages() {
            let package_imported = package.is_imported_by(view);
            for helper in package.helpers() {
                let Some(body) = helper.body else {
                    continue; // an `import_name` ordering edge — no source to inject.
                };
                let gate_open = match helper.gate {
                    HelperGate::Always => false, // rendered inline in `get_mfb`.
                    // `WhenUsed` fires only when the OWNING package is imported and a
                    // gated member is referenced (the `strings` scalar-seam gate).
                    HelperGate::WhenUsed(names) => package_imported && view.references_any(names),
                    // `WhenImported` is a cross-package bridge keyed on `other` being
                    // imported — the OWNING package need NOT be imported (an
                    // `astrings`-only program does not import `strings`, yet the injected
                    // `astrings` companion calls the `strings` scalar seam, so the seam
                    // must ride in). `other` is matched by raw import name so it works for
                    // a non-registry package too (`astrings`/`term`, not yet migrated).
                    HelperGate::WhenImported(other) => view.imports(other),
                    // `WhenBothImported` is a cross-package bridge whose body references
                    // BOTH packages' surface, so it must inject only when both are
                    // imported (the `term`↔`astrings` `drawText(AttributedString)`
                    // bridge — legacy `term::bridge_uses_package`). Over-injecting on
                    // either alone would drag the other package's surface in as dead
                    // code, shifting the injected `.ir`/`build.log` of a program that
                    // imports only one.
                    HelperGate::WhenBothImported(a, b) => view.imports(a) && view.imports(b),
                };
                if !gate_open || !injected_helpers.insert(helper.name) {
                    continue;
                }
                let label = format!("<builtin-{}>", helper.name);
                let doc = format!("builtins/{}.mfb", helper.name);
                // The chunk is labelled by the HELPER, but it is the OWNING
                // package's source, so the package is handed over explicitly --
                // the label alone would leave its declarations unqualified
                // (bug-480 Phase 4b).
                let file = crate::ast::parse_source_builtin(
                    std::path::Path::new(&label),
                    &doc,
                    &format!("{}\n", body.trim_end()),
                    package.import_name(),
                )?;
                synthetic_files.push(file);
            }
        }

        Ok(synthetic_files)
    }

    // Generic-dispatch queries. Each answers, for a call/type, the fact the old
    // `REGISTRY`-based generic dispatch answered, so a caller can dual-path
    // `registry().X(name).or(old(name))`. `None`/`false` means "no migrated package
    // owns this", i.e. fall through to the old path.

    /// Whether a migrated package declares the call `qualified` (`"csv.parse"`).
    pub(crate) fn is_member(&self, qualified: &str) -> bool {
        self.resolve_func(qualified).is_some()
    }

    /// Whether the migrated call `qualified` is an **internal-only** member — a native
    /// primitive resolvable only from toolchain-provided (`internal`) source
    /// (`astrings.readSpans`/`writeSpans`/`scalarLen`). The registry half of
    /// `builtins::is_internal_only_call`, replacing the deleted per-package
    /// `astrings::is_astrings_internal_call`.
    pub(crate) fn is_internal_only_member(&self, qualified: &str) -> bool {
        self.resolve_func(qualified)
            .is_some_and(|resolved| resolved.function.internal_only)
    }

    /// The import name of the migrated package that owns `qualified`, or `None`.
    pub(crate) fn owning_package(&self, qualified: &str) -> Option<&'static str> {
        self.resolve_func(qualified)
            .map(|resolved| resolved.package.import_name)
    }

    /// The `(min, max)` argument arity of the migrated call `qualified`, or `None`.
    /// `min` counts the required (non-defaulted) params; `max` is the widest overload.
    pub(crate) fn arity(&self, qualified: &str) -> Option<(usize, usize)> {
        self.resolve_func(qualified)
            .and_then(|resolved| resolved.function.arity())
    }

    /// Whether the migrated call `qualified` declares `error_name` among any of its
    /// implementations' errors — the half of the `raise_error` "a builtin must declare
    /// the errors it raises" check.
    pub(crate) fn declares_error(&self, qualified: &str, error_name: &str) -> bool {
        self.resolve_func(qualified)
            .is_some_and(|resolved| resolved.function.declares_error(error_name))
    }

    /// Whether `name` is a value type (`EXPORT TYPE`/`UNION`/`ENUM`) declared by any
    /// migrated package (`CsvReader`/`CsvRow`), a source-declared/opaque type
    /// (`datetime.Instant`, thread's `Thread`/`ThreadWorker`), or a **parametric**
    /// spelling of an opaque type (`Thread OF Msg TO Out`) whose head token before the
    /// first ` OF ` names a source-declared type.
    pub(crate) fn is_builtin_type(&self, name: &str) -> bool {
        // The head token of a parametric spelling (`Thread OF …`): a source-declared
        // opaque type used with type arguments. `List`/`Map`/… are never source types,
        // so their `X OF …` spellings are correctly not matched here.
        //
        // plan-111-F: taken from the one grammar's own variants rather than a
        // local `split_once(" OF ")`.
        //
        // `UserOf` alone is NOT enough: `Thread OF Integer TO String` parses to
        // `ThreadHandle`, not `UserOf`, so a `UserOf`-only read silently stopped
        // recognizing the thread handles — which are precisely the
        // "source-declared opaque type used with type arguments" this exists for
        // (caught by `thread::tests::opaque_handle_types_recognized`). `List`,
        // `Set`, `Map` and `Result` are never source types, so they still yield
        // no head, exactly as the string split did.
        let head_owned = match crate::types::ParameterType::declared(name) {
            crate::types::ParameterType::UserOf(base, _) => Some(base.resolve().to_string()),
            crate::types::ParameterType::ThreadHandle { worker, .. } => Some(
                if worker {
                    crate::types::THREAD_WORKER_TYPE
                } else {
                    crate::types::THREAD_TYPE
                }
                .to_string(),
            ),
            _ => None,
        };
        let head = head_owned.as_deref();
        // bug-480 Phase 4b: a builtin value type is named `crypto.Sealed` now, while
        // a registry row still carries the bare member id it declares. Accept both --
        // callers hand this whatever the type system holds, which is the qualified
        // form, and matching only the bare id made every such probe answer `false`.
        let matches = |package: &RegistryPackage, row: &str| {
            row == name
                || name
                    .strip_prefix(package.import_name())
                    .and_then(|rest| rest.strip_prefix('.'))
                    .is_some_and(|leaf| leaf == row)
        };
        self.packages().iter().any(|package| {
            package
                .records()
                .iter()
                .any(|record| matches(package, record.name))
                || package.unions().iter().any(|union| matches(package, union.name))
                || package
                    .enums()
                    .iter()
                    .any(|r#enum| matches(package, r#enum.name))
                // `datetime`'s value records/enums live in its injected companion source.
                || package
                    .source_types()
                    .iter()
                    .any(|source| matches(package, source))
                || head.is_some_and(|head| package.source_types().contains(&head))
        })
    }

    /// Rewrite every descriptor-held TYPE REFERENCE from its bare leaf spelling to
    /// the package-qualified identity a value type now carries (bug-480 Phase 4b).
    ///
    /// The descriptors are written bare — `ParameterType::named(KEYPAIR_TYPE)`,
    /// where `KEYPAIR_TYPE` is `"KeyPair"` — because within `crypto` that IS the
    /// name, and the governing rule says a package's own members need no prefix.
    /// Once the declared identity is `crypto.KeyPair`, a signature still saying
    /// `KeyPair` denotes nothing: observed as `native record type 'KeyPair' does
    /// not resolve` out of codegen, and as `MATCH on open type Attribute` where a
    /// bare-typed scrutinee met qualified `CASE` arms.
    ///
    /// Done as one pass here rather than at ~400 construction sites because only
    /// the type FIELDS need rewriting — `name` is `&'static str` and stays the
    /// bare member id that `resolve_type` matches after splitting the qualifier.
    /// One choke point also means one place where the owner rule is stated:
    ///
    ///   1. the package that declares the leaf, when it is this package's own —
    ///      so `http`'s `Stream` is `http.Stream` and `process`'s is
    ///      `process.Stream`, which is the whole of bug-481;
    ///   2. otherwise the single package that declares it, for a genuine
    ///      cross-package reference (`http::Response` naming `net::Url`);
    ///   3. otherwise unchanged — a primitive, a generic parameter, or an
    ///      ambiguous leaf no package can claim.
    fn qualify_value_type_references(&mut self) {
        // leaf -> the packages declaring it. Built before any rewrite so a
        // cross-package reference resolves against the whole registry.
        let mut owners: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for package in &self.packages {
            let pkg = package.import_name.to_string();
            let mut add = |leaf: &str| {
                owners
                    .entry(leaf.to_string())
                    .or_default()
                    .push(pkg.clone());
            };
            for record in &package.records {
                add(record.name);
            }
            for union in &package.unions {
                add(union.name);
            }
            for r#enum in &package.enums {
                add(r#enum.name);
            }
            for source_type in &package.source_types {
                add(source_type);
            }
        }

        let packages = std::mem::take(&mut self.packages);
        self.packages = packages
            .into_iter()
            .map(|mut package| {
                let pkg = package.import_name;
                let map = |ty: &ParameterType| qualify_type_leaves(ty, pkg, &owners);
                // Record FIELD types are rewritten too (bug-484). They used to be
                // skipped, on the reasoning that a record round-trips through
                // injectable source where a qualifier is `::` and a `.` would parse
                // as field access. True, but it argues for rendering `::`, not for
                // leaving the field bare -- and `source_spelling` already does that
                // rewrite for every rendered type. Leaving them bare put a name that
                // is IMPORTED into the declaring package (`udp::Datagram`'s `from`
                // is net's `Address`) into the companion with no prefix, against the
                // governing rule, and left the parser to guess it back from the
                // file's imports. An ambiguous leaf it could not guess was left bare
                // and unresolved -- silently, which is the bug-483 class again.
                for record in &mut package.records {
                    for prop in &mut record.props {
                        prop.ty = qualify_type_leaves_for_source(&prop.ty, pkg, &owners);
                    }
                }
                for function in &mut package.functions {
                    for imp in &mut function.implementations {
                        for param in &mut imp.params {
                            param.ty = map(&param.ty);
                        }
                        imp.return_type = map(&imp.return_type);
                    }
                }
                package
            })
            .collect();
    }

    /// A `package.Type` reference (`"csv.CsvReader"`) resolved to the DECLARED type
    /// id when the migrated package declares it, else `None`.
    ///
    /// bug-480 Phase 4b changed what "the declared id" means. It used to be the
    /// bare member name (`net.Url` -> `Url`), because every builtin value type was
    /// declared bare in one flat top-level namespace. That namespace is why
    /// `http::Stream` (a union) and `process::Stream` (an enum) could not coexist
    /// (bug-481), and why a bare `Response` resolved from a consumer that should
    /// have had to write `http::Response`. A value type is now addressed
    /// `net.Url`, exactly as a RESOURCE has been since plan-97 — so this returns
    /// the qualified spelling unchanged, and the two `Stream`s are two names.
    pub(crate) fn qualified_builtin_type(&self, qualified: &str) -> Option<String> {
        // Match the resolved kind rather than discarding it: the member id it
        // carries is what the qualified spelling must be built from, and reading it
        // keeps the resolution honest (a row whose `name` disagreed with the
        // qualifier would otherwise pass silently).
        if let Some(resolved) = self.resolve_type(qualified) {
            let member = match resolved {
                ResolvedType::Record(record) => record.name,
                ResolvedType::Union(union) => union.name,
                ResolvedType::Enum(r#enum) => r#enum.name,
                ResolvedType::Resource(resource) => resource.name,
            };
            let (package, leaf) = qualified.split_once('.')?;
            debug_assert_eq!(leaf, member, "registry row name disagrees with lookup");
            return Some(format!("{package}.{member}"));
        }
        // A source-declared value type (`datetime.Instant`) authored only in the
        // package's injected companion, not modeled as a record/union/enum.
        let (pkg_name, type_name) = qualified.split_once('.')?;
        self.packages()
            .iter()
            .find(|p| p.import_name == pkg_name)
            .filter(|p| p.source_types().contains(&type_name))
            .map(|_| qualified.to_string())
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
/// Render `ty` the way SOURCE spells it.
///
/// The type system's qualifier is a dot (`net.Address`), matching the parser's
/// internal normalization. MFBASIC source spells it `net::Address` -- a dot there
/// is FIELD ACCESS, so emitting the internal form into injectable source makes the
/// companion unparseable (`<builtin-udp>:10 Field name must be an identifier`).
///
/// Only the qualifier is rewritten; container spellings (`List OF`, `Map OF … TO`)
/// and their nesting are already source-shaped.
fn source_spelling(ty: &ParameterType) -> String {
    let rendered = ty.name().into_owned();
    let mut out = rendered.clone();
    for package in registry().packages() {
        let dotted = format!("{}.", package.import_name());
        if out.contains(&dotted) {
            out = out.replace(&dotted, &format!("{}::", package.import_name()));
        }
    }
    out
}

/// Map every NOMINAL leaf of `ty` from its bare spelling to the package-qualified
/// identity, per the owner rule in
/// [`Registry::qualify_value_type_references`]. Container shapes
/// (`List OF`, `Map OF … TO …`, `Result OF`, a user generic's arguments) are
/// descended into, so `List OF Json` becomes `List OF json.Json`.
///
/// An already-qualified leaf is left alone: resources have carried their package
/// since plan-97, and a descriptor that spells one out is already correct.
fn qualify_type_leaves(
    ty: &ParameterType,
    package: &str,
    owners: &std::collections::HashMap<String, Vec<String>>,
) -> ParameterType {
    qualify_type_leaves_inner(ty, package, owners, true)
}

/// The governing rule's two cases, for a type reference that is RENDERED BACK INTO
/// SOURCE (a record field): a name defined **locally** needs no prefix, a name that
/// is **imported** requires one. So the own-package arm leaves the leaf bare here,
/// where `qualify_type_leaves` (used for signatures, which are type-system
/// identities and never round-trip through source) qualifies it.
///
/// Getting this backwards is not cosmetic: `regex`'s companion declares PRIVATE
/// types like `#regex_Cont` locally, and prefixing those made the descriptor's
/// field type `regex.#regex_Cont` disagree with the `#regex_Cont` the parser
/// produces for the local declaration — `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH` on
/// the package's own source (bug-484).
fn qualify_type_leaves_for_source(
    ty: &ParameterType,
    package: &str,
    owners: &std::collections::HashMap<String, Vec<String>>,
) -> ParameterType {
    qualify_type_leaves_inner(ty, package, owners, false)
}

fn qualify_type_leaves_inner(
    ty: &ParameterType,
    package: &str,
    owners: &std::collections::HashMap<String, Vec<String>>,
    qualify_own: bool,
) -> ParameterType {
    let qualify_leaf = |leaf: &str| -> Option<String> {
        if leaf.contains('.') {
            return None; // already qualified (a resource id, or a spelled-out reference)
        }
        // Not a registry-declared value type at all: a scalar nominal (`Scalar`,
        // `Error`, `ErrorLoc`, `AttributedString`), a C ABI spelling, or a generic
        // parameter. Nothing to qualify, and not an error.
        let declaring = owners.get(leaf)?;
        if declaring.iter().any(|owner| owner == package) {
            // Declared by THIS package, so it is local. A signature carries the
            // qualified identity; a rendered field keeps the bare local name.
            return qualify_own.then(|| format!("{package}.{leaf}"));
        }
        // Not declared by this package, so the bare leaf denotes nothing: under the
        // governing rule a bare name is a LOCAL name, and this package has no such
        // declaration. How many OTHER packages happen to export the leaf is
        // irrelevant — one is not more resolvable than three, it is just a guess
        // that happens to have one candidate. The pass used to make exactly that
        // guess for a single owner and leave the reference bare for several, both
        // silently. A descriptor must spell a cross-package reference out
        // (`net::ADDRESS_TYPE_ID`, not `net::ADDRESS_TYPE`), so reaching here is an
        // authoring error, not a user one (bug-484).
        panic!(
            "registry: `{package}` references the bare type name `{leaf}`, but \
             declares no such type. A bare name is a LOCAL name; `{leaf}` is \
             declared by {declaring:?}. Spell the reference out as \
             `<package>.{leaf}`."
        )
    };
    match ty {
        ParameterType::Named(sym) => match qualify_leaf(sym.resolve()) {
            Some(qualified) => ParameterType::named(&qualified),
            None => ty.clone(),
        },
        ParameterType::UserOf(head, args) => {
            let args = args
                .iter()
                .map(|arg| qualify_type_leaves_inner(arg, package, owners, qualify_own))
                .collect::<Vec<_>>();
            match qualify_leaf(head.resolve()) {
                Some(qualified) => ParameterType::user_of(&qualified, args),
                None => ParameterType::user_of(head.resolve(), args),
            }
        }
        ParameterType::ListOf(inner) => ParameterType::list_of(qualify_type_leaves_inner(
            inner,
            package,
            owners,
            qualify_own,
        )),
        ParameterType::SetOf(inner) => ParameterType::set_of(qualify_type_leaves_inner(
            inner,
            package,
            owners,
            qualify_own,
        )),
        ParameterType::MapOf(key, value) => ParameterType::map_of(
            qualify_type_leaves_inner(key, package, owners, qualify_own),
            qualify_type_leaves_inner(value, package, owners, qualify_own),
        ),
        ParameterType::ResultOf(inner) => ParameterType::result_of(qualify_type_leaves_inner(
            inner,
            package,
            owners,
            qualify_own,
        )),
        // A resource's STATE clause and a `RES` wrapper both hold a nominal that has
        // to be qualified too: `http::startRead` returns `Stream STATE PendingState`,
        // and leaving the state bare made the initializer disagree with the binding
        // (`declares STATE http.PendingState but its initializer carries STATE
        // PendingState`).
        ParameterType::Stateful { base, state } => ParameterType::Stateful {
            base: Box::new(qualify_type_leaves_inner(
                base,
                package,
                owners,
                qualify_own,
            )),
            state: Box::new(qualify_type_leaves_inner(
                state,
                package,
                owners,
                qualify_own,
            )),
        },
        ParameterType::Res(inner) => ParameterType::Res(Box::new(qualify_type_leaves_inner(
            inner,
            package,
            owners,
            qualify_own,
        ))),
        other => other.clone(),
    }
}

fn build() -> Registry {
    let mut r = Registry::new();
    crate::codegen::builtins::app::register(&mut r);
    crate::codegen::builtins::astrings::register(&mut r);
    crate::codegen::builtins::audio::register(&mut r);
    crate::codegen::builtins::bits::register(&mut r);
    crate::codegen::builtins::canvas::register(&mut r);
    crate::codegen::builtins::color::register(&mut r);
    crate::codegen::builtins::csv::register(&mut r);
    crate::codegen::builtins::json::register(&mut r);
    crate::codegen::builtins::math::register(&mut r);
    crate::codegen::builtins::regex::register(&mut r);
    crate::codegen::builtins::strings::register(&mut r);
    crate::codegen::builtins::term::register(&mut r);
    crate::codegen::builtins::testing::register(&mut r);
    crate::codegen::builtins::process::register(&mut r);
    crate::codegen::builtins::datetime::register(&mut r);
    crate::codegen::builtins::encoding::register(&mut r);
    crate::codegen::builtins::errorcode::register(&mut r);
    crate::codegen::builtins::collections::register(&mut r);
    crate::codegen::builtins::money::register(&mut r);
    crate::codegen::builtins::os::register(&mut r);
    crate::codegen::builtins::fs::register(&mut r);
    crate::codegen::builtins::general::register(&mut r);
    crate::codegen::builtins::io::register(&mut r);
    crate::codegen::builtins::crypto::register(&mut r);
    crate::codegen::builtins::tls::register(&mut r);
    crate::codegen::builtins::tcp::register(&mut r);
    crate::codegen::builtins::udp::register(&mut r);
    crate::codegen::builtins::net::register(&mut r);
    crate::codegen::builtins::http::register(&mut r);
    crate::codegen::builtins::thread::register(&mut r);
    crate::codegen::builtins::vector::register(&mut r);
    r.qualify_value_type_references();
    r
}

//
// Everything below this should be depricated
//

/// One runtime helper call a migrated native package emits: its package-qualified
/// call name (`"process.spawn"`) and its ABI return type. Derived from the registry
/// by [`runtime_specs`] so a migrated native package needs no hand-written spec
/// table — the parallel `src/target/shared/runtime/*_specs.rs` "second catalog".
#[derive(Clone, Debug)]
pub(crate) struct RuntimeCall {
    pub(crate) name: &'static str,
    pub(crate) return_type: ParameterType,
}

/// The runtime helper calls every **migrated** native package emits, derived from
/// the registry so there is one source of truth. Each `Body::AbiFunction` OS-seam member
/// contributes its qualified call (`pkg.member`) typed by the member's `return_type`,
/// plus each `os_aliases` code-layer overload-split form (`spawnEnv`, `sendTimeout`,
/// …) — sharing that member's return; each resource contributes its close op
/// (`process.__drop`, return `Nothing`). Calls are deduped by name, first occurrence
/// wins: a member listed on two overloads (`spawn` on both spawn forms) collapses to
/// one, and a rewrite-away overload that shares its function name (a future
/// `net.poll` scalar vs the `List` overload emitting `net.pollList`) resolves to the
/// scalar's `net.poll` while the `List` overload contributes only `pollList` — each
/// with the correct return. Frozen once for the `ptr::eq` catalog identity (bug-382).
///
/// Only OS-seam members (`Body::AbiFunction`) are runtime helpers; pure-source
/// (`Body::Mfb`) and `abi_inline` inline lowerings emit none, so pure packages
/// (csv/json/regex/…) contribute nothing — mirroring their absent `*_specs.rs`.
pub(crate) fn runtime_specs() -> &'static [RuntimeCall] {
    static SPECS: OnceLock<Vec<RuntimeCall>> = OnceLock::new();
    SPECS.get_or_init(|| {
        let mut calls: Vec<RuntimeCall> = Vec::new();
        for package in registry().packages() {
            let pkg = package.import_name();
            for function in package.functions() {
                for implementation in function.implementations() {
                    // Only runtime-helper members contribute a `_mfb_rt_*` call:
                    // `Body::AbiFunction`. `abi_inline` inline lowerings emit no runtime
                    // helper. Runtime-helper members carry `os_aliases`.
                    let os_aliases: &[&str] = match &implementation.body {
                        Body::AbiFunction { os_aliases, .. } => os_aliases,
                        _ => continue,
                    };
                    push_runtime_call(
                        &mut calls,
                        qualify_runtime_call(pkg, function.name),
                        &implementation.return_type,
                    );
                    for alias in os_aliases.iter().copied() {
                        push_runtime_call(
                            &mut calls,
                            qualify_runtime_call(pkg, alias),
                            &implementation.return_type,
                        );
                    }
                }
            }
            for resource in package.resources() {
                // The close op is already package-qualified (`process.__drop`).
                push_runtime_call(&mut calls, resource.close_function, &ParameterType::Nothing);
            }
        }
        calls
    })
}

/// Append `name` → `return_type`, unless `name` is already present (dedup, first wins).
fn push_runtime_call(
    calls: &mut Vec<RuntimeCall>,
    name: &'static str,
    return_type: &ParameterType,
) {
    if !calls.iter().any(|call| call.name == name) {
        calls.push(RuntimeCall {
            name,
            return_type: return_type.clone(),
        });
    }
}

/// `pkg.member`, leaked to `'static` — called once behind [`runtime_specs`]'s `OnceLock`.
fn qualify_runtime_call(pkg: &str, member: &str) -> &'static str {
    Box::leak(format!("{pkg}.{member}").into_boxed_str())
}

/// The [`RegistryConstant`] a migrated package declares for `qualified`
/// (`"math.pi"`, `"vector.zeroFloat3"`), or `None`. The single lookup behind the four
/// constant boundary fns below — splits `pkg.member`, finds the package, finds the
/// constant.
fn find_constant(qualified: &str) -> Option<&'static RegistryConstant> {
    let (_, member) = qualified.split_once('.')?;
    registry()
        .resolve_package(qualified)?
        .constants()
        .iter()
        .find(|constant| constant.name == member)
}

/// Whether a migrated package declares the compile-time constant `qualified` — the
/// registry half of the `builtins::is_package_constant` dual-path. `false` (fall
/// through to the hand tables) for every un-migrated package.
pub(crate) fn is_package_constant(qualified: &str) -> bool {
    find_constant(qualified).is_some()
}

/// The type name the migrated constant `qualified` evaluates to (scalar type or record
/// type), or `None`.
pub(crate) fn constant_type_name(qualified: &str) -> Option<ParameterType> {
    // plan-111-C: one API, typed. [`RegistryConstant::type_name`] is a
    // `&'static str` DESCRIPTOR literal and stays one (§Non-goals forbids a
    // descriptor change), so this is the single place the canonical grammar is
    // applied to it — callers stay typed instead of each classifying the
    // spelling themselves. Storing a `ParameterType` in the descriptor directly
    // is a const-context change, not this plan's.
    find_constant(qualified).map(|constant| {
        // bug-480 Phase 4b: `type_name` is a `&'static str` descriptor literal and
        // stays the bare member id, but a constant's TYPE has to be the qualified
        // identity or a folded record constant (`vector::zeroFloat3`) denotes a
        // type that no longer exists. Qualify with the constant's own package,
        // which is the head of `qualified`.
        let declared = ParameterType::declared(constant.type_name);
        match qualified.split_once('.') {
            Some((package, _)) if !constant.type_name.contains('.') => {
                let candidate = format!("{package}.{}", constant.type_name);
                if registry().is_builtin_type(&candidate) {
                    ParameterType::declared(&candidate)
                } else {
                    declared
                }
            }
            _ => declared,
        }
    })
}

/// The literal a migrated **scalar** constant `qualified` folds to, or `None` (a record
/// constant, or an un-migrated package) — the registry half of
/// `builtins::package_constant_value`.
pub(crate) fn constant_value(qualified: &str) -> Option<&'static str> {
    find_constant(qualified).and_then(|constant| constant.value)
}

/// The ordered per-field literals a migrated **record** constant `qualified` inlines
/// into a constructor of its [`type_name`](RegistryConstant::type_name), or `None` (a
/// scalar constant, or an un-migrated package) — read by `registry_record_constant` in
/// `ir/lower.rs` to fold a `vector` record constant (`vector.zeroFloat3`) into a
/// constructor.
pub(crate) fn constant_components(qualified: &str) -> Option<&'static [&'static str]> {
    find_constant(qualified).and_then(|constant| constant.components)
}

/// The migrated `errorCode` package's constant with the bare error `name`
/// (`"ErrNotFound"`, NOT the qualified `errorCode.ErrNotFound`), or `None`. The
/// single lookup behind the three error-**emission** free fns below — the codegen
/// error path keys on the bare name a builtin declares in its `errors` list, while
/// constant-folding keys on the qualified `errorCode.<name>` via [`find_constant`].
fn errorcode_constant(name: &str) -> Option<&'static RegistryConstant> {
    registry()
        .resolve_package("errorCode")?
        .constants()
        .iter()
        .find(|constant| constant.name == name)
}

/// The `(code, message)` for a runtime error *name* (e.g. `"ErrIndexOutOfRange"`), as
/// declared in a builtin's `errors` list, or `None` if the name is not a known
/// `errorCode` constant. The codegen-facing lookup the native error-emission path
/// resolves a builtin's declared error to before passing it to `emit_error_code_return`.
/// Distinct from [`constant_value`], which takes the package-qualified
/// `errorCode.<Name>` key and returns only the code for constant folding.
pub(crate) fn runtime_error(name: &str) -> Option<(&'static str, &'static str)> {
    let constant = errorcode_constant(name)?;
    Some((constant.value?, constant.message?))
}

/// The `(code, message-symbol)` for a runtime error *name*, or `None` if unknown. The
/// fixed-runtime-helper emission lookup: `raise_error_into` sets the code immediate and
/// loads the message data-object symbol, reproducing the historical lightweight
/// fixed-helper error sequence byte-for-byte from the registered constant.
pub(crate) fn runtime_error_emission(name: &str) -> Option<(&'static str, &'static str)> {
    let constant = errorcode_constant(name)?;
    Some((constant.value?, constant.symbol?))
}

/// The full `(code, message, symbol)` for a runtime error *name*, all borrowed from the
/// migrated `errorCode` package's constant. Feeds the codegen data-object tables that
/// emit the fixed `_mfb_str_error_*` string objects.
pub(crate) fn runtime_error_triple(
    name: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    let constant = errorcode_constant(name)?;
    Some((constant.value?, constant.message?, constant.symbol?))
}

/// The internal `__pkg_*` helper a migrated package provides as an **override** of the
/// overridable general builtin `builtin` over `arg_type`, or `None` — the registry half
/// of the `builtins::general_override_target` dual-path. `None` (fall through to the
/// hand match) for every un-migrated package.
pub(crate) fn general_override_target(
    builtin: &str,
    arg_type: &ParameterType,
) -> Option<&'static str> {
    // plan-111-C: the QUERY takes a type; the DESCRIPTOR still spells its
    // `arg_type` (a `&'static str` row, and §Non-goals forbids changing a
    // descriptor), so the comparison renders the argument. Identical by the
    // `parse`<->`name` round trip — the old form compared the same two
    // spellings.
    let spelled = arg_type.name();
    // bug-480 Phase 4b: the argument now arrives package-qualified (`vector.Float2`),
    // while the descriptor row still spells the bare member id it declares
    // (`Float2`) -- `arg_type` is a `&'static str` and stays one. Compare against
    // the owning package's qualified spelling as well, so `toString(vector::abs(v))`
    // still finds `__vector_float2ToString` instead of falling through to the
    // general builtin and reporting the vector type as un-stringable.
    let overrides_arg_type = |package: &RegistryPackage, o: &RegistryOverride| {
        o.arg_type == spelled
            || format!("{}.{}", package.import_name(), o.arg_type) == spelled.as_ref()
    };
    registry().packages().iter().find_map(|package| {
        package
            .overrides()
            .iter()
            .find(|o| o.builtin == builtin && overrides_arg_type(package, o))
            .map(|o| o.helper)
    })
}

/// The typed twin of [`call_return_type`] (plan-106-A). The descriptor already
/// holds a [`ParameterType`]; this hands back a clone instead of rendering it,
/// so the static-nominal return path costs no allocation and crosses no string.
pub(crate) fn call_return_type_typed(qualified: &str) -> Option<ParameterType> {
    let return_type = &registry()
        .resolve_func(qualified)?
        .function
        .implementations
        .first()?
        .return_type;

    if contains_var(return_type) {
        return None;
    }

    Some(return_type.clone())
}

/// The static return type of a runtime-call **`os_alias`** (`audio.openOutputDevice`,
/// `audio.openInputDevice`, …): the aliased [`Implementation`]'s own `return_type`,
/// package-qualified exactly like [`call_return_type`]'s. An alias is not a registry
/// member (`resolve_func` sees only surface names), but the code layer meets alias
/// names directly when a surface call was rewritten at IR level
/// (`audio::runtime_overload_name`). Without this, the alias fell through to the
/// derived runtime spec, whose ABI spelling **bares** a resource name
/// (`abi_return_name`: `audio.AudioOutput` → `AudioOutput`) — and a bare resource
/// spelling is invisible to the resource classification, so an inline-`TRAP`'d
/// `openOutput(device, …)` tried to flat-copy the handle and died with
/// "native inlined field size not available for type 'AudioOutput'".
pub(crate) fn alias_call_return_type(qualified: &str) -> Option<Cow<'static, str>> {
    let (pkg_name, alias) = qualified.split_once('.')?;
    let package = registry()
        .packages()
        .iter()
        .find(|p| p.import_name() == pkg_name)?;
    for function in package.functions() {
        for implementation in function.implementations() {
            let Body::AbiFunction { os_aliases, .. } = &implementation.body else {
                continue;
            };
            if os_aliases.contains(&alias) {
                if contains_var(&implementation.return_type) {
                    return None;
                }
                return Some(implementation.return_type.name());
            }
        }
    }
    None
}

/// Whether a concrete leaf type is compatible with a *scalar or nominal* parameter
/// type (the [`unify`] leaf case). Exact types match, and two *different known scalars*
/// are the only definite incompatibility — a nominal vs anything else is accepted
/// conservatively (the type checker never emits a false rejection). Container,
/// [`Var`](ParameterType::Var), and [`Unknown`](ParameterType::Unknown) cases never
/// reach here; [`unify`] handles them first.
fn leaf_matches(pattern: &ParameterType, concrete: &ParameterType, strict: bool) -> bool {
    // The `RES ` collection-element ownership marker is transparent to matching:
    // historically `parse` stripped it before the matcher ever saw it, so a
    // `Res(inner)` on either side matches exactly as `inner` would. Unwrap
    // symmetrically to reproduce that behavior byte-for-byte.
    if let ParameterType::Res(inner) = concrete {
        return leaf_matches(pattern, inner, strict);
    }
    if let ParameterType::Res(inner) = pattern {
        return leaf_matches(inner, concrete, strict);
    }
    if pattern == concrete {
        return true;
    }
    // A container CONCRETE against a RESOURCE leaf pattern matches only when their spelled
    // names are equal. A genuine resource nominal (`Named("tls.Socket")`) is NOT a
    // container spelling, so it is correctly rejected against a `List OF RES tls::Socket`
    // concrete — which lets a same-arity resource-nominal-vs-list overload pair
    // (`tls::poll`: scalar `tls::Socket → Boolean` vs `List OF RES tls::Socket → tls::Socket`)
    // select by argument shape on the lenient dispatch / return-inference path.
    // Gated on the pattern being a RESOURCE (not any nominal): a NON-resource leaf keeps the
    // pre-existing coarse `true` below — a value nominal or a string-blob spelling like
    // fs::pathJoin's `Named("List OF String")` must still accept a container arg, and
    // broadening the name-equal rule to all nominals perturbs the lenient overload dispatch
    // of every package with container-arg overloads (json/crypto/http/…). (Container
    // PATTERNS are handled by the arms in `unify`; this is the mirror case.)
    if is_resource_type_name(&pattern.name())
        && matches!(
            concrete,
            ParameterType::ListOf(_)
                | ParameterType::SetOf(_)
                | ParameterType::MapOf(_, _)
                | ParameterType::Func(_, _, _)
        )
    {
        return pattern.name() == concrete.name();
    }
    // Two unequal scalars never match, in either mode.
    if pattern.is_scalar() && concrete.is_scalar() {
        return false;
    }
    // STRICT (argument validation): a scalar and a nominal/non-scalar leaf are never
    // compatible either — a `Named("Json")` parameter cannot accept a `String` argument
    // (bug-443). LENIENT (overload dispatch / return-type inference): stay coarse so a
    // nominally-spelled or not-yet-resolved argument is not rejected, which would perturb
    // overload selection and type propagation on valid programs (e.g. a `csv::stringify`
    // nested-list argument degrading to `List OF Unknown`).
    if strict && (pattern.is_scalar() || concrete.is_scalar()) {
        return false;
    }
    // STRICT (argument validation): a RESOURCE parameter demands exact base-resource
    // identity — bug-427 STATE/ownership-agnostic, so a `File STATE Cursor` argument still
    // satisfies a `File` parameter, but an unrelated resource or a resource UNION does NOT
    // satisfy a concrete resource close-op parameter (`fs::close(<Stream union>)` must be
    // rejected — a use-after-free class error the legacy exact-name `DefaultResolver`
    // caught). A NON-resource nominal parameter stays coarse: a value-UNION parameter like
    // `Json` must still accept a variant that widens into it (`json::stringify(JsonNull)`),
    // and lenient dispatch/inference stays coarse everywhere so overload selection and type
    // propagation are unperturbed.
    if strict && is_resource_type_name(&pattern.name()) {
        return resource_base_eq(pattern, concrete);
    }
    true
}

/// Whether `name` (a parameter's type leaf, possibly carrying a `STATE` clause) names a
/// resource handle — a legacy builtin resource (`net.Socket`) or a migrated-package
/// resource registered via [`RegistryPackage::add_resource`] (`fs.File`, addressed by its
/// package-qualified id whose bare tail is the `RegistryResource::name`). Only a resource
/// parameter triggers the strict exact-base match in [`leaf_matches`]; value nominals
/// (unions, records) stay coarse.
fn is_resource_type_name(name: &str) -> bool {
    // A single bare-name scan of the registry: a qualified builtin id (`fs.File`) and
    // its bare base (`File`) both reduce to the same tail, so one scan answers both
    // the package-qualified and bare-base cases the two former branches split out.
    let base = crate::codegen::resource::base_resource_name(name);
    let bare = base.rsplit('.').next().unwrap_or(base);
    registry()
        .packages()
        .iter()
        .any(|package| package.resources().iter().any(|r| r.name == bare))
}

/// Structurally unify a parameter-type `pattern` — which may contain
/// [`ParameterType::Var`] type variables — against a `concrete` argument type,
/// recording every variable binding in `bindings`. Returns `false` on a structural or
/// scalar mismatch, or on a variable bound inconsistently to two different types
/// (`get(List OF Integer, String)` fails: `T` is bound to `Integer` by arg 0, then
/// contradicted by `String`). An [`Unknown`](ParameterType::Unknown) concrete (an
/// unresolved argument) matches any pattern; a bare variable it meets is bound to
/// `Unknown` so [`substitute`] propagates the unresolved-ness rather than inventing a
/// type.
fn unify(
    pattern: &ParameterType,
    concrete: &ParameterType,
    bindings: &mut BTreeMap<Symbol, ParameterType>,
    strict: bool,
) -> bool {
    // `RES ` is transparent to unification (historically stripped by `parse` before
    // the matcher): unwrap it on either side so a `Var` binds to the unwrapped inner
    // (dropping the marker, exactly as before) and container recursion is unperturbed.
    if let ParameterType::Res(inner) = concrete {
        return unify(pattern, inner, bindings, strict);
    }
    if let ParameterType::Res(inner) = pattern {
        return unify(inner, concrete, bindings, strict);
    }
    if matches!(concrete, ParameterType::Unknown) {
        if let ParameterType::Var(name) = pattern {
            bindings.entry(*name).or_insert(ParameterType::Unknown);
        }
        return true;
    }

    match (pattern, concrete) {
        (ParameterType::Var(name), _) => {
            // STRICT validation: a type variable never binds to `Nothing`. A
            // value-returning callback parameter (`transform`'s `FUNC(T) AS U`) is not
            // satisfied by a `SUB` / `FUNC(..) AS Nothing` argument, and no value can be
            // "nothing" (bug-443). The lenient dispatch/inference path still binds it, so
            // `Nothing`-returning callbacks (`forEach`) keep lowering.
            if strict && matches!(concrete, ParameterType::Nothing) {
                return false;
            }
            match bindings.get(name) {
                // A prior binding to `Unknown` (an earlier *unresolved* occurrence of this
                // variable) is REFINED by a later concrete occurrence, not treated as a
                // conflict: `Unknown` means "not yet resolved", so a re-occurring variable
                // whose first sighting was an unknown-typed argument still unifies with a
                // later concrete one — `send(Thread OF Unknown TO Out, Integer)` binds
                // `Msg = Unknown` from the handle then refines it to `Integer` from the
                // message arg, instead of failing `resource_base_eq(Unknown, Integer)`.
                Some(ParameterType::Unknown) => {
                    bindings.insert(*name, concrete.clone());
                    true
                }
                // A re-occurring variable must match its binding — but resource element
                // types compare STATE/ownership-agnostically (bug-427): a bound element
                // `Handle STATE Cursor` accepts an item spelled `Handle`, mirroring
                // `general::element_accepts_item`. `resource_base_eq` is plain `==` for
                // every non-resource type.
                Some(bound) => resource_base_eq(bound, concrete),
                None => {
                    bindings.insert(*name, concrete.clone());
                    true
                }
            }
        }
        (ParameterType::ListOf(elem), ParameterType::ListOf(concrete_elem)) => {
            unify(elem, concrete_elem, bindings, strict)
        }
        (ParameterType::SetOf(elem), ParameterType::SetOf(concrete_elem)) => {
            unify(elem, concrete_elem, bindings, strict)
        }
        (ParameterType::MapOf(key, value), ParameterType::MapOf(concrete_key, concrete_value)) => {
            unify(key, concrete_key, bindings, strict)
                && unify(value, concrete_value, bindings, strict)
        }
        (
            ParameterType::MapEntryOf(key, value),
            ParameterType::MapEntryOf(concrete_key, concrete_value),
        ) => {
            unify(key, concrete_key, bindings, strict)
                && unify(value, concrete_value, bindings, strict)
        }
        (ParameterType::ResultOf(success), ParameterType::ResultOf(concrete_success)) => {
            unify(success, concrete_success, bindings, strict)
        }
        // A user generic unifies head-then-arguments, exactly like the container arms
        // (plan-105-B). Same template name and same arity, then each argument
        // recursively — so `Stack OF T` binds `T` from `Stack OF Integer` instead of
        // comparing two opaque `Named` blobs by string equality.
        (
            ParameterType::UserOf(name, args),
            ParameterType::UserOf(concrete_name, concrete_args),
        ) => {
            name == concrete_name
                && args.len() == concrete_args.len()
                && args
                    .iter()
                    .zip(concrete_args.iter())
                    .all(|(a, c)| unify(a, c, bindings, strict))
        }
        (
            ParameterType::Func(params, ret, isolated),
            ParameterType::Func(concrete_params, concrete_ret, concrete_isolated),
        ) => {
            // An isolated worker entry (`thread::start`) only accepts an isolated
            // concrete, and a plain callback param only a plain concrete.
            isolated == concrete_isolated
                && params.len() == concrete_params.len()
                && params
                    .iter()
                    .zip(concrete_params.iter())
                    .all(|(p, c)| unify(p, c, bindings, strict))
                && unify(ret, concrete_ret, bindings, strict)
        }
        // Two thread handles unify structurally, exactly like the container arms: the
        // kind (parent `Thread` vs worker `ThreadWorker`) must match — the two never
        // interconvert — and each slot unifies via [`thread_slot_unifies`]. A slot the
        // member does NOT echo is spelled `Unknown` and wildcards (accepting any
        // concrete slot, including a `Nothing` message/output or an absent resource
        // plane); an echoed slot is a `Var`/concrete and unifies STATE-agnostically
        // (bug-427), so a `File STATE Cursor` handle satisfies a `File` plane and a
        // data-only handle's `Nothing` resource plane fails a `Var` `res` under strict.
        (
            ParameterType::ThreadHandle {
                worker: p_worker,
                msg: p_msg,
                res: p_res,
                out: p_out,
            },
            ParameterType::ThreadHandle {
                worker: c_worker,
                msg: c_msg,
                res: c_res,
                out: c_out,
            },
        ) => {
            p_worker == c_worker
                && thread_slot_unifies(p_msg, c_msg, bindings, strict)
                && thread_slot_unifies(p_out, c_out, bindings, strict)
                && thread_slot_unifies(p_res, c_res, bindings, strict)
        }
        // A container/function/thread pattern against a non-matching concrete fails.
        (
            ParameterType::ListOf(_)
            | ParameterType::SetOf(_)
            | ParameterType::MapOf(_, _)
            | ParameterType::MapEntryOf(_, _)
            | ParameterType::ResultOf(_)
            | ParameterType::UserOf(_, _)
            | ParameterType::Func(_, _, _)
            | ParameterType::ThreadHandle { .. },
            _,
        ) => false,
        // Scalar or nominal leaf.
        (leaf, _) => leaf_matches(leaf, concrete, strict),
    }
}

/// Unify one slot (`msg`/`res`/`out`) of a [`ParameterType::ThreadHandle`]. A member
/// that does NOT echo the slot spells it [`ParameterType::Unknown`], which wildcards —
/// it accepts any concrete slot, including a `Nothing` message/output (a resource-only
/// or `Nothing`-returning thread) or an absent resource plane — so the strict-`Nothing`
/// guard only bites where a slot is genuinely captured. A member that ECHOES the slot
/// spells it a `Var`/concrete, which unifies STATE-agnostically through the normal
/// recursion (the `Var` arm uses [`resource_base_eq`]); a data-only concrete `res`
/// (`Nothing`) then fails the `Var`'s strict-`Nothing` guard, correctly rejecting
/// `accept`/`transfer` on a resource-free handle exactly as the legacy resolver did.
fn thread_slot_unifies(
    pattern: &ParameterType,
    concrete: &ParameterType,
    bindings: &mut BTreeMap<Symbol, ParameterType>,
    strict: bool,
) -> bool {
    if matches!(pattern, ParameterType::Unknown) {
        return true;
    }
    unify(pattern, concrete, bindings, strict)
}

/// STATE/ownership-agnostic type equality, matching
/// `general::element_accepts_item`: two resource types with the same base name (a
/// trailing `STATE T` clause stripped) are compatible, and every non-resource type
/// reduces to plain `==` (`base_resource_name` is the identity there).
fn resource_base_eq(a: &ParameterType, b: &ParameterType) -> bool {
    if a == b {
        return true;
    }
    a.without_state() == b.without_state()
}

/// Substitute `bindings` into a (possibly generic) type `pattern`, producing a
/// concrete type — or `None` if it names a variable that never got bound (a
/// `List OF T` return whose `T` no argument pinned down, e.g. `get` on an `Unknown`).
fn substitute(
    pattern: &ParameterType,
    bindings: &BTreeMap<Symbol, ParameterType>,
) -> Option<ParameterType> {
    Some(match pattern {
        ParameterType::Var(name) => bindings.get(name)?.clone(),
        ParameterType::ListOf(elem) => ParameterType::list_of(substitute(elem, bindings)?),
        ParameterType::SetOf(elem) => ParameterType::set_of(substitute(elem, bindings)?),
        ParameterType::MapOf(key, value) => {
            ParameterType::map_of(substitute(key, bindings)?, substitute(value, bindings)?)
        }
        ParameterType::MapEntryOf(key, value) => {
            ParameterType::map_entry_of(substitute(key, bindings)?, substitute(value, bindings)?)
        }
        ParameterType::ResultOf(success) => {
            ParameterType::result_of(substitute(success, bindings)?)
        }
        ParameterType::UserOf(name, args) => ParameterType::UserOf(
            *name,
            args.iter()
                .map(|a| substitute(a, bindings))
                .collect::<Option<Vec<_>>>()?,
        ),
        ParameterType::Func(params, ret, isolated) => {
            let params = params
                .iter()
                .map(|p| substitute(p, bindings))
                .collect::<Option<Vec<_>>>()?;
            let ret = substitute(ret, bindings)?;
            if *isolated {
                ParameterType::func_isolated(params, ret)
            } else {
                ParameterType::func(params, ret)
            }
        }
        // A thread-handle return (only `start` builds one) rebuilds each slot with the
        // bindings unified from the call — the fresh parent `Thread` handle whose
        // `msg`/`res`/`out` echo the worker's, exactly like the `List`/`Map` arms.
        ParameterType::ThreadHandle {
            worker,
            msg,
            res,
            out,
        } => ParameterType::thread_handle(
            *worker,
            substitute(msg, bindings)?,
            substitute(res, bindings)?,
            substitute(out, bindings)?,
        ),
        // A `RES`-wrapped pattern substitutes its inner (descriptors don't build one
        // today, but keep it structural rather than falling through to a bare clone).
        ParameterType::Res(inner) => ParameterType::res(substitute(inner, bindings)?),
        other => other.clone(),
    })
}

/// Whether a type mentions any [`ParameterType::Var`] — i.e. it is generic and has no
/// single static nominal type independent of a call's arguments.
fn contains_var(ty: &ParameterType) -> bool {
    match ty {
        // `Var` is arg-dependent; `Arg(_)` echoes an argument verbatim — neither has a
        // single static nominal type independent of the call.
        ParameterType::Var(_) | ParameterType::Arg(_) => true,
        ParameterType::ListOf(elem) | ParameterType::SetOf(elem) => contains_var(elem),
        ParameterType::ResultOf(success) => contains_var(success),
        ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
            contains_var(key) || contains_var(value)
        }
        ParameterType::UserOf(_, args) => args.iter().any(contains_var),
        ParameterType::Func(params, ret, _) => params.iter().any(contains_var) || contains_var(ret),
        ParameterType::ThreadHandle { msg, res, out, .. } => {
            contains_var(msg) || contains_var(res) || contains_var(out)
        }
        ParameterType::Res(inner) => contains_var(inner),
        _ => false,
    }
}
/// The spelling form of [`resolve_call_typed`], for the per-package
/// registration tests **only**.
///
/// plan-111-C collapsed this query's dual API: `resolve_call_typed` is the one
/// production entry, and the `&[String]`-in/`Option<String>`-out original is
/// gone. What is left is ~140 registration assertions across the per-package
/// modules, and a SPELLING is the right thing for those to assert — a
/// descriptor's job is to resolve to a particular type, and its name is how the
/// test says which. Rewriting each as `.map(|t| t.name().into_owned())` would
/// make them harder to read for no behavioural gain, so the shim is
/// `#[cfg(test)]` and cannot become a second production API.
#[cfg(test)]
pub(crate) fn resolve_call(qualified: &str, arg_types: &[String], strict: bool) -> Option<String> {
    let args: Vec<ParameterType> = arg_types.iter().map(|a| ParameterType::parse(a)).collect();
    resolve_call_typed(qualified, &args, strict).map(|type_| type_.name().into_owned())
}

/// The typed resolution entry (plan-104-C): the same selection as
/// [`resolve_call`] with **no parse** — codegen already holds `ParameterType`
/// arguments. An [`ParameterType::Arg`]-marked return echoes the caller's typed
/// argument (whose `name()` round-trips the original spelling, `RES` markers
/// included). The string [`resolve_call`] is a thin wrapper over the same
/// [`resolved_return_type`] core, so there is exactly one algorithm.
pub(crate) fn resolve_call_typed(
    qualified: &str,
    arg_types: &[ParameterType],
    strict: bool,
) -> Option<ParameterType> {
    let call = CallShape {
        args: arg_types.to_vec(),
    };
    resolved_return_type(qualified, &call, strict).map(|return_type| match return_type {
        ParameterType::Arg(n) => arg_types[n].clone(),
        other => other,
    })
}

/// The one overload-selection core behind [`resolve_call`] and
/// [`resolve_call_typed`]: select an overload for `call` and hand back its raw
/// substituted return type — including an unresolved [`ParameterType::Arg`]
/// marker, which each entry point resolves against its own argument
/// representation (the string form echoes the caller's original string
/// verbatim; the typed form echoes the typed argument).
fn resolved_return_type(qualified: &str, call: &CallShape, strict: bool) -> Option<ParameterType> {
    let function = registry().resolve_func(qualified)?.function;
    // `strict` (argument validation) rejects a scalar-for-nominal argument; the lenient
    // mode (return-type inference feeding IR lowering / codegen) coarsely accepts it.
    let selection = if strict {
        function.resolve(call)
    } else {
        function.dispatch(call)
    };
    selection.map(|selection| selection.return_type)
}

/// The internal symbol the migrated call `qualified` rewrites to at IR lowering, or
/// `None`. Overload-aware: an arity-routed member (datetime's `instant`/`parse`, whose
/// overloads rewrite to `__datetime_instant{N}`) carries a distinct rewrite target per
/// overload, so the call's argument types select which one. A single-overload member
/// resolves the same regardless of the arguments.
pub(crate) fn rewrite_target(qualified: &str, arg_types: &[ParameterType]) -> Option<&'static str> {
    let function = registry().resolve_func(qualified)?.function;
    let call = CallShape {
        args: arg_types.to_vec(),
    };
    // Prefer STRICT selection: a call whose arguments precisely name one overload's
    // types picks that overload. This is required to disambiguate two overloads that
    // differ only by a resource-nominal parameter — `http::handleRequest`'s
    // `net::Listener` vs `tls::Listener` forms, which each rewrite to a distinct
    // transport body. Lenient `dispatch` treats unequal resource nominals as
    // interchangeable (kept coarse so a not-yet-resolved argument does not perturb
    // overload/return inference on valid programs) and would resolve both to the first
    // form. `resolve`'s `resource_base_eq` rejects the mismatched nominal, so the tls
    // form selects `__http_handleRequestSSL`. Fall back to lenient dispatch (imprecise
    // argument types) then to the sole/first implementation.
    if let Some(selection) = function.resolve(&call).or_else(|| function.dispatch(&call)) {
        return selection.implementation.body.rewrite_target();
    }
    // The call shape did not select an overload (e.g. unknown argument types); fall
    // back to the sole/first implementation — unambiguous for a single-overload member.
    function.implementations.first()?.body.rewrite_target()
}

/// The qualified member whose call lowering rewrites to the internal symbol
/// `target` (either spelling: the descriptor's `__pkg_name` or the internalized
/// `#pkg_name` the IR carries), or `None` when no member rewrites to it. The
/// inverse of [`rewrite_target`], for the IR-level checks that must see a
/// rewritten call as the builtin the source wrote (plan-107-E).
pub(crate) fn rewrite_owner(target: &str) -> Option<String> {
    for package in registry().packages() {
        for function in package.functions() {
            for implementation in function.implementations() {
                let Some(rewrite) = implementation.body.rewrite_target() else {
                    continue;
                };
                if rewrite == target || crate::internal_name::internalize(rewrite) == target {
                    return Some(format!("{}.{}", package.import_name(), function.name));
                }
            }
        }
    }
    None
}

/// The [`AbiInline`] lowering for `qualified`, or `None`. The inline dual-path
/// (`try_abi_inline_lower`) consults this at the call site.
pub(crate) fn abi_inline_lower(qualified: &str) -> Option<AbiInline> {
    for implementation in &registry().resolve_func(qualified)?.function.implementations {
        if let Body::AbiInline(lower) = &implementation.body {
            return Some(*lower);
        }
    }
    None
}

/// The [`AbiFunction`] lowering for `qualified`, plus the member's parameter count (so
/// the runtime-helper wrapper can bind that many incoming ABI argument registers).
/// `None` when `qualified` is not an `abi_function` member.
pub(crate) fn abi_function_lower(qualified: &str) -> Option<(AbiFunction, usize)> {
    // Direct member lookup: the member's own qualified name.
    if let Some(resolved) = registry().resolve_func(qualified) {
        for implementation in &resolved.function.implementations {
            if let Body::AbiFunction { lower, .. } = &implementation.body {
                return Some((*lower, implementation.params.len()));
            }
        }
    }
    // Fall back to an `os_aliases` code form of an `abi_function` member — the
    // IR-level overload-split runtime calls (`net.connectTcpAddr`/`net.pollList`;
    // `audio.openInputDevice`/`readTimeout`/…) that are not descriptor members but
    // share a member's body. The wrapper binds the owning implementation's parameter
    // count; the shared body reads `AbiCtx::call` to pick the alias arm.
    let (pkg_name, member) = qualified.split_once('.')?;
    let package = registry()
        .packages()
        .iter()
        .find(|p| p.import_name == pkg_name)?;
    for function in package.functions() {
        for implementation in function.implementations() {
            if let Body::AbiFunction { lower, os_aliases } = &implementation.body {
                if os_aliases.contains(&member) {
                    return Some((*lower, implementation.params.len()));
                }
            }
        }
    }
    None
}

/// Whether `qualified` names an [`AbiFunction`] member (a runtime-helper-backed
/// unified lowering). Routes `helper_for_call` to classify it as a runtime call so
/// the IR emits a `RuntimeCall` for it.
pub(crate) fn is_abi_function_call(qualified: &str) -> bool {
    abi_function_lower(qualified).is_some()
}

/// The inline-`TRAP` fallibility of a migrated **inline-native** member (a
/// [`Body::AbiInline`] call-site lowering — the `bits` ops,
/// collections' `get`/`transform`/…): `Some(true)` when it declares at least one
/// error (so an inline `TRAP` on it must route through the raw-capture path),
/// `Some(false)` when it declares none (an inline `TRAP` is always-`Ok`), and
/// `None` when `qualified` is not an inline-native member. This grounds the
/// inline-`TRAP` fallibility census (`builtins::inline_builtin_raw_supported` /
/// `inline_builtin_is_infallible`) in registry data instead of a per-package
/// name predicate (`is_bits_shift`).
pub(crate) fn native_member_declares_error(qualified: &str) -> Option<bool> {
    let function = registry().resolve_func(qualified)?.function;
    // An inline call-site lowering is the `abi_inline` mode (pre-lowered `bits`, or
    // self-lowering collections/strings); it feeds the inline-`TRAP` fallibility
    // census, so a fallible migrated member is recognized as fallible.
    let mut inline_native = false;
    let mut declares = false;
    for implementation in &function.implementations {
        if matches!(implementation.body, Body::AbiInline(_)) {
            inline_native = true;
            if !implementation.errors.is_empty() {
                declares = true;
            }
        }
    }
    inline_native.then_some(declares)
}

/// The native HOF **fast path** for a generic `Body::Mfb` monomorph `target`
/// (`#collections_sort$Integer` → `collections::sort`'s fast path), or `None` when
/// the member's implementation carries none. The `try_mfb_fast_path` codegen seam
/// consults this before instantiating the injected `.mfb` body. The fast path rides
/// on the member's [`Body::Mfb`] `fast_path` slot
/// ([`Body::mfb_with_fast_path`]), so the accelerator lives beside the body it
/// accelerates in the member's `func_*.rs`.
pub(crate) fn mfb_fast_path(target: &str) -> Option<MfbFastPath> {
    let (pkg_name, rest) = target.strip_prefix('#')?.split_once('_')?;
    let member = rest.split('$').next()?;
    registry()
        .packages()
        .iter()
        .find(|p| p.import_name == pkg_name)?
        .function(member)?
        .implementations
        .iter()
        .find_map(|implementation| match &implementation.body {
            Body::Mfb {
                fast_path: Some(fast_path),
                ..
            } => Some(*fast_path),
            _ => None,
        })
}

/// The bare native-codegen name a migrated call `qualified` dequalifies to for the
/// legacy bare-name native path (`collections.get` → `get`), or `None`. A member
/// qualifies when it owns a [`Body::AbiInline`] **call-site** lowering — the
/// collections native members (`get`, `set`, `transform`, …). This is the generic
/// form of the old `collections::native_member_bare`; it deliberately yields `None`
/// for the OS-seam members (`Body::AbiFunction`, which lower to a runtime helper, not
/// a bare inline op) and for the source-backed intrinsics (`encoding`), which are not
/// bare-name native members. The three `Body::Intrinsic` List overloads
/// (`find`/`mid`/`replace`) are handled by their caller
/// (`crate::codegen::builtins::native_builtin_target`), which shares them with `strings::`.
pub(crate) fn native_bare_target(qualified: &str) -> Option<&'static str> {
    let function = registry().resolve_func(qualified)?.function;
    for implementation in &function.implementations {
        // The `abi_inline` variant is an inline call-site lowering and dequalifies to
        // the bare native name. `abi_function` (a `bl`'d runtime helper) is NOT an
        // inline call-site lowering, so it is excluded.
        if matches!(implementation.body, Body::AbiInline(_)) {
            return Some(function.name);
        }
    }
    None
}

/// Whether the migrated call `qualified` takes a **unary callback** — a parameter
/// whose type is a `FUNC(<one param>) AS <ret>` (a [`ParameterType::Func`] with
/// exactly one parameter). These are the higher-order members whose callback
/// parameter type is the collection's element type (not written at the call site),
/// so a bare general built-in predicate at that position must be bound before it can
/// be typed (bug-368): `collections::filter`/`transform`/`forEach`. A binary callback
/// (`reduce`'s `FUNC(U, T) AS U`) has two parameters and is excluded. Generic form of
/// the old `collections::unary_callback_member`.
pub(crate) fn callback_member(qualified: &str) -> bool {
    registry()
        .resolve_func(qualified)
        .is_some_and(|resolved| function_has_unary_callback(resolved.function))
}

/// The bare-member form of [`callback_member`], for the unqualified call spelling
/// (`filter`, `transform`, `forEach`) that reaches `ir::lower` before
/// canonicalization. Matches any migrated package's function of that bare name with a
/// unary-callback parameter.
pub(crate) fn callback_member_bare(member: &str) -> bool {
    registry().packages().iter().any(|package| {
        package
            .function(member)
            .is_some_and(function_has_unary_callback)
    })
}

/// Whether any of `function`'s implementations declares a parameter of type
/// `FUNC(<one param>) AS <ret>` (a unary function value).
fn function_has_unary_callback(function: &RegistryFunction) -> bool {
    function.implementations.iter().any(|implementation| {
        implementation.params.iter().any(
            |param| matches!(&param.ty, ParameterType::Func(params, _, _) if params.len() == 1),
        )
    })
}

/// The **machine** argument-coercion table for a migrated call: each parameter's
/// concrete type name in signature order (`json.getOr` → `["Json", "List OF String",
/// "Json"]`). `None` for an overload set (>1 implementation — no single positional
/// signature) or a member with a generic (`Var`) parameter (`collections`'s
/// `List OF T` — nothing concrete to coerce against).
///
/// Deliberately SEPARATE from [`expected_arguments`] (the human diagnostic string,
/// which carries optional-tail brackets / unions the coercion path must not see):
/// IR lowering consumes THIS to decide per-argument expected types (e.g. union
/// wrapping), so widening the diagnostic wording never perturbs codegen (bug-443).
pub(crate) fn argument_types(qualified: &str) -> Option<Vec<String>> {
    Some(
        argument_types_typed(qualified)?
            .into_iter()
            .map(|ty| ty.name().into_owned())
            .collect(),
    )
}

/// The typed twin of [`argument_types`] (plan-106-A): the descriptor's parameter
/// types cloned rather than rendered.
pub(crate) fn argument_types_typed(qualified: &str) -> Option<Vec<ParameterType>> {
    let function = &registry().resolve_func(qualified)?.function;
    if function.implementations.len() > 1 {
        return None;
    }
    let params = &function.implementations.first()?.params;
    if params
        .iter()
        .any(|param| contains_var(&param.ty) || matches!(param.ty, ParameterType::Arg(_)))
    {
        return None;
    }
    Some(params.iter().map(|param| param.ty.clone()).collect())
}

/// The expected type of argument `index` when EVERY overload agrees on it, for an
/// overloaded member that [`argument_types_typed`] declines to answer for.
///
/// plan-120-D. `argument_types_typed` returns `None` for an overload set because
/// the positions can disagree, and IR lowering uses it to decide per-argument
/// expected types — **including union wrapping**. So the moment a member gains a
/// second overload, a union-typed parameter stops being wrapped, and a call that
/// passes a union MEMBER type silently lowers the bare record where the callee
/// expects a tagged union. The symptom is not a diagnostic: it is wrong output.
/// `json::stringify(json::JsonNull[NOTHING])` returned "" and
/// `json::stringify(json::JsonStr["Ada"])` returned "null" — the tag read from the
/// wrong place — the moment `stringify` gained its indent overloads.
///
/// The safe answer is the one no overload disputes: if every implementation that
/// has a parameter at `index` declares the SAME type there, that type is the
/// expected type whichever overload is eventually selected, so wrapping against it
/// is correct without knowing the selection. Positions where the overloads differ
/// (`stringify`'s `indent`, `Integer` vs `String`) return `None` exactly as before,
/// leaving those to the existing selection path.
pub(crate) fn agreed_argument_type(qualified: &str, index: usize) -> Option<ParameterType> {
    let function = &registry().resolve_func(qualified)?.function;
    if function.implementations.len() < 2 {
        return None;
    }
    let mut agreed: Option<&ParameterType> = None;
    for implementation in &function.implementations {
        let param = implementation.params.get(index)?;
        if contains_var(&param.ty) || matches!(param.ty, ParameterType::Arg(_)) {
            return None;
        }
        match agreed {
            None => agreed = Some(&param.ty),
            Some(seen) if seen == &param.ty => {}
            Some(_) => return None,
        }
    }
    agreed.cloned()
}

pub(crate) fn expected_arguments(qualified: &str) -> Option<&'static str> {
    let function = &registry().resolve_func(qualified)?.function;
    // A hand-authored phrasing on the descriptor wins — the union/range/generic-`or`
    // forms the per-position render below cannot reproduce.
    if let Some(hint) = function.expected_arguments {
        return Some(hint);
    }
    // A multi-implementation (overloaded / variadic) member has no single positional
    // rendering — its overloads can disagree on position 0 — so it yields None here (a
    // package needing a diagnostic supplies the hint above).
    if function.implementations.len() > 1 {
        return None;
    }
    // Render the WHOLE parameter list (`json.get` → "Json, List OF String"), not just
    // position 0: the argument-mismatch diagnostic's "expected …" clause names every
    // parameter. Required parameters join with ", "; trailing OPTIONAL parameters
    // (`Fill`/`Optional`) render in a bracket — `regex.find` → "String, String[, Integer]".
    // This is diagnostic-only; the coercion path uses [`argument_types`] (bug-443).
    let params = &function.implementations.first()?.params;
    if params.is_empty() {
        return None;
    }
    let mut required = Vec::new();
    let mut optional = Vec::new();
    for param in params {
        if matches!(param.default, DefaultValue::None) {
            required.push(param.ty.name());
        } else {
            optional.push(param.ty.name());
        }
    }
    let mut rendered = required.join(", ");
    if !optional.is_empty() {
        let tail = optional.join(", ");
        if rendered.is_empty() {
            rendered = format!("[{tail}]");
        } else {
            rendered = format!("{rendered}[, {tail}]");
        }
    }
    Some(Box::leak(rendered.into_boxed_str()))
}

/// Whether a function's overloads disagree on their parameter-name **layout** —
/// i.e. a merged per-position table (`[name, alias…]` unioned across overloads at
/// each index) would place the same parameter *name* at two different positions
/// and therefore misbind a named argument (bug-349/bug-94). Two shapes trip this:
///
/// * a front-dropping/variadic-from-the-left constructor (datetime's
///   `instant`/`duration`/`fixedOffset`), where the same name (`seconds`) slides
///   across indices as leading parameters are added; and
/// * overloads of differing arity that share a **trailing** optional name
///   (`crypto::open`'s `aad`, at index 5 on the 6-param overload and index 4 on
///   the 5-param one).
///
/// Both are detected uniformly: build the flat position table and return true if
/// any name lands at more than one index. The position-0 disagreement is kept as
/// an explicit disjunct so this stays a strict superset of the historical check.
/// Members that trip this carry no merged [`call_param_names`] table (it returns
/// `None`) and are normalized through the per-overload
/// [`call_param_name_overloads`] table instead. A single-overload member, or
/// overloads that name every shared parameter at a consistent index (collections
/// `get`'s `value`/`collection`, encoding `utf8Decode`'s `value`), agree and merge.
fn overloads_disagree_on_layout(function: &RegistryFunction) -> bool {
    // Position-0 disagreement (historical check — kept as a strict superset guard).
    let mut first_name: Option<&str> = None;
    for implementation in &function.implementations {
        let name = implementation.params.first().map(|param| param.name);
        match (first_name, name) {
            (None, _) => first_name = name,
            (Some(seen), Some(name)) if seen == name => {}
            (Some(_), _) => return true,
        }
    }
    // Any name (or alias) that a merged per-position table would place at two
    // different indices across overloads.
    let mut seen_index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for implementation in &function.implementations {
        for (index, param) in implementation.params.iter().enumerate() {
            for name in std::iter::once(param.name).chain(param.aliases.iter().copied()) {
                match seen_index.get(name) {
                    Some(&prior) if prior != index => return true,
                    _ => {
                        seen_index.entry(name).or_insert(index);
                    }
                }
            }
        }
    }
    false
}

/// The per-position `[name, alias…]` keyword-matching lists for the migrated call
/// `qualified`, or `None`.
///
/// Overload-aware: position `i` is the union — first-seen order, deduped — of the
/// `[name, alias…]` any overload allows at position `i`. A single-overload member
/// unions to exactly its own per-position lists; an overload set that agrees on the
/// position-0 name (collections `get`/`getOr`/`set`/`append`/`contains`/`sum`,
/// encoding `utf8Encode`/`utf8Decode`) merges its overloads into one table. Members
/// whose overloads disagree on position 0 ([`overloads_disagree_on_layout`]) yield
/// `None` — a merged table would misbind (bug-349/bug-94) — and are normalized
/// through [`call_param_name_overloads`].
pub(crate) fn call_param_names(qualified: &str) -> Option<Vec<Vec<&'static str>>> {
    let function = &registry().resolve_func(qualified)?.function;
    if overloads_disagree_on_layout(function) {
        return None;
    }
    let width = function
        .implementations
        .iter()
        .map(|implementation| implementation.params.len())
        .max()
        .unwrap_or(0);
    let mut positions: Vec<Vec<&'static str>> = vec![Vec::new(); width];
    for implementation in &function.implementations {
        for (index, param) in implementation.params.iter().enumerate() {
            let slot = &mut positions[index];
            for name in std::iter::once(param.name).chain(param.aliases.iter().copied()) {
                if !slot.contains(&name) {
                    slot.push(name);
                }
            }
        }
    }
    Some(positions)
}

/// Per-overload parameter-name tables for a migrated call whose overloads place
/// names at structurally different positions ([`overloads_disagree_on_layout`] —
/// datetime `instant`/`duration`/`fixedOffset`), or `None`. Each entry is one
/// overload's parameter names, in order (no aliases: these members declare none).
/// Normalization selects the overload first, then binds names within it, so a
/// front-dropping constructor's named arguments bind to the right slot. Replaces
/// the per-package `call_param_name_overloads`. Only the three constructor families
/// qualify, and only at a call site that mixes named arguments.
pub(crate) fn call_param_name_overloads(qualified: &str) -> Option<Vec<Vec<&'static str>>> {
    let function = &registry().resolve_func(qualified)?.function;
    if !overloads_disagree_on_layout(function) {
        return None;
    }
    Some(
        function
            .implementations
            .iter()
            .map(|implementation| {
                implementation
                    .params
                    .iter()
                    .map(|param| param.name)
                    .collect()
            })
            .collect(),
    )
}

/// The `(type, expr)` constants to append after `provided` real arguments so a
/// migrated call's injected body receives its full arity — the `Fill` params past
/// `provided`. Empty when no migrated package owns `qualified`.
pub(crate) fn default_argument_padding(
    qualified: &str,
    provided: usize,
    first_argument: Option<&ParameterType>,
) -> Vec<(ParameterType, &'static str)> {
    let Some(resolved) = registry().resolve_func(qualified) else {
        return Vec::new();
    };
    let implementations = &resolved.function.implementations;
    // Which overload the call matched decides how many trailing `Fill` params it
    // is missing, and of what type. Taking `implementations.first()` is only
    // right for a single-implementation member: on an overloaded one (plan-110-D
    // gave `tls::connect` a `net::Address` form beside its host/port form) it
    // pads an Address call up to the host form's arity, producing a call with
    // one argument too many that fails type checking. Select by the first
    // argument's type when the caller can supply it, and require the overload to
    // actually have room for the arguments already provided.
    //
    // Selection is by SPELLED NAME, not `leaf_matches`: the lenient matcher
    // answers `true` for a scalar pattern against a nominal concrete (that
    // coarseness is load-bearing for container-arg overload dispatch), so it
    // would accept the host form's `String` for an `Address` argument and pick
    // the wrong padding. A name comparison is exactly the question being asked.
    let implementation = implementations
        .iter()
        .filter(|implementation| implementation.params.len() >= provided)
        .find(
            |implementation| match (first_argument, implementation.params.first()) {
                (Some(argument), Some(param)) => param.ty.name() == argument.name(),
                _ => false,
            },
        )
        // No usable first-argument type: fall back to the historical choice, the
        // first implementation. Preferring an overload the call fills exactly
        // looks tempting but is wrong — `crypto::open`'s AEAD form has an
        // exact-arity sibling at 5 arguments whose `aad` must still be padded
        // (`aead_aad_default_padding`).
        .or_else(|| implementations.first());
    let Some(implementation) = implementation else {
        return Vec::new();
    };
    implementation
        .params
        .iter()
        .skip(provided)
        .filter_map(|param| match &param.default {
            // plan-106-A: the descriptor already holds a `ParameterType`; hand it
            // back rather than rendering it for the caller to re-parse.
            DefaultValue::Fill { type_name, expr } => Some((type_name.clone(), *expr)),
            _ => None,
        })
        .collect()
}

/// The project facts the builtin-source injectors gate on: which packages the
/// program `IMPORT`s, and which call callees it names.
///
/// plan-106-D: both pipelines inject the same builtin sources — the AST one
/// (`resolver::augment_project`, before monomorphization) and the former source checker's HIR
/// one — and the gates read exactly these two things. Collecting them once, from
/// either domain, is what lets ONE injector serve both. The alternative was a
/// second copy of `is_imported_by` plus the ~100-line `references_any` AST walk
/// per domain, which is the duplication this whole plan exists to remove.
#[derive(Clone, Default)]
pub(crate) struct ProjectView {
    packages: std::collections::HashSet<String>,
    /// Every call callee reduced to its final segment across `::` and `.`.
    ///
    /// A callee may be source-qualified (`strings::toScalars`), aliased
    /// (`s::toScalars`), or already canonicalized to the dotted form
    /// (`strings.toScalars`), so [`HelperGate::WhenUsed`] matched the final
    /// segment. Reducing once at collection is the same match, done once per
    /// project instead of once per gate. Over-matching (a user's own
    /// `toScalars`) only injects a helper unnecessarily, never wrongly.
    callees: std::collections::HashSet<String>,
}

impl ProjectView {
    pub(crate) fn of_ast(ast: &crate::ast::AstProject) -> Self {
        let mut view = Self::default();
        for file in &ast.files {
            view.absorb_ast_file(file);
        }
        view
    }

    pub(crate) fn of_hir(hir: &crate::hir::HirProject) -> Self {
        let mut view = Self::default();
        for file in &hir.files {
            view.absorb_imports(&file.imports);
            for item in &file.items {
                hir_item_callees(item, &mut view.callees);
            }
        }
        view
    }

    /// Fold a newly injected source file into the view.
    ///
    /// The late passes run in a chain (`http` before `net` before `encoding`)
    /// precisely because an injected companion carries its OWN imports — the
    /// injected `http` source `IMPORT net`s, and `net`'s gate must see it. Each
    /// pass therefore re-reads a view that includes what the previous one added,
    /// exactly as the old chain re-read a progressively augmented `AstProject`.
    pub(crate) fn absorb_ast_file(&mut self, file: &crate::ast::AstFile) {
        self.absorb_imports(&file.imports);
        for item in &file.items {
            item_callees(item, &mut self.callees);
        }
    }

    fn absorb_imports(&mut self, imports: &[crate::ast::Import]) {
        for import in imports {
            self.packages.insert(import.package_name().to_string());
        }
    }

    /// Whether the program imports `package`.
    pub(crate) fn imports(&self, package: &str) -> bool {
        self.packages.contains(package)
    }

    /// Whether the program calls any of `names` (a [`HelperGate::WhenUsed`] gate).
    pub(crate) fn references_any(&self, names: &[&'static str]) -> bool {
        names.iter().any(|name| self.callees.contains(*name))
    }
}

/// Inject a late-pass package's source companion (`http`/`net`/`encoding`) into
/// an AST project.
///
/// These three are skipped by [`Registry::augment_project`]'s single pass and
/// injected in their own dependency order afterwards, because each carries
/// transitive imports the single pass cannot see (`http`'s companion `IMPORT
/// net`s). plan-106-D: one implementation, since all three bodies were identical.
pub(crate) fn inject_late_pass(
    ast: &crate::ast::AstProject,
    package: &str,
    label: &str,
    doc: &str,
) -> Result<crate::ast::AstProject, ()> {
    match late_pass_file(package, label, doc, &ProjectView::of_ast(ast))? {
        Some(file) => {
            let mut augmented = ast.clone();
            augmented.files.push(file);
            Ok(augmented)
        }
        None => Ok(ast.clone()),
    }
}

/// The same injection onto the elaborated project the former source checker consumes.
pub(crate) fn inject_late_pass_hir(
    hir: &crate::hir::HirProject,
    package: &str,
    label: &str,
    doc: &str,
) -> Result<crate::hir::HirProject, ()> {
    match late_pass_file(package, label, doc, &ProjectView::of_hir(hir))? {
        Some(file) => {
            let mut augmented = hir.clone();
            augmented.files.push(crate::hir::elaborate_file(&file));
            Ok(augmented)
        }
        None => Ok(hir.clone()),
    }
}

fn late_pass_file(
    package: &str,
    label: &str,
    doc: &str,
    view: &ProjectView,
) -> Result<Option<crate::ast::AstFile>, ()> {
    let Some(pkg) = registry().resolve_package(package) else {
        return Ok(None);
    };
    if !pkg.is_imported_by(view) {
        return Ok(None);
    }
    Ok(Some(crate::ast::parse_source_internal(
        std::path::Path::new(label),
        doc,
        &pkg.get_mfb(),
    )?))
}

/// The final segment of a callee across `::` and `.` — the form a `WhenUsed` gate
/// matches.
fn short_callee(callee: &str) -> &str {
    callee
        .rsplit("::")
        .next()
        .unwrap_or(callee)
        .rsplit('.')
        .next()
        .unwrap_or(callee)
}

fn item_callees(item: &crate::ast::Item, out: &mut std::collections::HashSet<String>) {
    use crate::ast::Item;
    match item {
        Item::Function(f) => f.body.iter().for_each(|s| stmt_callees(s, out)),
        Item::Binding(b) => {
            if let Some(value) = &b.value {
                expr_callees(value, out);
            }
        }
        Item::Testing(block) => block.groups.iter().for_each(|g| group_callees(g, out)),
        _ => {}
    }
}

/// A `TESTING` block is the verbatim AST node in HIR too, so one walk serves both.
fn group_callees(group: &crate::ast::TestGroup, out: &mut std::collections::HashSet<String>) {
    use crate::ast::TestGroupMember;
    for member in &group.members {
        match member {
            TestGroupMember::Case(case) => case.body.iter().for_each(|s| stmt_callees(s, out)),
            TestGroupMember::Group(nested) => group_callees(nested, out),
        }
    }
}

fn stmt_callees(stmt: &crate::ast::Statement, out: &mut std::collections::HashSet<String>) {
    use crate::ast::Statement;
    match stmt {
        Statement::Let { value, .. }
        | Statement::Return { value, .. }
        | Statement::Recover { value, .. }
        | Statement::Exit { code: value, .. } => {
            if let Some(value) = value {
                expr_callees(value, out);
            }
        }
        Statement::Fail { error, .. } => expr_callees(error, out),
        Statement::Assign { value, .. } | Statement::StateAssign { value, .. } => {
            expr_callees(value, out)
        }
        Statement::Expression { expression, .. } => expr_callees(expression, out),
        Statement::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            expr_callees(condition, out);
            then_body.iter().for_each(|s| stmt_callees(s, out));
            else_body.iter().for_each(|s| stmt_callees(s, out));
        }
        Statement::Match {
            expression, cases, ..
        } => {
            expr_callees(expression, out);
            for case in cases {
                case.body.iter().for_each(|s| stmt_callees(s, out));
            }
        }
        Statement::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            expr_callees(start, out);
            expr_callees(end, out);
            if let Some(step) = step {
                expr_callees(step, out);
            }
            body.iter().for_each(|s| stmt_callees(s, out));
        }
        Statement::ForEach { iterable, body, .. } => {
            expr_callees(iterable, out);
            body.iter().for_each(|s| stmt_callees(s, out));
        }
        Statement::While {
            condition, body, ..
        }
        | Statement::DoUntil {
            condition, body, ..
        } => {
            expr_callees(condition, out);
            body.iter().for_each(|s| stmt_callees(s, out));
        }
        Statement::Continue { .. } | Statement::Propagate { .. } => {}
    }
}

fn expr_callees(expr: &crate::ast::Expression, out: &mut std::collections::HashSet<String>) {
    use crate::ast::{CallArg, ConstructorArg, Expression};
    match expr {
        Expression::Call {
            callee, arguments, ..
        } => {
            out.insert(short_callee(callee).to_string());
            for argument in arguments {
                match argument {
                    CallArg::Positional(v) | CallArg::Named { value: v, .. } => {
                        expr_callees(v, out)
                    }
                }
            }
        }
        Expression::Binary { left, right, .. } => {
            expr_callees(left, out);
            expr_callees(right, out);
        }
        Expression::Unary { operand, .. } => expr_callees(operand, out),
        Expression::Lambda { body, .. } => expr_callees(body, out),
        Expression::Constructor { arguments, .. } => {
            for argument in arguments {
                match argument {
                    ConstructorArg::Positional(v) | ConstructorArg::Named { value: v, .. } => {
                        expr_callees(v, out)
                    }
                }
            }
        }
        Expression::WithUpdate { target, updates } => {
            expr_callees(target, out);
            updates.iter().for_each(|u| expr_callees(&u.value, out));
        }
        Expression::ListLiteral(values) => values.iter().for_each(|v| expr_callees(v, out)),
        Expression::SetLiteral { elements, .. } => {
            elements.iter().for_each(|v| expr_callees(v, out))
        }
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                expr_callees(key, out);
                expr_callees(value, out);
            }
        }
        Expression::MemberAccess { target, .. } => expr_callees(target, out),
        Expression::Trapped {
            expression,
            handler,
            ..
        } => {
            expr_callees(expression, out);
            handler.iter().for_each(|s| stmt_callees(s, out));
        }
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Scalar(_)
        | Expression::Boolean(_)
        | Expression::Identifier(_) => {}
    }
}

fn hir_item_callees(item: &crate::hir::HirItem, out: &mut std::collections::HashSet<String>) {
    use crate::hir::HirItem;
    match item {
        HirItem::Function(f) => f.body.iter().for_each(|s| hir_stmt_callees(s, out)),
        HirItem::Binding(b) => {
            if let Some(value) = &b.value {
                hir_expr_callees(value, out);
            }
        }
        // HIR reuses the AST `TestingBlock` verbatim.
        HirItem::Testing(block) => block.groups.iter().for_each(|g| group_callees(g, out)),
        _ => {}
    }
}

fn hir_stmt_callees(stmt: &crate::hir::HirStatement, out: &mut std::collections::HashSet<String>) {
    use crate::hir::HirStatement;
    match stmt {
        HirStatement::Let { value, .. }
        | HirStatement::Return { value, .. }
        | HirStatement::Recover { value, .. }
        | HirStatement::Exit { code: value, .. } => {
            if let Some(value) = value {
                hir_expr_callees(value, out);
            }
        }
        HirStatement::Fail { error, .. } => hir_expr_callees(error, out),
        HirStatement::Assign { value, .. } | HirStatement::StateAssign { value, .. } => {
            hir_expr_callees(value, out)
        }
        HirStatement::Expression { expression, .. } => hir_expr_callees(expression, out),
        HirStatement::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            hir_expr_callees(condition, out);
            then_body.iter().for_each(|s| hir_stmt_callees(s, out));
            else_body.iter().for_each(|s| hir_stmt_callees(s, out));
        }
        HirStatement::Match {
            expression, cases, ..
        } => {
            hir_expr_callees(expression, out);
            for case in cases {
                case.body.iter().for_each(|s| hir_stmt_callees(s, out));
            }
        }
        HirStatement::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            hir_expr_callees(start, out);
            hir_expr_callees(end, out);
            if let Some(step) = step {
                hir_expr_callees(step, out);
            }
            body.iter().for_each(|s| hir_stmt_callees(s, out));
        }
        HirStatement::ForEach { iterable, body, .. } => {
            hir_expr_callees(iterable, out);
            body.iter().for_each(|s| hir_stmt_callees(s, out));
        }
        HirStatement::While {
            condition, body, ..
        }
        | HirStatement::DoUntil {
            condition, body, ..
        } => {
            hir_expr_callees(condition, out);
            body.iter().for_each(|s| hir_stmt_callees(s, out));
        }
        HirStatement::Continue { .. } | HirStatement::Propagate { .. } => {}
    }
}

fn hir_expr_callees(expr: &crate::hir::HirExpression, out: &mut std::collections::HashSet<String>) {
    use crate::hir::{HirCallArg, HirConstructorArg, HirExpression};
    match expr {
        HirExpression::Call {
            callee, arguments, ..
        } => {
            out.insert(short_callee(callee).to_string());
            for argument in arguments {
                match argument {
                    HirCallArg::Positional(v) | HirCallArg::Named { value: v, .. } => {
                        hir_expr_callees(v, out)
                    }
                }
            }
        }
        HirExpression::Binary { left, right, .. } => {
            hir_expr_callees(left, out);
            hir_expr_callees(right, out);
        }
        HirExpression::Unary { operand, .. } => hir_expr_callees(operand, out),
        HirExpression::Lambda { body, .. } => hir_expr_callees(body, out),
        HirExpression::Constructor { arguments, .. } => {
            for argument in arguments {
                match argument {
                    HirConstructorArg::Positional(v)
                    | HirConstructorArg::Named { value: v, .. } => hir_expr_callees(v, out),
                }
            }
        }
        HirExpression::WithUpdate { target, updates } => {
            hir_expr_callees(target, out);
            updates.iter().for_each(|u| hir_expr_callees(&u.value, out));
        }
        HirExpression::ListLiteral(values) => values.iter().for_each(|v| hir_expr_callees(v, out)),
        HirExpression::SetLiteral { elements, .. } => {
            elements.iter().for_each(|v| hir_expr_callees(v, out))
        }
        HirExpression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                hir_expr_callees(key, out);
                hir_expr_callees(value, out);
            }
        }
        HirExpression::MemberAccess { target, .. } => hir_expr_callees(target, out),
        HirExpression::Trapped {
            expression,
            handler,
            ..
        } => {
            hir_expr_callees(expression, out);
            handler.iter().for_each(|s| hir_stmt_callees(s, out));
        }
        HirExpression::String(_)
        | HirExpression::Number(_)
        | HirExpression::Scalar(_)
        | HirExpression::Boolean(_)
        | HirExpression::Identifier(_) => {}
    }
}

#[cfg(test)]
mod qualification_tests {
    use super::*;
    use std::collections::HashMap;

    /// `net` and `udp` each declare a type of their own; `Socket` is declared by
    /// two packages, so a third package naming it bare declares no such type.
    fn owners() -> HashMap<String, Vec<String>> {
        HashMap::from([
            ("Address".to_string(), vec!["net".to_string()]),
            ("Datagram".to_string(), vec!["udp".to_string()]),
            (
                "Socket".to_string(),
                vec!["tcp".to_string(), "udp".to_string()],
            ),
        ])
    }

    fn for_source(ty: &str, package: &str) -> String {
        qualify_type_leaves_for_source(&ParameterType::declared(ty), package, &owners())
            .name()
            .into_owned()
    }

    fn for_signature(ty: &str, package: &str) -> String {
        qualify_type_leaves(&ParameterType::declared(ty), package, &owners())
            .name()
            .into_owned()
    }

    /// The governing rule, source side: a locally-declared name needs no prefix.
    /// `net` declares `Address`, `udp` declares `Datagram`, so each keeps the bare
    /// leaf in its own rendered companion — `regex`'s PRIVATE `#regex_Cont` is the
    /// case that matters, since prefixing it stopped the descriptor's field type
    /// matching the local declaration the parser produces (bug-484).
    #[test]
    fn a_rendered_field_leaves_a_local_name_bare() {
        assert_eq!(for_source("Address", "net"), "Address");
        assert_eq!(for_source("Datagram", "udp"), "Datagram");
        assert_eq!(for_source("List OF Datagram", "udp"), "List OF Datagram");
    }

    /// A signature is a type-system identity, never rendered back into source, so
    /// it carries the qualified id even for the declaring package's own types.
    #[test]
    fn a_signature_qualifies_its_own_package() {
        assert_eq!(for_signature("Address", "net"), "net.Address");
        assert_eq!(for_signature("Datagram", "udp"), "udp.Datagram");
    }

    /// A cross-package reference is spelled out by the descriptor author, and both
    /// passes then leave it exactly as written.
    #[test]
    fn a_spelled_out_cross_package_reference_is_left_alone() {
        assert_eq!(for_source("net.Address", "udp"), "net.Address");
        assert_eq!(for_signature("net.Address", "udp"), "net.Address");
        assert_eq!(
            for_source("List OF net.Address", "udp"),
            "List OF net.Address"
        );
    }

    /// Names that are not registry-declared types — scalar nominals, C ABI
    /// spellings, generic parameters — are left exactly as written, and must not
    /// trip the not-declared-here check.
    #[test]
    fn a_non_registry_leaf_is_untouched() {
        for leaf in [
            "Scalar",
            "Error",
            "ErrorLoc",
            "AttributedString",
            "CPtr",
            "T",
        ] {
            assert_eq!(for_source(leaf, "net"), leaf, "{leaf}");
            assert_eq!(for_signature(leaf, "net"), leaf, "{leaf}");
        }
    }

    /// bug-484: a bare leaf the referencing package does not declare is UNDEFINED,
    /// and how many other packages export it is irrelevant — one is not more
    /// resolvable than three, it is the same guess with a smaller search space.
    /// `udp` naming a bare `Address` (net's, single owner) is refused...
    #[test]
    #[should_panic(expected = "declares no such type")]
    fn a_bare_cross_package_leaf_is_refused_even_with_one_owner() {
        let _ = for_source("Address", "udp");
    }

    /// ...and so is the same shape in a signature.
    #[test]
    #[should_panic(expected = "declares no such type")]
    fn a_bare_cross_package_leaf_is_refused_in_a_signature() {
        let _ = for_signature("Address", "udp");
    }

    /// Several owners is the same error, not a different one.
    #[test]
    #[should_panic(expected = "declares no such type")]
    fn a_bare_leaf_with_several_owners_is_refused() {
        let _ = for_source("Socket", "http");
    }

    /// A shared leaf the referencing package DOES declare is local, and local
    /// always wins: `Socket` inside `tcp` is tcp's. That other packages export the
    /// same leaf never makes it ambiguous — the local declaration decides.
    #[test]
    fn a_shared_leaf_the_referencing_package_declares_is_local() {
        assert_eq!(for_source("Socket", "tcp"), "Socket");
        assert_eq!(for_signature("Socket", "tcp"), "tcp.Socket");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// plan-111-C Phase 3's overload-resolution regression guard.
    ///
    /// Collapsing the dual API means every overload is now selected from typed
    /// arguments. The distinction that must survive is the one `leaf_matches`
    /// draws in STRICT mode between a **resource** parameter and a **value**
    /// nominal parameter — and it is asymmetric on purpose:
    ///
    /// * a resource parameter demands exact base-resource identity, so a
    ///   resource UNION does not satisfy a concrete resource close-op parameter.
    ///   `fs::close(<some union>)` must stay rejected — the legacy exact-name
    ///   resolver caught it, and it is a use-after-free class error;
    /// * a value nominal stays coarse, so a variant still widens into its union
    ///   (`json::stringify(JsonNull)` against a `Json` parameter);
    /// * and STATE/ownership are transparent either way (bug-427): a
    ///   `File STATE Cursor` argument satisfies a bare `File` parameter, and a
    ///   `RES ` marker matches through on either side.
    ///
    /// Getting any of these backwards changes which overload wins, which
    /// plan-111-C §Non-goals names as the single failure mode this letter must
    /// not have.
    #[test]
    fn strict_matching_separates_resource_params_from_value_union_params() {
        let file = ParameterType::parse("fs.File");
        let stateful_file = ParameterType::parse("fs.File STATE Cursor");
        let res_file = ParameterType::parse("RES fs.File");
        let socket = ParameterType::parse("net.Socket");

        // A resource parameter: exact base identity, STATE- and RES-transparent.
        assert!(leaf_matches(&file, &file, true));
        assert!(
            leaf_matches(&file, &stateful_file, true),
            "bug-427: a stateful resource still satisfies its bare parameter"
        );
        assert!(
            leaf_matches(&file, &res_file, true),
            "the RES ownership marker is transparent to matching"
        );
        assert!(
            !leaf_matches(&file, &socket, true),
            "a DIFFERENT resource must not satisfy a concrete resource parameter"
        );
        assert!(
            !leaf_matches(&file, &ParameterType::parse("Json"), true),
            "a non-resource nominal must not widen into a concrete resource parameter"
        );
        assert!(
            !leaf_matches(&file, &ParameterType::String, true),
            "bug-443: a scalar never satisfies a nominal parameter in strict mode"
        );

        // A VALUE nominal parameter stays coarse, so a variant widens into it.
        let json = ParameterType::parse("Json");
        assert!(
            leaf_matches(&json, &ParameterType::parse("JsonNull"), true),
            "a value-union parameter must still accept a variant that widens into it"
        );
        assert!(
            !leaf_matches(&json, &ParameterType::String, true),
            "bug-443: but not a scalar"
        );

        // LENIENT mode (overload dispatch / return inference) stays coarse
        // everywhere, so an unresolved or nominally-spelled argument does not
        // perturb selection.
        assert!(leaf_matches(&file, &ParameterType::parse("Json"), false));
        assert!(leaf_matches(&json, &ParameterType::String, false));

        // Two unequal scalars never match, in either mode.
        assert!(!leaf_matches(
            &ParameterType::Integer,
            &ParameterType::String,
            true
        ));
        assert!(!leaf_matches(
            &ParameterType::Integer,
            &ParameterType::String,
            false
        ));
    }

    /// No registered descriptor may declare a [`ParameterType::Named`] whose name
    /// *spells something with structure*: for every `Named(n)` in the catalog,
    /// `parse(n)` must be that same `Named`.
    ///
    /// The failure this catches is silent. `ParameterType::named("List OF String")`
    /// is a `Named` whose name merely *spells* a container — a different value from
    /// `list_of(String)`, but with the identical rendering. Every pre-plan-106
    /// consumer rendered `.name()` and re-parsed, which normalized it on the way
    /// through; the typed accessors plan-106-A introduced read the raw variant, and
    /// `ir::lower` then inferred the element type of `fs::pathJoin([a, b])` as
    /// `Unknown` rather than `String` (caught by the byte-identity gate on
    /// `rt-behavior/project/project-fs-createTempFile-package-valid`, its one
    /// `.ir` golden).
    ///
    /// `named` is for a BARE NOMINAL — a record, union, or user type. Anything with
    /// structure must be built with the matching constructor.
    ///
    /// Deliberately scoped to `Named` and not "everything round-trips": a
    /// [`Var`](ParameterType::Var) and an [`Arg`](ParameterType::Arg) render as a
    /// bare name and re-parse as `Named` **by design** — `parse` classifies grammar,
    /// and it cannot know a name is a type variable without the declaring scope.
    /// Those are sanctioned; a structure-spelling `Named` is not.
    #[test]
    fn descriptor_named_types_are_bare_nominals() {
        fn check(type_: &ParameterType, where_: &str, failures: &mut Vec<String>) {
            if let ParameterType::Named(symbol) = type_ {
                let name = symbol.resolve();
                let reparsed = ParameterType::parse(name);
                if !matches!(reparsed, ParameterType::Named(_)) {
                    failures.push(format!(
                        "{where_}: `{name}` is a bare `Named` but its own spelling parses to \
                         {reparsed:?} — build it with the matching constructor, not `named`"
                    ));
                }
            }
            // Recurse so a bad leaf nested in a real container is caught too.
            match type_ {
                ParameterType::ListOf(inner)
                | ParameterType::SetOf(inner)
                | ParameterType::ResultOf(inner)
                | ParameterType::Res(inner) => check(inner, where_, failures),
                ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
                    check(key, where_, failures);
                    check(value, where_, failures);
                }
                ParameterType::UserOf(_, args) => {
                    for arg in args {
                        check(arg, where_, failures);
                    }
                }
                ParameterType::Func(params, ret, _) => {
                    for param in params {
                        check(param, where_, failures);
                    }
                    check(ret, where_, failures);
                }
                ParameterType::ThreadHandle { msg, res, out, .. } => {
                    check(msg, where_, failures);
                    check(res, where_, failures);
                    check(out, where_, failures);
                }
                _ => {}
            }
        }

        let mut failures = Vec::new();
        let mut checked = 0usize;
        for package in registry().packages() {
            for function in &package.functions {
                for (index, implementation) in function.implementations.iter().enumerate() {
                    for param in &implementation.params {
                        let where_ = format!(
                            "{}.{} impl {index} param `{}`",
                            package.import_name, function.name, param.name
                        );
                        check(&param.ty, &where_, &mut failures);
                        checked += 1;
                        if let DefaultValue::Fill { type_name, .. } = &param.default {
                            let where_ = format!("{where_} (Fill default)");
                            check(type_name, &where_, &mut failures);
                            checked += 1;
                        }
                    }
                    let where_ = format!(
                        "{}.{} impl {index} return",
                        package.import_name, function.name
                    );
                    check(&implementation.return_type, &where_, &mut failures);
                    checked += 1;
                }
            }
            for record in &package.records {
                for prop in &record.props {
                    let where_ = format!(
                        "{}.{} field `{}`",
                        package.import_name, record.name, prop.name
                    );
                    check(&prop.ty, &where_, &mut failures);
                    checked += 1;
                }
            }
        }
        assert!(
            checked > 0,
            "the registry exposed no descriptor types to check"
        );
        assert!(
            failures.is_empty(),
            "{} descriptor type(s) are not structurally what they spell:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    // --- test builders (named-literal wrappers with throwaway docs) ---

    fn func(name: &'static str, implementations: Vec<Implementation>) -> RegistryFunction {
        RegistryFunction {
            name,
            intro: "i",
            desc: "d",
            example: "e",
            expected_arguments: None,
            internal_only: false,
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
            description: "",
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

    fn enum_variant(name: &'static str) -> EnumVariant {
        EnumVariant {
            name,
            description: "variant doc",
            advisory: None,
        }
    }

    fn enm(name: &'static str, export: bool, variants: Vec<EnumVariant>) -> RegistryEnum {
        RegistryEnum {
            name,
            export,
            variants,
        }
    }

    fn res(name: &'static str, export: bool, close_function: &'static str) -> RegistryResource {
        RegistryResource {
            name,
            export,
            description: "resource doc",
            close_function,
            sendable: true,
            live_slots: &[],
            close_may_fail: true,
            kind: crate::codegen::resource::ResourceKind::Builtin,
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
            body: Body::mfb(body, rewrite),
        }
    }

    fn intrinsic(return_type: ParameterType) -> Implementation {
        Implementation {
            params: vec![],
            return_type,
            errors: vec![],
            body: Body::Intrinsic,
        }
    }

    fn rewrite_impl(symbol: &'static str) -> Implementation {
        Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
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
    fn package_constants_and_overrides_round_trip_through_the_builders() {
        // Scalar constant, record constant, and an override on one throwaway package.
        let mut pkg = RegistryPackage::new("demo", "i", "d");
        pkg.add_constant(RegistryConstant {
            name: "pi",
            type_name: "Float",
            value: Some("3.14159"),
            components: None,
            message: None,
            symbol: None,
        })
        .add_constant(RegistryConstant {
            name: "zero3",
            type_name: "Float3",
            value: None,
            components: Some(&["0.0", "0.0", "0.0"]),
            message: None,
            symbol: None,
        })
        .add_override(RegistryOverride {
            builtin: "toString",
            arg_type: "Float3",
            helper: "__demo_toString_float3",
        });

        // Accessors expose exactly what was added.
        assert_eq!(pkg.constants().len(), 2);
        assert_eq!(pkg.overrides().len(), 1);

        // The scalar constant answers `value` (not `components`).
        let scalar = pkg.constants().iter().find(|c| c.name == "pi").unwrap();
        assert_eq!(scalar.type_name, "Float");
        assert_eq!(scalar.value, Some("3.14159"));
        assert_eq!(scalar.components, None);

        // The record constant answers `components` (not `value`).
        let record = pkg.constants().iter().find(|c| c.name == "zero3").unwrap();
        assert_eq!(record.type_name, "Float3");
        assert_eq!(record.value, None);
        assert_eq!(record.components, Some(&["0.0", "0.0", "0.0"][..]));

        // The override is keyed by (builtin, arg_type).
        let ov = &pkg.overrides()[0];
        assert_eq!(
            (ov.builtin, ov.arg_type, ov.helper),
            ("toString", "Float3", "__demo_toString_float3")
        );
    }

    #[test]
    fn constant_and_override_boundary_fns_fall_through_for_unowned_names() {
        // A throwaway `demo` package built in the test above is not wired into the
        // frozen `registry()`, so its constant/override names stay absent and the
        // dual-path in `builtins`/`ir::lower` falls through to the hand tables.
        assert!(!is_package_constant("demo.pi"));
        assert_eq!(constant_type_name("demo.pi"), None);
        assert_eq!(constant_value("demo.pi"), None);
        assert_eq!(constant_components("demo.zero3"), None);
        // A general override the frozen registry does NOT own falls through.
        assert_eq!(
            general_override_target("toString", &crate::types::ParameterType::parse("Nope")),
            None
        );
        // The migrated `vector` package DOES own `toString(Float3)` now (add_override).
        assert_eq!(
            general_override_target("toString", &crate::types::ParameterType::parse("Float3")),
            Some("__vector_toString_float3")
        );

        // A malformed (unqualified) name never panics.
        assert!(!is_package_constant("bare"));
        assert_eq!(constant_components("bare"), None);
    }

    #[test]
    fn frozen_registry_exposes_the_csv_package() {
        let pkg = registry().resolve_package("csv").expect("csv registered");
        assert_eq!(pkg.import_name(), "csv");
        assert_eq!(pkg.functions().len(), 4);
        assert!(registry().resolve_package("nope").is_none());
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
                    body: Body::Rewrite("__describe_int"),
                },
                Implementation {
                    params: vec![param("v", ParameterType::String)],
                    return_type: ParameterType::String,
                    errors: vec![],
                    body: Body::Rewrite("__describe_str"),
                },
            ],
        );
        let impls = &describe.implementations;
        assert_eq!(impls.len(), 2);
        assert_eq!(impls[0].params[0].ty.name(), "Integer");
        assert_eq!(impls[1].params[0].ty.name(), "String");
    }

    /// A call shape from concrete argument type names.
    fn call(args: &[&str]) -> CallShape {
        CallShape {
            args: args.iter().map(|arg| ParameterType::parse(arg)).collect(),
        }
    }

    // Concise `ParameterType` constructors for the generic-resolution tests.
    fn list_of(elem: ParameterType) -> ParameterType {
        ParameterType::list_of(elem)
    }
    fn map_of(key: ParameterType, value: ParameterType) -> ParameterType {
        ParameterType::map_of(key, value)
    }

    /// A minimal generic overload: parameter types (with type variables) and a return
    /// type, no docs/body/errors.
    fn generic_impl(params: Vec<ParameterType>, return_type: ParameterType) -> Implementation {
        Implementation {
            params: params.into_iter().map(|ty| param("x", ty)).collect(),
            return_type,
            errors: vec![],
            body: Body::Intrinsic,
        }
    }

    #[test]
    fn select_binds_type_variables_and_substitutes_the_return() {
        use ParameterType::Integer;
        // get(List OF T, Integer) AS T  |  get(Map OF K TO V, K) AS V
        let get = func(
            "get",
            vec![
                generic_impl(
                    vec![list_of(ParameterType::var("T")), Integer],
                    ParameterType::var("T"),
                ),
                generic_impl(
                    vec![
                        map_of(ParameterType::var("K"), ParameterType::var("V")),
                        ParameterType::var("K"),
                    ],
                    ParameterType::var("V"),
                ),
            ],
        );

        // List overload: element type of arg 0.
        assert_eq!(
            get.dispatch(&call(&["List OF Integer", "Integer"]))
                .unwrap()
                .return_type
                .name(),
            "Integer"
        );
        // Nested containers substitute structurally.
        assert_eq!(
            get.dispatch(&call(&["List OF List OF String", "Integer"]))
                .unwrap()
                .return_type
                .name(),
            "List OF String"
        );
        // Map overload: the value type; the key overload's `List OF T` param rejects a
        // `Map` arg, so this resolves against the second implementation.
        assert_eq!(
            get.dispatch(&call(&["Map OF String TO Integer", "String"]))
                .unwrap()
                .return_type
                .name(),
            "Integer"
        );
    }

    // --- thread ThreadHandle probe: empirical reproduction of the legacy resolver ---
    //
    // (`src/builtins/thread.rs:719-1006`) against the new `parse`/`unify`/`substitute`.
    // These assert the LEGACY behavior and are the regression guard for the settled
    // two-overload model (plan §"PART B obstacle resolution"): the resource plane is a
    // SIGNATURE-LEVEL overload split on the existing `RES` spelling (`start` = a
    // resource overload tried first + a data overload; `accept`/`transfer` =
    // resource-only), the strict-`Nothing` guard rejects a data-only handle from the
    // resource overload FOR us, and the `Var`-refines-`Unknown` rule (bug-443 latent
    // fix) makes `send(Thread OF Unknown TO Out, Integer)` resolve.
    fn th(
        worker: bool,
        msg: ParameterType,
        res: ParameterType,
        out: ParameterType,
    ) -> ParameterType {
        ParameterType::thread_handle(worker, msg, res, out)
    }

    #[test]
    fn thread_probe_waitfor_receive_send_poll() {
        use ParameterType::{Integer, Nothing, String, Unknown};
        let data_handle = |m, o| th(false, m, Unknown, o);
        // waitFor(Thread OF Msg TO Out) AS Out
        let wait_for = func(
            "waitFor",
            vec![generic_impl(
                vec![data_handle(
                    ParameterType::var("Msg"),
                    ParameterType::var("Out"),
                )],
                ParameterType::var("Out"),
            )],
        );
        let parent = th(false, Integer, Nothing, String);
        assert_eq!(
            wait_for
                .resolve(&CallShape {
                    args: vec![parent.clone()]
                })
                .map(|s| s.return_type.name().into_owned()),
            Some("String".into()),
            "waitFor parent (strict)"
        );
        // worker handle rejected (kind mismatch)
        assert!(
            wait_for
                .resolve(&CallShape {
                    args: vec![th(true, Integer, Nothing, String)]
                })
                .is_none(),
            "waitFor rejects worker"
        );

        // receive(Thread OF Msg TO Out) AS Msg — including the Unknown-message case.
        let receive = func(
            "receive",
            vec![generic_impl(
                vec![data_handle(
                    ParameterType::var("Msg"),
                    ParameterType::var("Out"),
                )],
                ParameterType::var("Msg"),
            )],
        );
        assert_eq!(
            receive
                .dispatch(&CallShape {
                    args: vec![parent.clone()]
                })
                .map(|s| s.return_type.name().into_owned()),
            Some("Integer".into()),
            "receive Msg"
        );
        assert_eq!(
            receive
                .dispatch(&CallShape {
                    args: vec![th(false, Unknown, Nothing, String)]
                })
                .map(|s| s.return_type.name().into_owned()),
            Some("Unknown".into()),
            "receive Unknown message"
        );

        // send(Thread OF Msg TO Out, Msg) AS Nothing — cross-param constraint.
        let send = func(
            "send",
            vec![generic_impl(
                vec![
                    data_handle(ParameterType::var("Msg"), ParameterType::var("Out")),
                    ParameterType::var("Msg"),
                ],
                Nothing,
            )],
        );
        assert_eq!(
            send.resolve(&CallShape {
                args: vec![parent.clone(), Integer]
            })
            .map(|s| s.return_type.name().into_owned()),
            Some("Nothing".into()),
            "send matching message"
        );
        assert!(
            send.resolve(&CallShape {
                args: vec![parent.clone(), String]
            })
            .is_none(),
            "send message mismatch rejected"
        );
        // Legacy resolve_send_unknown_message: handle message Unknown accepts any arg.
        assert_eq!(
            send.resolve(&CallShape {
                args: vec![th(false, Unknown, Nothing, String), Integer]
            })
            .map(|s| s.return_type.name().into_owned()),
            Some("Nothing".into()),
            "send Unknown-message accepts any arg (LEGACY)"
        );
    }

    #[test]
    fn thread_probe_transfer_accept_resource_plane() {
        use ParameterType::{Integer, Nothing, String};
        let file = ParameterType::named("fs.File");
        // transfer(Thread OF Msg RES Res TO Out, Res) AS Nothing — resource-ONLY overload.
        let transfer = func(
            "transfer",
            vec![generic_impl(
                vec![
                    th(
                        false,
                        ParameterType::var("Msg"),
                        ParameterType::var("Res"),
                        ParameterType::var("Out"),
                    ),
                    ParameterType::var("Res"),
                ],
                Nothing,
            )],
        );
        assert_eq!(
            transfer
                .resolve(&CallShape {
                    args: vec![th(false, Integer, file.clone(), String), file.clone()]
                })
                .map(|s| s.return_type.name().into_owned()),
            Some("Nothing".into()),
            "transfer matching resource"
        );
        // A data-only handle has no resource plane: STRICT validation rejects it (the
        // resource overload's `res` Var can't bind `Nothing`) -> TYPE_CALL_ARGUMENT_MISMATCH.
        assert!(
            transfer
                .resolve(&CallShape {
                    args: vec![th(false, Integer, Nothing, String), file.clone()]
                })
                .is_none(),
            "transfer on data-only handle rejected by strict validation (LEGACY)"
        );

        // accept(Thread OF Msg RES Res TO Out) AS Res — resource-ONLY overload.
        let accept = func(
            "accept",
            vec![generic_impl(
                vec![th(
                    false,
                    ParameterType::var("Msg"),
                    ParameterType::var("Res"),
                    ParameterType::var("Out"),
                )],
                ParameterType::var("Res"),
            )],
        );
        assert_eq!(
            accept
                .dispatch(&CallShape {
                    args: vec![th(false, Integer, file.clone(), String)]
                })
                .map(|s| s.return_type.name().into_owned()),
            Some("fs.File".into()),
            "accept returns resource"
        );
        // A data-only handle is rejected by STRICT validation (the strict-`Nothing`
        // guard), exactly as legacy `resolve_call` returns None on a data-only thread.
        assert!(
            accept
                .resolve(&CallShape {
                    args: vec![th(false, Integer, Nothing, String)]
                })
                .is_none(),
            "accept on data-only handle rejected by strict validation (LEGACY)"
        );
    }

    #[test]
    fn thread_probe_start_extraction_and_resource_echo() {
        use ParameterType::{Integer, Nothing, String};
        let file = ParameterType::named("fs.File");
        // start = TWO overloads (resource first, data second), the settled model.
        //   RESOURCE: ISOLATED FUNC(ThreadWorker OF Msg RES Res TO Out, In) AS Out
        //             -> Thread OF Msg RES Res TO Out
        //   DATA:     ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out
        //             -> Thread OF Msg TO Out  (worker/return carry NO res var)
        let opt_int = |name| Parameter {
            name,
            desc: "",
            aliases: &[],
            ty: Integer,
            default: DefaultValue::Optional,
        };
        let overload = |worker_res: ParameterType, ret_res: ParameterType| Implementation {
            params: vec![
                param(
                    "f",
                    ParameterType::func_isolated(
                        vec![
                            th(
                                true,
                                ParameterType::var("Msg"),
                                worker_res,
                                ParameterType::var("Out"),
                            ),
                            ParameterType::var("In"),
                        ],
                        ParameterType::var("Out"),
                    ),
                ),
                param("data", ParameterType::var("In")),
                opt_int("inboundLimit"),
                opt_int("outboundLimit"),
            ],
            return_type: th(
                false,
                ParameterType::var("Msg"),
                ret_res,
                ParameterType::var("Out"),
            ),
            errors: vec![],
            body: Body::Intrinsic,
        };
        let start = func(
            "start",
            vec![
                overload(ParameterType::var("Res"), ParameterType::var("Res")),
                overload(Nothing, Nothing),
            ],
        );

        // Data-only worker: strict validation MUST accept (via the data overload), and
        // dispatch echoes a data-only parent handle (via the resource overload, whose
        // `Res` binds `Nothing` under lenient and elides).
        let data_fn =
            ParameterType::func_isolated(vec![th(true, Integer, Nothing, String), Integer], String);
        assert!(
            start
                .resolve(&CallShape {
                    args: vec![data_fn.clone(), Integer]
                })
                .is_some(),
            "start STRICT-validates a data-only worker"
        );
        assert_eq!(
            start
                .dispatch(&CallShape {
                    args: vec![data_fn, Integer]
                })
                .map(|s| s.return_type.name().into_owned()),
            Some("Thread OF Integer TO String".into()),
            "start echoes a data-only worker"
        );

        // Resourced worker: start echoes the resource plane onto the parent handle.
        let res_fn =
            ParameterType::func_isolated(vec![th(true, Integer, file, String), Integer], String);
        assert!(
            start
                .resolve(&CallShape {
                    args: vec![res_fn.clone(), Integer]
                })
                .is_some(),
            "start STRICT-validates a resourced worker"
        );
        assert_eq!(
            start
                .dispatch(&CallShape {
                    args: vec![res_fn, Integer]
                })
                .map(|s| s.return_type.name().into_owned()),
            Some("Thread OF Integer RES fs.File TO String".into()),
            "start echoes a resourced worker's plane"
        );

        // A plain (non-isolated) worker is NOT accepted where start demands ISOLATED.
        let plain_fn =
            ParameterType::func(vec![th(true, Integer, Nothing, String), Integer], String);
        assert!(
            start
                .resolve(&CallShape {
                    args: vec![plain_fn, Integer]
                })
                .is_none(),
            "start rejects a non-isolated worker"
        );
    }

    #[test]
    fn thread_probe_parse_isolated_func_arg() {
        // The type checker hands `start`'s callback arg as `ISOLATED FUNC(...) AS ...`;
        // `parse` decomposes it into an isolated `Func` whose first param is a
        // `ThreadHandle`, and `name()` round-trips the `ISOLATED ` marker byte-exactly.
        let spelling = "ISOLATED FUNC(ThreadWorker OF Integer TO String, Integer) AS String";
        let parsed = ParameterType::parse(spelling);
        assert_eq!(parsed.name(), spelling, "ISOLATED FUNC round-trips");
        let ParameterType::Func(params, _, isolated) = &parsed else {
            panic!("parse decomposes ISOLATED FUNC into a Func (got {parsed:?})");
        };
        assert!(isolated, "the Func is marked isolated");
        assert!(
            matches!(&params[0], ParameterType::ThreadHandle { worker: true, .. }),
            "the first param is a ThreadWorker handle (got {:?})",
            params[0]
        );
    }

    #[test]
    fn select_rejects_inconsistent_variable_binding() {
        // set(List OF T, T) AS List OF T — the element must match the list's element.
        let set = func(
            "set",
            vec![generic_impl(
                vec![list_of(ParameterType::var("T")), ParameterType::var("T")],
                list_of(ParameterType::var("T")),
            )],
        );
        assert!(set
            .dispatch(&call(&["List OF Integer", "Integer"]))
            .is_some());
        // T is bound to Integer by arg 0, then contradicted by a String element.
        assert!(set
            .dispatch(&call(&["List OF Integer", "String"]))
            .is_none());
    }

    #[test]
    fn select_unknown_argument_is_a_wildcard() {
        use ParameterType::Integer;
        let get = func(
            "get",
            vec![generic_impl(
                vec![list_of(ParameterType::var("T")), Integer],
                ParameterType::var("T"),
            )],
        );
        // An Unknown collection leaves `T` unbound, so there is no concrete return.
        assert!(get.dispatch(&call(&["Unknown", "Integer"])).is_none());
        // An Unknown index still binds `T` from the concrete list.
        assert_eq!(
            get.dispatch(&call(&["List OF Integer", "Unknown"]))
                .unwrap()
                .return_type
                .name(),
            "Integer"
        );
    }

    #[test]
    fn select_scalar_mismatch_and_wrong_arity_are_rejected() {
        let f = func(
            "f",
            vec![generic_impl(
                vec![ParameterType::String, ParameterType::Integer],
                ParameterType::Boolean,
            )],
        );
        assert!(f.dispatch(&call(&["String", "Integer"])).is_some());
        // Two different known scalars never unify.
        assert!(f.dispatch(&call(&["Boolean", "Integer"])).is_none());
        // Too few / too many arguments.
        assert!(f.dispatch(&call(&["String"])).is_none());
        assert!(f
            .dispatch(&call(&["String", "Integer", "Integer"]))
            .is_none());
    }

    #[test]
    fn contains_var_detects_generic_types() {
        use ParameterType::{Integer, String};
        assert!(contains_var(&ParameterType::var("T")));
        assert!(contains_var(&list_of(ParameterType::var("T"))));
        assert!(contains_var(&map_of(String, ParameterType::var("V"))));
        assert!(contains_var(&ParameterType::map_entry_of(
            ParameterType::var("K"),
            Integer,
        )));
        assert!(contains_var(&ParameterType::result_of(ParameterType::var(
            "T"
        ))));
        assert!(!contains_var(&list_of(Integer)));
        assert!(!contains_var(&ParameterType::result_of(Integer)));
        assert!(!contains_var(&ParameterType::named("Instant")));
    }

    #[test]
    fn unify_substitute_over_map_entry_and_result_variants() {
        use ParameterType::{Integer, String};
        // MapEntry OF K TO V: unify binds K/V from the concrete pair, then substitute
        // rebuilds the concrete pair from a fully-generic pattern.
        let pattern = ParameterType::map_entry_of(ParameterType::var("K"), ParameterType::var("V"));
        let concrete = ParameterType::map_entry_of(String, Integer);
        let mut bindings = BTreeMap::new();
        assert!(unify(&pattern, &concrete, &mut bindings, false));
        assert_eq!(substitute(&pattern, &bindings), Some(concrete.clone()));
        // A mismatched shape (Map OF vs MapEntry OF) does not unify.
        assert!(!unify(
            &pattern,
            &map_of(String, Integer),
            &mut BTreeMap::new(),
            false
        ));

        // Result OF T: bind T, substitute back.
        let rpattern = ParameterType::result_of(ParameterType::var("T"));
        let rconcrete = ParameterType::result_of(Integer);
        let mut rbindings = BTreeMap::new();
        assert!(unify(&rpattern, &rconcrete, &mut rbindings, false));
        assert_eq!(substitute(&rpattern, &rbindings), Some(rconcrete));
        assert!(!unify(
            &rpattern,
            &list_of(Integer),
            &mut BTreeMap::new(),
            false
        ));
    }

    #[test]
    fn add_function_takes_a_function_value() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("t", "intro", "desc");
        pkg.add_function(func("f", vec![intrinsic(ParameterType::Nothing)]));
        r.add_package(pkg);
        assert_eq!(r.packages().len(), 1);
        assert_eq!(r.resolve_package("t").unwrap().functions().len(), 1);
    }

    #[test]
    fn resolve_func_finds_the_owning_package() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("csv", "i", "d");
        pkg.add_function(func("parse", vec![rewrite_impl("__csv_parse")]));
        r.add_package(pkg);

        assert_eq!(
            r.resolve_func("csv.parse")
                .map(|resolved| resolved.package.import_name()),
            Some("csv"),
        );
        assert!(r.resolve_func("csv.nope").is_none());
        assert!(r.resolve_func("nope.parse").is_none());
        assert!(r.resolve_func("toString").is_none());
        // Works against the frozen registry too (a real migrated member).
        assert_eq!(
            registry()
                .resolve_func("csv.parse")
                .map(|resolved| resolved.package.import_name()),
            Some("csv"),
        );
    }

    fn sample_fast_path<'a>(
        _b: &mut crate::codegen::engine::builder::CodeBuilder<'a>,
        _target: &str,
        _args: &[crate::target::shared::nir::NirValue],
    ) -> Result<Option<crate::codegen::engine::builder::ValueResult>, String> {
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
    fn get_mfb_renders_helper_functions_before_member_bodies() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("demo", "intro", "desc");
        pkg.add_helper(RegistryHelper::always(
            "demo",
            "FUNC __demo_helper() AS Nothing\nEND FUNC",
        ));
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

        let src = r.resolve_package("demo").unwrap().get_mfb();
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
        // A package with only Intrinsic/Rewrite members and no records/unions/helpers
        // injects nothing.
        let mut empty = Registry::new();
        let mut pkg = RegistryPackage::new("nomfb", "i", "d");
        pkg.add_function(func("a", vec![rewrite_impl("__a")]));
        empty.add_package(pkg);
        assert_eq!(empty.resolve_package("nomfb").unwrap().get_mfb(), "");

        // A package whose only injectable content is a helper-function chunk (the
        // native `process` shape: an `EXPORT ENUM` companion, no records/unions/bodies)
        // still emits that chunk — helper chunks are standalone injectable source.
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("bare", "i", "d");
        pkg.add_imports(vec!["strings"]);
        pkg.add_helper(RegistryHelper::always(
            "demo",
            "FUNC __helper() AS Nothing\nEND FUNC",
        ));
        r.add_package(pkg);
        assert_eq!(
            r.resolve_package("bare").unwrap().get_mfb(),
            "IMPORT strings\n\nFUNC __helper() AS Nothing\nEND FUNC\n",
        );
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

        let pkg = r.resolve_package("json").unwrap();
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

        let pkg = r.resolve_package("json").unwrap();
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
    fn add_enum_renders_the_enum_declaration() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("process", "intro", "desc");
        pkg.add_enum(enm(
            "Stream",
            true,
            vec![enum_variant("StdOut"), enum_variant("StdErr")],
        ));
        pkg.add_enum(enm("Internal", false, vec![enum_variant("A")]));
        r.add_package(pkg);

        let pkg = r.resolve_package("process").unwrap();
        assert_eq!(pkg.enums().len(), 2);
        assert!(pkg.enums()[0].export);
        assert!(!pkg.enums()[1].export);
        assert_eq!(
            pkg.enums()[0].render(),
            "EXPORT ENUM Stream\n  StdOut\n  StdErr\nEND ENUM"
        );
        assert_eq!(pkg.enums()[1].render(), "ENUM Internal\n  A\nEND ENUM");
    }

    #[test]
    fn enum_variant_advisory_is_keyed_by_package_enum_and_member() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("demo", "i", "d");
        let advisory = EnumAdvisory {
            rule: "CRYPTO_SHA1_INSECURE",
            detail: "legacy only",
        };
        pkg.add_enum(enm(
            "Algo",
            true,
            vec![
                EnumVariant {
                    name: "Weak",
                    description: "weak",
                    advisory: Some(advisory),
                },
                enum_variant("Strong"),
            ],
        ));
        r.add_package(pkg);
        // A same-named enum in ANOTHER package carries no advisory: the lookup is
        // keyed by the owning package, never a bare enum-name scan.
        let mut other = RegistryPackage::new("other", "i", "d");
        other.add_enum(enm("Algo", true, vec![enum_variant("Weak")]));
        r.add_package(other);

        assert_eq!(
            r.enum_variant_advisory("demo", "Algo", "Weak"),
            Some(advisory)
        );
        assert_eq!(r.enum_variant_advisory("demo", "Algo", "Strong"), None);
        assert_eq!(r.enum_variant_advisory("demo", "Algo", "Missing"), None);
        assert_eq!(r.enum_variant_advisory("demo", "Nope", "Weak"), None);
        assert_eq!(r.enum_variant_advisory("other", "Algo", "Weak"), None);
        assert_eq!(r.enum_variant_advisory("absent", "Algo", "Weak"), None);
        // The advisory's rule must be a real `warn` row of the rule table, or the
        // `ir::verify` emit site would trip the unknown-rule guard.
        let production = registry().enum_variant_advisory("crypto", "Hash", "SHA1");
        let rule = production
            .expect("crypto Hash.SHA1 carries an advisory")
            .rule;
        assert!(
            !crate::rules::is_error(rule),
            "{rule} must be warn-severity"
        );
        assert_eq!(
            registry().enum_variant_advisory("crypto", "Hash", "SHA256"),
            None
        );
    }

    #[test]
    fn enums_resolve_as_builtin_types() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("process", "i", "d");
        pkg.add_enum(enm("Stream", true, vec![enum_variant("StdOut")]));
        r.add_package(pkg);

        assert!(matches!(
            r.resolve_type("process.Stream"),
            Some(ResolvedType::Enum(e)) if e.name == "Stream"
        ));
        assert!(r.resolve_type("process.Nope").is_none());
    }

    #[test]
    fn add_resource_records_name_export_and_close_function() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("fs", "i", "d");
        pkg.add_resource(res("File", true, "fs.close"));
        pkg.add_resource(res("Internal", false, "fs.__drop"));
        r.add_package(pkg);

        let pkg = r.resolve_package("fs").unwrap();
        assert_eq!(pkg.resources().len(), 2);
        assert_eq!(pkg.resources()[0].name, "File");
        assert!(pkg.resources()[0].export);
        assert_eq!(pkg.resources()[0].close_function, "fs.close");
        assert!(!pkg.resources()[1].export);
        // Resources carry no injectable source, so they never render into get_mfb.
        assert!(!pkg.get_mfb().contains("File"));
        // resolve_type resolves a resource type by name.
        assert!(matches!(
            r.resolve_type("fs.File"),
            Some(ResolvedType::Resource(resource)) if resource.close_function == "fs.close"
        ));
    }

    #[test]
    fn get_mfb_places_enums_between_unions_and_helpers() {
        let mut r = Registry::new();
        let mut pkg = RegistryPackage::new("process", "intro", "desc");
        pkg.add_union(uni("U", true, vec![variant("A")]));
        pkg.add_enum(enm(
            "Stream",
            true,
            vec![enum_variant("StdOut"), enum_variant("StdErr")],
        ));
        pkg.add_helper(RegistryHelper::always(
            "process",
            "FUNC __process_helper() AS Nothing\nEND FUNC",
        ));
        r.add_package(pkg);

        assert_eq!(
            r.resolve_package("process").unwrap().get_mfb(),
            "EXPORT UNION U\n  A\nEND UNION\n\n\
             EXPORT ENUM Stream\n  StdOut\n  StdErr\nEND ENUM\n\n\
             FUNC __process_helper() AS Nothing\nEND FUNC\n",
        );
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
        pkg.add_helper(RegistryHelper::always(
            "json",
            "FUNC __json_helper() AS Nothing\nEND FUNC",
        ));
        pkg.add_function(func(
            "render",
            vec![mfb_impl(
                "FUNC __json_render() AS String\n  RETURN \"\"\nEND FUNC",
            )],
        ));
        r.add_package(pkg);

        let pkg = r.resolve_package("json").unwrap();
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
        pkg.add_helper(RegistryHelper::always(
            "demo",
            "FUNC __demo_helper()\nEND FUNC",
        ));
        pkg.add_record(rec("Rec", true, vec![prop("f", ParameterType::Integer)]));
        pkg.add_union(uni("Uni", false, vec![variant("V")]));
        pkg.add_function(RegistryFunction {
            name: "fn1",
            intro: "fn intro",
            desc: "fn desc",
            example: "fn example",
            expected_arguments: None,
            internal_only: false,
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
                body: Body::Rewrite("__demo_fn1"),
            }],
        });
        r.add_package(pkg);

        let pkg = r.resolve_package("demo").unwrap();
        assert_eq!(pkg.intro(), "pkg intro");
        assert_eq!(pkg.desc(), "pkg desc");
        assert_eq!(
            pkg.always_helper_bodies(),
            vec!["FUNC __demo_helper()\nEND FUNC"]
        );

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
        let csv = r.resolve_package("csv").expect("csv package");

        let parse = |src: &str| {
            let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
                .expect("parse");
            crate::ast::AstProject {
                name: "test".to_string(),
                files: vec![file],
            }
        };
        assert!(csv.is_imported_by(&ProjectView::of_ast(&parse(
            "IMPORT csv\nSUB main\nEND SUB\n"
        ))));
        assert!(!csv.is_imported_by(&ProjectView::of_ast(&parse("SUB main\nEND SUB\n"))));
    }

    /// plan-104-C: the typed entry and the string wrapper must agree on every
    /// call shape — same overload selection, same return (typed's `name()` ==
    /// string's answer) — across containers, `RES` markers, unions, `Unknown`,
    /// and a strict-`Nothing` rejection. The corpus resolves against the real
    /// frozen registry so it exercises the production descriptors.
    #[test]
    fn typed_and_string_resolution_agree() {
        let corpus: &[(&str, &[&str], bool)] = &[
            // scalar + container generics
            ("collections.get", &["List OF Integer", "Integer"], false),
            ("collections.get", &["List OF Integer", "Integer"], true),
            ("collections.append", &["List OF String", "String"], false),
            ("collections.keys", &["Map OF String TO Integer"], false),
            // RES-marked collection element: Arg-echo preserves the marker
            (
                "collections.append",
                &["List OF RES fs.File", "fs.File"],
                false,
            ),
            // set + map-entry shapes
            ("collections.toList", &["Set OF Integer"], false),
            // Unknown wildcard argument (lenient dispatch accepts)
            ("collections.get", &["List OF Integer", "Unknown"], false),
            // higher-order FUNC parameter
            (
                "collections.transform",
                &["List OF Integer", "FUNC(Integer) AS String"],
                false,
            ),
            // strict-Nothing rejection: both must reject identically
            ("collections.get", &["Nothing", "Integer"], true),
            // union-typed argument through a value-union parameter (json)
            ("json.encode", &["Integer"], false),
            // a call no package owns
            ("nope.missing", &["Integer"], false),
        ];
        for (callee, args, strict) in corpus {
            let arg_strings: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            let arg_typed: Vec<ParameterType> =
                args.iter().map(|a| ParameterType::parse(a)).collect();
            let via_string = resolve_call(callee, &arg_strings, *strict);
            let via_typed = resolve_call_typed(callee, &arg_typed, *strict)
                .map(|type_| type_.name().into_owned());
            assert_eq!(
                via_string, via_typed,
                "typed/string disagree for {callee}({args:?}, strict={strict})"
            );
        }
        // The strict-Nothing case really is a rejection, not a vacuous agreement.
        assert_eq!(
            resolve_call_typed(
                "collections.get",
                &[ParameterType::Nothing, ParameterType::Integer],
                true
            ),
            None
        );
        // And the RES echo really preserves the marker through the typed path.
        let echoed = resolve_call_typed(
            "collections.append",
            &[
                ParameterType::parse("List OF RES fs.File"),
                ParameterType::parse("fs.File"),
            ],
            false,
        );
        assert_eq!(
            echoed.map(|type_| type_.name().into_owned()),
            Some("List OF RES fs.File".to_string())
        );
    }

    /// plan-120-D. `argument_types_typed` declines for an overload SET, and IR
    /// lowering uses it to decide union wrapping — so without an answer for the
    /// positions the overloads AGREE on, a member that gains a second overload
    /// silently stops wrapping a union-typed argument. That failure has no
    /// diagnostic: `json::stringify(json::JsonNull[NOTHING])` returned `""` and
    /// `json::stringify(json::JsonStr["Ada"])` returned `"null"`, the tag read
    /// from the wrong place.
    ///
    /// This pins both halves of the rule, so a future overload cannot quietly
    /// reintroduce it: agreement answers, disagreement stays `None`.
    #[test]
    fn agreed_argument_type_answers_where_overloads_agree() {
        // `json::stringify` is overloaded three ways and every one takes `Json`
        // first — the position that must stay wrapped.
        assert_eq!(
            agreed_argument_type("json.stringify", 0).map(|t| t.name().into_owned()),
            Some("json.Json".to_string()),
            "position 0 is `Json` in all three overloads and must be answered"
        );
        // Position 1 is `Integer` in one overload and `String` in another, so
        // there is no single expected type and the honest answer is `None`.
        assert_eq!(
            agreed_argument_type("json.stringify", 1),
            None,
            "position 1 disagrees (Integer vs String) and must NOT be guessed"
        );
        // Past every overload's parameter list.
        assert_eq!(agreed_argument_type("json.stringify", 2), None);
        // A single-implementation member is `argument_types_typed`'s job; this
        // function deliberately declines so the two cannot both answer.
        // `json.get` stands in for `json.parse` here, which plan-120-E gave a
        // second overload — the parse cases moved below.
        assert_eq!(agreed_argument_type("json.get", 0), None);
        // And it agrees with `argument_types_typed` about what position 0 IS,
        // for a member where that function does answer.
        assert_eq!(
            argument_types_typed("json.get").and_then(|p| p.first().map(|t| t.name().into_owned())),
            Some("json.Json".to_string())
        );
        // plan-120-E: now that `json.parse` is overloaded the two roles swap —
        // this function answers and `argument_types_typed` declines. They are
        // exact complements (exactly 1 implementation vs 2 or more), which is
        // what stops both from answering for the same member; asserting BOTH
        // halves is what makes that a property rather than a coincidence.
        assert_eq!(
            agreed_argument_type("json.parse", 0).map(|t| t.name().into_owned()),
            Some("String".to_string()),
            "both parse overloads take String first"
        );
        assert_eq!(argument_types_typed("json.parse"), None);
        // Position 1 exists in only ONE of the two overloads. A position past
        // some overload's parameter list is not agreement, so it must decline —
        // otherwise a 1-arg call site would be handed the reviver's type.
        assert_eq!(agreed_argument_type("json.parse", 1), None);
        // An unknown member is not an answer.
        assert_eq!(agreed_argument_type("nope.missing", 0), None);
    }
}
