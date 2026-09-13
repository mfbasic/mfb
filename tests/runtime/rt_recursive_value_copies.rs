//! plan-134-D: a recursive value stored at an owning store is independent of its source.
//!
//! ## Why this exists
//!
//! A value of a recursive type (`TYPE Node / kids AS List OF Node`, a recursive
//! `UNION Tree`) is not `memcpy`-copyable, so `lower_value_owned` and
//! `lower_returned_value` skipped the owning copy for it: `MUT ys = xs`, a global
//! assigned from a local, a closure capture and `RETURN h.kids` all stored the SAME
//! block as their source. The in-place collection arms assume one owner and never check
//! it, so an in-place `append` on the copy grew or reallocated the source's buffer —
//! bug-601's recursive row printed `ys=6 xs=112`, a length read from freed memory.
//!
//! Measured on the pre-plan compiler (`cc1012bdc`), this program printed:
//!
//! ```text
//! field a=96 ak=2
//! removeAt xs=2 ys=1
//! append xs=160 zs=3
//! global g=96 local=2
//! return h=96 got=2
//! closure f=96 cap=2
//! tree ys=6 xs=112
//! ```
//!
//! Every wrong number is a source read after its copy was grown in place. `removeAt`
//! was already right only because gate `G24` declines the in-place arm for a recursive
//! element and rebuilds the list.
//!
//! ## What it runs
//!
//! One program, one line per owning store: a field copied into a `MUT` binding, a `MUT`
//! copy shrunk (`removeAt`) and grown (`append`), a global assigned from a local, a
//! function returning a field, a closure capturing a list, and bug-601's recursive-union
//! shape. Each line mutates the copy and then reads the source.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const COPIES_SOURCE: &str = r#"IMPORT io
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

TYPE Leaf
  v AS Integer
END TYPE

TYPE Branch
  kids AS List OF Tree
END TYPE

UNION Tree
  Leaf
  Branch
END UNION

MUT gKids AS List OF Node = []

FUNC mk(tag AS Integer) AS Node
  RETURN Node[kids := [], tag := tag]
END FUNC

FUNC kidsOf(h AS Node) AS List OF Node
  RETURN h.kids
END FUNC

FUNC counter(xs AS List OF Node) AS FUNC(Integer) AS Integer
  RETURN LAMBDA(n AS Integer) -> len(xs) + n
END FUNC

FUNC main AS Integer
  LET a AS Node = Node[kids := [mk(1)], tag := 10]
  MUT ak AS List OF Node = a.kids
  ak = collections::append(ak, mk(2))
  io::print("field a=" & toString(len(a.kids)) & " ak=" & toString(len(ak)))

  LET xs AS List OF Node = [mk(1), mk(2)]
  MUT ys AS List OF Node = xs
  ys = collections::removeAt(ys, 0)
  io::print("removeAt xs=" & toString(len(xs)) & " ys=" & toString(len(ys)))

  MUT zs AS List OF Node = xs
  zs = collections::append(zs, mk(3))
  io::print("append xs=" & toString(len(xs)) & " zs=" & toString(len(zs)))

  MUT local AS List OF Node = [mk(1)]
  gKids = local
  local = collections::append(local, mk(2))
  io::print("global g=" & toString(len(gKids)) & " local=" & toString(len(local)))

  LET h AS Node = Node[kids := [mk(1)], tag := 5]
  MUT got AS List OF Node = kidsOf(h)
  got = collections::append(got, mk(2))
  io::print("return h=" & toString(len(h.kids)) & " got=" & toString(len(got)))

  MUT cap AS List OF Node = [mk(1)]
  LET f = counter(cap)
  cap = collections::append(cap, mk(2))
  io::print("closure f=" & toString(f(0)) & " cap=" & toString(len(cap)))

  LET t AS Tree = Leaf[v := 1]
  LET txs AS List OF Tree = [t]
  MUT tys AS List OF Tree = txs
  MUT i AS Integer = 0
  WHILE i < 5
    tys = collections::append(tys, t)
    i = i + 1
  END WHILE
  io::print("tree ys=" & toString(len(tys)) & " xs=" & toString(len(txs)))
  RETURN 0
END FUNC
"#;

#[test]
fn a_recursive_value_copy_is_independent_of_its_source() {
    let project = common::temp_project("p134d_recursive_copies", COPIES_SOURCE);
    let exe = common::build_project(&project);

    let output = Command::new(&exe)
        .output()
        .expect("run the recursive copy probe");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        output.status.success(),
        "plan-134-D: mutating a copy of a recursive value crashed (status {:?}): two \
         owners share one block.\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    assert_eq!(
        stdout.trim(),
        [
            "field a=1 ak=2",
            "removeAt xs=2 ys=1",
            "append xs=2 zs=3",
            "global g=1 local=2",
            "return h=1 got=2",
            "closure f=1 cap=2",
            "tree ys=6 xs=1",
        ]
        .join("\n"),
        "plan-134-D: every copy of a recursive value must change only itself.\nstderr:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&project);
}

/// The stores plan-134-C's analysis turns into moves print exactly what they printed
/// before any copy or move existed (measured on `cc1012bdc`): the json-shaped builder,
/// where `item` is rebound every iteration and appended — the decoders' shape — and a
/// `LET b = a` whose source is never read again.
const MOVES_SOURCE: &str = r#"IMPORT io
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

FUNC render(n AS Node) AS String
  MUT out AS String = toString(n.tag)
  IF len(n.kids) > 0 THEN
    out = out & "["
    FOR EACH k IN n.kids
      out = out & render(k) & ","
    NEXT
    out = out & "]"
  END IF
  RETURN out
END FUNC

FUNC buildRow(width AS Integer) AS List OF Node
  MUT acc AS List OF Node = []
  MUT i AS Integer = 0
  WHILE i < width
    LET item AS Node = Node[kids := [Node[kids := [], tag := i * 10]], tag := i]
    acc = collections::append(acc, item)
    i = i + 1
  END WHILE
  RETURN acc
END FUNC

FUNC main AS Integer
  LET row AS List OF Node = buildRow(3)
  LET parent AS Node = Node[kids := row, tag := 99]
  io::print("tree=" & render(parent))
  LET a AS Node = Node[kids := [Node[kids := [], tag := 7]], tag := 1]
  LET b AS Node = a
  io::print("moved=" & render(b))
  RETURN 0
END FUNC
"#;

#[test]
fn a_moved_recursive_value_prints_what_it_printed_before() {
    let project = common::temp_project("p134d_recursive_moves", MOVES_SOURCE);
    let exe = common::build_project(&project);

    let output = Command::new(&exe)
        .output()
        .expect("run the recursive move probe");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        output.status.success(),
        "plan-134-D: a moved recursive value crashed (status {:?}).\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    assert_eq!(
        stdout.trim(),
        "tree=99[0[0,],1[10,],2[20,],]\nmoved=1[7,]",
        "plan-134-D: a move must not change the value.\nstderr:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&project);
}
