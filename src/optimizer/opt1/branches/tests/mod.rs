use super::super::local_rewrites::testutil::*;
use super::*;
use crate::ir::resource_escape::ResOwner;
use crate::optimizer::{with_opt_level, OptLevel};
use crate::target::shared::nir::NirFunction;
use std::collections::HashMap;

fn boolean(value: bool) -> NirValue {
    typed_const(ParameterType::Boolean, if value { "true" } else { "false" })
}

fn eval(value: NirValue) -> NirOp {
    NirOp::Eval { value }
}

fn run(body: Vec<NirOp>, level: u8) -> Vec<NirOp> {
    run_with_owners(body, level, HashMap::new())
}

fn run_with_owners(
    body: Vec<NirOp>,
    level: u8,
    resource_owners: HashMap<String, ResOwner>,
) -> Vec<NirOp> {
    let function = NirFunction {
        name: "f".to_string(),
        visibility: "private".to_string(),
        kind: "function".to_string(),
        isolated: false,
        params: vec![],
        returns: ParameterType::Integer,
        body,
        file: "main.mfb".to_string(),
        resource_owners,
    };
    let mut module = test_module(vec![function]);
    with_opt_level(OptLevel(level), || simplify(&mut module));
    module.functions.remove(0).body
}

/// `IF TRUE` splices the then-arm (trap-capable statements included — they
/// were reachable and stay); the else-arm vanishes even though *it* holds
/// trap-capable code (it is unreachable, no trap gate applies).
#[test]
fn constant_true_keeps_then_and_drops_else() {
    let body = run(
        vec![NirOp::If {
            condition: boolean(true),
            then_body: vec![eval(binary(BinaryOp::Add, local("a"), local("b")))],
            else_body: vec![eval(binary(BinaryOp::Divide, local("x"), int_const("0")))],
        }],
        2,
    );
    assert_eq!(body.len(), 1);
    let NirOp::Eval { value } = &body[0] else {
        panic!("expected the spliced then-arm Eval");
    };
    assert!(matches!(value, NirValue::Binary { op, .. } if *op == BinaryOp::Add));
}

/// A constant condition that is not a Boolean literal is left alone.
///
/// `constant_condition` reads a `Const` and then asks two questions the
/// happy path never fails: is its type `Boolean`, and is its text one of the
/// two literals. Both refusals had never run, and both are what stops the
/// fold from firing on something it has not actually proved. An `IF` whose
/// condition is an Integer constant is not a program the checkers admit --
/// but a lowering bug that produced one, folded on the truthiness of `1`,
/// would silently delete the else-arm of a program that should not have
/// compiled at all. The pass declines instead, and the IF survives to be
/// caught by something that reports.
#[test]
fn a_constant_condition_that_is_not_a_boolean_literal_is_not_folded() {
    for (what, condition) in [
        (
            "an Integer constant",
            typed_const(ParameterType::Integer, "1"),
        ),
        (
            "a Boolean-typed constant with text neither literal",
            typed_const(ParameterType::Boolean, "maybe"),
        ),
    ] {
        let body = run(
            vec![NirOp::If {
                condition,
                then_body: vec![eval(binary(BinaryOp::Add, local("a"), local("b")))],
                else_body: vec![],
            }],
            2,
        );
        assert!(
            matches!(body.as_slice(), [NirOp::If { .. }]),
            "{what} is not a Boolean the pass can fold on; the IF must \
             survive, and it became {}",
            body.len()
        );
    }
}

/// `IF FALSE` keeps the else-arm; an empty else-arm means the whole IF
/// vanishes.
#[test]
fn constant_false_keeps_else() {
    let body = run(
        vec![
            NirOp::If {
                condition: boolean(false),
                then_body: vec![eval(local("dead"))],
                else_body: vec![eval(local("live"))],
            },
            NirOp::If {
                condition: boolean(false),
                then_body: vec![eval(local("dead"))],
                else_body: vec![],
            },
        ],
        2,
    );
    assert_eq!(body.len(), 1, "second IF vanishes entirely");
}

/// `WHILE FALSE` runs zero times and is dropped whole; `WHILE TRUE` and
/// non-constant conditions are untouched.
#[test]
fn while_false_drops_and_while_true_stays() {
    let loop_with = |condition: NirValue| NirOp::While {
        kind: crate::ast::LoopKind::While,
        condition,
        body: vec![eval(local("x"))],
    };
    let body = run(
        vec![
            loop_with(boolean(false)),
            loop_with(boolean(true)),
            loop_with(local("c")),
        ],
        2,
    );
    assert_eq!(body.len(), 2, "only WHILE FALSE vanishes");
}

/// Nested constant branches fold bottom-up: an inner `IF TRUE` inside a
/// kept arm splices before the outer arm does.
#[test]
fn nested_constant_branches_fold() {
    let body = run(
        vec![NirOp::If {
            condition: boolean(true),
            then_body: vec![NirOp::If {
                condition: boolean(false),
                then_body: vec![eval(local("dead"))],
                else_body: vec![eval(local("kept"))],
            }],
            else_body: vec![],
        }],
        2,
    );
    assert_eq!(body.len(), 1);
    assert!(matches!(&body[0], NirOp::Eval { value: NirValue::Local(name) } if name == "kept"));
}

/// The row is off at `-O1`.
#[test]
fn level_one_disables_the_row() {
    let body = run(
        vec![NirOp::If {
            condition: boolean(true),
            then_body: vec![eval(local("x"))],
            else_body: vec![],
        }],
        1,
    );
    assert!(matches!(body[0], NirOp::If { .. }));
}

fn bind(name: &str, type_: ParameterType) -> NirOp {
    NirOp::Bind {
        mutable: false,
        name: name.to_string(),
        type_,
        value: Some(local("src")),
    }
}

fn named(name: &str) -> ParameterType {
    ParameterType::Named(crate::intern::Symbol::intern(name))
}

/// An arm that declares a `RES` binding is a cleanup scope: splicing it would
/// move the binding's close from the arm's end to the function's end, past the
/// statements that follow (resource-owner-scope-rt case 4 printed an empty
/// file at -O2: the buffered handle flushed only after the read). The IF stays.
#[test]
fn an_arm_declaring_a_res_binding_is_not_spliced() {
    let body = run_with_owners(
        vec![
            NirOp::If {
                condition: boolean(true),
                then_body: vec![bind("q", named("fs.File"))],
                else_body: vec![],
            },
            eval(local("after")),
        ],
        2,
        HashMap::from([("q".to_string(), ResOwner::Local)]),
    );
    assert!(
        matches!(body.as_slice(), [NirOp::If { .. }, NirOp::Eval { .. }]),
        "the RES arm must keep its own scope, the body became {} ops",
        body.len()
    );
}

/// The same for a binding that is not itself `RES` but whose type can carry
/// one: a `List OF RES` adopts the owned-list of a callee's returned list
/// (case 3), and a record or union can hold a resource field. Only a type the
/// pass can prove resource-free lets the arm splice.
#[test]
fn an_arm_declaring_a_resource_carrying_binding_is_not_spliced() {
    let list_of_res =
        ParameterType::ListOf(Box::new(ParameterType::Res(Box::new(named("fs.File")))));
    for type_ in [list_of_res, named("Holder")] {
        let body = run(
            vec![NirOp::If {
                condition: boolean(true),
                then_body: vec![bind("q", type_.clone())],
                else_body: vec![],
            }],
            2,
        );
        assert!(
            matches!(body.as_slice(), [NirOp::If { .. }]),
            "{type_:?} can carry a resource; the arm must stay, the body became {} ops",
            body.len()
        );
    }
}

/// A binding of a provably resource-free type has no scope-exit effect, so the
/// arm still splices (including `IF FALSE`'s else-arm).
#[test]
fn an_arm_declaring_only_resource_free_bindings_still_splices() {
    let table = ParameterType::MapOf(
        Box::new(ParameterType::String),
        Box::new(ParameterType::ListOf(Box::new(ParameterType::Integer))),
    );
    let body = run(
        vec![
            NirOp::If {
                condition: boolean(true),
                then_body: vec![bind("n", ParameterType::Integer), bind("m", table)],
                else_body: vec![],
            },
            NirOp::If {
                condition: boolean(false),
                then_body: vec![],
                else_body: vec![bind("s", ParameterType::String)],
            },
        ],
        2,
    );
    assert_eq!(body.len(), 3, "both arms splice");
    assert!(body.iter().all(|op| matches!(op, NirOp::Bind { .. })));
}

/// End to end through lowering: at -O2 the `IF TRUE` block holding `RES q`
/// survives into the NIR codegen sees, so `q` closes (and flushes) before the
/// `fs::readText` after it — the reduced resource-owner-scope-rt case 4.
#[test]
fn a_constant_if_holding_a_res_binding_keeps_its_scope_through_lowering() {
    const SOURCE: &str = r#"
IMPORT io
IMPORT fs

FUNC fresh() AS RES fs::File
  RES f AS fs::File = fs::openFile("pick.txt", "w")
  fs::setBuffered(f, TRUE)
  fs::writeAll(f, "flushed")
  RETURN f
END FUNC

FUNC main AS Integer
  IF TRUE THEN
    RES q AS fs::File = fresh()
  END IF
  io::print(fs::readText("pick.txt"))
  RETURN 0
END FUNC
"#;
    let module = with_opt_level(OptLevel(2), || {
        crate::testutil::nir_for_src(
            SOURCE,
            crate::testutil::CodeTarget::LinuxX86_64,
            crate::target::NativeBuildMode::Console,
        )
    })
    .expect("the program lowers to NIR");
    let main = module
        .functions
        .iter()
        .find(|function| function.name == "main")
        .expect("main is lowered");
    let binds_q = |op: &NirOp| matches!(op, NirOp::Bind { name, .. } if name == "q");
    assert!(
        !main.body.iter().any(binds_q),
        "RES q was spliced into main's own scope",
    );
    assert!(
        main.body
            .iter()
            .any(|op| matches!(op, NirOp::If { then_body, .. } if then_body.iter().any(binds_q))),
        "RES q must stay inside its IF scope at -O2",
    );
}
