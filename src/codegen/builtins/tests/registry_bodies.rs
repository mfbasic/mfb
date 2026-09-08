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
use crate::codegen::engine::types::CodegenPlatform;
use crate::codegen::engine::util::vreg_frame::Vregs;
use crate::codegen::registry::{registry, AbiCtx, Body};
use crate::os::linux::flavor::LinuxFlavor;

/// The five real backends, for a sweep that has to cross the OS seam.
///
/// One platform is not enough. An `abi_function` body that reaches the OS
/// branches on `platform.family()` and calls `emit_external_call` in EACH arm,
/// so sweeping only Linux runs the POSIX arm and leaves the Windows arm's import
/// guard exactly as dead as it was — two uncovered lines in ~40 `func_*.rs`
/// files, every one of them the same `?`.
///
/// The real backends, not `TestPlatform`: these bodies reach the OS through
/// hooks the stub leaves `unimplemented!()`, and a stub's `emit_external_call`
/// never consults the import list at all, which is the very thing under test.
fn os_seam_platforms() -> Vec<(&'static str, Box<dyn CodegenPlatform>)> {
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

/// Every `abi_function` body that a backend can lower, lowered with an EMPTY
/// import list, on all five backends.
fn sweep_abi_function_bodies(
    target: &str,
    platform: &dyn CodegenPlatform,
    leaked: &mut Vec<String>,
    panicked: &mut Vec<String>,
) -> usize {
    let mut swept = 0;
    let windows = platform.family() == crate::codegen::engine::types::PlatformFamily::Windows;
    for package in registry().packages() {
        for function in package.functions() {
            for (index, implementation) in function.implementations().iter().enumerate() {
                let Body::AbiFunction { lower, .. } = implementation.body else {
                    continue;
                };
                swept += 1;
                let call = format!("{}.{}", package.import_name(), function.name);
                let mut vregs = Vregs::new();
                let args: Vec<ValueResult> = implementation
                    .params
                    .iter()
                    .map(|param| ValueResult {
                        type_: param.ty.clone(),
                        location: Operand::from(vregs.next().as_str()),
                        text: param.name.to_string(),
                        origin: None,
                    })
                    .collect();
                let harness = BuilderHarness::default();
                let mut builder = harness.builder("_mfb_rt_probe", platform);
                let base = harness.abi_ctx(platform);
                let ctx = AbiCtx {
                    call: &call,
                    ..base
                };
                // A body that refuses is not a finding: plenty of members are
                // implemented on some backends and not others, and refusing is
                // how they say so. The finding is a body that SUCCEEDS while
                // naming a library nothing declared.
                //
                // A body that PANICS is not automatically a finding either --
                // `AppSupport::require_gtk` hard-stops an ISA with no app-mode
                // port at the boundary, deliberately -- but it is not waved
                // through: the message is recorded, and the caller asserts the
                // only ones are that documented hard-stop. Swallowing every
                // panic here would hide a real crash in a body nothing else
                // lowers.
                let lowered = crate::testutil::silence_panics(|| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        lower(&mut builder, &args, &ctx)
                    }))
                });
                let lowered = match lowered {
                    Ok(result) => result,
                    Err(payload) => {
                        let message = payload
                            .downcast_ref::<String>()
                            .cloned()
                            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                            .unwrap_or_else(|| "panicked with no message".to_string());
                        panicked.push(format!(
                            "{target} {}::{}#{index}: {message}",
                            package.import_name(),
                            function.name
                        ));
                        continue;
                    }
                };
                if lowered.is_err() {
                    continue;
                }
                for relocation in &builder.relocations {
                    let Some(library) = &relocation.library else {
                        continue;
                    };
                    if windows {
                        // Win32 names the DLL at the emit site, so an empty
                        // import list does not suppress the relocation. The
                        // contract there is the complementary one: the reloc is
                        // SELF-DESCRIBING. A `Some("")` would bind to whatever
                        // the loader turns up.
                        if library.is_empty() {
                            leaked.push(format!(
                                "{target} {}::{}#{index} -> `{}` bound externally \
                                 with an empty library name",
                                package.import_name(),
                                function.name,
                                relocation.to
                            ));
                        }
                    } else {
                        leaked.push(format!(
                            "{target} {}::{}#{index} -> {} ({library})",
                            package.import_name(),
                            function.name,
                            relocation.to
                        ));
                    }
                }
            }
        }
    }
    swept
}

/// No `abi_function` body binds to a symbol nothing declared — on any backend.
///
/// Swept on all five backends because the guard is per-ARM, not per-body: a body
/// that branches win/posix has one `emit_external_call` in each, and a
/// single-platform sweep proves nothing about the arm it did not take. That is
/// two uncovered lines in ~40 `func_*.rs` files, every one of them the same `?`.
///
/// **The invariant is not the same on both sides of that branch, and asserting
/// the POSIX one everywhere reported 16 false findings.** On POSIX,
/// `emit_external_call` resolves the symbol through `platform_imports`, so an
/// EMPTY list must yield no externally-bound relocation at all — that is the
/// guard, and a body that emitted one anyway would produce an executable that
/// does not link, or worse binds to whatever the loader finds. Win32 does not
/// work that way: `call_external` names the DLL at the emit site (`kernel32`),
/// deliberately, because "naming the library here keeps the reloc
/// self-describing" and the trait methods that need it carry no
/// `platform_imports` at all. So Windows legitimately emits `library:
/// Some("kernel32.dll")` from an empty list, and the contract to check there is
/// the complementary one: the name is never EMPTY.
#[test]
fn no_abi_function_body_emits_a_call_the_plan_never_declared() {
    let mut leaked = Vec::new();
    let mut panicked = Vec::new();
    let mut swept = 0;
    for (target, platform) in os_seam_platforms() {
        swept += sweep_abi_function_bodies(target, platform.as_ref(), &mut leaked, &mut panicked);
    }
    // The one sanctioned hard-stop: riscv64 has no app-mode port, and every GTK
    // hook refuses at the boundary "rather than after assembling
    // wrong-convention instructions". Anything else that panicked is a body that
    // crashes on a backend nothing else lowers it for.
    let unexpected: Vec<&String> = panicked
        .iter()
        .filter(|p| !p.contains("not ported"))
        .collect();
    assert!(
        unexpected.is_empty(),
        "{} abi_function body(ies) panicked for a reason that is not the \
         documented rv64 app-mode hard-stop:\n  {}",
        unexpected.len(),
        unexpected
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert!(
        swept >= 500,
        "the sweep found only {swept} abi_function implementations across five          backends; there were >= 100 per backend when this was written, so the          registry walk has broken"
    );
    assert!(
        leaked.is_empty(),
        "{} abi_function body(ies) emitted an externally-bound relocation with no          platform import declared:\n  {}",
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
