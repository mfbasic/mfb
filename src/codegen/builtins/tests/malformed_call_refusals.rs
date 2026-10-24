//! Three native fast paths that took a collection argument on trust, handed a module
//! the front end would have rejected (`nir_retype.rs`), and lowered it instead of
//! refusing it:
//!
//! * `#collections_mapValues$K$V$U`'s native rewrite treated a `Set` argument as a
//!   map and rewrote "value payloads" a set does not have;
//! * bug-767's copying `sort`, handed a list whose element has no `<` (a recursive
//!   union), declined to the `.mfb` body, which was then called with an argument of
//!   another type than its parameter;
//! * the copying `sortBy`, handed such a list with a key function over another
//!   element type, called the key function on elements it does not take.
//!
//! Each is now refused with a sentence naming the mismatch. A real program reaches
//! none of them (the checker types each call); `sortBy` over a recursive union with
//! a key function that takes the union is valid source and still sorts — see the
//! last row.

use super::nir_retype::{list, refusal, retyped};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};
use crate::types::ParameterType;

const SRC: &str = r#"IMPORT collections
IMPORT io

TYPE Leaf
  v AS Integer
END TYPE

TYPE Node2
  left AS Tree
  right AS Tree
END TYPE

UNION Tree
  Leaf
  Node2
END UNION

FUNC weight(t AS Tree) AS Integer
  MATCH t
    CASE Leaf(l)
      RETURN l.v
    CASE Node2(n)
      RETURN weight(n.left) + weight(n.right)
  END MATCH
END FUNC

FUNC mv(m AS Map OF String TO Integer, f AS FUNC(Integer) AS Integer) AS Integer
  LET ys AS Map OF String TO Integer = collections::mapValues(m, f)
  RETURN len(ys)
END FUNC

FUNC cs(xs AS List OF Float) AS Integer
  LET ys AS List OF Float = collections::sort(xs)
  RETURN len(ys)
END FUNC

FUNC csb(xs AS List OF Float) AS Integer
  LET ys AS List OF Float = collections::sortBy(xs, fident)
  RETURN len(ys)
END FUNC

FUNC fident(n AS Float) AS Float
  RETURN n
END FUNC

FUNC bump(n AS Integer) AS Integer
  RETURN n + 1
END FUNC

FUNC main() AS Integer
  LET a AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1 }
  LET trees AS List OF Tree = [Node2[left := Leaf[v := 5], right := Leaf[v := 4]], Leaf[v := 1]]
  LET sorted AS List OF Tree = collections::sortBy(trees, weight)
  io::print(toString(mv(a, bump) + cs([2.0, 1.0]) + csb([3.0]) + len(sorted)))
  RETURN 0
END FUNC
"#;

#[test]
fn map_values_refuses_a_set_argument() {
    let set = ParameterType::parse("Set OF String");
    let err = refusal(retyped(SRC, "mv", &[("m", set)]), "mapValues over a Set");
    assert!(
        err.contains("native mapValues expects a Map, got Set OF String"),
        "{err}"
    );
}

#[test]
fn the_copying_sort_refuses_a_list_with_no_ordered_element() {
    let err = refusal(
        retyped(SRC, "cs", &[("xs", list("Tree"))]),
        "copying sort over List OF Tree",
    );
    assert!(
        err.contains("native sort: List OF Tree has no ordered element type"),
        "{err}"
    );
}

#[test]
fn the_copying_sort_by_refuses_a_key_function_over_another_element() {
    let err = refusal(
        retyped(SRC, "csb", &[("xs", list("Tree"))]),
        "copying sortBy over List OF Tree with a Float key function",
    );
    assert!(
        err.contains("native copying sortBy: keyFn takes Float, but the list holds Tree"),
        "{err}"
    );
}

/// The valid shape next to the refusals: `sortBy` over a recursive union whose key
/// function takes the union is not refused (the refusal keys on the parameter, not
/// on the element being a graph). Its runtime order and its alloc/free balance were
/// checked by building and running the same program (see the commit message).
#[test]
fn sort_by_over_a_recursive_union_with_a_matching_key_function_lowers() {
    let _ = code_for_src_cached(SRC, CodeTarget::LinuxX86_64, Console);
}
