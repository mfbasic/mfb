//! plan-86 K1 at a `RETURN` (`engine/control/builder_exits.rs`): a function every
//! one of whose returns is a bare parameter hands that parameter back uncopied —
//! its caller reads the result as a borrow of its own argument and copies only at
//! an owning store — while a function that returns a parameter on one path and a
//! fresh value on another copies it, since its caller owns the result.
//!
//! Both are shown for a flat value (a `List OF Integer`, copied inline:
//! `tight_copy_*` scratch) and a recursive one (a `Node` holding a `List OF Node`,
//! copied by its graph walker `_mfb_thread_copy_Node_*`), each in a function that
//! also owns a local of its own, so the "the returned local owns no block" check
//! has a cleanup to look through.

use super::lowered::{calls_on, slots_on};
use crate::testutil::CodeTarget;

const SOURCE: &str = "\
IMPORT collections
IMPORT io

TYPE Node
  v AS Integer
  kids AS List OF Node
END TYPE

FUNC pick(a AS List OF Integer) AS List OF Integer
  LET tmp AS List OF Integer = [len(a), 2]
  io::print(toString(len(tmp)))
  RETURN a
END FUNC

FUNC pickOr(a AS List OF Integer, b AS List OF Integer) AS List OF Integer
  IF len(a) > 0 THEN
    RETURN a
  END IF
  RETURN [len(b)]
END FUNC

FUNC pickNode(n AS Node) AS Node
  LET tmp AS List OF Integer = [n.v, 2]
  io::print(toString(len(tmp)))
  RETURN n
END FUNC

FUNC pickNodeOr(n AS Node, m AS Node) AS Node
  IF n.v > 0 THEN
    RETURN n
  END IF
  RETURN WITH m { v := 1 }
END FUNC

FUNC main() AS Integer
  LET xs AS List OF Integer = pick([1, 2, 3])
  LET ys AS List OF Integer = pickOr([1], [2])
  LET leaf AS Node = Node[v := 1, kids := []]
  LET root AS Node = pickNode(Node[v := 0, kids := [leaf]])
  LET other AS Node = pickNodeOr(Node[v := 0, kids := []], Node[v := 2, kids := []])
  io::print(toString(len(xs) + len(ys) + root.v + other.v))
  RETURN 0
END FUNC
";

fn copies_graph(function: &str, target: CodeTarget) -> bool {
    calls_on(SOURCE, function, target)
        .iter()
        .any(|call| call.starts_with("_mfb_thread_copy_Node"))
}

fn copies_flat(function: &str, target: CodeTarget) -> bool {
    slots_on(SOURCE, function, target)
        .iter()
        .any(|slot| slot == "tight_copy_source")
}

#[test]
fn a_pure_passthrough_returns_its_parameter_uncopied() {
    for target in CodeTarget::ALL {
        assert!(!copies_flat("pick", target), "{}: pick", target.name());
        assert!(copies_flat("pickOr", target), "{}: pickOr", target.name());
        assert!(
            !copies_graph("pickNode", target),
            "{}: pickNode",
            target.name()
        );
        assert!(
            copies_graph("pickNodeOr", target),
            "{}: pickNodeOr",
            target.name()
        );
    }
}
