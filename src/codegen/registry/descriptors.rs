//! plan-176-M: the registry's descriptor data model — parameters, bodies, helpers, functions,
//! records, unions, enums, resources, constants and overrides. `Registry` and `build()` live in
//! `codegen/registry/mod.rs`.

use super::*;

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
    /// The module (project) name — the build identity `os.appResourcePath` bakes into
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
    /// plan-94-A §4.4a: the arena offset of the mouse-state region (the decoded-event
    /// ring, its cursors and the decoder's partial-sequence buffer), or `None` when
    /// the program uses no `enableMouse`/`pollMouse`. Read by the mouse `abi_function`
    /// bodies and by the stdin decoder plan-94-B installs; carries the `ArenaLayout`
    /// value byte-for-byte. Every other abi body ignores it.
    ///
    /// The mouse-MODE word is deliberately not here: it is process-global
    /// (`MOUSE_MODE_SYMBOL`), because a UI-thread handler has no arena state.
    pub(crate) mouse_state_offset: Option<usize>,
    /// bug-699: the arena offset of the input-buffer region (the bytes a stdin
    /// subscriber has pulled but no reader has consumed), or `None` when no reader
    /// in the program decodes through it — see `io::input::INPUT_BUFFER_READERS`.
    /// Read by the `io` reader bodies; every other abi body ignores it.
    pub(crate) input_state_offset: Option<usize>,
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
    /// plan-130-C: whether the `--debug` arena registry is active. `thread.start` and
    /// `canvas.startGraphics` consume it to register the arena they create; `false` on
    /// the inline (`abi_inline`) path.
    pub(crate) debug_arena_registry: bool,
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

/// A parameter that consumes the resources reachable from its argument (plan-116-J).
///
/// Distinct from a resource type's `close_function`, which says *how* a resource is
/// closed; this says *that passing a container closes what is inside it*. The two meet in
/// the move checker: the containment closure of the argument is consumed, and each
/// consumed binding is then closed by its own type's close op.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConsumingParameter {
    /// The member's unqualified name, e.g. `"setGroup"`.
    pub(crate) function: &'static str,
    /// The parameter's canonical name, e.g. `"items"`.
    pub(crate) parameter: &'static str,
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
    /// (the `FUNC` the body declares — e.g. `csv.parse`'s `File` overload →
    /// `__csv_parseFile`, which the public name cannot derive) and an optional native `fast_path` that lowers a
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
    /// Render inline like [`Always`](Self::Always), but only in a build WITHOUT
    /// `--debug`: the normal half of a [`RegistryHelper::debug_split`] (plan-130-E).
    NormalBuildOnly,
    /// Render inline like [`Always`](Self::Always), but only in a `--debug` build: the
    /// debug half of a [`RegistryHelper::debug_split`] (plan-130-E).
    DebugBuildOnly,
    /// Inject only when the program references at least one of these **local**
    /// function names — the `strings` scalar-seam gate (`isLetter`/`isDigit`/…),
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
    /// Inject only when the owning package is imported and the program calls one of
    /// the first names **or** constructs a record of one of the second (plan-172-K).
    ///
    /// A record type has no call to gate on: canvas's `AutoPaint` is a value a program
    /// builds and hands to `present`, and the graphics-thread code that gives it a
    /// state exists for nothing else. A value of a record type can only come from a
    /// constructor expression naming it, so "the program constructs one" is "the
    /// program can hold one". Type names are matched like callees, by their final
    /// segment, and over-matching (a user's own `AutoPaint`) only injects the chunk
    /// unnecessarily.
    WhenUsedOrConstructed(&'static [&'static str], &'static [&'static str]),
    /// The complement of a `WhenUsedOrConstructed(&[], names)` gate: inject only when
    /// the owning package is imported and the program constructs **none** of these
    /// record types (plan-172-K).
    ///
    /// It carries the bodies the package's unconditional source calls when the gated
    /// chunk is absent, so that source can call one name and get the real function or
    /// the function for a program that has nothing for it to do. Exactly one of the
    /// pair is injected into any program that imports the package.
    WhenNotConstructed(&'static [&'static str]),
    /// Inject only when the owning package is imported and the program calls one of
    /// the first names **or NAMES one of the second, as a type** — in a constructor
    /// or in any annotation: a binding's, a parameter's, a return type's, a field's
    /// (plan-164-B).
    ///
    /// Wider than [`WhenUsedOrConstructed`](Self::WhenUsedOrConstructed), for a record
    /// every field of which has a default: `MUT c AS canvas::PolyCurve` followed by
    /// `WITH c { … }` holds a value no constructor expression made, and a gate keyed on
    /// constructors alone would leave such a program the stand-in. A value of a record
    /// type cannot come to be in a program whose source never names the type, so "the
    /// program names it" is "the program can hold one". Names are matched by their
    /// final segment, anywhere in the annotation (`List OF canvas::PolyCurve` names
    /// it), and over-matching only injects the chunk unnecessarily.
    WhenCalledOrNamed(&'static [&'static str], &'static [&'static str]),
    /// The exact complement of a [`WhenCalledOrNamed`](Self::WhenCalledOrNamed) gate
    /// with the same two lists: the owning package is imported, none of the first
    /// names is called and none of the second is named. Exactly one of the pair is
    /// injected into any program that imports the package, so — unlike the
    /// constructed pair, whose complement ignores the calls — the two chunks may
    /// define the same functions.
    WhenNeitherCalledNorNamed(&'static [&'static str], &'static [&'static str]),
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
    /// Whether a native lowering branches to this helper by its bare reserved
    /// symbol (`_mfb_ifn_<name>`) rather than through an IR call.
    ///
    /// Several `crypto` members dispatch on an enum ordinal at run time and hand
    /// the chosen branch to an MFBASIC helper: `hash` to a SHA core, `seal`/`open`
    /// to an AEAD core, and `sign`/`verify`/`generate` to a software-curve core.
    /// Each of those lowerings is emitted once per unit as a standalone
    /// `abi_function` body with no calling function in scope, so the symbol it
    /// branches to is fixed — it cannot be the identity-prefixed name a merged
    /// package's copy carries. Marking the helper here is what tells
    /// `ir::merge_packages` to make sure the bare name is defined even when the
    /// program itself never imported the package (bug-557).
    ///
    /// Forgetting the mark on a NEW natively-called helper is not silent: the
    /// build of an executable importing a package that reaches it fails with
    /// `internal relocation target '_mfb_ifn_<name>' is not defined`. That is the
    /// symptom bug-557 was found by, and the fix is to add the mark here.
    pub(crate) natively_called: bool,
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
            natively_called: false,
        }
    }

    /// An [`Always`](HelperGate::Always) source chunk that a native lowering also
    /// branches to by its bare reserved symbol — see
    /// [`natively_called`](RegistryHelper::natively_called). Declared on the helper
    /// rather than beside the lowering so the two cannot drift: the `func_*.rs`
    /// that emits the branch and the `helper_*.rs` that carries the body sit in
    /// the same package directory.
    pub(crate) fn always_natively_called(name: &'static str, body: &'static str) -> Self {
        RegistryHelper {
            natively_called: true,
            ..Self::always(name, body)
        }
    }

    /// A source chunk with one body for a normal build and another for a `--debug`
    /// build (plan-130-E): reporting hooks (a stats file, a frame dump) live only in
    /// `debug_body`, so a normal build's source does not contain them at all. Both halves
    /// render inline in [`get_mfb_for`](RegistryPackage::get_mfb_for) at the position the
    /// pair was added, so the two bodies must declare the same functions the rest of the
    /// package calls; `debug_body` may add functions of its own.
    pub(crate) fn debug_split(
        name: &'static str,
        normal_body: &'static str,
        debug_body: &'static str,
    ) -> [Self; 2] {
        [
            RegistryHelper {
                gate: HelperGate::NormalBuildOnly,
                ..Self::always(name, normal_body)
            },
            RegistryHelper {
                gate: HelperGate::DebugBuildOnly,
                ..Self::always(name, debug_body)
            },
        ]
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
        self.match_overload(call, true, &NoTypeKinds)
    }

    /// [`resolve`](Self::resolve) with the caller's declared-type oracle, so a top-level
    /// type variable bound to a union accepts a variant of it (bug-988).
    pub(crate) fn resolve_with_kinds(
        &self,
        call: &CallShape,
        kinds: &dyn TypeKinds,
    ) -> Option<Selection<'_>> {
        self.match_overload(call, true, kinds)
    }

    /// LENIENT overload resolution for **dispatch / return-type inference**: like
    /// [`resolve`](Self::resolve) but a scalar argument coarsely satisfies a nominal
    /// parameter. Used where a not-yet-resolved or nominally-spelled argument must not be
    /// rejected — overload dispatch (rewrite targets) and the return-type oracle that
    /// feeds IR lowering / codegen — because rejecting it there perturbs type propagation
    /// on valid programs. Validation strictness belongs on [`resolve`](Self::resolve).
    pub(crate) fn dispatch(&self, call: &CallShape) -> Option<Selection<'_>> {
        self.match_overload(call, false, &NoTypeKinds)
    }

    /// [`dispatch`](Self::dispatch) with the caller's declared-type oracle (bug-988).
    pub(crate) fn dispatch_with_kinds(
        &self,
        call: &CallShape,
        kinds: &dyn TypeKinds,
    ) -> Option<Selection<'_>> {
        self.match_overload(call, false, kinds)
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
    fn match_overload(
        &self,
        call: &CallShape,
        strict: bool,
        kinds: &dyn TypeKinds,
    ) -> Option<Selection<'_>> {
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
            let pinned = RefCell::new(BTreeSet::new());
            let unifies = implementation
                .params
                .iter()
                .zip(call.args.iter())
                .all(|(param, arg)| {
                    unify_with_kinds(&param.ty, arg, &mut bindings, strict, kinds, &pinned)
                });
            if !unifies {
                continue;
            }

            if let Some(return_type) = substitute(&implementation.return_type, &bindings) {
                // bug-989: an `Arg(n)` return echoes the caller's n-th argument type.
                // If that argument is accepted only by CONVERSION to its union form
                // (a variant collection where the parameter resolved to the union's),
                // the call returns the converted collection, so its type is the
                // parameter's, not the argument's. No collections member reaches
                // this today — a collection parameter is the bucket and fixes its
                // variable — but echoing the argument there would mistype the call.
                let return_type = match return_type {
                    ParameterType::Arg(n) => implementation
                        .params
                        .get(n)
                        .and_then(|param| substitute(&param.ty, &bindings))
                        .zip(call.args.get(n))
                        .filter(|(param, arg)| converts_to_union_form(kinds, arg, param))
                        .map_or(ParameterType::Arg(n), |(param, _)| param),
                    other => other,
                };
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
    pub(super) fn render(&self) -> String {
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
            out.push_str(&prop.ty.display());
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
    /// Unions of the same package whose members this one also has
    /// (`UNION X INCLUDES Y`, plan-172-K). An included union's members come FIRST, in
    /// its own order, then `variants` (`binary_repr::writer::concrete_union_variants`),
    /// so a union and one it includes agree on every tag they share.
    ///
    /// Inclusion is of MEMBERS, not of the union: a value of the included union's type
    /// is not accepted where this one is expected; each of its member records is.
    pub(crate) includes: Vec<&'static str>,
}

impl RegistryUnion {
    /// Render the `[EXPORT] UNION … END UNION` declaration (no trailing newline).
    pub(super) fn render(&self) -> String {
        let mut out = String::new();
        if self.export {
            out.push_str("EXPORT ");
        }
        out.push_str("UNION ");
        out.push_str(self.name);
        if !self.includes.is_empty() {
            out.push_str(" INCLUDES ");
            out.push_str(&self.includes.join(", "));
        }
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
    pub(super) fn render(&self) -> String {
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
    /// Why this handle stays on the thread that opened it, when
    /// [`sendable`](Self::sendable) is `false` — one clause, rendered onto the
    /// type page after the derived "stays on the thread that opened it" line.
    ///
    /// `None` for a sendable resource, where there is nothing to explain. The
    /// **transferability itself is derived from the bit**, never restated in
    /// prose: that is the drift bug-522 recorded, where `thread::transfer`'s
    /// page denied a capability the registry had granted a year earlier. Only
    /// the reason is prose, because only the reason is per-resource.
    pub(crate) unsendable_reason: Option<&'static str>,
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
