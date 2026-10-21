//! plan-136-A: a native `LINK` function's parameter defaults. A LINK function
//! has no IR body, so every call that leaves a defaulted argument out is
//! filled at the call site: a literal is copied in, a package constant folds
//! to its value, and a computed default becomes a call to a synthesized
//! hidden default function, evaluated again on every such call.

use super::*;

const LINKED: &str = "\
IMPORT math
FUNC seed() AS Integer
  RETURN 41
END FUNC
LINK \"m\" AS lib
  FUNC nap(ms AS Integer = 10, extra AS Integer = seed(), scale AS Float = math::pi) AS Integer
    SYMBOL \"nap\"
    ABI (ms CInt32, extra CInt32, scale CDouble) AS status CInt32
    RETURN status
  END FUNC
END LINK
FUNC main() AS Integer
  RETURN lib::nap() + lib::nap(1)
END FUNC
";

/// The argument lists of every call to `target` in `function`'s RETURNs.
fn calls_to(ir: &IrProject, function_name: &str, target: &str) -> Vec<Vec<IrValue>> {
    let mut found = Vec::new();
    for op in &function(ir, function_name).body {
        if let IrOp::Return {
            value: Some(value), ..
        } = op
        {
            crate::ir::value::visit_value(value, &mut |node| {
                if let IrValue::Call {
                    target: called,
                    args,
                    ..
                } = node
                {
                    if called == target {
                        found.push(args.clone());
                    }
                }
            });
        }
    }
    found
}

#[test]
fn a_call_leaving_link_defaults_out_is_filled_at_the_call_site() {
    let ir = lower_src_concrete(LINKED, None);
    let calls = calls_to(&ir, "main", "lib.nap");
    assert_eq!(calls.len(), 2, "both calls reach the link function");
    for args in &calls {
        assert_eq!(args.len(), 3, "every parameter is supplied");
        assert!(
            matches!(&args[1], IrValue::Call { target, args, .. } if args.is_empty() && target != "seed"),
            "the computed default calls the synthesized default function"
        );
        assert!(
            matches!(&args[2], IrValue::Const { type_, .. } if *type_ == ParameterType::Float),
            "the package constant default folds to a Float constant"
        );
    }
    assert!(matches!(&calls[0][0], IrValue::Const { value, .. } if value == "10"));
    assert!(matches!(&calls[1][0], IrValue::Const { value, .. } if value == "1"));
    // The hidden default function exists and calls `seed`.
    let IrValue::Call { target: hidden, .. } = &calls[0][1] else {
        unreachable!("checked above");
    };
    let body = &function(&ir, hidden).body;
    let mut calls_seed = false;
    for op in body {
        if let IrOp::Return {
            value: Some(value), ..
        } = op
        {
            crate::ir::value::visit_value(value, &mut |node| {
                calls_seed |= matches!(node, IrValue::Call { target, .. } if target == "seed");
            });
        }
    }
    assert!(calls_seed, "the default function evaluates `seed()`");
}
