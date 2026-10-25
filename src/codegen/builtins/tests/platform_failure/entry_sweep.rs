//! The program entry stub (`engine/function/entry`), swept like the registry bodies.
//!
//! The entry is not reached through a [`FailAt`] handed to the module lowering: each
//! backend's `emit_program_entry` passes ITS OWN platform to `lower_program_entry`,
//! so the wrapper sees the one outer call and none of the stub's own. The stub is a
//! long sequence of platform calls — the stack limit, the standard-descriptor
//! reservation, the `--debug` entry work, argv capture, the signal handlers and
//! `SIGPIPE`, the RNG seed, the console code page, Winsock, the exit — and a failure
//! at any one of them must stop the stub, not leave a program whose entry silently
//! skips installing its signal handlers or seeding its generator.
//!
//! So `lower_program_entry` is driven directly with the wrapper, on every backend,
//! with every optional part switched on (and once as an app-mode entry called as a
//! function, which takes the other arm of the descriptor and argv blocks).

use super::*;
use crate::types::ParameterType;

/// One entry configuration, everything optional switched on except where `called`
/// selects the app-mode shape (entered by a call, so no descriptor reservation).
fn lower_entry(
    platform: &dyn CodegenPlatform,
    imports: &HashMap<String, String>,
    called: bool,
) -> Result<CodeFunction, String> {
    crate::codegen::engine::function::lower_program_entry(
        "_main",
        "_mfb_fn_main",
        &ParameterType::Integer,
        true,
        Some("_mfb_global_init"),
        Some("_mfb_link_init"),
        Some("_mfb_closure_init"),
        Some("_mfb_module_teardown"),
        4096,
        2,
        imports,
        platform,
        true,
        true,
        true,
        true,
        true,
        called,
        true,
        Some(8),
        crate::codegen::debug::DEBUG_FEATURES,
        true,
    )
}

/// The imports a clean lowering needs, discovered the way
/// `what_a_clean_lowering_needs` does: `FailAt` grants what `emit_external_call`
/// names, but a part of the stub that consults the import map itself refuses with
/// "requires `X` import", and is granted `X` and lowered again.
fn imports_for(platform: &dyn CodegenPlatform, called: bool) -> HashMap<String, String> {
    let mut imports = HashMap::new();
    for _ in 0..32 {
        let Err(message) = lower_entry(&FailAt::counting(platform), &imports, called) else {
            break;
        };
        let Some(symbol) = between(&message, "requires ", " import") else {
            break;
        };
        if imports.insert(symbol, "c".to_string()).is_some() {
            break;
        }
    }
    imports
}

#[test]
fn every_platform_call_in_the_entry_stub_propagates_its_failure() {
    let mut swallowed = Vec::new();
    let mut injected = 0usize;
    for (target, platform) in os_seam_platforms() {
        for called in [false, true] {
            let imports = imports_for(platform.as_ref(), called);
            let counting = FailAt::counting(platform.as_ref());
            let clean = lower_entry(&counting, &imports, called);
            assert!(
                clean.is_ok(),
                "{target} (called: {called}): the clean entry must lower before its \
                 failures mean anything: {:?}",
                clean.err()
            );
            for n in 0..counting.calls() {
                injected += 1;
                match lower_entry(&FailAt::new(platform.as_ref(), n), &imports, called) {
                    Err(message) if message.contains(INJECTED) => {}
                    Err(message) => swallowed.push(format!(
                        "{target} (called: {called}) call #{n}: a different refusal came \
                         back: {message}"
                    )),
                    Ok(_) => swallowed.push(format!(
                        "{target} (called: {called}) call #{n}: the emit failed and the \
                         entry lowered anyway"
                    )),
                }
            }
        }
    }
    assert!(
        swallowed.is_empty(),
        "{} of {injected} injected failures did not propagate:\n{}",
        swallowed.len(),
        swallowed.join("\n")
    );
    // Measured when written; the floor catches a sweep that stopped finding calls.
    assert!(
        injected > 40,
        "only {injected} injected failures across five backends"
    );
}
