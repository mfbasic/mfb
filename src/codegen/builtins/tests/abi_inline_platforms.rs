//! No `abi_inline` lowering binds to a symbol the plan never declared — on any backend.
//!
//! `abi_inline.rs` sweeps the inline lowerings against `TestPlatform`, whose OS seam is a
//! stub, so the per-OS bodies — the macOS crypto and audio emitters, the Windows
//! variants of the same members — were never asked to resolve a libc symbol against an
//! empty import list. `registry_bodies.rs` does that for the OS-seam bodies on all five
//! real backends; this does it for the inline ones: lower every `abi_inline`
//! implementation with NO imports declared, on each backend, and assert it either refuses
//! (the import guard the plan's derived import list makes unreachable from a program) or
//! emits no externally-bound relocation. A body that emitted one anyway would produce an
//! executable that does not link, or worse binds to whatever the loader turns up.
//!
//! Win32 names the DLL at the emit site, so an empty list does not suppress its relocation:
//! there the contract is that the library name is never empty.

use super::platforms::platform_for;
use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::BuilderHarness;
use crate::codegen::engine::util::vreg_frame::Vregs;
use crate::codegen::registry::{registry, Body};
use crate::testutil::CodeTarget;

#[test]
fn no_abi_inline_lowering_emits_an_undeclared_call_on_any_backend() {
    let mut leaked = Vec::new();
    let mut refused = 0;
    let mut swept = 0;
    for target in CodeTarget::ALL {
        let platform = platform_for(target);
        let windows = target == CodeTarget::WindowsX86_64;
        for package in registry().packages() {
            for function in package.functions() {
                for (index, implementation) in function.implementations().iter().enumerate() {
                    let Body::AbiInline(lower) = implementation.body else {
                        continue;
                    };
                    swept += 1;
                    let mut vregs = Vregs::new();
                    let args: Vec<ValueResult> = implementation
                        .params
                        .iter()
                        .map(|param| ValueResult {
                            type_: param.ty.clone(),
                            location: Operand::from(vregs.next()),
                            text: param.name.to_string(),
                            origin: None,
                        })
                        .collect();
                    let outcome = crate::testutil::silence_panics(|| {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            crate::codegen::engine::mir::set_backend(platform.backend());
                            let harness = BuilderHarness::default();
                            let mut builder = harness.builder("_mfb_abi_inline_probe", &*platform);
                            let ctx = harness.abi_ctx(&*platform);
                            let result = lower(&mut builder, &args, &ctx);
                            (
                                result.is_ok(),
                                builder
                                    .relocations
                                    .iter()
                                    .filter_map(|r| {
                                        r.library.clone().map(|library| (r.to.clone(), library))
                                    })
                                    .collect::<Vec<_>>(),
                            )
                        }))
                    });
                    // A panic is the documented hard-stop of an ISA with no app-mode port; it
                    // is not a leak.
                    let Ok((ok, external)) = outcome else {
                        continue;
                    };
                    if !ok {
                        refused += 1;
                        continue;
                    }
                    for (to, library) in external {
                        if !windows || library.is_empty() {
                            leaked.push(format!(
                                "{} {}::{}#{index} -> {to} ({library})",
                                target.name(),
                                package.import_name(),
                                function.name
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(swept >= 800, "the sweep reached only {swept} lowerings");
    assert!(
        refused >= 300,
        "only {refused} lowerings refused with no imports declared; the import guard has gone"
    );
    assert!(
        leaked.is_empty(),
        "{} abi_inline lowering(s) emitted an externally-bound relocation with no platform \
         import declared:\n  {}",
        leaked.len(),
        leaked.join("\n  ")
    );
}
