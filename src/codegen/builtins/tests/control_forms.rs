//! Control-flow lowering routes of `engine/control/**` and `engine/value/**` that
//! only particular source shapes reach, asserted by the labels and stack objects
//! each route leaves: the walked-block hand-off of a `FOR EACH` whose body
//! reassigns its iterable (bug-1036), the boxed `Result` of an inline `TRAP` on a
//! call through a function value (bug-448), and the gates of `RETURN`'s
//! move-out-of-the-frame routes (plan-147-B).

use super::lowered::{labels_on, slots_on};
use crate::testutil::CodeTarget;

fn has(names: &[String], wanted: &str) -> bool {
    names.iter().any(|name| name == wanted)
}

fn any_starts(names: &[String], prefix: &str) -> bool {
    names.iter().any(|name| name.starts_with(prefix))
}

const LINUX: CodeTarget = CodeTarget::LinuxX86_64;

/// bug-1036: a graph-typed list is walked in place, so a body that reassigns the
/// binding hands the replaced block to the loop (`for_each_replaced`), which frees
/// it when the loop ends; a body that does not reassign has no such hand-off.
#[test]
fn a_for_each_whose_body_reassigns_its_iterable_hands_the_old_block_to_the_loop() {
    let source = "\
IMPORT collections
IMPORT io

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

FUNC mk(t AS Integer) AS Node
  RETURN Node[kids := [Node[kids := [], tag := t]], tag := t]
END FUNC

FUNC grownUntil(stop AS Integer) AS List OF Node
  MUT xs AS List OF Node = [mk(1), mk(2), mk(3)]
  FOR EACH v IN xs
    xs = collections::append(xs, mk(v.tag * 10))
    IF v.tag = stop THEN
      RETURN xs
    END IF
  NEXT
  RETURN xs
END FUNC

FUNC readOnly() AS Integer
  MUT xs AS List OF Node = [mk(1), mk(2)]
  MUT n AS Integer = 0
  FOR EACH v IN xs
    n = n + v.tag
  NEXT
  RETURN n
END FUNC

FUNC main() AS Integer
  io::print(toString(len(grownUntil(2))) & toString(readOnly()))
  RETURN 0
END FUNC
";
    let grown_slots = slots_on(source, "grownUntil", LINUX);
    assert!(has(&grown_slots, "for_each_replaced"), "{grown_slots:?}");
    let grown_labels = labels_on(source, "grownUntil", LINUX);
    assert!(
        has(&grown_labels, "reassign_old_walked"),
        "{grown_labels:?}"
    );
    assert!(has(&grown_labels, "reassign_old_not_walked"));
    let read_only = slots_on(source, "readOnly", LINUX);
    assert!(!has(&read_only, "for_each_replaced"), "{read_only:?}");
}

/// bug-448: an inline `TRAP` on a call through a `FUNC`-typed value consumes a
/// boxed `Result` built from the call's result registers: the tag is tested, the
/// payload wrapped on `Ok` and the error adopted otherwise. A scalar payload takes
/// the unboxed form instead (bug-778), which fills the tag/value slots in place and
/// builds no `Result` block; a recursive graph payload is the case that keeps the
/// boxed one.
#[test]
fn an_inline_trap_on_a_function_value_call_boxes_a_graph_result() {
    let program = |payload: &str, recover: &str, read: &str, pass: &str| {
        format!(
            "IMPORT io

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

FUNC half(v AS Integer) AS Integer
  RETURN v / 2
END FUNC

FUNC node(v AS Integer) AS Node
  RETURN Node[kids := [], tag := v]
END FUNC

FUNC apply(f AS FUNC(Integer) AS {payload}, x AS Integer) AS Integer
  LET r AS {payload} = f(x) TRAP(e)
    RECOVER {recover}
  END TRAP
  RETURN {read}
END FUNC

FUNC main() AS Integer
  io::print(toString(apply({pass}, 8)))
  RETURN 0
END FUNC
"
        )
    };
    let boxed = program("Node", "Node[kids := [], tag := -1]", "r.tag", "node");
    let slots = slots_on(&boxed, "apply", LINUX);
    for kind in ["raw_result_tag", "raw_result_payload", "raw_result"] {
        assert!(has(&slots, kind), "{kind} in {slots:?}");
    }
    let labels = labels_on(&boxed, "apply", LINUX);
    assert!(has(&labels, "result_wrap_error"), "{labels:?}");
    assert!(has(&labels, "result_have_payload"), "{labels:?}");
    let unboxed = program("Integer", "-1", "r", "half");
    let slots = slots_on(&unboxed, "apply", LINUX);
    assert!(has(&slots, "raw_result_tag"), "{slots:?}");
    assert!(!has(&slots, "raw_result_payload"), "{slots:?}");
}

/// plan-147-B: `RETURN append(xs, v)` appends into the local's own block and moves
/// that block out — but only when the local owns it. A parameter (the caller's
/// block), a `FOR EACH` iterable still being walked, and a `String` whose
/// capacity lives only in this frame's shadow each keep the copying return.
#[test]
fn a_returned_self_update_moves_only_a_block_the_frame_owns() {
    let source = "\
IMPORT collections
IMPORT io
IMPORT strings

FUNC owned() AS List OF Integer
  MUT xs AS List OF Integer = [1, 2]
  RETURN collections::append(xs, 3)
END FUNC

FUNC borrowed(xs AS List OF Integer) AS List OF Integer
  RETURN collections::append(xs, 3)
END FUNC

FUNC walked() AS List OF Integer
  MUT xs AS List OF Integer = [1, 2, 3]
  FOR EACH x IN xs
    IF x = 2 THEN
      RETURN collections::append(xs, 9)
    END IF
  NEXT
  RETURN xs
END FUNC

FUNC folded() AS String
  MUT s AS String = \"abc\"
  RETURN strings::upper(s)
END FUNC

FUNC grown(n AS Integer) AS String
  MUT s AS String = toString(n)
  s = s & \"x\"
  RETURN strings::upper(s)
END FUNC

FUNC main() AS Integer
  io::print(toString(len(owned()) + len(borrowed([1])) + len(walked())) & folded() & grown(1))
  RETURN 0
END FUNC
";
    let owned = slots_on(source, "owned", LINUX);
    let borrowed = slots_on(source, "borrowed", LINUX);
    let walked = slots_on(source, "walked", LINUX);
    assert!(
        any_starts(&owned, "append_inplace"),
        "the owned local is appended in place: {owned:?}"
    );
    assert!(
        !any_starts(&borrowed, "append_inplace"),
        "a parameter is the caller's block: {borrowed:?}"
    );
    assert!(
        !any_starts(&walked, "append_inplace"),
        "a walked iterable must not be appended to in place by the return: {walked:?}"
    );
}
