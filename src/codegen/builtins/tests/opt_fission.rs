//! Loop fission (`optimizer::opt1::fission`) on the body shape lowering
//! produces: the user's variable re-bound from the loop's iteration temp at
//! the head (`LET i = $for_iter1`) and read by both halves.
//!
//! That mirror bind is in both halves' worlds and writes the same value, so it
//! must not block the split — and the second loop must re-declare it, under a
//! fresh name bound from the second loop's own (fresh) variable, because NIR
//! locals are function-unique.

use super::nir_build::*;
use crate::target::shared::nir::{NirOp, NirValue};

#[test]
fn a_mirror_bind_is_carried_into_the_second_loop_under_a_fresh_name() {
    let body = run_row(
        3,
        vec![for_loop(
            "$for_iter1",
            int_const("0"),
            int_const("9"),
            int_const("1"),
            vec![
                bind("i", local("$for_iter1")),
                assign("a1", local("i")),
                assign("a2", local("a1")),
                assign("b1", local("i")),
                assign("b2", local("b1")),
            ],
        )],
        crate::optimizer::opt1::fission::split,
    );
    assert_eq!(body.len(), 2, "the loop is split in two");
    let NirOp::For {
        name: first_var,
        body: first,
        ..
    } = &body[0]
    else {
        panic!("first phase");
    };
    let NirOp::For {
        name: second_var,
        body: second,
        ..
    } = &body[1]
    else {
        panic!("second phase");
    };
    assert_eq!(first_var, "$for_iter1");
    assert_eq!(first.len(), 3, "the mirror and the first phase");
    assert_ne!(second_var, first_var, "the second loop's variable is fresh");
    assert_eq!(second.len(), 3, "the carried mirror and the second phase");
    let NirOp::Bind {
        name: carried,
        value: Some(NirValue::Local(source)),
        ..
    } = &second[0]
    else {
        panic!("the second loop opens with the carried mirror bind");
    };
    assert_ne!(
        carried, "i",
        "the carried bind is re-declared under a fresh name"
    );
    assert_eq!(
        source, second_var,
        "it binds from the second loop's own variable"
    );
    assert!(
        matches!(&second[1], NirOp::Assign { name, value: NirValue::Local(read) }
            if name == "b1" && read == carried),
        "the second phase reads the carried bind"
    );
}
