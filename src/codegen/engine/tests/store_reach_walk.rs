//! The store-reachability walk (`engine/value/store_reach.rs`) asked directly,
//! with a builder whose function table is a real module's.
//!
//! Its three guards (G25's `STATE` update, bug-665's global argument copy,
//! bug-666's `FOR EACH` over a global) each turn a false "cannot reach" into a
//! use-after-free, so the walk must answer "reaches" for every call whose body it
//! cannot see, and may answer "cannot reach" only for what it has proved. Each row
//! below is one of those two answers for one shape of call or argument.

use crate::codegen::engine::builder::{GlobalValue, TypeModel};
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::codegen::engine::value::store_reach::{global_root, local_root, StoreLeaf};
use crate::operators::BinaryOp;
use crate::target::shared::nir::{NirModule, NirOp, NirSourceLoc, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const SOURCE: &str = "\
IMPORT collections
IMPORT io

MUT counter AS Integer = 0
MUT other AS Integer = 0

FUNC bump() AS Integer
  counter = counter + 1
  RETURN counter
END FUNC

FUNC viaBump() AS Integer
  RETURN bump() + 1
END FUNC

FUNC ping(n AS Integer) AS Integer
  IF n > 0 THEN
    RETURN pong(n - 1)
  END IF
  RETURN 0
END FUNC

FUNC pong(n AS Integer) AS Integer
  RETURN ping(n)
END FUNC

FUNC touchOther(n AS Integer) AS Integer
  other = n
  RETURN n
END FUNC

FUNC double(n AS Integer) AS Integer
  RETURN n * 2
END FUNC

FUNC main() AS Integer
  LET xs AS List OF Integer = collections::transform([1, 2], double)
  io::print(toString(viaBump() + ping(2) + touchOther(1) + len(xs)))
  RETURN 0
END FUNC
";

fn module() -> NirModule {
    nir_for_src(SOURCE, CodeTarget::LinuxX86_64, Console)
        .unwrap_or_else(|error| panic!("the probe lowers to NIR: {error}"))
}

fn integer(value: &str) -> NirValue {
    NirValue::Const {
        type_: ParameterType::Integer,
        value: value.to_string(),
    }
}

fn call(target: &str, args: Vec<NirValue>) -> NirValue {
    NirValue::Call {
        target: target.to_string(),
        args,
        loc: NirSourceLoc::default(),
    }
}

fn func_type() -> ParameterType {
    ParameterType::Func(
        vec![ParameterType::Integer],
        Box::new(ParameterType::Integer),
        false,
    )
}

fn list() -> NirValue {
    NirValue::Local("xs".to_string())
}

/// `(case, call, whether it reaches a store of `counter`)`.
fn rows() -> Vec<(&'static str, NirValue, bool)> {
    vec![
        (
            "a module function that stores the global",
            call("bump", vec![]),
            true,
        ),
        (
            "a module function calling one that does",
            call("viaBump", vec![]),
            true,
        ),
        (
            "mutual recursion that stores nothing",
            call("ping", vec![integer("3")]),
            false,
        ),
        (
            "a function storing a different global",
            call("touchOther", vec![integer("1")]),
            false,
        ),
        (
            "a name nothing resolves",
            call("noSuchFunction", vec![]),
            true,
        ),
        (
            "a general builtin takes no callback",
            call("len", vec![list()]),
            false,
        ),
        (
            "a callback naming a module function that stores",
            call(
                "collections.transform",
                vec![
                    list(),
                    NirValue::FunctionRef {
                        name: "bump".to_string(),
                        type_: func_type(),
                    },
                ],
            ),
            true,
        ),
        (
            "a callback naming a builtin runs no user code",
            call(
                "collections.transform",
                vec![
                    list(),
                    NirValue::FunctionRef {
                        name: "toString".to_string(),
                        type_: func_type(),
                    },
                ],
            ),
            false,
        ),
        (
            "a callback naming something unresolved",
            call(
                "collections.transform",
                vec![
                    list(),
                    NirValue::FunctionRef {
                        name: "elsewhere".to_string(),
                        type_: func_type(),
                    },
                ],
            ),
            true,
        ),
        (
            "a FUNC binding in the callback position is opaque",
            call(
                "collections.transform",
                vec![list(), NirValue::Local("f".to_string())],
            ),
            true,
        ),
        (
            "a captured FUNC in the callback position is opaque",
            call(
                "collections.transform",
                vec![
                    list(),
                    NirValue::Capture {
                        index: 0,
                        type_: func_type(),
                        by_ref: false,
                    },
                ],
            ),
            true,
        ),
        (
            "a by-reference FUNC in the callback position is opaque",
            call(
                "collections.transform",
                vec![
                    list(),
                    NirValue::LocalRef {
                        name: "f".to_string(),
                        type_: func_type(),
                    },
                ],
            ),
            true,
        ),
        (
            "a constant in the callback position is no function",
            call("collections.transform", vec![list(), integer("1")]),
            false,
        ),
        (
            "an operator in the callback position is no function",
            call(
                "collections.transform",
                vec![
                    list(),
                    NirValue::Binary {
                        op: BinaryOp::Add,
                        left: Box::new(integer("1")),
                        right: Box::new(integer("2")),
                        loc: NirSourceLoc::default(),
                    },
                ],
            ),
            false,
        ),
        (
            "a declared Integer global in the callback position is no function",
            call(
                "collections.transform",
                vec![
                    list(),
                    NirValue::Global {
                        name: "counter".to_string(),
                        type_: ParameterType::named(""),
                    },
                ],
            ),
            false,
        ),
        (
            "an undeclared global in the callback position is opaque",
            call(
                "collections.transform",
                vec![
                    list(),
                    NirValue::Global {
                        name: "unknownGlobal".to_string(),
                        type_: ParameterType::named(""),
                    },
                ],
            ),
            true,
        ),
    ]
}

#[test]
fn a_call_reaches_a_global_store_unless_the_walk_proves_otherwise() {
    let module = module();
    let mut harness = BuilderHarness {
        type_model: TypeModel::from_module(&module).expect("the probe's type model builds"),
        functions: module
            .functions
            .iter()
            .map(|f| (f.name.clone(), f))
            .collect(),
        ..BuilderHarness::default()
    };
    harness.globals.insert(
        "counter".to_string(),
        GlobalValue {
            type_: ParameterType::Integer,
            offset: 0,
        },
    );
    let builder = harness.builder("probe", &TestPlatform);
    let leaf = StoreLeaf::Global("counter");
    for (case, value, expected) in rows() {
        assert_eq!(
            builder.values_reach_store(std::slice::from_ref(&value), leaf),
            expected,
            "{case}"
        );
        // The same answer through the two other entry points: the call alone, and
        // a statement evaluating it.
        if let NirValue::Call { target, args, .. } = &value {
            assert_eq!(
                builder.call_reaches_store(target, args, leaf),
                expected,
                "{case} (call_reaches_store)"
            );
        }
        let statement = NirOp::Eval {
            value: value.clone(),
        };
        assert_eq!(
            builder.ops_reach_store(std::slice::from_ref(&statement), leaf),
            expected,
            "{case} (ops_reach_store)"
        );
    }
}

/// A trapped call (`CallResult`) and a runtime-helper call are calls too: the
/// walk follows the first into its body exactly as a plain `Call`.
#[test]
fn a_trapped_call_is_followed_like_a_plain_one() {
    let module = module();
    let harness = BuilderHarness {
        functions: module
            .functions
            .iter()
            .map(|f| (f.name.clone(), f))
            .collect(),
        ..BuilderHarness::default()
    };
    let builder = harness.builder("probe", &TestPlatform);
    let leaf = StoreLeaf::Global("counter");
    let trapped = NirValue::CallResult {
        target: "viaBump".to_string(),
        args: Vec::new(),
        loc: NirSourceLoc::default(),
    };
    assert!(builder.values_reach_store(std::slice::from_ref(&trapped), leaf));
    let harmless = NirValue::CallResult {
        target: "ping".to_string(),
        args: vec![integer("1")],
        loc: NirSourceLoc::default(),
    };
    assert!(!builder.values_reach_store(std::slice::from_ref(&harmless), leaf));
    // A store found directly in the statements, with no call at all.
    let store = NirOp::StoreGlobal {
        name: "counter".to_string(),
        type_: ParameterType::Integer,
        value: Some(integer("2")),
    };
    assert!(builder.ops_reach_store(std::slice::from_ref(&store), leaf));
    assert!(!builder.ops_reach_store(std::slice::from_ref(&store), StoreLeaf::Global("other")));
    assert!(!builder.ops_reach_store(std::slice::from_ref(&store), StoreLeaf::StateAssign));
}

/// A value read out of a global (or a local) through a field, a union payload, or
/// either half of a `Result` points into that global's (local's) block; anything
/// computed does not.
#[test]
fn a_projection_keeps_its_root_and_a_computed_value_has_none() {
    let projections: Vec<fn(NirValue) -> NirValue> = vec![
        |v| NirValue::MemberAccess {
            target: Box::new(v),
            member: "field".to_string(),
        },
        |v| NirValue::UnionExtract {
            type_: ParameterType::named("Circle"),
            value: Box::new(v),
        },
        |v| NirValue::ResultValue { value: Box::new(v) },
        |v| NirValue::ResultError { value: Box::new(v) },
    ];
    for project in projections {
        let global = project(NirValue::Global {
            name: "g".to_string(),
            type_: ParameterType::named(""),
        });
        assert_eq!(global_root(&global), Some("g"));
        assert_eq!(local_root(&global), None);
        let local = project(NirValue::Local("l".to_string()));
        assert_eq!(local_root(&local), Some("l"));
        assert_eq!(global_root(&local), None);
        let computed = project(call("bump", vec![]));
        assert_eq!(global_root(&computed), None);
        assert_eq!(local_root(&computed), None);
    }
}
