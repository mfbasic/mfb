//! bug-567: the pending-temp disposition vocabulary must stay TOTAL.
//!
//! The primary enforcement is the compiler: `transfer_temp_disposition`
//! matches every `NirOp` variant with no wildcard, so a new statement kind is
//! a build error until someone classifies it. These read the source to keep
//! that property from being "fixed" away — a wildcard arm would turn the
//! build error back into the silent default that bug-567 was.

/// This file's own text, and the file that defines the vocabulary it
/// classifies.
const CLASSIFIER: &str = include_str!("../control/builder_control/mod.rs");
const NIR: &str = include_str!("../../../target/shared/nir/mod.rs");

/// The body of `fn transfer_temp_disposition`, from its signature to the
/// closing brace of its `match`.
fn classifier_body() -> &'static str {
    let start = CLASSIFIER
        .find("fn transfer_temp_disposition(op: &NirOp) -> TransferTemps {")
        .expect("the classifier is defined in this file");
    let rest = &CLASSIFIER[start..];
    let end = rest
        .find("\n    }")
        .expect("the classifier's closing brace");
    &rest[..end]
}

/// Every variant name declared by `enum NirOp`.
fn nir_op_variants() -> Vec<String> {
    let start = NIR.find("pub(crate) enum NirOp {").expect("the NirOp enum");
    let rest = &NIR[start..];
    let end = rest.find("\n}").expect("the NirOp enum's closing brace");
    let mut variants: Vec<String> = rest[..end]
        .lines()
        // A variant is declared at exactly one indent level inside the enum,
        // and its fields at two — so the four-space prefix is the filter.
        .filter_map(|line| {
            let name = line.strip_prefix("    ")?;
            if name.starts_with(' ') || name.starts_with("//") || name.starts_with('/') {
                return None;
            }
            let name = name.trim_end_matches(" {");
            if name.is_empty() || !name.starts_with(char::is_uppercase) {
                return None;
            }
            Some(name.to_string())
        })
        .collect();
    variants.sort();
    variants.dedup();
    variants
}

/// The one that would have let bug-567 back in. A `_ =>` arm means a new
/// statement kind inherits some existing ownership answer without anyone
/// deciding it does — and the answer it would inherit is a truncation, which
/// is a leak when the statement carries an expression and a double free when
/// it does not.
#[test]
fn the_transfer_temp_classifier_has_no_wildcard_arm() {
    let body = classifier_body();
    assert!(
        !body.contains("_ =>"),
        "`transfer_temp_disposition` grew a wildcard arm. Every `NirOp` must \
         pick a `TransferTemps` class explicitly; the exhaustive match is the \
         assertion that it did. Body:\n{body}"
    );
}

/// And the classifier must actually enumerate the whole `NirOp` vocabulary —
/// a variant reachable only through a nested `|` group that someone deleted,
/// or a rename that left the arm matching nothing, would compile but stop
/// being a decision.
#[test]
fn every_nir_op_is_classified_by_name() {
    let body = classifier_body();
    let variants = nir_op_variants();
    assert!(
        variants.len() >= 15,
        "the NirOp scrape found only {} variants, so the parse is wrong \
         before the assertion below means anything: {variants:?}",
        variants.len()
    );
    let missing: Vec<&String> = variants
        .iter()
        .filter(|v| !body.contains(&format!("NirOp::{v} ")))
        .collect();
    assert!(
        missing.is_empty(),
        "these `NirOp` variants are not named in `transfer_temp_disposition`: \
         {missing:?}. Each one owes an answer to a single question — what \
         happens to the pending temporaries the statement registered — and \
         the four non-`StatementScope` answers are not interchangeable: \
         `FreedBeforeBranch` frees them (a leak if wrong), \
         `AdoptedByTheCatcher` must NOT (a double free if wrong)"
    );
}

/// The classification itself, pinned so a reshuffle is a deliberate act. Each
/// line is a claim about ownership that was measured, not assumed.
#[test]
fn the_control_transfer_classes_are_the_measured_ones() {
    use crate::codegen::engine::control::TransferTemps::*;
    let body = classifier_body();
    for (arm, class) in [
        (
            "NirOp::Return { .. } => TransferTemps::FreedBeforeBranch",
            FreedBeforeBranch,
        ),
        (
            "NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => TransferTemps::NoExpression",
            NoExpression,
        ),
        (
            "NirOp::Fail { .. } => TransferTemps::AdoptedByTheCatcher",
            AdoptedByTheCatcher,
        ),
        (
            "NirOp::ExitProgram { .. } => TransferTemps::ProcessTerminates",
            ProcessTerminates,
        ),
    ] {
        assert!(
            body.contains(arm),
            "the `{class:?}` classification changed. `{arm}` is no longer in \
             `transfer_temp_disposition`. Reclassifying is allowed, but each \
             direction has a measured consequence: moving a statement OFF \
             `FreedBeforeBranch` restores bug-567's 64 B per call, and moving \
             `Fail` ONTO it frees the `Error` block the catcher adopts"
        );
    }
}
