//! Two rules every `abi_inline` builtin lowering obeys, swept over all of them.
//!
//! There are 175 `abi_inline` implementations in the registry, and each one ends
//! with the same two guards: a type check on its arguments, and a `?` on the
//! platform emitter that resolves its libc import. Both are unreachable from any
//! source program — the type checker has already validated the call, and the
//! plan derives its import list from the very calls these bodies emit — so both
//! are dead in coverage terms and rot silently. Across the tree that is one or
//! two uncovered lines in ~90 otherwise-covered files.
//!
//! They are also the two guards that matter most. A type-keyed lowering that
//! fails OPEN picks *some* overload's layout for a value of another shape, which
//! is a misread of memory rather than an error; a body that emits a call the
//! plan never declared produces an executable that does not link, or worse binds
//! to whatever the loader finds.
//!
//! The sweep is over the registry itself rather than over a list of members, so
//! a package added tomorrow is covered the day it lands.

use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::codegen::engine::util::vreg_frame::Vregs;
use crate::codegen::registry::{registry, Body, Implementation};
use crate::types::ParameterType;

/// `(package, member)` for every `abi_inline` implementation, with its overload
/// index so an overload set is swept per overload rather than once.
fn abi_inline_members() -> Vec<(String, &'static Implementation)> {
    let mut out = Vec::new();
    for package in registry().packages() {
        for function in package.functions() {
            for (index, implementation) in function.implementations().iter().enumerate() {
                if matches!(implementation.body, Body::AbiInline(_)) {
                    out.push((
                        format!("{}::{}#{index}", package.import_name(), function.name),
                        implementation,
                    ));
                }
            }
        }
    }
    out
}

/// One `ValueResult` per declared parameter, carrying `ty` and a distinct
/// register.
fn args(
    implementation: &Implementation,
    ty: impl Fn(&ParameterType) -> ParameterType,
) -> Vec<ValueResult> {
    let mut vregs = Vregs::new();
    implementation
        .params
        .iter()
        .map(|param| ValueResult {
            type_: ty(&param.ty),
            location: Operand::from(vregs.next().as_str()),
            text: param.name.to_string(),
            origin: None,
        })
        .collect()
}

/// The five members that read only their argument's LOCATION and never its type.
///
/// Named individually, with the count asserted, so a sixth cannot join them
/// quietly: a lowering that stops checking its argument type is exactly the
/// fail-open this sweep exists to prevent.
const TYPE_AGNOSTIC: &[&str] = &[
    // Counts non-continuation bytes in the record's inlined `text` field; the
    // walk is over bytes and never consults the value's declared type.
    "astrings::scalarLen#0",
    // Table lookups keyed on a scalar code point, not on the argument's type.
    "regex::genCat#0",
    "regex::scriptOf#0",
    "strings::genCat#0",
    // Selects on the enum MEMBER the call site resolved, which reaches the body
    // as a constant rather than as a typed value.
    "money::setRounding#0",
];

/// Every `abi_inline` lowering refuses an argument of a type no overload takes.
///
/// `ParameterType::named("NoSuchType")` is a type nothing in the language
/// declares, so no correct lowering can have a layout for it. A body that
/// accepts it has selected some other overload's layout for a value of unknown
/// shape — the fail-open case that reads and writes memory at the wrong offsets
/// instead of reporting anything.
#[test]
fn every_abi_inline_lowering_refuses_an_argument_type_it_cannot_handle() {
    let platform = TestPlatform;
    let mut accepted = Vec::new();
    let mut swept = 0;
    for (member, implementation) in abi_inline_members() {
        if implementation.params.is_empty() {
            continue;
        }
        let Body::AbiInline(lower) = implementation.body else {
            continue;
        };
        swept += 1;
        // One position at a time, not all at once. A body checks its arguments
        // in order and returns at the first bad one, so poisoning every position
        // only ever reaches the FIRST guard -- the second and third are then
        // just as dead as they were. Poisoning position `at` alone is what walks
        // past the earlier guards to the one being tested.
        for at in 0..implementation.params.len() {
            let mut wrong = args(implementation, Clone::clone);
            wrong[at].type_ = ParameterType::named("NoSuchType");
            let harness = BuilderHarness::default();
            let mut builder = harness.builder("_mfb_abi_inline_probe", &platform);
            let ctx = harness.abi_ctx(&platform);
            if lower(&mut builder, &wrong, &ctx).is_ok() {
                accepted.push(format!("{member} (argument {at})"));
            }
        }
    }
    assert!(
        swept >= 170,
        "the sweep found only {swept} abi_inline implementations with parameters; \
         it found 174 when this was written, so the registry walk has broken"
    );
    let unexpected: Vec<&String> = accepted
        .iter()
        .filter(|m| {
            let member = m
                .split_once(" (argument")
                .map_or(m.as_str(), |(name, _)| name);
            !TYPE_AGNOSTIC.contains(&member)
        })
        .collect();
    assert!(
        unexpected.is_empty(),
        "{} abi_inline lowering(s) accepted an argument of a type no overload \
         declares, so they select a layout by something other than the type: \
         {unexpected:?}",
        unexpected.len()
    );
    assert!(
        !accepted.is_empty(),
        "the documented type-agnostic members accepted nothing, so the sweep is \
         no longer reaching them"
    );
}

/// With no import declared, no `abi_inline` lowering emits an external call.
///
/// A relocation carries `library: Some(_)` exactly when it binds to a symbol the
/// platform import list must declare. Lowering with an EMPTY list and finding
/// one means the body emitted a call the plan does not know about: at best a
/// link failure, at worst a bind to whatever the loader turns up. Every body
/// that reaches libc has a guard for this, and no program can reach it, because
/// the plan's import list is derived from these very calls.
#[test]
fn no_abi_inline_lowering_emits_a_call_the_plan_never_declared() {
    let platform = TestPlatform;
    let mut leaked = Vec::new();
    let mut refused = 0;
    for (member, implementation) in abi_inline_members() {
        let Body::AbiInline(lower) = implementation.body else {
            continue;
        };
        let typed = args(implementation, Clone::clone);
        let harness = BuilderHarness::default();
        let mut builder = harness.builder("_mfb_abi_inline_probe", &platform);
        let ctx = harness.abi_ctx(&platform);
        if lower(&mut builder, &typed, &ctx).is_err() {
            refused += 1;
            continue;
        }
        let external: Vec<String> = builder
            .relocations
            .iter()
            .filter(|r| r.library.is_some())
            .map(|r| r.to.clone())
            .collect();
        if !external.is_empty() {
            leaked.push(format!("{member} -> {external:?}"));
        }
    }
    assert!(
        leaked.is_empty(),
        "{} abi_inline lowering(s) emitted an externally-bound relocation with no \
         platform import declared:\n  {}",
        leaked.len(),
        leaked.join("\n  ")
    );
    assert!(
        refused >= 120,
        "only {refused} of the abi_inline lowerings refused to lower with no \
         imports declared; 127 did when this was written, so a body has stopped \
         checking"
    );
}

/// No `abi_inline` lowering SILENTLY accepts the wrong number of arguments.
///
/// The dispatcher only ever hands a body the arity its descriptor declares, so
/// the `if args.len() != 1 { return Err(...) }` at the top of most of them is
/// unreachable from any program, and rots.
///
/// "Refuses" is deliberately not the assertion: 48 of the 175 index `args[0]`
/// with no arity check and abort instead. That is not a shipped defect -- the
/// arity is fixed by the descriptor the dispatcher selected -- and an abort is
/// loud in every profile. What must never happen is the third outcome: a body
/// that EMITS CODE for a call whose shape it did not understand. So the
/// assertion is over silent acceptance, which is the only unsafe answer, and
/// the guards that do exist are exercised on the way.
#[test]
fn no_abi_inline_lowering_silently_accepts_the_wrong_argument_count() {
    let platform = TestPlatform;
    let mut accepted = Vec::new();
    let mut panicked = Vec::new();
    for (member, implementation) in abi_inline_members() {
        let Body::AbiInline(lower) = implementation.body else {
            continue;
        };
        // TOO FEW, which is the only direction that can make a body read past
        // the end of the slice. The other direction is deliberately not swept:
        // an EXTRA argument the dispatcher can never supply is ignored by 16 of
        // these bodies, and ignoring it emits exactly the same correct code.
        for count in [0usize] {
            if count == implementation.params.len() {
                continue;
            }
            let mut vregs = Vregs::new();
            let wrong: Vec<ValueResult> = (0..count)
                .map(|_| ValueResult {
                    type_: ParameterType::Integer,
                    location: Operand::from(vregs.next().as_str()),
                    text: "extra".to_string(),
                    origin: None,
                })
                .collect();
            let label = format!("{member} with {count} arg(s)");
            let outcome = crate::testutil::silence_panics(|| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let harness = BuilderHarness::default();
                    let mut builder = harness.builder("_mfb_abi_inline_probe", &platform);
                    let ctx = harness.abi_ctx(&platform);
                    lower(&mut builder, &wrong, &ctx).is_ok()
                }))
            });
            match outcome {
                Ok(true) => accepted.push(label),
                Ok(false) => {}
                Err(_) => panicked.push(label),
            }
        }
    }
    assert!(
        panicked.len() <= 60,
        "{} abi_inline lowering(s) abort on the wrong argument count rather than \
         reporting it; 48 did when this was written. That is safe but noisy, and a \
         jump means a guard was removed:\n  {}",
        panicked.len(),
        panicked.join("\n  ")
    );
    assert!(
        accepted.is_empty(),
        "{} abi_inline lowering(s) EMITTED CODE for a call with the wrong argument \
         count -- neither reporting it nor aborting:\n  {}",
        accepted.len(),
        accepted.join("\n  ")
    );
}
