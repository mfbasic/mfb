//! Regression test for bug-538: `collections::get` of an element whose type
//! reaches a **type cycle** handed back an ALIAS into the container's own data
//! region instead of the independent value every other element type gets, so a
//! later `append` that GREW the list (realloc + `emit_free_pre_grow_buffer`)
//! freed the block the fetched value pointed into. Reading the fetched value's
//! recursive field afterwards was a use-after-free — reproducibly `exit 139` on
//! macos-aarch64 before the fix.
//!
//! The contract under test is `materialize_owned_element`'s (plan-02 Phase 8):
//! `collections::get` returns an **owned** value the caller may bind, store and
//! read, and it stays valid whatever is done to the container afterwards.
//! `.ai/collections.md` records the sibling case — plan-121's gate `G24` declines
//! an in-place `removeAt` for this class because the compaction relocates the
//! payload a fetched value points into. `append`'s grow path is a relocation too.
//!
//! Two tests, deliberately:
//!
//! * the NEGATIVE one reproduces the actual shape (a real recursive type, a real
//!   `collections::get`, a real growing `append`), not a proxy that merely shares
//!   the symptom;
//! * the POSITIVE one pins that ordinary recursive-type use is unchanged —
//!   construction, read-back, re-fetch, iteration, nested reads and a
//!   `removeAt`-driven tree walk all still produce exactly the values they did
//!   before, so the added copy did not alter any correct program's behaviour.

mod common;

use std::process::Command;

fn run(source: &str, name: &str) -> (i32, String) {
    let project = common::temp_project(name, source);
    let exe = common::build_project(&project);
    let output = Command::new(&exe).output().expect("run built program");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // A SIGSEGV has no exit code, only a signal; report it as the shell's 128+n
    // so the assertion message names the 139 the bug report recorded.
    let code = match output.status.code() {
        Some(code) => code,
        None => {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                128 + output.status.signal().unwrap_or(0)
            }
            #[cfg(not(unix))]
            {
                -1
            }
        }
    };
    (code, text)
}

/// `Rep` is the shape bug-510's first regex matcher used: a record holding a
/// value of a recursive union. Note that `Rep` itself does NOT participate in the
/// cycle (`Tree` and `Node2` do) — it merely *reaches* one — which is why the
/// narrower `type_participates_in_cycle` predicate the `removeAt` gate uses was
/// not enough to describe this class.
const RECURSIVE_SRC: &str = "IMPORT io\n\
IMPORT collections\n\
TYPE Leaf\n  v AS Integer\nEND TYPE\n\
TYPE Node2\n  left AS Tree\n  right AS Tree\nEND TYPE\n\
UNION Tree\n  Leaf\n  Node2\nEND UNION\n\
TYPE Rep\n  child AS Tree\n  lo AS Integer\nEND TYPE\n\
FUNC describe(t AS Tree) AS String\n\
  MATCH t\n\
    CASE Leaf(l)\n      RETURN \"Leaf(\" & toString(l.v) & \")\"\n\
    CASE Node2(n)\n      RETURN \"Node2(\" & describe(n.left) & \",\" & describe(n.right) & \")\"\n\
  END MATCH\n\
END FUNC\n\
SUB main()\n\
  MUT reps AS List OF Rep = []\n\
  reps = collections::append(reps, Rep[child := Node2[left := Leaf[v := 1], right := Leaf[v := 2]], lo := 3])\n\
  LET back AS Rep = collections::get(reps, 0)\n\
  io::print(\"before=\" & describe(back.child))\n\
  MUT i AS Integer = 0\n\
  WHILE i < 200\n\
    reps = collections::append(reps, Rep[child := Leaf[v := 100 + i], lo := i])\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"len=\" & toString(len(reps)))\n\
  io::print(\"after=\" & describe(back.child))\n\
END SUB\n";

#[test]
fn a_recursive_element_read_out_survives_a_growing_append() {
    let (code, out) = run(RECURSIVE_SRC, "bug538_get_then_grow");
    assert_eq!(
        code, 0,
        "the value read by `collections::get` must survive the list's growth \
         (bug-538 saw exit 139 here); output was:\n{out}"
    );
    assert!(
        out.contains("before=Node2(Leaf(1),Leaf(2))"),
        "the fetched value must read correctly BEFORE the growth:\n{out}"
    );
    assert!(
        out.contains("len=201"),
        "the appends must all have landed:\n{out}"
    );
    assert!(
        out.contains("after=Node2(Leaf(1),Leaf(2))"),
        "the fetched value must read the SAME after the growth — it is an \
         independent value, not a window into the list's storage:\n{out}"
    );
}

/// The positive pin. bug-497's new guard began silently rejecting valid programs
/// and only a positive test caught it; the same trap applies here, because this
/// fix widens a *predicate* that selects a copy. Every ordinary use of a
/// recursive type must still work and still produce exactly the values it did
/// before: construction, `get`, nested field reads, `FOR EACH`, `removeAt`, and a
/// full tree walk over a list that grows while values read out of it are live.
const POSITIVE_SRC: &str = "IMPORT io\n\
IMPORT collections\n\
TYPE ElementNode\n  tag AS String\n  children AS List OF Node\nEND TYPE\n\
TYPE TextNode\n  text AS String\nEND TYPE\n\
UNION Node\n  ElementNode\n  TextNode\nEND UNION\n\
TYPE Slot\n  node AS Node\n  depth AS Integer\nEND TYPE\n\
FUNC render(root AS Node) AS String\n\
  MUT out AS String = \"\"\n\
  MUT stack AS List OF Node = [root]\n\
  DO WHILE len(stack) > 0\n\
    LET i AS Integer = len(stack) - 1\n\
    LET node AS Node = collections::get(stack, i)\n\
    stack = collections::removeAt(stack, i)\n\
    MATCH node\n\
      CASE ElementNode(e)\n\
        MUT j AS Integer = len(e.children) - 1\n\
        DO WHILE j >= 0\n\
          stack = collections::append(stack, collections::get(e.children, j))\n\
          j = j - 1\n\
        LOOP\n\
      CASE TextNode(t)\n\
        out = out & t.text\n\
    END MATCH\n\
  LOOP\n\
  RETURN out\n\
END FUNC\n\
SUB main()\n\
  LET tree AS Node = ElementNode[\"ul\", [ElementNode[\"li\", [TextNode[\"a\"], TextNode[\"b\"]]], TextNode[\"c\"]]]\n\
  io::print(\"render=\" & render(tree))\n\
  ' A list of records holding recursive values: read one out, keep growing, and\n\
  ' check every element still reads right from BOTH the list and the copies.\n\
  MUT slots AS List OF Slot = []\n\
  MUT i AS Integer = 0\n\
  WHILE i < 40\n\
    slots = collections::append(slots, Slot[node := TextNode[\"t\" & toString(i)], depth := i])\n\
    i = i + 1\n\
  END WHILE\n\
  LET first AS Slot = collections::get(slots, 0)\n\
  LET last AS Slot = collections::get(slots, 39)\n\
  slots = collections::append(slots, Slot[node := TextNode[\"tail\"], depth := 99])\n\
  io::print(\"first=\" & render(first.node) & \" last=\" & render(last.node))\n\
  MUT joined AS String = \"\"\n\
  MUT total AS Integer = 0\n\
  FOR EACH s IN slots\n\
    joined = joined & render(s.node)\n\
    total = total + s.depth\n\
  NEXT\n\
  io::print(\"joined=\" & joined)\n\
  io::print(\"total=\" & toString(total))\n\
  io::print(\"refetch=\" & render(collections::get(slots, 0).node))\n\
  ' Removing in place must not disturb a value already read out.\n\
  slots = collections::removeAt(slots, 0)\n\
  io::print(\"len=\" & toString(len(slots)) & \" first-still=\" & render(first.node))\n\
END SUB\n";

#[test]
fn ordinary_recursive_type_use_is_unchanged() {
    let (code, out) = run(POSITIVE_SRC, "bug538_positive");
    assert_eq!(code, 0, "the positive program must exit 0:\n{out}");
    // `render` is an iterative pre-order walk: `ul` -> `li` -> "a","b" then "c".
    assert!(out.contains("render=abc"), "tree walk changed:\n{out}");
    assert!(
        out.contains("first=t0 last=t39"),
        "values read out before a growing append must still read right:\n{out}"
    );
    let expected: String = (0..40).map(|i| format!("t{i}")).collect::<String>() + "tail";
    assert!(
        out.contains(&format!("joined={expected}")),
        "every element must still render through FOR EACH:\n{out}"
    );
    // 0+1+...+39 = 780, plus the 99 the tail carries.
    assert!(out.contains("total=879"), "scalar fields changed:\n{out}");
    assert!(out.contains("refetch=t0"), "re-fetch changed:\n{out}");
    assert!(
        out.contains("len=40 first-still=t0"),
        "an in-place-declined removeAt must leave a fetched value intact:\n{out}"
    );
}
