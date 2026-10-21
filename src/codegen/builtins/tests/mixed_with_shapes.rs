//! The shapes plan-145-C's mixed `WITH` route (`try_inplace_mixed_with`,
//! `engine/control/builder_control/field_replace.rs`) must decline.
//!
//! The route stores scalar updates into the owner's slots and runs one collection
//! arm against its own field, in place, with every scalar computed BEFORE the arm
//! mutates anything. That is sound only for its exact shape: a pointer field's new
//! value (a function value) would be built before an arm that can still fail and
//! leak on that path; a scalar written after the arm in source order but computed
//! before it is unobservable only when it has no effect; and an arm that is not a
//! builtin call has no in-place form at all. Each declined shape takes the
//! whole-record rebuild, which reserves no `mixed_with_scalar` slot — and each row
//! is checked against the same record and statement with only that one thing
//! changed, which does take the route.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

const TYPES: &str = "\
IMPORT collections
IMPORT fs
IMPORT io

FUNC say(v AS Integer) AS Integer
  io::print(\"say\")
  RETURN v
END FUNC

TYPE Bag
  hp AS Integer
  n AS Integer
  xs AS List OF Integer
  f AS FUNC(Integer) AS Integer
END TYPE
";

fn takes_mixed_route(update: &str) -> bool {
    let source = format!(
        "{TYPES}\nFUNC main() AS Integer\n  MUT b AS Bag = Bag[hp := 1, n := 0, xs := [1], f := LAMBDA(x AS Integer) -> x]\n  \
         LET k AS Integer = 5\n  FOR i = 1 TO 4\n    b = WITH b {{ {update} }}\n  NEXT\n  \
         io::print(toString(b.n + len(b.xs)))\n  RETURN 0\nEND FUNC\n"
    );
    mixed_slot(&source)
}

fn mixed_slot(source: &str) -> bool {
    code_function(
        code_for_src_cached(source, CodeTarget::LinuxX86_64, Console),
        "main",
    )
    .stack_slots
    .iter()
    .any(|slot| slot.type_ == "mixed_with_scalar")
}

const ARM: &str = "xs := collections::append(b.xs, k)";

#[test]
fn a_function_value_update_beside_the_arm_is_rebuilt() {
    assert!(takes_mixed_route(&format!("hp := b.hp + 1, {ARM}, n := k")));
    assert!(!takes_mixed_route(&format!(
        "hp := b.hp + 1, {ARM}, f := LAMBDA(x AS Integer) -> x + k"
    )));
}

#[test]
fn an_effectful_scalar_after_the_arm_is_rebuilt_and_before_it_is_not() {
    assert!(takes_mixed_route(&format!(
        "n := say(k), {ARM}, hp := b.hp + 1"
    )));
    assert!(!takes_mixed_route(&format!(
        "hp := b.hp + 1, {ARM}, n := say(k)"
    )));
}

#[test]
fn a_collection_update_that_is_no_builtin_call_is_rebuilt() {
    assert!(!takes_mixed_route("hp := b.hp + 1, xs := [k, i], n := k"));
    // The same over a resource's `STATE` payload.
    let state = format!(
        "{TYPES}\nTYPE P\n  hp AS Integer\n  n AS Integer\n  xs AS List OF Integer\nEND TYPE\n\n\
         FUNC main() AS Integer\n  RES h AS fs::File STATE P = fs::openFile(\"/dev/null\")\n  \
         LET k AS Integer = 5\n  h.state = WITH h.state {{ xs := [k], n := 5 }}\n  \
         io::print(toString(h.state.n))\n  RETURN 0\nEND FUNC\n"
    );
    assert!(!mixed_slot(&state));
}
