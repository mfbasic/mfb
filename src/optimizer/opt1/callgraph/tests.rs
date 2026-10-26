use super::super::local_rewrites::testutil::*;
use super::*;
use crate::optimizer::{with_opt_level, OptLevel};
use crate::target::shared::nir::{
    global_initializer_name, NirEntryPoint, NirFunction, NirGlobal, NirOp, NirParam, NirValue,
};
use crate::types::ParameterType;
use std::collections::HashMap;

fn function(name: &str, body: Vec<NirOp>) -> NirFunction {
    NirFunction {
        name: name.to_string(),
        visibility: "private".to_string(),
        kind: "func".to_string(),
        isolated: false,
        params: vec![],
        returns: ParameterType::Integer,
        body,
        file: "main.mfb".to_string(),
        resource_owners: HashMap::new(),
    }
}

fn calls(target: &str) -> NirOp {
    NirOp::Eval {
        value: NirValue::Call {
            target: target.to_string(),
            args: vec![],
            loc: Default::default(),
        },
    }
}

fn function_ref(name: &str) -> NirValue {
    NirValue::FunctionRef {
        name: name.to_string(),
        type_: ParameterType::Unknown,
    }
}

/// A module whose entry is `main`.
fn program(functions: Vec<NirFunction>) -> NirModule {
    let mut module = test_module(functions);
    module.entry = Some(NirEntryPoint {
        name: "main".to_string(),
        returns: ParameterType::Integer,
        accepts_args: false,
    });
    module
}

fn run(module: &mut NirModule, level: u8) {
    with_opt_level(OptLevel(level), || eliminate(module));
}

fn names(module: &NirModule) -> Vec<&str> {
    module
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect()
}

/// The motivating program: `neverCalled` and the `orphanA`/`orphanB` cycle are
/// unreachable from `main`; the transitive callee survives, and the survivors
/// keep their original order.
#[test]
fn removes_uncalled_functions_and_unreachable_cycles() {
    let mut module = program(vec![
        function("helper", vec![calls("leaf")]),
        function("neverCalled", vec![calls("helper")]),
        function("orphanA", vec![calls("orphanB")]),
        function("orphanB", vec![calls("orphanA")]),
        function("leaf", vec![]),
        function("main", vec![calls("helper"), calls("io.print")]),
    ]);
    run(&mut module, 1);
    assert_eq!(names(&module), vec!["helper", "leaf", "main"]);
}

/// A function handed out as a value (a callback, a lambda) is kept, and so is
/// what it reaches; a value mention inside a removed function keeps nothing.
#[test]
fn a_function_value_from_live_code_keeps_its_function() {
    let mut module = program(vec![
        function(
            "main",
            vec![
                NirOp::Eval {
                    value: NirValue::Call {
                        target: "collections.forEach".to_string(),
                        args: vec![function_ref("callback")],
                        loc: Default::default(),
                    },
                },
                NirOp::Eval {
                    value: NirValue::Closure {
                        name: "$lambda0".to_string(),
                        type_: ParameterType::Unknown,
                        captures: vec![],
                    },
                },
            ],
        ),
        function("callback", vec![calls("fromCallback")]),
        function("fromCallback", vec![]),
        function("$lambda0", vec![]),
        function(
            "dead",
            vec![NirOp::Eval {
                value: function_ref("onlyDeadMentionsMe"),
            }],
        ),
        function("onlyDeadMentionsMe", vec![]),
    ]);
    run(&mut module, 1);
    assert_eq!(
        names(&module),
        vec!["main", "callback", "fromCallback", "$lambda0"]
    );
}

/// The global initializer is a root (it runs at startup), so is everything it
/// reaches, and so is a function a global's initializer value names.
#[test]
fn the_initializer_and_global_values_are_roots() {
    let initializer = global_initializer_name("test");
    let mut module = program(vec![
        function(&initializer, vec![calls("initHelper")]),
        function("initHelper", vec![]),
        function("heldByGlobal", vec![]),
        function("main", vec![]),
        function("dead", vec![]),
    ]);
    module.globals = vec![NirGlobal {
        name: "handler".to_string(),
        symbol: "_g_handler".to_string(),
        visibility: "private".to_string(),
        mutable: false,
        type_: ParameterType::Unknown,
        value: Some(function_ref("heldByGlobal")),
    }];
    run(&mut module, 1);
    assert_eq!(
        names(&module),
        vec![initializer.as_str(), "initHelper", "heldByGlobal", "main"]
    );
}

/// Helpers codegen enters by symbol have no NIR caller and are kept anyway,
/// with what they reach: a natively-called builtin helper, and the whole
/// `canvas` package half (the render loop and the functions the app backends
/// gate on), including a merged package's identity-prefixed copy.
#[test]
fn by_symbol_callees_are_roots() {
    let native = crate::codegen::registry::natively_called_helpers()
        .into_iter()
        .next()
        .expect("the registry declares natively-called helpers");
    let render_loop = crate::internal_name::internalize("__canvas_renderLoop");
    let canvas_unused = crate::internal_name::internalize("__canvas_neverCalled");
    let prefixed = format!(
        "0123456789abcdef.pkg.{}",
        crate::internal_name::internalize("__canvas_copy")
    );
    let mut module = program(vec![
        function(&native, vec![calls("nativeCore")]),
        function("nativeCore", vec![]),
        function(&render_loop, vec![calls("fromCanvas")]),
        function(&canvas_unused, vec![]),
        function(&prefixed, vec![]),
        function("fromCanvas", vec![]),
        function("main", vec![]),
        function("dead", vec![]),
        function("#vector_unused", vec![]),
    ]);
    run(&mut module, 1);
    assert_eq!(
        names(&module),
        vec![
            native.as_str(),
            "nativeCore",
            render_loop.as_str(),
            canvas_unused.as_str(),
            prefixed.as_str(),
            "fromCanvas",
            "main",
        ]
    );
}

/// A reached function's parameter default names its hidden default function
/// (plan-136-A); the NIR validator resolves that name, so it must survive even
/// when no call site omits the argument.
#[test]
fn a_parameter_default_keeps_the_function_it_names() {
    let mut withdefault = function("withDefault", vec![]);
    withdefault.params = vec![NirParam {
        name: "x".to_string(),
        type_: ParameterType::Integer,
        default: Some(NirValue::Call {
            target: "$default$withDefault$0".to_string(),
            args: vec![],
            loc: Default::default(),
        }),
    }];
    let mut module = program(vec![
        function("main", vec![calls("withDefault")]),
        withdefault,
        function("$default$withDefault$0", vec![]),
    ]);
    run(&mut module, 1);
    assert_eq!(
        names(&module),
        vec!["main", "withDefault", "$default$withDefault$0"]
    );
}

/// `-O0` keeps every function (the dial contract), and a module without an
/// entry function has no root to reason from, so it is left alone too.
#[test]
fn level_zero_and_entryless_modules_keep_everything() {
    let mut module = program(vec![function("main", vec![]), function("dead", vec![])]);
    run(&mut module, 0);
    assert_eq!(names(&module), vec!["main", "dead"]);

    let mut entryless = test_module(vec![function("main", vec![]), function("dead", vec![])]);
    run(&mut entryless, 1);
    assert_eq!(names(&entryless), vec!["main", "dead"]);

    // An entry naming no function is the same: no root, so nothing is removed
    // (`validate_nir` then reports the missing entry).
    let mut missing = program(vec![function("helper", vec![]), function("dead", vec![])]);
    run(&mut missing, 1);
    assert_eq!(names(&missing), vec!["helper", "dead"]);
}
