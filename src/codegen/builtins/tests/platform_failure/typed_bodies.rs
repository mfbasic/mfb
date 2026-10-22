//! The registry bodies the main sweep skips, swept with a real type model.
//!
//! `an_emit_failure_past_the_first_one_still_propagates` lowers each body with the
//! harness's EMPTY type model, so a body that builds a package record refuses before
//! its first platform call — `native record type 'KeyPair' does not resolve` — and is
//! skipped (its own comment says so: "a type model is more than a name, and those
//! stay out"). `crypto::generate` is one: on macOS it opens Security.framework,
//! resolves a dozen `SecKey*`/`CF*` symbols and runs the key-generation sequence,
//! and a failure at any of those emits must stop the body, not leave a key pair
//! built from a call that was never emitted.
//!
//! So: give the harness the type model of a program that uses those records, and
//! sweep every body that refused only for want of one.

use super::*;
use crate::codegen::engine::builder::TypeModel;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};

/// A program whose module declares the `crypto` records the bodies build.
const CRYPTO: &str = "\
IMPORT crypto

FUNC main() AS Integer
  LET k AS crypto::KeyPair = crypto::generate(crypto::Certificate.Ed25519)
  RETURN 0
END FUNC
";

/// [`lower_body`] with `type_model` in the harness instead of the empty one.
fn lower_typed(
    lower: crate::codegen::registry::AbiFunction,
    params: &[crate::codegen::registry::Parameter],
    call: &str,
    platform: &dyn CodegenPlatform,
    needs: &Needs,
    type_model: &TypeModel,
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
        type_model: type_model.clone(),
        ..BuilderHarness::default()
    };
    let mut builder = harness.builder("_mfb_rt_probe", platform);
    let ctx = AbiCtx {
        call,
        term_state_offset: Some(0),
        presentation_mode_offset: Some(8),
        ..harness.abi_ctx(platform)
    };
    crate::testutil::silence_panics(|| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            lower(&mut builder, &args, &ctx)
        }))
    })
    .ok()
}

/// The imports and strings a clean typed lowering asks for, discovered as
/// [`what_a_clean_lowering_needs`] does.
fn typed_needs(
    lower: crate::codegen::registry::AbiFunction,
    params: &[crate::codegen::registry::Parameter],
    call: &str,
    platform: &dyn CodegenPlatform,
    type_model: &TypeModel,
) -> Needs {
    let mut needs = Needs::default();
    for _ in 0..128 {
        let counting = FailAt::counting(platform);
        let Some(Err(message)) = lower_typed(lower, params, call, &counting, &needs, type_model)
        else {
            break;
        };
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
            break;
        }
    }
    needs
}

#[test]
fn a_record_building_body_propagates_every_emit_failure() {
    let module = nir_for_src(CRYPTO, CodeTarget::MacosAarch64, Console)
        .unwrap_or_else(|err| panic!("the probe lowers to NIR: {err}"));
    let type_model = TypeModel::from_module(&module).expect("the probe's type model builds");

    let mut swept = Vec::new();
    let mut injected = 0usize;
    let mut swallowed = Vec::new();
    for (target, platform) in os_seam_platforms() {
        for package in registry().packages() {
            for function in package.functions() {
                for implementation in function.implementations() {
                    let Body::AbiFunction { lower, .. } = implementation.body else {
                        continue;
                    };
                    let call = format!("{}.{}", package.import_name(), function.name);
                    let params = &implementation.params;
                    // Only the bodies the untyped sweep could not lower for want of a
                    // record type; every other body is that sweep's.
                    let untyped =
                        what_a_clean_lowering_needs(lower, params, &call, platform.as_ref());
                    let refused_for_a_type = matches!(
                        lower_body(lower, params, &call, &FailAt::counting(platform.as_ref()), &untyped),
                        Some(Err(message)) if message.ends_with("does not resolve")
                    );
                    if !refused_for_a_type {
                        continue;
                    }
                    let needs = typed_needs(lower, params, &call, platform.as_ref(), &type_model);
                    let counting = FailAt::counting(platform.as_ref());
                    let Some(Ok(_)) =
                        lower_typed(lower, params, &call, &counting, &needs, &type_model)
                    else {
                        continue;
                    };
                    swept.push(format!("{target} {call}"));
                    for n in 0..counting.calls() {
                        injected += 1;
                        let failing = FailAt::new(platform.as_ref(), n);
                        match lower_typed(lower, params, &call, &failing, &needs, &type_model) {
                            Some(Err(message)) if message.contains(INJECTED) => {}
                            Some(Err(message)) => swallowed.push(format!(
                                "{target} {call} call #{n}: a different refusal came back: \
                                 {message}"
                            )),
                            Some(Ok(_)) => swallowed.push(format!(
                                "{target} {call} call #{n}: the emit failed and the body \
                                 returned Ok"
                            )),
                            None => swallowed.push(format!(
                                "{target} {call} call #{n}: the body PANICKED on the error \
                                 path"
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
    // The body this file exists for must be among the swept, on the backend whose
    // sequence is longest.
    assert!(
        swept
            .iter()
            .any(|row| row == "macos-aarch64 crypto.generate"),
        "crypto.generate was not swept on macOS: {swept:?}"
    );
}
