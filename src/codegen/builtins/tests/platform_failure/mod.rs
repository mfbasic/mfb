//! An emit failure PAST THE FIRST one in a body must propagate too.
//!
//! `registry_bodies.rs` and `abi_inline.rs` already lower every registry body
//! with an EMPTY import list, on all five backends, which forces the FIRST
//! `platform.emit_external_call(…)?` in each body to fail. That is the import
//! guard, and it is covered.
//!
//! What it cannot reach is every `?` after it. A body is a sequence — open a
//! socket, set it non-blocking, connect, read — and the first failure aborts
//! the rest, so the second and later `?` sites stay exactly as dead as before.
//! They are the single largest identifiable class left in the coverage gap: 931
//! `?`-propagation lines across the files below the floor, 580 of them in
//! `src/codegen/builtins` alone, and `tls/gen_openssl/` has 80 by itself.
//!
//! Each one is a real contract. A body that swallowed the failure to emit
//! `connect` would emit a function that opens a socket and then reads from one
//! nothing connected — not a crash, a wrong program.
//!
//! [`FailAt`] wraps a real platform and fails its Nth fallible call. Driving N
//! from 0 to however many a clean lowering of that body makes walks its `?`
//! sites one at a time, and each must come back as a refusal carrying the
//! injected message.
//!
//! Two details that decide whether this measures anything:
//!
//! **It delegates rather than stubs.** `CodegenPlatform` has 36 defaulted
//! methods, so an impl overriding only the interesting ones would substitute
//! DEFAULT behaviour for the real platform's everywhere else — lowering a
//! different body than the one under test. The forwarding impl is generated
//! from the trait definition, so a method added to the trait is a compile error
//! here rather than a silent hole.
//!
//! **It grants the import each call names.** `emit_external_call` refuses a
//! symbol the plan never declared, which is what the empty-list sweep tests; if
//! this ran with the same empty list every body would stop at its first call
//! again and there would be nothing new. So the wrapper inserts the symbol into
//! the map it forwards. That is the one place it lies to the emitter, and it
//! lies in the direction that lets the body run.

use std::cell::Cell;
use std::collections::HashMap;

use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::BuilderHarness;
use crate::codegen::engine::types::{
    AppEntrySpec, ArenaInitSymbols, CodeDataObject, CodeFunction, CodeInstruction, CodeRelocation,
    CodegenPlatform, FsPathOperation, PlatformFamily, ProgramEntrySpec, TerminalControlCall,
};
use crate::codegen::engine::util::vreg_frame::Vregs;
use crate::codegen::registry::{registry, AbiCtx, Body};
use crate::os::linux::flavor::LinuxFlavor;

mod entry_sweep;
mod forwarding;
mod module_sweep;
mod typed_bodies;

/// The message the injected failure carries, distinctive enough that a refusal
/// from another cause cannot be mistaken for it.
const INJECTED: &str = "$injected platform emit failure";

/// A real platform whose Nth fallible call is replaced by a failure.
struct FailAt<'a> {
    inner: &'a dyn CodegenPlatform,
    /// The index of the one call to fail. `usize::MAX` fails none, which is how
    /// the clean run counts them.
    target: Cell<usize>,
    made: Cell<usize>,
}

impl<'a> FailAt<'a> {
    fn new(inner: &'a dyn CodegenPlatform, at: usize) -> Self {
        FailAt {
            inner,
            target: Cell::new(at),
            made: Cell::new(0),
        }
    }

    /// A platform that never fails, used to count the calls a clean lowering of
    /// one body makes.
    fn counting(inner: &'a dyn CodegenPlatform) -> Self {
        FailAt::new(inner, usize::MAX)
    }

    fn calls(&self) -> usize {
        self.made.get()
    }

    /// Fail EXACTLY the target call, and let every other one through.
    ///
    /// Not "fail from the target onward". A wrapper that stayed failed made the
    /// sweep insensitive to the very defect it is for: a body that swallowed
    /// call #n would run on to call #n+1, fail THERE, and return the same `Err`
    /// the test was looking for. Verified by making `os/func_has_env.rs`
    /// swallow its `getenv` emit — the sweep passed, until this.
    fn tick(&self) -> Result<(), String> {
        let index = self.made.get();
        self.made.set(index + 1);
        if index == self.target.get() {
            return Err(INJECTED.to_string());
        }
        Ok(())
    }

    /// The caller's import map with `symbol` declared, so the emitter's import
    /// guard passes and the body proceeds to its next call.
    fn granting(imports: &HashMap<String, String>, symbol: &str) -> HashMap<String, String> {
        let mut granted = imports.clone();
        granted.insert(symbol.to_string(), "c".to_string());
        granted
    }
}

/// The five real backends. Copied in shape from `registry_bodies.rs` and for
/// the same reason: a body that reaches the OS branches on `family()` and has a
/// different call sequence in each arm, so sweeping one backend leaves the
/// others' `?` sites exactly as dead as they were.
fn os_seam_platforms() -> Vec<(&'static str, Box<dyn CodegenPlatform>)> {
    vec![
        (
            "macos-aarch64",
            Box::new(crate::target::macos_aarch64::code::Platform) as Box<dyn CodegenPlatform>,
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

/// Everything a body asks the harness for before it will lower.
#[derive(Default, Clone)]
struct Needs {
    /// Symbol -> library, as a `NativePlan`'s platform imports render.
    imports: HashMap<String, String>,
    /// String literal -> data-object symbol.
    strings: HashMap<String, String>,
}

/// The import list a clean lowering of this body needs, discovered by asking.
///
/// `FailAt` grants the symbol `emit_external_call` names, which is enough for a
/// body that reaches libc through the platform. It is NOT enough for one that
/// consults `ctx.platform_imports` ITSELF — every `audio.*` member does, and
/// stopped at its first call with "runtime helper requires `_X` import", so the
/// whole `gen_macos_*` family was swept zero times and this sweep silently did
/// not cover it.
///
/// So: lower, read the symbol out of the refusal, grant it, lower again. It
/// terminates because each round adds one symbol and a body names finitely many;
/// the cap is a guard against a refusal whose wording changes, not against a
/// real loop. This is the same list a real plan derives from the calls the body
/// emits — discovered the same way, one call at a time.
fn what_a_clean_lowering_needs(
    lower: crate::codegen::registry::AbiFunction,
    params: &[crate::codegen::registry::Parameter],
    call: &str,
    platform: &dyn CodegenPlatform,
) -> Needs {
    let mut needs = Needs::default();
    for _ in 0..128 {
        let counting = FailAt::counting(platform);
        let Some(Err(message)) = lower_body(lower, params, call, &counting, &needs) else {
            return needs;
        };
        // Each arm reads what the refusal names and provides it. A refusal this
        // does not recognise ends the loop and the body is skipped -- which is
        // how `native record type 'KeyPair' does not resolve` (5 bodies) is
        // handled: a type model is more than a name, and those stay out.
        let satisfied = if let Some(symbol) = between(&message, "requires ", " import") {
            needs.imports.insert(symbol, "c".to_string()).is_none()
        } else if let Some(literal) = between(&message, "string literal '", "' has no data object")
        {
            let symbol = format!("_mfb_str_probe_{}", needs.strings.len());
            needs.strings.insert(literal, symbol).is_none()
        } else {
            false
        };
        if !satisfied {
            // Asked for the same thing twice, or asked for something this does
            // not know how to give.
            return needs;
        }
    }
    needs
}

/// The text between `open` and `close`, if both are present in that order.
fn between(haystack: &str, open: &str, close: &str) -> Option<String> {
    let rest = haystack.split(open).nth(1)?;
    Some(rest.split(close).next()?.to_string())
}

/// Lower one `abi_function` body with `platform`, swallowing a panic.
///
/// A body that PANICS is not a finding here: `AppSupport::require_gtk`
/// hard-stops an ISA with no app-mode port at the boundary, deliberately, and
/// `registry_bodies.rs` already asserts that is the only one. This sweep is
/// about the bodies that RUN, so a panic just ends that body's walk.
fn lower_body(
    lower: crate::codegen::registry::AbiFunction,
    params: &[crate::codegen::registry::Parameter],
    call: &str,
    platform: &dyn CodegenPlatform,
    needs: &Needs,
) -> Option<Result<ValueResult, String>> {
    let mut vregs = Vregs::new();
    let args: Vec<ValueResult> = params
        .iter()
        .map(|param| ValueResult {
            type_: param.ty.clone(),
            location: Operand::from(vregs.next()),
            text: param.name.to_string(),
            origin: None,
        })
        .collect();
    let harness = BuilderHarness {
        platform_imports: needs.imports.clone(),
        string_symbols: needs.strings.clone(),
        ..BuilderHarness::default()
    };
    let mut builder = harness.builder("_mfb_rt_probe", platform);
    let base = harness.abi_ctx(platform);
    // Both slots reserved, because 130 bodies refuse without them -- "native
    // code plan emits '_mfb_rt_probe' without reserving term state" and "…
    // without reserving the presentation-mode slot". A real app-mode plan
    // reserves both; the harness's default leaves them `None`, which is the
    // right default for a suite that is not about app mode and the wrong one
    // for a sweep that wants every body to run.
    let ctx = AbiCtx {
        call,
        term_state_offset: Some(0),
        presentation_mode_offset: Some(8),
        ..base
    };

    let lowered = crate::testutil::silence_panics(|| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            lower(&mut builder, &args, &ctx)
        }))
    });
    lowered.ok()
}

/// Every fallible platform call an `abi_function` body makes, failed one at a
/// time, must come back as a refusal.
///
/// The clean run of each body comes first and its call count is that body's
/// loop bound, so this measures the surface rather than hardcoding it: a body
/// that grows another `emit_*` is covered the day it lands.
///
/// A failure that does NOT propagate is the defect this is written for — the
/// body returns `Ok`, having emitted a sequence with a hole in it. The
/// assertion names the member and the index so the call site is findable.
#[test]
fn an_emit_failure_past_the_first_one_still_propagates() {
    let mut swept = 0usize;
    let mut injected = 0usize;
    let mut swallowed: Vec<String> = Vec::new();

    for (target, platform) in os_seam_platforms() {
        for package in registry().packages() {
            for function in package.functions() {
                for implementation in function.implementations() {
                    let Body::AbiFunction { lower, .. } = implementation.body else {
                        continue;
                    };
                    let call = format!("{}.{}", package.import_name(), function.name);

                    // How many fallible platform calls a clean lowering makes.
                    // A body that refuses outright, or panics, contributes none:
                    // refusing is how a member says it is not implemented here.
                    let needs = what_a_clean_lowering_needs(
                        lower,
                        &implementation.params,
                        &call,
                        platform.as_ref(),
                    );
                    let counting = FailAt::counting(platform.as_ref());
                    let Some(Ok(_)) =
                        lower_body(lower, &implementation.params, &call, &counting, &needs)
                    else {
                        continue;
                    };
                    let calls = counting.calls();
                    if calls == 0 {
                        continue;
                    }
                    swept += 1;

                    for n in 0..calls {
                        let failing = FailAt::new(platform.as_ref(), n);
                        injected += 1;
                        match lower_body(lower, &implementation.params, &call, &failing, &needs) {
                            Some(Err(message)) if message.contains(INJECTED) => {}
                            Some(Err(message)) => swallowed.push(format!(
                                "{target} {call} call #{n}: a different refusal came \
                                 back, so something between the emit and the caller \
                                 replaced the error rather than propagating it: \
                                 {message}"
                            )),
                            Some(Ok(_)) => swallowed.push(format!(
                                "{target} {call} call #{n}: the emit failed and the \
                                 body returned Ok. It swallowed the failure, so the \
                                 sequence it emitted is missing that call."
                            )),
                            None => swallowed.push(format!(
                                "{target} {call} call #{n}: the body PANICKED on the \
                                 error path, though it lowered cleanly without the \
                                 injection. An error path that panics is worse than \
                                 one that propagates."
                            )),
                        }
                    }
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
    // The row that keeps the rest honest: every assertion above holds against a
    // sweep that found no bodies at all.
    // Measured at 730 bodies / 4,577 injections. It was 331 / 1,997 when the
    // sweep landed, and every step since has been a harness gap rather than a
    // registry change: the import fixpoint took it to 710, and satisfying the
    // string-literal and reserved-slot refusals took it here. The bound is set
    // well below so ordinary registry churn does not red it, and well above
    // zero so the sweep going quiet does.
    assert!(
        swept > 650 && injected > 4000,
        "only {swept} bodies and {injected} injected failures across five \
         backends -- the sweep stopped finding the bodies it is for"
    );
}
