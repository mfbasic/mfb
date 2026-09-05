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
    implementation
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| ValueResult {
            type_: ty(&param.ty),
            location: Operand::from(format!("x{}", 9 + index).as_str()),
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
        let wrong = args(implementation, |_| ParameterType::named("NoSuchType"));
        let harness = BuilderHarness::default();
        let mut builder = harness.builder("_mfb_abi_inline_probe", &platform);
        let ctx = harness.abi_ctx(&platform);
        if lower(&mut builder, &wrong, &ctx).is_ok() {
            accepted.push(member);
        }
    }
    assert!(
        swept >= 170,
        "the sweep found only {swept} abi_inline implementations with parameters; \
         it found 174 when this was written, so the registry walk has broken"
    );
    let unexpected: Vec<&String> = accepted
        .iter()
        .filter(|m| !TYPE_AGNOSTIC.contains(&m.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "{} abi_inline lowering(s) accepted an argument of a type no overload \
         declares, so they select a layout by something other than the type: \
         {unexpected:?}",
        unexpected.len()
    );
    assert_eq!(
        accepted.len(),
        TYPE_AGNOSTIC.len(),
        "one of the documented type-agnostic members has started checking its \
         argument type. That is an improvement — remove it from TYPE_AGNOSTIC. \
         Still accepting: {accepted:?}"
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
