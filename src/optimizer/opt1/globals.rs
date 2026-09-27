//! The Opt1 global rows (`planning/optimizations.md`), all reading one
//! whole-module census ([`super::plans::globals`]). One is Level 1 — **literal
//! `LET` folding** ([`fold_literal_lets`], plan-158-A), which needs no privacy
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
    // binding is stored once by the synthetic `__mfb_init_globals_*` SUB, so a
    // removed global leaves that SUB storing to a name that no longer exists —
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
/// initializer (`plans::globals::initializer_reachable`, bug-744). Those
/// functions — and the initializer SUB itself — keep their reads.
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
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};
    use crate::target::shared::nir::{NirFunction, NirGlobal};
    use crate::types::ParameterType;
    use std::collections::HashMap;

    fn global(name: &str, mutable: bool, visibility: &str, value: Option<NirValue>) -> NirGlobal {
        NirGlobal {
            name: name.to_string(),
            symbol: format!("_g_{name}"),
            visibility: visibility.to_string(),
            mutable,
            type_: ParameterType::Integer,
            value,
        }
    }

    fn function(body: Vec<NirOp>) -> NirFunction {
        NirFunction {
            name: "f".to_string(),
            visibility: "private".to_string(),
            kind: "function".to_string(),
            isolated: false,
            params: vec![],
            returns: ParameterType::Integer,
            body,
            file: "main.mfb".to_string(),
            resource_owners: HashMap::new(),
        }
    }

    fn read(name: &str) -> NirValue {
        NirValue::Global {
            name: name.to_string(),
            type_: ParameterType::Integer,
        }
    }

    fn run(module: &mut NirModule, level: u8) {
        with_opt_level(OptLevel(level), || simplify(module));
    }

    /// A never-written private global with a literal initializer: its reads
    /// become the literal, it is marked read-only, and — now unmentioned — it
    /// is collected, all in one pass.
    #[test]
    fn constant_globals_inline_and_then_die() {
        let mut module = test_module(vec![function(vec![NirOp::Return {
            value: Some(read("limit")),
        }])]);
        module.globals = vec![global("limit", true, "private", Some(int_const("42")))];
        run(&mut module, 2);

        let NirOp::Return { value: Some(value) } = &module.functions[0].body[0] else {
            panic!("expected the return");
        };
        assert!(
            matches!(value, NirValue::Const { value, .. } if value == "42"),
            "the read became the literal"
        );
        assert!(module.globals.is_empty(), "the global is now unmentioned");
    }

    /// A written global keeps its storage, its reads, and its mutability.
    #[test]
    fn written_globals_are_untouched() {
        let mut module = test_module(vec![function(vec![
            NirOp::StoreGlobal {
                name: "counter".to_string(),
                type_: ParameterType::Integer,
                value: Some(int_const("1")),
            },
            NirOp::Return {
                value: Some(read("counter")),
            },
        ])]);
        module.globals = vec![global("counter", true, "private", Some(int_const("0")))];
        run(&mut module, 2);

        assert_eq!(module.globals.len(), 1);
        assert!(module.globals[0].mutable, "still written, still mutable");
        let NirOp::Return { value: Some(value) } = &module.functions[0].body[1] else {
            panic!("expected the return");
        };
        assert!(
            matches!(value, NirValue::Global { .. }),
            "read stays a read"
        );
    }

    /// An exported global may be read or written by an importer: no row may
    /// touch it, however unused it looks from here.
    #[test]
    fn exported_globals_are_never_touched() {
        let mut module = test_module(vec![function(vec![])]);
        module.globals = vec![global("shared", true, "public", Some(int_const("7")))];
        run(&mut module, 2);
        assert_eq!(module.globals.len(), 1);
        assert!(module.globals[0].mutable);
    }

    /// A never-written global whose initializer is *not* a literal keeps its
    /// storage (duplicating a call or allocation into every read would change
    /// how often it happens) but is still inferred read-only.
    #[test]
    fn non_literal_initializers_are_read_only_but_not_inlined() {
        let call = NirValue::Call {
            target: "compute".to_string(),
            args: vec![],
            loc: Default::default(),
        };
        let mut module = test_module(vec![function(vec![NirOp::Return {
            value: Some(read("cached")),
        }])]);
        module.globals = vec![global("cached", true, "private", Some(call))];
        run(&mut module, 2);

        assert_eq!(module.globals.len(), 1, "storage stays");
        assert!(!module.globals[0].mutable, "but it is provably read-only");
        let NirOp::Return { value: Some(value) } = &module.functions[0].body[0] else {
            panic!("expected the return");
        };
        assert!(matches!(value, NirValue::Global { .. }));
    }

    /// A global read only from another global's initializer is still live.
    #[test]
    fn initializer_reads_keep_a_global_alive() {
        let mut module = test_module(vec![function(vec![])]);
        module.globals = vec![
            global("seed", true, "private", Some(int_const("3"))),
            global("derived", true, "private", Some(read("seed"))),
        ];
        run(&mut module, 2);
        assert!(
            module.globals.iter().any(|g| g.name == "seed"),
            "still named by derived's initializer"
        );
    }

    /// bug-744: a function the global initializer can reach may run before a
    /// global's own store, when the slot still reads zero — so it keeps its
    /// read (and so does the initializer SUB itself), while a function that
    /// only runs after initialization gets the literal.
    #[test]
    fn reads_that_can_run_during_initialization_are_kept() {
        let initializer = crate::target::shared::nir::global_initializer_name("test");
        let mut early = function(vec![NirOp::Return {
            value: Some(read("limit")),
        }]);
        early.name = "early".to_string();
        let mut init = function(vec![
            NirOp::StoreGlobal {
                name: "first".to_string(),
                type_: ParameterType::Integer,
                value: Some(NirValue::Binary {
                    op: crate::operators::BinaryOp::Add,
                    left: Box::new(read("limit")),
                    right: Box::new(NirValue::Call {
                        target: "early".to_string(),
                        args: vec![],
                        loc: Default::default(),
                    }),
                    loc: Default::default(),
                }),
            },
            NirOp::StoreGlobal {
                name: "limit".to_string(),
                type_: ParameterType::Integer,
                value: Some(int_const("42")),
            },
        ]);
        init.name = initializer;
        let mut late = function(vec![NirOp::Return {
            value: Some(read("limit")),
        }]);
        late.name = "late".to_string();
        let mut module = test_module(vec![init, early, late]);
        // `first` is public so the dead-global row keeps it (and the store
        // this test inspects); only `limit` is a constification candidate.
        module.globals = vec![
            global("first", true, "public", None),
            global("limit", true, "private", Some(int_const("42"))),
        ];
        run(&mut module, 2);

        let returned = |index: usize| match &module.functions[index].body[0] {
            NirOp::Return { value: Some(value) } => value.clone(),
            _ => panic!("expected the return"),
        };
        assert!(
            matches!(returned(1), NirValue::Global { .. }),
            "`early` runs inside the initializer: its read stays a read"
        );
        assert!(
            matches!(returned(2), NirValue::Const { value, .. } if value == "42"),
            "`late` runs after initialization: its read is the literal"
        );
        let NirOp::StoreGlobal {
            value: Some(NirValue::Binary { left, .. }),
            ..
        } = &module.functions[0].body[0]
        else {
            panic!("expected the initializer's first store");
        };
        assert!(
            matches!(**left, NirValue::Global { .. }),
            "the initializer SUB's own read runs before `limit` is stored"
        );
        assert_eq!(module.globals.len(), 2, "`limit` is still read");
    }

    fn fold(module: &mut NirModule, level: u8) {
        with_opt_level(OptLevel(level), || fold_literal_lets(module));
    }

    fn returned(module: &NirModule, function: usize) -> NirValue {
        match &module.functions[function].body[0] {
            NirOp::Return { value: Some(value) } => value.clone(),
            _ => panic!("expected the return"),
        }
    }

    fn typed_const(type_: ParameterType, value: &str) -> NirValue {
        NirValue::Const {
            type_,
            value: value.to_string(),
        }
    }

    /// plan-158-A: a `LET` (immutable) with a literal of its own scalar type is
    /// folded at `-O1` whatever its visibility — an importer cannot write a
    /// `LET` either — and its storage stays for anything that still names it.
    #[test]
    fn literal_lets_fold_at_level_one_regardless_of_visibility() {
        for (type_, value) in [
            (ParameterType::Integer, "29"),
            (ParameterType::Byte, "7"),
            (ParameterType::Boolean, "true"),
            (ParameterType::Float, "1.5"),
            (ParameterType::Fixed, "2.25"),
            (ParameterType::Money, "3.10"),
        ] {
            let mut module = test_module(vec![function(vec![NirOp::Return {
                value: Some(read("rows")),
            }])]);
            let mut rows = global(
                "rows",
                false,
                "public",
                Some(typed_const(type_.clone(), value)),
            );
            rows.type_ = type_.clone();
            module.globals = vec![rows];
            fold(&mut module, 1);
            assert!(
                matches!(returned(&module, 0), NirValue::Const { value: v, type_: t } if v == value && t == type_),
                "{type_:?} LET folds to its literal"
            );
            assert_eq!(module.globals.len(), 1, "the storage stays");
        }
    }

    /// Everything that is not "an immutable binding holding a literal of its
    /// own word-sized scalar type" keeps its read.
    #[test]
    fn only_literal_word_scalar_lets_fold() {
        let call = NirValue::Call {
            target: "compute".to_string(),
            args: vec![],
            loc: Default::default(),
        };
        let cases: Vec<(&str, NirGlobal)> = vec![
            ("a MUT", global("g", true, "private", Some(int_const("1")))),
            (
                "a computed initializer",
                global("g", false, "public", Some(call)),
            ),
            ("no initializer", global("g", false, "public", None)),
            ("a literal of another type (a coercion)", {
                let mut g = global("g", false, "public", Some(int_const("1")));
                g.type_ = ParameterType::Float;
                g
            }),
            ("a String literal", {
                let mut g = global(
                    "g",
                    false,
                    "public",
                    Some(typed_const(ParameterType::String, "hi")),
                );
                g.type_ = ParameterType::String;
                g
            }),
        ];
        for (what, global) in cases {
            let mut module = test_module(vec![function(vec![NirOp::Return {
                value: Some(read("g")),
            }])]);
            module.globals = vec![global];
            fold(&mut module, 1);
            assert!(
                matches!(returned(&module, 0), NirValue::Global { .. }),
                "{what} is not folded"
            );
        }
    }

    /// A `LET` some function stores to (which the resolver never produces) is
    /// not trusted to hold its initializer: the structural check backs up the
    /// declaration.
    #[test]
    fn a_stored_let_is_not_folded() {
        let mut module = test_module(vec![function(vec![
            NirOp::StoreGlobal {
                name: "g".to_string(),
                type_: ParameterType::Integer,
                value: Some(int_const("2")),
            },
            NirOp::Return {
                value: Some(read("g")),
            },
        ])]);
        module.globals = vec![global("g", false, "public", Some(int_const("1")))];
        fold(&mut module, 1);
        let NirOp::Return { value: Some(value) } = &module.functions[0].body[1] else {
            panic!("expected the return");
        };
        assert!(matches!(value, NirValue::Global { .. }));
    }

    /// bug-744's guard applies to this row too: a function the initializer
    /// reaches keeps its read.
    #[test]
    fn literal_let_folding_keeps_reads_that_can_run_during_initialization() {
        let initializer = crate::target::shared::nir::global_initializer_name("test");
        let mut init = function(vec![NirOp::Eval {
            value: NirValue::Call {
                target: "early".to_string(),
                args: vec![],
                loc: Default::default(),
            },
        }]);
        init.name = initializer;
        let mut early = function(vec![NirOp::Return {
            value: Some(read("g")),
        }]);
        early.name = "early".to_string();
        let mut late = function(vec![NirOp::Return {
            value: Some(read("g")),
        }]);
        late.name = "late".to_string();
        let mut module = test_module(vec![init, early, late]);
        module.globals = vec![global("g", false, "public", Some(int_const("5")))];
        fold(&mut module, 1);
        assert!(matches!(returned(&module, 1), NirValue::Global { .. }));
        assert!(matches!(returned(&module, 2), NirValue::Const { .. }));
    }

    /// Off at `-O0`.
    #[test]
    fn literal_let_folding_is_off_at_level_zero() {
        let mut module = test_module(vec![function(vec![NirOp::Return {
            value: Some(read("g")),
        }])]);
        module.globals = vec![global("g", false, "public", Some(int_const("5")))];
        fold(&mut module, 0);
        assert!(matches!(returned(&module, 0), NirValue::Global { .. }));
    }

    /// The rows are off at `-O1`.
    #[test]
    fn level_one_disables_the_rows() {
        let mut module = test_module(vec![function(vec![])]);
        module.globals = vec![global("unused", true, "private", Some(int_const("1")))];
        run(&mut module, 1);
        assert_eq!(module.globals.len(), 1);
        assert!(module.globals[0].mutable);
    }
}
