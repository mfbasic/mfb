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

/// bug-747: a function the global initializer can reach may run before a
/// global's own store, when the slot still reads zero — so it keeps its
/// read (and so does the initializer function itself), while a function that
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
        "the initializer function's own read runs before `limit` is stored"
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

/// plan-161-A: a `LET` (immutable) with a literal of its own scalar type is
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

/// bug-747's guard applies to this row too: a function the initializer
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
