//! Shared engine behind the per-package "declares exactly the errors its
//! lowering raises" guards (bug-694 for `fs`, generalized to every package by
//! bug-698).
//!
//! `mfb man <pkg> <member>` derives its Errors table from the descriptor's
//! `errors`, and nothing ties that list to the code a member's lowering
//! actually emits: a descriptor is hand-written, the raise can live in a
//! shared helper several calls away, and the textual guard
//! `every_raise_error_site_is_declared_in_its_descriptor` only sees a
//! two-string-literal `CodeBuilder::raise_error` call — not
//! `raise_error_into(symbol, "Err…", …)` inside an `abi_function` helper, nor
//! a code a shared emitter raises on the caller's behalf.
//!
//! So this module does not read the source: it LOWERS each member's real body
//! on all five backends and collects every error code the emitted
//! instructions load. Every raise path (`raise_error_into`,
//! `CodeBuilder::raise_error_bare`) moves the error's numeric `errorCode`
//! value into a register, so an 8-digit code immediate is the one signal
//! shared by all of them. The POSIX and Windows errno mappers dispatch to
//! different code sets, so one backend is not enough.
//!
//! A body can also return an error raised by a helper it calls. Every
//! internal call target must be classified: either scanned too
//! ([`Propagating`]) or listed as raising nothing of its own. An
//! unclassified callee fails the sweep, so a new call cannot hide a new
//! error.

use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::BuilderHarness;
use crate::codegen::engine::types::{
    CodeInstruction, CodeRelocation, CodegenPlatform, RelocIntent,
};
use crate::codegen::engine::util::vreg_frame::Vregs;
use crate::codegen::registry::{registry, AbiCtx, Body, Implementation, RegistryFunction};
use crate::os::linux::flavor::LinuxFlavor;
use crate::types::ParameterType;
use std::collections::{BTreeMap, BTreeSet};

/// The five real backends. The stub `TestPlatform` leaves the OS hooks
/// `unimplemented!()`, and the errno mapping differs between POSIX and
/// Windows.
pub(crate) fn platforms() -> Vec<(&'static str, Box<dyn CodegenPlatform>)> {
    vec![
        (
            "macos-aarch64",
            Box::new(crate::target::macos_aarch64::code::Platform),
        ),
        (
            "windows-x86_64",
            Box::new(crate::target::win_x86_64::code::Platform),
        ),
        (
            "linux-aarch64",
            Box::new(crate::target::linux_common::code::Platform::for_test(
                crate::target::linux_aarch64::code::Aarch64,
                LinuxFlavor::Glibc,
            )),
        ),
        (
            "linux-x86_64",
            Box::new(crate::target::linux_common::code::Platform::for_test(
                crate::target::linux_x86_64::code::X86_64,
                LinuxFlavor::Glibc,
            )),
        ),
        (
            "linux-riscv64",
            Box::new(crate::target::linux_common::code::Platform::for_test(
                crate::target::linux_riscv64::code::Riscv64,
                LinuxFlavor::Glibc,
            )),
        ),
    ]
}

/// An internal callee's classification for a package's sweep.
pub(crate) enum Propagating {
    /// Another member of the SAME package, by its `RegistryFunction::name`.
    Member(&'static str),
    /// A standalone helper that is not a registry member, re-lowered here to
    /// scan for its raises (only viable for a helper whose lowering needs no
    /// `platform`/`platform_imports` of its own).
    Helper(fn() -> (Vec<CodeInstruction>, Vec<CodeRelocation>)),
    /// A standalone helper's raise set, verified by READING its lowering
    /// source rather than re-driving it here — the sanctioned escape hatch
    /// for a helper whose own `lower_*` takes a `platform`/`platform_imports`
    /// (so it cannot be re-lowered standalone the way [`Helper`] re-lowers a
    /// platform-agnostic one). Same evidentiary standard as
    /// `UNREACHABLE_VIA_CALLEE`/`unreachable_own_raise`: cite the exact
    /// `raise_error_into`/`raise_error_bare` site in the classification's
    /// reason string.
    Known(&'static [&'static str]),
}

/// Error codes by numeric value (`"77030001"` -> `"ErrPathNotFound"`).
pub(crate) fn codes_by_value() -> BTreeMap<&'static str, &'static str> {
    registry()
        .resolve_package("errorCode")
        .expect("errorCode package")
        .constants()
        .iter()
        .filter_map(|constant| Some((constant.value?, constant.name)))
        .collect()
}

/// The error codes an instruction stream loads, and the internal symbols it
/// calls.
pub(crate) fn scan(
    instructions: &[CodeInstruction],
    relocations: &[CodeRelocation],
    codes: &BTreeMap<&'static str, &'static str>,
    raised: &mut BTreeSet<&'static str>,
    callees: &mut BTreeSet<String>,
) {
    for instruction in instructions {
        if let Some(name) = instruction
            .get("value")
            .and_then(|value| codes.get(value.as_str()))
        {
            raised.insert(name);
        }
    }
    for relocation in relocations {
        // Only a `Call`-intent relocation is a call at all — `DataAddrHi`/
        // `DataAddrLo`/`GotLoadHi`/`GotLoadLo` are addresses of data (error-message
        // strings, dlopen/dlsym name literals, …), never something that can itself
        // raise. External (libc/kernel32) calls report failure through errno, which
        // the body maps itself, so only an internal (no `library`) call is a callee
        // to classify.
        if relocation.kind == RelocIntent::Call && relocation.library.is_none() {
            callees.insert(relocation.to.clone());
        }
    }
}

/// Lower one implementation of a `<package>.<member>` on one backend and
/// return the emitted instructions and relocations.
///
/// A body refuses when an import or string literal it needs is not declared.
/// The plan derives both from the program, so supply each one it names and
/// lower again until it succeeds.
pub(crate) fn lower_body(
    package: &str,
    function: &RegistryFunction,
    implementation: &Implementation,
    platform: &dyn CodegenPlatform,
) -> Result<(Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    lower_body_call(
        &format!("{package}.{}", function.name),
        implementation,
        platform,
    )
}

/// Every `ctx.call` form a member's implementation can be lowered under: its
/// own `package.name`, plus each `os_aliases` code-form
/// (`RegistryFunction::os_aliases`, e.g. `udp.poll`'s list overload arrives
/// under `udp.pollList`) — a body dispatches on `ctx.call`, so an alias-only
/// branch is invisible unless the sweep drives it under its OWN call string,
/// not the member's public name.
pub(crate) fn call_forms(package: &str, function: &RegistryFunction, implementation: &Implementation) -> Vec<String> {
    let mut calls = vec![format!("{package}.{}", function.name)];
    if let Body::AbiFunction { os_aliases, .. } = &implementation.body {
        for alias in os_aliases.iter() {
            calls.push(format!("{package}.{alias}"));
        }
    }
    calls
}

/// Lower one implementation under a specific `ctx.call` string (see
/// [`call_forms`]) on one backend and return the emitted instructions and
/// relocations.
///
/// A body refuses when an import or string literal it needs is not declared.
/// The plan derives both from the program, so supply each one it names and
/// lower again until it succeeds.
/// Bind every [`ParameterType::Var`] in `ty` to `Integer` — a real call site
/// unifies a generic member's type variable against its concrete argument
/// (`collections::get(List OF T, Integer) AS T` binding `T`); this sweep has
/// no call site, so it picks a stand-in concrete type instead, recursing
/// through every structural wrapper so `List OF T`/`Map OF K TO V` become
/// `List OF Integer`/`Map OF Integer TO Integer` — concrete enough for the
/// native collection layout to lower (`collections`/`strings`' generic
/// members otherwise refuse with "packed payload does not support type 'T'").
fn concretize(ty: &ParameterType) -> ParameterType {
    match ty {
        ParameterType::Var(_) => ParameterType::Integer,
        ParameterType::ListOf(inner) => ParameterType::ListOf(Box::new(concretize(inner))),
        ParameterType::SetOf(inner) => ParameterType::SetOf(Box::new(concretize(inner))),
        ParameterType::ResultOf(inner) => ParameterType::ResultOf(Box::new(concretize(inner))),
        ParameterType::Res(inner) => ParameterType::Res(Box::new(concretize(inner))),
        ParameterType::MapOf(key, value) => {
            ParameterType::MapOf(Box::new(concretize(key)), Box::new(concretize(value)))
        }
        ParameterType::MapEntryOf(key, value) => {
            ParameterType::MapEntryOf(Box::new(concretize(key)), Box::new(concretize(value)))
        }
        ParameterType::Stateful { base, state } => ParameterType::Stateful {
            base: Box::new(concretize(base)),
            state: Box::new(concretize(state)),
        },
        ParameterType::Func(params, ret, isolated) => ParameterType::Func(
            params.iter().map(concretize).collect(),
            Box::new(concretize(ret)),
            *isolated,
        ),
        other => other.clone(),
    }
}

pub(crate) fn lower_body_call(
    call: &str,
    implementation: &Implementation,
    platform: &dyn CodegenPlatform,
) -> Result<(Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    let mut vregs = Vregs::new();
    let args: Vec<ValueResult> = implementation
        .params
        .iter()
        .map(|param| ValueResult {
            type_: concretize(&param.ty),
            location: Operand::from(vregs.next()),
            text: param.name.to_string(),
            origin: None,
        })
        .collect();
    let mut harness = BuilderHarness::default();
    let mut last_error = String::new();
    for _ in 0..64 {
        let mut builder = harness.builder("_mfb_rt_probe", platform);
        let ctx = AbiCtx {
            call,
            // `app::getMode`/`setMode` (and every `term::` cursor/style member)
            // require an arena presentation-mode slot; `canvas::enableMouse`/
            // `pollMouse` require a mouse-state slot. Supply placeholder
            // offsets so those bodies lower instead of refusing, per
            // bug-698's "make the harness lower it" (not a real arena layout,
            // so the instructions this test scans never dereference it).
            presentation_mode_offset: Some(0),
            mouse_state_offset: Some(0),
            // `io`'s new readKey/readRaw/readScalar/readGrapheme and every
            // `term::` member need the arena input-state slot (bug-699).
            input_state_offset: Some(0),
            // Every `term::` member needs the arena term-state slot.
            term_state_offset: Some(0),
            ..harness.abi_ctx(platform)
        };
        let result = match implementation.body {
            Body::AbiFunction { lower, .. } | Body::AbiInline(lower) => {
                lower(&mut builder, &args, &ctx)
            }
            _ => Err(format!("{call}: not a native body")),
        };
        let error = match result {
            Ok(_) => return Ok((builder.instructions, builder.relocations)),
            Err(error) => error,
        };
        drop(builder);
        if let Some(import) = error
            .split_once("requires ")
            .and_then(|(_, rest)| rest.strip_suffix(" import"))
            .map(|rest| rest.strip_prefix("the ").unwrap_or(rest))
        {
            harness
                .platform_imports
                .insert(import.to_string(), "libc".to_string());
        } else if let Some(literal) = error
            .strip_prefix("native code string literal '")
            .and_then(|rest| rest.strip_suffix("' has no data object"))
        {
            harness
                .string_symbols
                .insert(literal.to_string(), "_mfb_str_probe".to_string());
        } else {
            return Err(error);
        }
        last_error = error;
    }
    Err(format!(
        "{call}: still refusing after supplying 64 imports (last: {last_error})"
    ))
}

/// Lower one member's every implementation on every backend and return the
/// codes its own instructions load plus the internal symbols it calls.
pub(crate) fn lower_member(
    package: &str,
    function: &RegistryFunction,
    codes: &BTreeMap<&'static str, &'static str>,
    skip_platforms: &[&str],
    failures: &mut Vec<String>,
) -> (BTreeSet<&'static str>, BTreeSet<String>) {
    let mut raised = BTreeSet::new();
    let mut callees = BTreeSet::new();
    for (target, platform) in platforms() {
        if skip_platforms.contains(&target) {
            continue;
        }
        for implementation in function.implementations() {
            // Only AbiFunction/AbiInline bodies lower here; Mfb bodies are
            // covered by the separate `mfb_body_declared_errors` guard.
            if !matches!(
                implementation.body,
                Body::AbiFunction { .. } | Body::AbiInline(_)
            ) {
                continue;
            }
            for call in call_forms(package, function, implementation) {
                match lower_body_call(&call, implementation, platform.as_ref()) {
                    Ok((instructions, relocations)) => scan(
                        &instructions,
                        &relocations,
                        codes,
                        &mut raised,
                        &mut callees,
                    ),
                    Err(error) => failures.push(format!("{target} {call} refused: {error}")),
                }
            }
        }
    }
    (raised, callees)
}

/// The result of sweeping one package: any lowering failure (a body the
/// harness could not drive), and any member whose declared `errors` differ
/// from what its lowering actually raises.
pub(crate) struct SweepResult {
    pub(crate) failures: Vec<String>,
    pub(crate) mismatched: Vec<String>,
    /// Number of `AbiFunction`/`AbiInline` members the sweep actually lowered
    /// (at least one implementation). Used to assert the sweep did not
    /// silently see zero members (e.g. a package renamed or the harness
    /// changed shape).
    pub(crate) lowered_members: usize,
}

/// Sweep every `AbiFunction`/`AbiInline` member of `package_name`, comparing
/// each member's declared `errors` (unioned across its implementations, since
/// this sweep does not distinguish overloads within one member — see the
/// per-overload guard for that) against the codes its lowering raises,
/// directly or through a classified callee.
pub(crate) fn sweep_package(
    package_name: &str,
    raises_nothing: &[(&str, &str)],
    propagating: &[(&str, Propagating)],
    unreachable_via_callee: &[(&str, &str, &str)],
    unreachable_own_raise: &[(&str, &str, &str)],
    declared_extra: &[(&str, &'static str, &str)],
    skip_platforms: &[&str],
    skip_members: &[&str],
) -> SweepResult {
    let codes = codes_by_value();
    let package = registry()
        .resolve_package(package_name)
        .unwrap_or_else(|| panic!("{package_name} package"));
    let mut failures = Vec::new();

    let mut own = BTreeMap::new();
    for function in package.functions() {
        if skip_members.contains(&function.name) {
            own.insert(function.name, (BTreeSet::new(), BTreeSet::new()));
            continue;
        }
        own.insert(
            function.name,
            lower_member(package_name, function, &codes, skip_platforms, &mut failures),
        );
    }
    let lowered_members = own
        .values()
        .filter(|(raised, callees)| !raised.is_empty() || !callees.is_empty())
        .count();

    let mut mismatched = Vec::new();
    for function in package.functions() {
        if skip_members.contains(&function.name) {
            continue;
        }
        let has_native_body = function
            .implementations()
            .iter()
            .any(|implementation| {
                matches!(
                    implementation.body,
                    Body::AbiFunction { .. } | Body::AbiInline(_)
                )
            });
        if !has_native_body {
            continue;
        }
        let (raised, callees) = &own[function.name];
        let mut expected: BTreeSet<&'static str> = raised
            .iter()
            .copied()
            .filter(|code| {
                !unreachable_own_raise
                    .iter()
                    .any(|(member, error, _)| *member == function.name && error == code)
            })
            .collect();
        for callee in callees {
            if raises_nothing.iter().any(|(symbol, _)| symbol == callee) {
                continue;
            }
            let Some((_, how)) = propagating.iter().find(|(symbol, _)| symbol == callee) else {
                failures.push(format!(
                    "{package_name}::{} calls `{callee}`, which is in neither \
                     RAISES_NOTHING nor PROPAGATING — classify it",
                    function.name
                ));
                continue;
            };
            let inherited = match how {
                Propagating::Member(member) => own
                    .get(member)
                    .unwrap_or_else(|| {
                        panic!("{package_name}::{member}: PROPAGATING target not registered")
                    })
                    .0
                    .clone(),
                Propagating::Helper(lower) => {
                    let (instructions, relocations) = lower();
                    let mut raised = BTreeSet::new();
                    scan(
                        &instructions,
                        &relocations,
                        &codes,
                        &mut raised,
                        &mut BTreeSet::new(),
                    );
                    raised
                }
                Propagating::Known(codes) => codes.iter().copied().collect(),
            };
            for code in inherited {
                let unreachable = unreachable_via_callee
                    .iter()
                    .any(|(caller, error, _)| *caller == function.name && *error == code);
                if !unreachable {
                    expected.insert(code);
                }
            }
        }
        // A code this sweep's single fixed harness configuration (Console
        // build_mode, no app-mode variant) cannot make reachable, but that a
        // real build config does raise -- verified by reading the source
        // (`declared_extra`'s reason string cites the exact gate), the same
        // evidentiary bar as `unreachable_own_raise`, just the mirror image:
        // that one REMOVES a measured-but-unreachable code, this one ADDS a
        // real-but-unmeasured one.
        for (member, code, _reason) in declared_extra {
            if *member == function.name {
                expected.insert(code);
            }
        }

        let declared: BTreeSet<&'static str> = function
            .implementations()
            .iter()
            .flat_map(|implementation| implementation.errors.iter().copied())
            .collect();
        if declared != expected {
            let missing: Vec<_> = expected.difference(&declared).collect();
            let extra: Vec<_> = declared.difference(&expected).collect();
            mismatched.push(format!(
                "{package_name}::{}: undeclared {missing:?}, declared but never raised {extra:?}",
                function.name
            ));
        }
    }

    SweepResult {
        failures,
        mismatched,
        lowered_members,
    }
}

/// Assert a package's sweep is clean: every native member's declared `errors`
/// equals what its lowering (own raise sites plus classified callees) can
/// raise, and every callee the sweep found is classified.
pub(crate) fn assert_package_declares_exactly_the_errors_its_lowering_raises(
    package_name: &str,
    raises_nothing: &[(&str, &str)],
    propagating: &[(&str, Propagating)],
    unreachable_via_callee: &[(&str, &str, &str)],
    min_lowered_members: usize,
) {
    assert_package_declares_exactly_the_errors_its_lowering_raises_on(
        package_name,
        raises_nothing,
        propagating,
        unreachable_via_callee,
        &[],
        &[],
        &[],
        &[],
        min_lowered_members,
    );
}

/// As [`assert_package_declares_exactly_the_errors_its_lowering_raises`], but
/// skips a named backend entirely — for a package with a member whose lowering
/// hard-`unimplemented!()`s on that ISA by design (e.g. `app`'s GTK hooks on
/// `linux-riscv64`, plan-05: "aarch64/x86-64 only", not a harness gap this
/// sweep should paper over).
pub(crate) fn assert_package_declares_exactly_the_errors_its_lowering_raises_on(
    package_name: &str,
    raises_nothing: &[(&str, &str)],
    propagating: &[(&str, Propagating)],
    unreachable_via_callee: &[(&str, &str, &str)],
    unreachable_own_raise: &[(&str, &str, &str)],
    declared_extra: &[(&str, &'static str, &str)],
    skip_platforms: &[&str],
    skip_members: &[&str],
    min_lowered_members: usize,
) {
    let result = sweep_package(
        package_name,
        raises_nothing,
        propagating,
        unreachable_via_callee,
        unreachable_own_raise,
        declared_extra,
        skip_platforms,
        skip_members,
    );
    assert!(
        result.failures.is_empty(),
        "the sweep could not lower or classify every {package_name} member:\n  {}",
        result.failures.join("\n  ")
    );
    assert!(
        result.lowered_members >= min_lowered_members,
        "the sweep found only {} {package_name} members with a native body; \
         there were at least {min_lowered_members} when this was written",
        result.lowered_members
    );
    assert!(
        result.mismatched.is_empty(),
        "{} {package_name} member(s) declare errors that differ from what their \
         lowering raises (fix the descriptor `errors` in \
         src/codegen/builtins/{package_name}/func_*.rs):\n  {}",
        result.mismatched.len(),
        result.mismatched.join("\n  ")
    );
}
