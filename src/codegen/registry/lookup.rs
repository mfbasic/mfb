//! plan-176-M: lookups over the built registry — runtime calls, constants, overrides, return
//! and parameter types, rewrite targets, ABI lowering and call-parameter names. `Registry` and
//! `build()` live in `codegen/registry/mod.rs`.

use super::*;

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
/// `ir/lower/` to fold a `vector` record constant (`vector.zeroFloat3`) into a
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
    // bug-480 Phase 4b: the argument arrives package-qualified (`vector.Float2`),
    // while the descriptor row still spells the bare member id it declares
    // (`Float2`) -- `arg_type` is a `&'static str` and stays one. So the comparison
    // qualifies the ROW, and `toString(vector::abs(v))` finds
    // `__vector_toString_float2` instead of falling through to the general builtin
    // and reporting the vector type as un-stringable.
    //
    // The match is on the package's own type IDENTITY, never on a bare spelling
    // (bug-668). A bare `Color`/`Url`/`Float2` is necessarily a USER type: since
    // bug-480 Phase 4b a built-in value type's declared identity IS the qualified
    // name, so a consumer writing the bare leaf gets SYMBOL_UNKNOWN_TYPE, and the
    // injected companions are qualified by the parser (`qualify_own_builtin_type`).
    // While the bare arm stood, a user `TYPE Color` matched the `color` row: the
    // call type-checked and the build then died with `NIR call target
    // '#color_toString' does not resolve`, because the program never imported
    // `color`. A user type the built-in rejects is a TYPE_CALL_ARGUMENT_MISMATCH,
    // exactly as a `TYPE Colour` already was.
    let overrides_arg_type = |package: &RegistryPackage, o: &RegistryOverride| {
        format!("{}.{}", package.import_name(), o.arg_type) == spelled.as_ref()
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

/// The one overload-selection core behind [`resolve_call`] and
/// [`resolve_call_typed`]: select an overload for `call` and hand back its raw
/// substituted return type — including an unresolved [`ParameterType::Arg`]
/// marker, which each entry point resolves against its own argument
/// representation (the string form echoes the caller's original string
/// verbatim; the typed form echoes the typed argument).
pub(super) fn resolved_return_type(
    qualified: &str,
    call: &CallShape,
    strict: bool,
    kinds: &dyn TypeKinds,
) -> Option<ParameterType> {
    let function = registry().resolve_func(qualified)?.function;
    // `strict` (argument validation) rejects a scalar-for-nominal argument; the lenient
    // mode (return-type inference feeding IR lowering / codegen) coarsely accepts it.
    // Lenient still prefers a STRICT match first, exactly as [`rewrite_target`] does:
    // lenient `leaf_matches` accepts a scalar against a nominal in either direction, so
    // an earlier `Integer` row captures a call whose arguments precisely name a later
    // `big.Int` row (`crypto::randomInt`, plan-127-D). Without the preference the
    // inferred return type (`Integer`) and the body that runs (`__crypto_randomIntBig`)
    // would come from two different implementations.
    let selection = if strict {
        function.resolve_with_kinds(call, kinds)
    } else {
        function
            .resolve_with_kinds(call, kinds)
            .or_else(|| function.dispatch_with_kinds(call, kinds))
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

/// The internal symbol a call to `qualified` **with `argc` arguments** rewrites to.
///
/// The arity-routed twin of [`rewrite_target`]. That one selects the overload by
/// argument TYPES, which is the right question for a member whose forms differ in
/// shape (`http::handleRequest`'s two transports). It cannot answer the question an
/// *optional-parameter split* asks, because both forms have the same types in the
/// same order and differ only in how many of them are present — which is exactly
/// `collections::findLastIndex`, whose two-argument form starts its backward scan at
/// the last element and whose three-argument form starts where it is told, in two
/// separate bodies (bug-527).
///
/// Falls back to the type-blind `rewrite_target` when no implementation declares
/// exactly `argc` parameters — the ordinary case of a call that omits a defaulted
/// trailing parameter, where the sole implementation is still the right answer.
pub(crate) fn rewrite_target_for_arity(qualified: &str, argc: usize) -> Option<&'static str> {
    let function = registry().resolve_func(qualified)?.function;
    let exact = function
        .implementations
        .iter()
        .find(|implementation| implementation.params.len() == argc)
        .and_then(|implementation| implementation.body.rewrite_target());
    exact.or_else(|| rewrite_target(qualified, &[]))
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

/// Every helper a native lowering branches to by its bare reserved name, as the
/// sigil name the IR knows it by (`#crypto_ed25519Sign`).
///
/// Collected from the [`RegistryHelper::always_natively_called`] declarations so
/// the list cannot drift from the bodies. `ir::merge_packages` is the one
/// consumer: a lowering emitted for a package's call still branches to the bare
/// symbol, so the bare name has to be defined even in a program that never
/// imported the package itself (bug-557).
pub(crate) fn natively_called_helpers() -> Vec<String> {
    let mut names: Vec<String> = registry()
        .packages()
        .iter()
        .flat_map(|package| package.helpers())
        .filter(|helper| helper.natively_called)
        .map(|helper| format!("{}{}", crate::internal_name::INTERNAL_SIGIL, helper.name))
        .collect();
    names.sort();
    names.dedup();
    names
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
/// bug-642: the member a user calls to reach an internal generic body, spelled as
/// they write it (`collections::sort` for `#collections_sort`), or `None` when no
/// registry member rewrites to `internal` — an internal helper such as
/// `#collections_slice`, which a user never calls directly.
pub(crate) fn public_member_for_rewrite(internal: &str) -> Option<String> {
    let rewrite = format!("__{}", crate::internal_name::strip_sigil(internal)?);
    registry().packages().iter().find_map(|package| {
        package.functions().iter().find_map(|function| {
            function
                .implementations()
                .iter()
                .any(|implementation| {
                    implementation.body.rewrite_target() == Some(rewrite.as_str())
                })
                .then(|| format!("{}::{}", package.import_name(), function.name))
        })
    })
}

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

/// Whether argument `index` of `qualified` is a GENERIC position: some overload's
/// parameter there mentions a type variable (`collections::append`'s item `T`).
/// Only there does a literal argument's slot type depend on the call, which is
/// what scopes the literal re-typing retry (`ir::lower::literal_retype`); a
/// concrete overload set (`math::cos`'s `Float` | `Fixed`) keeps exact selection.
pub(crate) fn generic_parameter_position(qualified: &str, index: usize) -> bool {
    registry().resolve_func(qualified).is_some_and(|resolved| {
        resolved
            .function
            .implementations
            .iter()
            .any(|implementation| {
                implementation
                    .params
                    .get(index)
                    .is_some_and(|param| contains_var(&param.ty))
            })
    })
}

/// The expected type of argument `index` for a call to `qualified` whose actual
/// argument types are `arg_types` — the *generic* case neither
/// [`argument_types_typed`] nor [`agreed_argument_type`] can answer.
///
/// bug-550 (`collections::append([], x)` type-checks and then fails to build —
/// NOT the archived bug-550 about `debug_assert!`). Both of those decline the
/// moment a parameter mentions a
/// [`ParameterType::Var`], because a generic parameter has no expected type
/// independent of the call. It does have one *given* the call: overload selection
/// already unifies every parameter against every actual argument and records the
/// variable bindings, so `collections::append([], 1)` binds `T := Unknown` from the
/// empty literal and then REFINES it to `Integer` from the item. Substituting that
/// back into parameter 0 yields `List OF Integer` — the expected type the empty
/// literal needs to lower as, instead of the `List OF Unknown` that reaches
/// `collection_argument_as_list_slot` and fails the item-type check with an
/// internal "must be Unknown, got Integer".
///
/// `expected_return`, when the call sits somewhere with a declared type (a `LET`
/// annotation, a parameter, a `RETURN`), SEEDS the unification: an `Arg(n)` return
/// is the n-th parameter's pattern, so `LET xs AS List OF String =
/// collections::append([], ["a", "b"])` binds `T := String` from the annotation
/// before any argument is looked at. Without that seed both `append` overloads
/// unify with `([], List OF String)` — the element form binding `T := List OF
/// String`, the concatenating form `T := String` — and the position has no single
/// answer; with it only the concatenating form survives.
///
/// Deliberately LENIENT: this feeds type propagation, not validation, so a
/// not-yet-resolved argument must not reject an overload. It also answers only when
/// the surviving overloads AGREE, mirroring [`agreed_argument_type`] — an ambiguous
/// position yields `None` and lowers exactly as it did before, never a guess.
/// Returns `None` too when no overload selects, when the position has no parameter,
/// when that parameter is not generic, or when its variables are not all bound.
pub(crate) fn resolved_parameter_type(
    qualified: &str,
    index: usize,
    arg_types: &[ParameterType],
    expected_return: Option<&ParameterType>,
    kinds: &dyn TypeKinds,
) -> Option<ParameterType> {
    let function = registry().resolve_func(qualified)?.function;
    // bug-989: the ARGUMENTS decide first, exactly as the call's own type is
    // decided (the return-type oracle is never seeded). The seed only breaks a tie
    // or fills an unresolved variable. Seeding first let a declared union form
    // re-type an argument the call's type still described by its variant form —
    // `LET xs AS List OF Shape = append(squares, Square[1.0])` would have converted
    // `squares` for a call still typed `List OF Square`, which the binding then
    // converted a second time.
    let unseeded = resolved_parameter_type_seeded(function, index, arg_types, None, kinds);
    if unseeded
        .as_ref()
        .is_some_and(|type_| !contains_unknown(type_))
    {
        return unseeded;
    }
    if expected_return.is_none() {
        return unseeded;
    }
    resolved_parameter_type_seeded(function, index, arg_types, expected_return, kinds)
}

fn resolved_parameter_type_seeded(
    function: &RegistryFunction,
    index: usize,
    arg_types: &[ParameterType],
    seed: Option<&ParameterType>,
    kinds: &dyn TypeKinds,
) -> Option<ParameterType> {
    let mut agreed: Option<ParameterType> = None;
    for implementation in &function.implementations {
        let required = implementation
            .params
            .iter()
            .filter(|param| matches!(param.default, DefaultValue::None))
            .count();
        if arg_types.len() < required || arg_types.len() > implementation.params.len() {
            continue;
        }
        let mut bindings = BTreeMap::new();
        let pinned = RefCell::new(BTreeSet::new());
        if let Some(expected) = seed {
            let pattern = match &implementation.return_type {
                ParameterType::Arg(n) => implementation.params.get(*n).map(|param| &param.ty),
                other => Some(other),
            };
            if let Some(pattern) = pattern {
                if !unify_with_kinds(pattern, expected, &mut bindings, false, kinds, &pinned) {
                    continue;
                }
            }
        }
        if !implementation
            .params
            .iter()
            .zip(arg_types.iter())
            .all(|(param, arg)| {
                unify_with_kinds(&param.ty, arg, &mut bindings, false, kinds, &pinned)
            })
        {
            continue;
        }
        // Only the GENERIC positions are this function's business. A monomorphic
        // parameter is already answered by `argument_types_typed` /
        // `agreed_argument_type`, and a position those two deliberately decline
        // (overloads that DISAGREE on a concrete type — `json::stringify`'s
        // `indent`) must keep going through the existing selection path.
        let param = implementation.params.get(index)?;
        if !contains_var(&param.ty) {
            return None;
        }
        let resolved = substitute(&param.ty, &bindings)?;
        match &agreed {
            None => agreed = Some(resolved),
            Some(seen) if seen == &resolved => {}
            Some(_) => return None,
        }
    }
    agreed
}

/// Whether `type_` still has an unresolved (`Unknown`) part anywhere.
fn contains_unknown(type_: &ParameterType) -> bool {
    match type_ {
        ParameterType::Unknown => true,
        ParameterType::ListOf(inner)
        | ParameterType::SetOf(inner)
        | ParameterType::ResultOf(inner)
        | ParameterType::Res(inner) => contains_unknown(inner),
        ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
            contains_unknown(key) || contains_unknown(value)
        }
        ParameterType::UserOf(_, args) => args.iter().any(contains_unknown),
        ParameterType::Func(params, ret, _) => {
            params.iter().any(contains_unknown) || contains_unknown(ret)
        }
        _ => false,
    }
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
/// For each overload in [`call_param_name_overloads`]'s table (same order), how
/// many LEADING parameters a call must supply: the position just past the last
/// parameter with no default (`DefaultValue::None`). A `Fill` or `Optional` tail
/// may be left out; anything before it may not (bug-596). `None` exactly when the
/// name table is `None`.
pub(crate) fn call_param_name_overload_required(qualified: &str) -> Option<Vec<usize>> {
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
                    .rposition(|param| matches!(param.default, DefaultValue::None))
                    .map_or(0, |index| index + 1)
            })
            .collect(),
    )
}

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

/// The `(type, expr)` constants to append after the call's real arguments so a
/// migrated call's injected body receives its full arity — the `Fill` params past
/// the ones provided. Empty when no migrated package owns `qualified`.
///
/// The defaults appended are those of the overload the call RESOLVES to, decided
/// by every provided argument's type (`arg_types`, one per argument written; an
/// argument whose type is not known is [`ParameterType::Unknown`], which any
/// parameter accepts): the same selection the resolver makes — strict first, then
/// lenient dispatch. Two overloads can agree on the first arguments and still
/// differ in what they default (`crypto::open`'s ciphertext/tag form and its
/// `Sealed` form both take an optional trailing `aad`, at different positions).
/// A call no overload accepts is padded with nothing, so the checker reports the
/// call as written, at its own span; such a call never reaches code generation,
/// whose [`call_carries_its_fill_defaults`] check turns an unpadded call that did
/// into an internal error.
pub(crate) fn default_argument_padding(
    qualified: &str,
    arg_types: &[ParameterType],
    kinds: &dyn TypeKinds,
) -> Vec<(ParameterType, &'static str)> {
    let Some(resolved) = registry().resolve_func(qualified) else {
        return Vec::new();
    };
    let function = resolved.function;
    let provided = arg_types.len();
    let call = CallShape {
        args: arg_types.to_vec(),
    };
    // The resolver's own selection, with the caller's declared-type oracle, so a
    // union variant given for a `T` bound by `List OF <union>` selects exactly as
    // it does in `resolved_return_type` (bug-863 round 2: `repeat(shapes,
    // Square[…], 2)`).
    let implementation = function
        .resolve_with_kinds(&call, kinds)
        .or_else(|| function.dispatch_with_kinds(&call, kinds))
        .map(|selection| selection.implementation);
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

/// Whether a call with `argc` arguments reaching code generation carries every
/// `Fill` default its overload declares: some implementation takes `argc`
/// arguments counting each `Fill` parameter as present (lowering pads it; only an
/// `Optional` parameter may still be missing). `true` for a member the registry
/// does not own.
pub(crate) fn call_carries_its_fill_defaults(qualified: &str, argc: usize) -> bool {
    let Some(resolved) = registry().resolve_func(qualified) else {
        return true;
    };
    resolved
        .function
        .implementations
        .iter()
        .any(|implementation| {
            let present = implementation
                .params
                .iter()
                .filter(|param| !matches!(param.default, DefaultValue::Optional))
                .count();
            (present..=implementation.params.len()).contains(&argc)
        })
}
