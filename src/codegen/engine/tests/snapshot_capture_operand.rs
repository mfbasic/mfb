//! An operand that reads a by-reference CAPTURE directly is snapshotted like the
//! by-reference local the IR normally binds it to
//! (`engine/value/operand_snapshot.rs`, `operand_reachable_by_later_call`).
//!
//! The IR binds every capture once, in the lambda's prologue
//! (`Bind acc = Capture { by_ref }`), and the body reads that local; so a
//! `Capture` value never stands as an operand in a lowered program. The snapshot
//! rule still answers for one — a capture held by reference reads through the
//! parent's slot, which a later operand's user call can reassign — so a module
//! whose body reads the capture in place (an inliner or forwarding pass is free
//! to produce one; the verifier admits a capture anywhere in the lambda) walks a
//! snapshot exactly as the bound form does. This builds that module from the
//! bound one and compares.

use crate::target::shared::nir::{NirModule, NirOp, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_nir, nir_for_src, CodeTarget};

const SOURCE: &str = "\
IMPORT collections
IMPORT io

FUNC say(v AS Integer) AS Integer
  io::print(\"say\")
  RETURN v
END FUNC

FUNC main() AS Integer
  MUT acc AS String = \"a\"
  collections::forEach([1, 2], LAMBDA(v AS Integer) -> io::print(acc & toString(say(v) + say(v))))
  acc = \"b\"
  io::print(acc)
  RETURN 0
END FUNC
";

/// Whether the lambda reserves an operand snapshot.
fn lambda_snapshots(module: &NirModule) -> bool {
    code_for_nir(module, CodeTarget::LinuxX86_64)
        .unwrap_or_else(|err| panic!("the module lowers: {err}"))
        .functions
        .iter()
        .filter(|function| function.name.contains("lambda"))
        .any(|function| {
            function
                .stack_slots
                .iter()
                .any(|slot| slot.name.starts_with("operand_snapshot"))
        })
}

fn module() -> NirModule {
    nir_for_src(SOURCE, CodeTarget::LinuxX86_64, Console)
        .unwrap_or_else(|err| panic!("the probe lowers to NIR: {err}"))
}

#[test]
fn a_capture_read_in_place_before_a_user_call_is_snapshotted() {
    let bound = module();
    assert!(
        lambda_snapshots(&bound),
        "the bound form: `acc` is a by-ref local, read before `say`"
    );

    // Read the capture in place: the concat's left operand becomes the prologue's
    // `Capture` value itself.
    let mut in_place = module();
    let lambda = in_place
        .functions
        .iter_mut()
        .find(|function| function.name.contains("lambda"))
        .expect("the callback lowers as its own function");
    let capture = lambda
        .body
        .iter()
        .find_map(|op| match op {
            NirOp::Bind {
                name,
                value: Some(value @ NirValue::Capture { by_ref: true, .. }),
                ..
            } if name == "acc" => Some(value.clone()),
            _ => None,
        })
        .expect("the prologue binds acc to its by-ref capture");
    let mut replaced = false;
    for op in &mut lambda.body {
        if let NirOp::Return {
            value: Some(NirValue::RuntimeCall { args, .. }),
        } = op
        {
            if let Some(NirValue::Binary { left, .. }) = args.first_mut() {
                if matches!(left.as_ref(), NirValue::Local(name) if name == "acc") {
                    **left = capture.clone();
                    replaced = true;
                }
            }
        }
    }
    assert!(replaced, "the concat reads acc first");
    assert!(
        lambda_snapshots(&in_place),
        "read in place, the by-ref capture is snapshotted too"
    );
}
