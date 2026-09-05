//! The same two sweeps as `abi_inline.rs`, for the registry's other two body
//! kinds.
//!
//! `Body::AbiFunction` is the OS-seam shape: the body is wrapped once into a
//! shared `_mfb_rt_*` helper and `bl`'d from every call site, so its arguments
//! arrive in registers rather than as typed values and it does not — and should
//! not — check their types. What it does share with `abi_inline` is the import
//! guard, and that guard is unreachable from a program for the same reason: the
//! plan's import list is derived from the calls these bodies emit.
//!
//! `Body::Mfb`'s optional `fast_path` is the third shape: a native lowering that
//! replaces the interpreted `.mfb` body for the instantiations it can handle and
//! **declines** for the rest. Every one of them opens by checking the monomorph
//! target is its own — the guard that keeps one member's call from being lowered
//! as another's — and no program can produce the pairing that guard rejects,
//! because the dispatcher only ever offers a fast path its own member's calls.

use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::codegen::registry::{registry, AbiCtx, Body};
use crate::os::linux::flavor::LinuxFlavor;

/// With no import declared, no `abi_function` body emits an external call.
///
/// A relocation carries `library: Some(_)` exactly when it binds to a symbol the
/// platform import list must declare; emitting one from an EMPTY list is a call
/// the plan does not know about. These bodies are where the OS actually gets
/// reached — `fs`, `net`, `process`, `audio`, `datetime` — so it is the family
/// where the guard matters most and the one no program can exercise.
#[test]
fn no_abi_function_body_emits_a_call_the_plan_never_declared() {
    // The REAL Linux platform, not the stub: these bodies reach the OS through
    // hooks `TestPlatform` leaves `unimplemented!()`, and a stub's
    // `emit_external_call` never consults the import list at all -- which is the
    // very thing under test.
    let platform = crate::target::linux_common::code::Platform::for_test(
        crate::target::linux_x86_64::code::X86_64,
        LinuxFlavor::Glibc,
    );
    let mut leaked = Vec::new();
    let mut swept = 0;
    for package in registry().packages() {
        for function in package.functions() {
            for (index, implementation) in function.implementations().iter().enumerate() {
                let Body::AbiFunction { lower, .. } = implementation.body else {
                    continue;
                };
                swept += 1;
                let call = format!("{}.{}", package.import_name(), function.name);
                let args: Vec<ValueResult> = implementation
                    .params
                    .iter()
                    .enumerate()
                    .map(|(at, param)| ValueResult {
                        type_: param.ty.clone(),
                        location: Operand::from(format!("x{}", 9 + at).as_str()),
                        text: param.name.to_string(),
                        origin: None,
                    })
                    .collect();
                let harness = BuilderHarness::default();
                let mut builder = harness.builder("_mfb_rt_probe", &platform);
                let base = harness.abi_ctx(&platform);
                let ctx = AbiCtx {
                    call: &call,
                    ..base
                };
                if lower(&mut builder, &args, &ctx).is_err() {
                    continue;
                }
                let external: Vec<String> = builder
                    .relocations
                    .iter()
                    .filter(|r| r.library.is_some())
                    .map(|r| r.to.clone())
                    .collect();
                if !external.is_empty() {
                    leaked.push(format!(
                        "{}::{}#{index} -> {external:?}",
                        package.import_name(),
                        function.name
                    ));
                }
            }
        }
    }
    assert!(
        swept >= 100,
        "the sweep found only {swept} abi_function implementations, so the \
         registry walk has broken"
    );
    assert!(
        leaked.is_empty(),
        "{} abi_function body(ies) emitted an externally-bound relocation with no \
         platform import declared:\n  {}",
        leaked.len(),
        leaked.join("\n  ")
    );
}

/// Every `Body::Mfb` fast path declines a monomorph target that is not its own.
///
/// A fast path is handed whatever runtime target the dispatcher is lowering, and
/// the `strip_prefix` at the top of each is what keeps one member's call from
/// being lowered as another's — silently, with another member's layout. Nothing
/// in a real program produces that pairing, so the guard is unreachable and
/// rots; this hands each of them a target belonging to a different member.
#[test]
fn every_mfb_fast_path_declines_another_members_target() {
    let platform = TestPlatform;
    let mut swept = 0;
    for package in registry().packages() {
        for function in package.functions() {
            for (index, implementation) in function.implementations().iter().enumerate() {
                let Body::Mfb {
                    fast_path: Some(fast),
                    ..
                } = implementation.body
                else {
                    continue;
                };
                swept += 1;
                // A target that is well-formed but belongs to no member: it has
                // the `#pkg_name$Args` shape every fast path strips, so a body
                // that declines it declined on the NAME rather than on the shape.
                let foreign = "#nosuchpackage_nosuchmember$Integer$Integer";
                let harness = BuilderHarness::default();
                let mut builder = harness.builder("_mfb_probe", &platform);
                let declined = fast(&mut builder, foreign, &[]);
                assert!(
                    matches!(declined, Ok(None)),
                    "{}::{}#{index}: a fast path must DECLINE a target that is not \
                     its own, so the dispatcher falls back to that member's own \
                     body; it did not",
                    package.import_name(),
                    function.name
                );
                assert!(
                    builder.instructions.len() <= 1,
                    "{}::{}#{index}: a declining fast path must emit nothing — it \
                     left {} instruction(s) in the caller's stream",
                    package.import_name(),
                    function.name,
                    builder.instructions.len()
                );
            }
        }
    }
    assert!(
        swept >= 10,
        "the sweep found only {swept} Mfb fast paths; there were 11 when this was \
         written, so the registry walk has broken"
    );
}
