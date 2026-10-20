//! The Opt1 global rows (`planning/optimizations.md`), all reading one
//! whole-module census ([`super::plans::globals`]). One is Level 1 — **literal
//! `LET` folding** ([`fold_literal_lets`], plan-161-A), which needs no privacy
//! proof because nothing can write a `LET` — and three are Level 2:
//!
//! - **Dead global elimination** — a private global no function and no other
//!   global's initializer ever mentions is removed outright. Nothing can
//!   observe storage that nothing names.
//! - **Global localization / constification** — a private global that is
//!   never written keeps its initializer as its only value, so every read is
//!   that constant. When the initializer is a literal the reads are replaced
//!   by it directly, which turns a memory load into an immediate and lets the
//!   constant-folding and propagation rows see through what used to be an
//!   opaque global.
//! - **Read-only memory inference** — the same never-written proof, recorded
//!   on the global by clearing its `mutable` flag, so storage planning may
//!   place it in the read-only data partition. This is the row's inference
//!   half; the placement itself is Plan1's existing `kind`-based split
//!   (`types.rs` puts `constant` objects in the rodata prefix).
//!
//! What makes all three safe is the same pair of facts, and both are checked
//! per global, never assumed: the global must be **private** (an exported one
//! may be read or written by an importer this module cannot see), and the
//! census counts occurrences by NIR *variant* — `NirValue::Global` and
//! `NirOp::StoreGlobal` — so a local that happens to share a name inflates
//! nothing. A global whose initializer is absent is left alone by the
//! constification half: there is no value to substitute.

use crate::target::shared::nir::{NirModule, NirOp, NirValue};

use super::plans::globals::{self, escapes, Census};

/// Apply the three global rows to the whole module. Each self-guards on its
/// catalog level (2); the counts feed `optimizer::stats`.
pub(crate) fn simplify(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let census = globals::census(module);

    // Read-only inference and constification first: both read the census,
    // and dropping dead globals afterwards cannot invalidate either verdict
    // (removing storage nothing names changes no other global's usage).
    let mut read_only = 0;
    for global in &mut module.globals {
        if escapes(global) || !global.mutable {
            continue;
        }
        let never_written = census
            .get(&global.name)
            .is_none_or(|usage| usage.never_written());
        if never_written {
            // The initializer is the only value this storage ever holds.
            global.mutable = false;
            read_only += 1;
        }
    }
    crate::optimizer::stats::count_globals_read_only(read_only);

    let substitutions = constify(module, &census);
    crate::optimizer::stats::count_globals_localized(substitutions);

    // Dead globals last, so a global whose only reads were just replaced by
    // its own literal is now unmentioned and collectible in this same pass.
    let after = globals::census(module);
    let before = module.globals.len();
    let mut removed: std::collections::HashSet<String> = std::collections::HashSet::new();
    module.globals.retain(|global| {
        let keep = escapes(global)
            || !after
                .get(&global.name)
                .cloned()
                .unwrap_or_default()
                .untouched();
        if !keep {
            removed.insert(global.name.clone());
        }
        keep
    });
    // bug-552: removing the global is only half of it. Every module-level
    // binding is stored once by the synthetic `__mfb_init_globals_*` function, so a
    // removed global leaves that function storing to a name that no longer exists —
    // which `target/shared/validate/body.rs` refuses outright ("NIR global
    // store targets unknown global"). The deadness of this row was hiding that;
    // the store has to go in the same change.
    //
    // Only the initializer's own stores are dropped. A store from anywhere else
    // would have made the global non-`untouched` and kept it alive, so there is
    // no live write to lose here.
    if !removed.is_empty() {
        let initializer = crate::target::shared::nir::global_initializer_name(&module.project);
        for function in &mut module.functions {
            if function.name != initializer {
                continue;
            }
            function.body.retain(|op| match op {
                NirOp::StoreGlobal { name, .. } => !removed.contains(name),
                _ => true,
            });
        }
    }
    crate::optimizer::stats::count_globals_eliminated((before - module.globals.len()) as u64);
}

/// Replace reads of never-written private globals whose initializer is a
/// literal with that literal.
fn constify(module: &mut NirModule, census: &Census) -> u64 {
    let constants: std::collections::HashMap<String, NirValue> = module
        .globals
        .iter()
        .filter(|global| !escapes(global))
        .filter(|global| {
            census
                .get(&global.name)
                .is_none_or(|usage| usage.never_written())
        })
        .filter_map(|global| match &global.value {
            // Only a literal substitutes: any other initializer may allocate,
            // call, or read another global, and duplicating it into every use
            // would change how often that happens.
            Some(value @ NirValue::Const { .. }) => Some((global.name.clone(), value.clone())),
            _ => None,
        })
        .collect();
    substitute_outside_initialization(module, &constants)
}

/// Replace every read of a global in `constants` with its literal, in every
/// function that cannot run during global initialization.
///
/// A function the initializer can reach may read a global before that
/// global's own store has run, and such a read observes zero, not the
/// initializer (`plans::globals::initializer_reachable`, bug-747). Those
/// functions — and the initializer function itself — keep their reads.
fn substitute_outside_initialization(
    module: &mut NirModule,
    constants: &std::collections::HashMap<String, NirValue>,
) -> u64 {
    if constants.is_empty() {
        return 0;
    }
    let initializing = globals::initializer_reachable(module);
    let mut substituted = 0;
    for function in &mut module.functions {
        if initializing.contains(&function.name) {
            continue;
        }
        for op in &mut function.body {
            substitute_op(op, constants, &mut substituted);
        }
    }
    substituted
}

/// The **Literal `LET` folding** row (Level 1): replace each read of a
/// module-level `LET` whose initializer is a scalar literal with that literal.
///
/// A `LET` binding can never be assigned — the language rejects it — so once
/// its initializer has run, every read yields the initializer's value. Three
/// checks make the rewrite exact rather than merely plausible, each per
/// global:
///
/// - **Immutable by declaration** (`!mutable`), and — as a second, structural
///   proof — no `StoreGlobal` of it anywhere but its own initializer store
///   (`GlobalUse::never_written`). Visibility does not matter: an importer
///   cannot write a `LET` either, so unlike the Level-2 constification row this
///   one needs no privacy proof.
/// - **A literal of the global's own scalar type**: a `Const` whose type is
///   the declared type, and that type one whose value is a single machine word
///   with no storage of its own (`Integer`, `Byte`, `Boolean`, `Float`,
///   `Fixed`, `Money`). Evaluating that `Const` at the read is what the
///   initializer's store evaluated, so the read and the literal agree bit for
///   bit; a computed initializer (a call, an arithmetic expression that could
///   trap, a string or collection that owns storage) is never folded.
/// - **Only where the initializer has already run**: a function the global
///   initializer can reach keeps its reads (see
///   [`substitute_outside_initialization`]).
pub(crate) fn fold_literal_lets(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(1) {
        return;
    }
    let census = globals::census(module);
    let constants: std::collections::HashMap<String, NirValue> = module
        .globals
        .iter()
        .filter(|global| !global.mutable)
        .filter(|global| {
            census
                .get(&global.name)
                .is_none_or(|usage| usage.never_written())
        })
        .filter_map(|global| match &global.value {
            Some(value @ NirValue::Const { type_, .. })
                if *type_ == global.type_ && is_word_scalar(type_) =>
            {
                Some((global.name.clone(), value.clone()))
            }
            _ => None,
        })
        .collect();
    let folded = substitute_outside_initialization(module, &constants);
    crate::optimizer::stats::count_lets_folded(folded);
}

/// A type whose value is one machine word with no storage of its own, so a
/// literal of it can stand anywhere a read of it did.
fn is_word_scalar(type_: &crate::types::ParameterType) -> bool {
    use crate::types::ParameterType;
    matches!(
        type_,
        ParameterType::Integer
            | ParameterType::Byte
            | ParameterType::Boolean
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money
    )
}

fn substitute_op(
    op: &mut NirOp,
    constants: &std::collections::HashMap<String, NirValue>,
    substituted: &mut u64,
) {
    let mut visit = |value: &mut NirValue| substitute_value(value, constants, substituted);
    match op {
        NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } => {
            if let Some(value) = value {
                visit(value);
            }
        }
        NirOp::Assign { value, .. } | NirOp::StateAssign { value, .. } => visit(value),
        NirOp::Return { value } => {
            if let Some(value) = value {
                visit(value);
            }
        }
        NirOp::ExitProgram { code } => visit(code),
        NirOp::Fail { error } => visit(error),
        NirOp::Eval { value } => visit(value),
        NirOp::If {
            condition,
            then_body,
            else_body,
        } => {
            visit(condition);
            for op in then_body.iter_mut().chain(else_body.iter_mut()) {
                substitute_op(op, constants, substituted);
            }
        }
        NirOp::Match { value, cases } => {
            visit(value);
            for case in cases {
                if let Some(guard) = &mut case.guard {
                    substitute_value(guard, constants, substituted);
                }
                for op in &mut case.body {
                    substitute_op(op, constants, substituted);
                }
            }
        }
        NirOp::While {
            condition, body, ..
        } => {
            visit(condition);
            for op in body {
                substitute_op(op, constants, substituted);
            }
        }
        NirOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            visit(start);
            visit(end);
            visit(step);
            for op in body {
                substitute_op(op, constants, substituted);
            }
        }
        NirOp::DoUntil { body, condition } => {
            for op in body.iter_mut() {
                substitute_op(op, constants, substituted);
            }
            substitute_value(condition, constants, substituted);
        }
        NirOp::ForEach { iterable, body, .. } => {
            visit(iterable);
            for op in body {
                substitute_op(op, constants, substituted);
            }
        }
        NirOp::Trap { body, .. } => {
            for op in body {
                substitute_op(op, constants, substituted);
            }
        }
        NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
    }
}

fn substitute_value(
    value: &mut NirValue,
    constants: &std::collections::HashMap<String, NirValue>,
    substituted: &mut u64,
) {
    if let NirValue::Global { name, .. } = value {
        if let Some(constant) = constants.get(name) {
            *value = constant.clone();
            *substituted += 1;
            return;
        }
    }
    match value {
        NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => {}
        NirValue::Closure { captures, .. } => {
            for capture in captures {
                substitute_value(capture, constants, substituted);
            }
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                substitute_value(arg, constants, substituted);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::Checked { value, .. } => substitute_value(value, constants, substituted),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            substitute_value(target, constants, substituted);
            for update in updates {
                substitute_value(&mut update.value, constants, substituted);
            }
        }
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
            for value in values {
                substitute_value(value, constants, substituted);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                substitute_value(key, constants, substituted);
                substitute_value(value, constants, substituted);
            }
        }
        NirValue::MemberAccess { target, .. } => substitute_value(target, constants, substituted),
        NirValue::Binary { left, right, .. } => {
            substitute_value(left, constants, substituted);
            substitute_value(right, constants, substituted);
        }
        NirValue::Unary { operand, .. } => substitute_value(operand, constants, substituted),
    }
}

#[cfg(test)]
mod tests;
