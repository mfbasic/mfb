//! The growing/shrinking in-place arms of a LOCAL collection
//! (`collection/assign/builder_inplace_assign/mod.rs`: append, prepend, insert,
//! repeat, set, removeKey and bug-774's self-slice append) handed a module the front
//! end would have rejected (`nir_retype.rs`), plus the one self-slice decline a
//! program reaches.
//!
//! Each arm's G9 gate names the collection kind it mutates; a binding of another
//! kind is declined and the copying lowering refuses it by name. The self-slice and
//! bulk-append arms commit on a STATIC type and re-check the lowered one, refusing a
//! module where the two disagree rather than copying bytes at the wrong width.

use super::nir_fault::walk_ops_mut;
use super::nir_retype::{lowered_module_with, refusal, retyped};
use crate::target::shared::nir::NirValue;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};
use crate::types::ParameterType;

const SRC: &str = r#"IMPORT collections
IMPORT io

MUT gl AS List OF Integer = [7, 8]

FUNC rk(m AS Map OF String TO Integer, k AS String) AS Integer
  MUT ys AS Map OF String TO Integer = m
  ys = collections::removeKey(ys, k)
  RETURN len(ys)
END FUNC

FUNC pp(xs AS List OF Integer, v AS Integer) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::prepend(ys, v)
  RETURN len(ys)
END FUNC

FUNC ins(xs AS List OF Integer, i AS Integer, v AS Integer) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::insert(ys, i, v)
  RETURN len(ys)
END FUNC

FUNC rp(xs AS List OF Integer) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::repeat(ys, 4, 2, 1)
  RETURN len(ys)
END FUNC

FUNC st(xs AS List OF Integer, i AS Integer, v AS Integer) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::set(ys, i, v)
  RETURN len(ys)
END FUNC

FUNC sl(xs AS List OF Integer, a AS Integer, n AS Integer) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::append(ys, collections::mid(ys, a, n))
  ys = collections::append(ys, collections::mid(ys, a * n, n))
  RETURN len(ys)
END FUNC

FUNC bulk(xs AS List OF Integer) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::append(ys, gl)
  RETURN len(ys)
END FUNC

FUNC main() AS Integer
  io::print(toString(rk(Map OF String TO Integer { "a" := 1 }, "a") + pp([1], 2) + ins([1], 0, 2) + rp([1]) + st([1], 0, 2) + sl([1, 2], 0, 1) + bulk([1])))
  RETURN 0
END FUNC
"#;

fn ty(name: &str) -> ParameterType {
    ParameterType::parse(name)
}

/// Each arm declines a binding of a collection kind it does not mutate; the
/// copying lowering then refuses that kind by name.
#[test]
fn each_arm_declines_a_binding_of_another_collection_kind() {
    let set = ty("Set OF Integer");
    for (func, member) in [
        ("pp", "prepend"),
        ("ins", "insert"),
        ("rp", "repeat"),
        ("st", "set"),
    ] {
        let err = refusal(
            retyped(SRC, func, &[("xs", set.clone()), ("ys", set.clone())]),
            &format!("{member} over a Set"),
        );
        assert!(
            err.contains(&format!(
                "native collection {member} does not accept Set OF Integer"
            )),
            "{member} over a Set: {err}"
        );
    }
    let list = ty("List OF Integer");
    let err = refusal(
        retyped(SRC, "rk", &[("m", list.clone()), ("ys", list)]),
        "removeKey over a List",
    );
    assert!(
        err.contains("List OF Integer"),
        "removeKey over a List: {err}"
    );
}

/// bug-774's self-slice append re-checks `start` and `count` once lowered: a start
/// the call's static typing accepted (a `Scalar`) is refused naming both types.
#[test]
fn the_self_slice_append_refuses_a_non_integer_start() {
    let err = refusal(
        retyped(SRC, "sl", &[("a", ty("Scalar"))]),
        "self-slice append with a Scalar start",
    );
    assert!(
        err.contains("native list mid start and count must be Integer, got Scalar and Integer"),
        "{err}"
    );
}

/// The bulk append commits on the right-hand side's static type and refuses a
/// global read whose lowered type is another list.
#[test]
fn the_bulk_append_refuses_a_sublist_that_lowers_to_another_list() {
    let err = refusal(
        lowered_module_with(SRC, |module| {
            for global in module.globals.iter_mut().filter(|g| g.name == "gl") {
                global.type_ = ty("List OF String");
            }
            for function in module.functions.iter_mut().filter(|f| f.name == "bulk") {
                walk_ops_mut(&mut function.body, &mut |_| {}, &mut |value| {
                    if let NirValue::Global { type_, .. } = value {
                        *type_ = ty("List OF Integer");
                    }
                });
            }
        }),
        "bulk append of a global declared List OF String, read as List OF Integer",
    );
    assert!(
        err.contains("native bulk append sublist must be List OF Integer, got List OF String"),
        "{err}"
    );
}

/// The self-slice copy runs `start`/`count` between its range check and the copy,
/// so it takes only operands built from literals, locals, `+` and `-`: a product
/// (which can raise on overflow) sends that statement to the copying `mid`.
#[test]
fn a_self_slice_with_a_product_start_takes_the_copying_path() {
    let plan = code_for_src_cached(SRC, CodeTarget::LinuxX86_64, Console);
    let starts = code_function(plan, "sl")
        .stack_slots
        .iter()
        .filter(|slot| slot.type_ == "self_slice_append_start")
        .count();
    assert_eq!(
        starts, 1,
        "`mid(ys, a, n)` is copied in place, `mid(ys, a * n, n)` is not"
    );
}
