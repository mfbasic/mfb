//! `ir::lower::statement` at the forms the corpora never write: a whole
//! `STATE` replacement under an inline `TRAP`, an `os::exit` call of the wrong
//! shape (lowering is total and must not choke on it), and the `#Result OF …`
//! scrutinee arms the lowering keeps for plan-20 totality.
//!
//! No source `MATCH` names the internal `#Result OF …` type (bug-945 made `Ok`
//! an ordinary name), so those arms are driven the two ways that reach them
//! honestly: `match_case_binding` directly, and a concrete HIR whose scrutinee
//! parameter is retyped to a `Result` before lowering.

use super::*;

/// bug-551: `resource.state = <expr> TRAP …` — the trap covers the whole
/// replacement, so the handler's `RECOVER` supplies the STATE record and the
/// assignment happens once, after the branch, from the delivery slot.
#[test]
fn a_trapped_state_replacement_assigns_once_after_the_handler() {
    let src = "\
IMPORT fs
TYPE Count
  value AS Integer
END TYPE
FUNC make(n AS String) AS Count
  RETURN Count[toInt(n)]
END FUNC
FUNC main() AS Integer
  RES a AS fs::File STATE Count = fs::createTempFile()
  a.state = make(\"x\") TRAP(e)
    RECOVER Count[0]
  END TRAP
  RETURN a.state.value
END FUNC
";
    assert!(check_src(src).is_empty(), "{:?}", check_src(src));
    let ir = lower_src_concrete(src, None);
    let main = function(&ir, "main");
    let mut assigns = Vec::new();
    walk_ops(&main.body, &mut |op| {
        if let IrOp::StateAssign {
            resource, value, ..
        } = op
        {
            assigns.push((resource.clone(), matches!(value, IrValue::Local(_))));
        }
    });
    assert_eq!(
        assigns,
        [("a".to_string(), true)],
        "one STATE assignment, from the trap's delivery slot"
    );
    // The handler path sits in an IF on the trap result, ahead of the assign.
    let assign_at = main
        .body
        .iter()
        .position(|op| matches!(op, IrOp::StateAssign { .. }))
        .expect("the assignment is a top-level statement");
    assert!(
        main.body[..assign_at]
            .iter()
            .any(|op| matches!(op, IrOp::If { .. })),
        "the trap's branch precedes the assignment"
    );
}

/// `os::exit` with two arguments is a checker error, and lowering — which runs
/// regardless — treats it as an ordinary call rather than the exit form.
#[test]
fn an_exit_call_of_the_wrong_arity_lowers_as_a_plain_call() {
    let src = "\
IMPORT os
FUNC main() AS Integer
  os::exit(1, 2)
  RETURN 0
END FUNC
";
    assert_eq!(check_src(src), ["TYPE_CALL_ARITY_MISMATCH"]);
    let ir = lower_src_concrete(src, None);
    let mut exits = 0;
    walk_ops(&function(&ir, "main").body, &mut |op| {
        if matches!(op, IrOp::ExitProgram { .. }) {
            exits += 1;
        }
    });
    assert_eq!(exits, 0, "the two-argument call is not the exit statement");
}

#[test]
fn a_result_case_binding_reads_the_value_or_the_error() {
    let result = ParameterType::result_of(ParameterType::Integer);
    let ok = HirMatchPattern::Union {
        type_: ParameterType::named(crate::types::RESULT_SUCCESS_MEMBER),
        binding: "v".to_string(),
    };
    let (name, type_, value) =
        crate::ir::lower::match_case_binding(&ok, "m", &result).expect("an Ok case binds");
    assert_eq!((name.as_str(), type_), ("v", ParameterType::Integer));
    assert!(matches!(value, IrValue::ResultValue { value, .. }
        if matches!(value.as_ref(), IrValue::Local(local) if local == "m")));

    let error = HirMatchPattern::Union {
        type_: ParameterType::named("Error"),
        binding: "e".to_string(),
    };
    let (name, type_, value) =
        crate::ir::lower::match_case_binding(&error, "m", &result).expect("an Error case binds");
    assert_eq!((name.as_str(), type_), ("e", ParameterType::named("Error")));
    assert!(matches!(value, IrValue::ResultError { .. }));

    let other = HirMatchPattern::Union {
        type_: ParameterType::named("Circle"),
        binding: "c".to_string(),
    };
    assert!(crate::ir::lower::match_case_binding(&other, "m", &result).is_none());
}

/// A `MATCH` over a `#Result OF Integer` scrutinee tests a `resultIsOk` flag,
/// and its `#Ok`/`Error` cases become `TRUE`/`FALSE` patterns.
#[test]
fn a_result_scrutinee_matches_on_its_ok_flag() {
    let src = "\
TYPE Circle
  r AS Integer
END TYPE
TYPE Square
  s AS Integer
END TYPE
UNION Shape
  Circle
  Square
END UNION
FUNC probe(x AS Shape) AS Integer
  MATCH x
    CASE Circle(c)
      RETURN 1
    CASE Square(q)
      RETURN 2
  END MATCH
END FUNC
FUNC main() AS Integer
  RETURN probe(Circle[1])
END FUNC
";
    let mut hir = crate::testutil::concrete_hir_from_src(src);
    let probe = hir
        .files
        .iter_mut()
        .flat_map(|file| file.items.iter_mut())
        .find_map(|item| match item {
            crate::hir::HirItem::Function(function) if function.name == "probe" => Some(function),
            _ => None,
        })
        .expect("probe");
    probe.params[0].type_ = ParameterType::result_of(ParameterType::Integer);
    let crate::hir::HirStatement::Match { cases, .. } = &mut probe.body[0] else {
        panic!("probe opens with its MATCH");
    };
    cases[0].pattern = HirMatchPattern::Union {
        type_: ParameterType::named(crate::types::RESULT_SUCCESS_MEMBER),
        binding: "c".to_string(),
    };
    cases[1].pattern = HirMatchPattern::Union {
        type_: ParameterType::named("Error"),
        binding: "q".to_string(),
    };
    let ir = crate::ir::lower_augmented_project(&hir, None, &HashMap::new(), &[], &[]);
    let body = &function(&ir, "probe").body;
    let flag = body
        .iter()
        .find_map(|op| match op {
            IrOp::Bind {
                name,
                value: Some(IrValue::ResultIsOk { .. }),
                ..
            } => Some(name.clone()),
            _ => None,
        })
        .expect("the scrutinee's ok flag is bound");
    let (value, cases) = body
        .iter()
        .find_map(|op| match op {
            IrOp::Match { value, cases, .. } => Some((value, cases)),
            _ => None,
        })
        .expect("the MATCH");
    assert!(matches!(value, IrValue::Local(name) if *name == flag));
    let patterns: Vec<&str> = cases
        .iter()
        .map(|case| match &case.pattern {
            crate::ir::IrMatchPattern::Value(IrValue::Const { value, .. }) => value.as_str(),
            _ => "other",
        })
        .collect();
    assert_eq!(patterns, ["true", "false"]);
}
