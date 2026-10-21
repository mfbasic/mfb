//! The rest of the lowering-shape rows (see `lowering_shapes.rs` for what a row
//! is): record fields that are or are not their owner's last inlined field, set
//! algebra, compaction, sort, `WITH` over a cycle-typed field, borrowed field
//! chains, and thread sends of collections, unions and recursive values.

use super::lowering_shapes::{assert_rows, Row};

const APPEND_SELF_ALIAS: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT xs AS List OF Integer = [1, 2]
  xs = collections::append(xs, xs)
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
"####;
const COMPARE_NOTHING: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT s AS Set OF Nothing = Set OF Nothing { Nothing }
  s = collections::add(s, Nothing)
  MUT n AS Integer = len(s)
  IF collections::contains(s, Nothing) THEN
    n = n + 1
  END IF
  MUT m AS Map OF Nothing TO Integer = Map OF Nothing TO Integer { Nothing := 1 }
  m = collections::set(m, Nothing, 2)
  n = n + len(m) + collections::getOr(m, Nothing, 0)
  LET xs AS List OF Nothing = [Nothing, Nothing]
  n = n + collections::find(xs, Nothing)
  IF collections::contains(xs, Nothing) THEN
    n = n + 1
  END IF
  io::print(toString(n))
  RETURN 0
END FUNC
"####;
const COMPARE_LIST_PAYLOADS: &str = r####"IMPORT collections
IMPORT io

TYPE Pt
  x AS Integer
  y AS Integer
END TYPE

TYPE Dot
  x AS Integer
END TYPE

TYPE Tag
  name AS String
END TYPE

UNION Shape
  Dot
  Tag
END UNION

FUNC main() AS Integer
  LET ps AS List OF Pt = [Pt[1, 2], Pt[3, 4]]
  LET ss AS List OF Shape = [Dot[x := 1], Tag[name := "t"]]
  MUT n AS Integer = 0
  IF collections::contains(ps, Pt[3, 4]) THEN
    n = n + 1
  END IF
  n = n + collections::find(ps, Pt[3, 4])
  MUT vs AS List OF Pt = ps
  vs = collections::replace(vs, Pt[1, 2], Pt[0, 0])
  io::print(toString(n + len(vs) + len(ss)))
  RETURN 0
END FUNC
"####;
const SHRINK_STRINGS_LOCAL: &str = r####"IMPORT collections
IMPORT io

FUNC keep(s AS String) AS Boolean
  RETURN len(s) > 1
END FUNC

FUNC main() AS Integer
  MUT xs AS List OF String = ["a", "bb", "bb", "ccc"]
  xs = collections::distinct(xs)
  xs = collections::filter(xs, keep)
  xs = collections::take(xs, 1)
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
"####;
const SHRINK_FIELD_NOT_LAST: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  names AS List OF String
  tail AS String
END TYPE

FUNC keep(s AS String) AS Boolean
  RETURN len(s) > 1
END FUNC

FUNC main() AS Integer
  MUT r AS Rec = Rec[names := ["a", "bb", "bb", "ccc"], tail := "t"]
  r = WITH r { names := collections::take(r.names, 3) }
  r = WITH r { names := collections::drop(r.names, 1) }
  r = WITH r { names := collections::filter(r.names, keep) }
  r = WITH r { names := collections::distinct(r.names) }
  io::print(toString(len(r.names)))
  RETURN 0
END FUNC
"####;
const SORT_COPYING_FORMS: &str = r####"IMPORT collections
IMPORT io

FUNC keyLen(s AS String) AS Integer
  RETURN len(s)
END FUNC

FUNC mk() AS List OF String
  RETURN ["bb", "a", "ccc"]
END FUNC

FUNC main() AS Integer
  LET a AS List OF String = collections::sort(mk())
  LET b AS List OF String = collections::sortBy(mk(), keyLen)
  LET src AS List OF String = ["z", "y"]
  LET c AS List OF String = collections::sort(src)
  LET d AS List OF Integer = collections::sort([3, 2, 1])
  io::print(toString(len(a) + len(b) + len(c) + len(d)))
  RETURN 0
END FUNC
"####;
const SET_ALGEBRA_ALIASES: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT s AS Set OF Integer = Set OF Integer { 1, 2, 3 }
  s = collections::union(s, s)
  s = collections::intersection(s, s)
  s = collections::difference(s, s)
  MUT t AS Set OF Integer = Set OF Integer { 4, 5 }
  t = collections::symmetricDifference(t, t)
  io::print(toString(len(s) + len(t)))
  RETURN 0
END FUNC
"####;
const SET_ALGEBRA_RECORDS: &str = r####"IMPORT collections
IMPORT io

TYPE Pt
  x AS Integer
  y AS Integer
END TYPE

ENUM Colour
  Red, Green, Blue
END ENUM

FUNC main() AS Integer
  MUT a AS Set OF Pt = Set OF Pt { Pt[1, 2], Pt[3, 4] }
  LET b AS Set OF Pt = Set OF Pt { Pt[3, 4], Pt[5, 6] }
  a = collections::difference(a, b)
  a = collections::symmetricDifference(a, b)
  a = collections::union(a, b)
  a = collections::intersection(a, b)
  MUT c AS Set OF Colour = Set OF Colour { Colour.Red, Colour.Green }
  LET d AS Set OF Colour = Set OF Colour { Colour.Green, Colour.Blue }
  c = collections::difference(c, d)
  c = collections::symmetricDifference(c, d)
  MUT m AS Set OF Money = Set OF Money { 1.00m, 2.00m }
  LET n AS Set OF Money = Set OF Money { 2.00m, 3.00m }
  m = collections::difference(m, n)
  m = collections::symmetricDifference(m, n)
  io::print(toString(len(a) + len(c) + len(m)))
  RETURN 0
END FUNC
"####;
const SET_ALGEBRA_STRINGS: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT a AS Set OF String = Set OF String { "alpha", "beta" }
  LET b AS Set OF String = Set OF String { "beta", "gamma-with-a-longer-name" }
  a = collections::union(a, b)
  a = collections::difference(a, b)
  a = collections::symmetricDifference(a, b)
  a = collections::intersection(a, b)
  io::print(toString(len(a)))
  RETURN 0
END FUNC
"####;
const SET_ALGEBRA_FIELD: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  names AS Set OF String
  tail AS String
END TYPE

TYPE RecLast
  tail AS String
  names AS Set OF String
END TYPE

FUNC main() AS Integer
  MUT r AS Rec = Rec[names := Set OF String { "a", "b" }, tail := "t"]
  MUT q AS RecLast = RecLast[tail := "t", names := Set OF String { "a", "b" }]
  LET o AS Set OF String = Set OF String { "b", "c" }
  r = WITH r { names := collections::union(r.names, o) }
  q = WITH q { names := collections::union(q.names, o) }
  q = WITH q { names := collections::symmetricDifference(q.names, o) }
  q = WITH q { names := collections::difference(q.names, q.names) }
  r = WITH r { names := collections::symmetricDifference(r.names, r.names) }
  q = WITH q { names := collections::union(q.names, Set OF String { q.tail }) }
  io::print(toString(len(r.names) + len(q.names)))
  RETURN 0
END FUNC
"####;
const FIELD_DECLINES: &str = r####"IMPORT collections
IMPORT io

TYPE First
  xs AS List OF String
  s AS String
END TYPE

TYPE LastList
  s AS String
  xs AS List OF String
END TYPE

TYPE LastSet
  s AS String
  names AS Set OF String
END TYPE

TYPE LastMap
  s AS String
  m AS Map OF String TO String
END TYPE

FUNC main() AS Integer
  MUT f AS First = First[xs := ["a", "b"], s := "t"]
  MUT l AS LastList = LastList[s := "t", xs := ["a", "b"]]
  MUT n AS LastSet = LastSet[s := "t", names := Set OF String { "n" }]
  MUT m AS LastMap = LastMap[s := "t", m := Map OF String TO String { "k" := "v" }]
  l = WITH l { xs := collections::append(l.xs, l.xs) }
  l = WITH l { xs := collections::append(l.xs, l.s) }
  f = WITH f { xs := collections::set(f.xs, 0, "q") }
  l = WITH l { xs := collections::set(l.xs, 0, l.s) }
  f = WITH f { xs := collections::insert(f.xs, 0, "q") }
  l = WITH l { xs := collections::insert(l.xs, 0, l.s) }
  l = WITH l { xs := collections::prepend(l.xs, l.s) }
  n = WITH n { names := collections::add(n.names, n.s) }
  m = WITH m { m := collections::set(m.m, m.s, "v") }
  m = WITH m { m := collections::set(m.m, "k2", m.s) }
  io::print(toString(len(f.xs) + len(l.xs) + len(n.names) + len(m.m)))
  RETURN 0
END FUNC
"####;
const FIELD_SETS_GROW: &str = r####"IMPORT collections
IMPORT io

TYPE LastList
  s AS String
  xs AS List OF String
END TYPE

TYPE LastSet
  s AS String
  names AS Set OF String
END TYPE

TYPE LastMap
  s AS String
  m AS Map OF String TO String
END TYPE

FUNC main() AS Integer
  MUT l AS LastList = LastList[s := "t", xs := ["a", "b"]]
  MUT n AS LastSet = LastSet[s := "t", names := Set OF String { "n" }]
  MUT m AS LastMap = LastMap[s := "t", m := Map OF String TO String { "k" := "v" }]
  l = WITH l { xs := collections::append(l.xs, "an appended element") }
  l = WITH l { xs := collections::append(l.xs, ["bulk", "elements"]) }
  l = WITH l { xs := collections::set(l.xs, 0, "a much longer replacement string") }
  l = WITH l { xs := collections::insert(l.xs, 1, "inserted") }
  l = WITH l { xs := collections::prepend(l.xs, "first") }
  n = WITH n { names := collections::add(n.names, "another name") }
  n = WITH n { names := collections::remove(n.names, "n") }
  m = WITH m { m := collections::set(m.m, "k2", "a longer value than before") }
  m = WITH m { m := collections::removeKey(m.m, "k") }
  io::print(toString(len(l.xs) + len(n.names) + len(m.m)))
  RETURN 0
END FUNC
"####;
const WITH_GRAPH_FIELD: &str = r####"IMPORT io

TYPE Leaf
  v AS Integer
END TYPE

TYPE Node
  kids AS List OF Tree
  n AS Integer
END TYPE

UNION Tree
  Node
  Leaf
END UNION

TYPE Holder
  tree AS Tree
  n AS Integer
END TYPE

FUNC main() AS Integer
  LET h AS Holder = Holder[tree := Node[kids := [Leaf[v := 1]], n := 1], n := 1]
  LET h2 AS Holder = WITH h { n := 2 }
  LET k AS Node = Node[kids := [Leaf[v := 5]], n := 3]
  LET k2 AS Node = WITH k { n := 4 }
  io::print(toString(h2.n + k2.n))
  RETURN 0
END FUNC
"####;
const BORROWED_FIELD_CHAIN: &str = r####"IMPORT collections
IMPORT io

TYPE Inner
  v AS Integer
  s AS String
END TYPE

TYPE Mid
  inner AS Inner
  tag AS String
END TYPE

TYPE Outer
  mid AS Mid
  n AS Integer
END TYPE

FUNC main() AS Integer
  LET xs AS List OF Outer = [Outer[mid := Mid[inner := Inner[v := 1, s := "a"], tag := "t"], n := 2]]
  LET t AS String = collections::get(xs, 0).mid.inner.s
  LET v AS Integer = collections::get(xs, 0).mid.inner.v
  LET m AS Mid = collections::get(xs, 0).mid
  io::print(t & toString(v) & m.tag)
  RETURN 0
END FUNC
"####;
const THREAD_SEND_GRAPHS: &str = r####"IMPORT collections
IMPORT io
IMPORT thread

TYPE Dot
  x AS Integer
END TYPE

TYPE Tag
  name AS String
END TYPE

UNION Shape
  Dot
  Tag
END UNION

TYPE Holder
  shape AS Shape
  label AS String
END TYPE

TYPE Leaf
  v AS Integer
END TYPE

TYPE Node
  kids AS List OF Tree
END TYPE

UNION Tree
  Node
  Leaf
END UNION

ISOLATED FUNC takeNested(w AS ThreadWorker OF List OF List OF String TO Integer, n AS Integer) AS Integer
  LET m AS List OF List OF String = thread::receive(w)
  RETURN len(m)
END FUNC

ISOLATED FUNC takeMap(w AS ThreadWorker OF Map OF String TO List OF String TO Integer, n AS Integer) AS Integer
  LET m AS Map OF String TO List OF String = thread::receive(w)
  RETURN len(m)
END FUNC

ISOLATED FUNC takeShapes(w AS ThreadWorker OF List OF Shape TO Integer, n AS Integer) AS Integer
  LET m AS List OF Shape = thread::receive(w)
  RETURN len(m)
END FUNC

ISOLATED FUNC takeHolders(w AS ThreadWorker OF List OF Holder TO Integer, n AS Integer) AS Integer
  LET m AS List OF Holder = thread::receive(w)
  RETURN len(m)
END FUNC

ISOLATED FUNC takeHolder(w AS ThreadWorker OF Holder TO Integer, n AS Integer) AS Integer
  LET m AS Holder = thread::receive(w)
  RETURN len(m.label)
END FUNC

ISOLATED FUNC takeTrees(w AS ThreadWorker OF List OF Tree TO Integer, n AS Integer) AS Integer
  LET m AS List OF Tree = thread::receive(w)
  RETURN len(m)
END FUNC

ISOLATED FUNC takeTree(w AS ThreadWorker OF Tree TO Integer, n AS Integer) AS Integer
  LET m AS Tree = thread::receive(w)
  RETURN n
END FUNC

ISOLATED FUNC takeMapShapes(w AS ThreadWorker OF Map OF String TO Shape TO Integer, n AS Integer) AS Integer
  LET m AS Map OF String TO Shape = thread::receive(w)
  RETURN len(m)
END FUNC

FUNC main() AS Integer
  MUT total AS Integer = 0
  RES a AS Thread OF List OF List OF String TO Integer = thread::start(takeNested, 1)
  thread::send(a, [["x", "y"], ["z"]], 100)
  total = total + thread::waitFor(a)
  RES b AS Thread OF Map OF String TO List OF String TO Integer = thread::start(takeMap, 1)
  thread::send(b, Map OF String TO List OF String { "k" := ["v1", "v2"] }, 100)
  total = total + thread::waitFor(b)
  RES c AS Thread OF List OF Shape TO Integer = thread::start(takeShapes, 1)
  thread::send(c, [Dot[x := 1], Tag[name := "t"]], 100)
  total = total + thread::waitFor(c)
  RES d AS Thread OF List OF Holder TO Integer = thread::start(takeHolders, 1)
  thread::send(d, [Holder[shape := Dot[x := 1], label := "a"], Holder[shape := Tag[name := "t"], label := "bb"]], 100)
  total = total + thread::waitFor(d)
  RES e AS Thread OF Holder TO Integer = thread::start(takeHolder, 1)
  thread::send(e, Holder[shape := Tag[name := "t"], label := "bb"], 100)
  total = total + thread::waitFor(e)
  RES f AS Thread OF List OF Tree TO Integer = thread::start(takeTrees, 1)
  thread::send(f, [Leaf[v := 1], Node[kids := [Leaf[v := 2]]]], 100)
  total = total + thread::waitFor(f)
  RES g AS Thread OF Tree TO Integer = thread::start(takeTree, 1)
  thread::send(g, Node[kids := [Leaf[v := 3]]], 100)
  total = total + thread::waitFor(g)
  RES h AS Thread OF Map OF String TO Shape TO Integer = thread::start(takeMapShapes, 1)
  thread::send(h, Map OF String TO Shape { "k" := Tag[name := "t"] }, 100)
  total = total + thread::waitFor(h)
  io::print(toString(total))
  RETURN 0
END FUNC
"####;
const ROWS: &[Row] = &[
    Row {
        name: "append_self_alias",
        source: APPEND_SELF_ALIAS,
        function: "main",
        have: &[],
        lack: &["bulk_append_", "append_inplace"],
        why: "append(xs, xs) must not take a path that frees the buffer it is still reading",
    },
    Row {
        name: "compare_nothing",
        source: COMPARE_NOTHING,
        function: "main",
        have: &["contains_loop", "list_find_item_loop", "map_get_or_default"],
        lack: &[],
        why: "the unit type has one value, so every payload of it matches",
    },
    Row {
        name: "compare_list_payloads",
        source: COMPARE_LIST_PAYLOADS,
        function: "main",
        have: &["compare_record_next_field", "contains_loop", "inplace_replace_count"],
        lack: &[],
        why: "record elements compare field by field; replace over them counts matches first",
    },
    Row {
        name: "shrink_strings_local",
        source: SHRINK_STRINGS_LOCAL,
        function: "main",
        have: &["inplace_distinct_loop", "inplace_filter_loop", "inplace_take_store"],
        lack: &[],
        why: "distinct/filter/take over variable-width elements compact in place",
    },
    Row {
        name: "shrink_field_not_last",
        source: SHRINK_FIELD_NOT_LAST,
        function: "main",
        have: &["filter_call_loop"],
        lack: &["inplace_filter", "inplace_take", "inplace_distinct"],
        why: "a compaction that may repack declines at a field that is not the last inlined one",
    },
    Row {
        name: "sort_copying_forms",
        source: SORT_COPYING_FORMS,
        function: "main",
        have: &["inplace_sort_merge", "inplace_sortby_keys"],
        lack: &[],
        why: "the copying sort of a call result or a bound list runs the in-place sort core",
    },
    Row {
        name: "set_algebra_aliases",
        source: SET_ALGEBRA_ALIASES,
        function: "main",
        have: &[],
        lack: &["inplace_claim", "inplace_add_all", "inplace_intersection_mark"],
        why: "union/intersection/difference/symmetricDifference of a set with itself touch no members",
    },
    Row {
        name: "set_algebra_records",
        source: SET_ALGEBRA_RECORDS,
        function: "main",
        have: &["inplace_claim_loop", "inplace_intersection_mark_loop", "inplace_add_all_loop"],
        lack: &[],
        why: "set algebra over record, enum and money elements (no hash probe) claims by scan",
    },
    Row {
        name: "set_algebra_strings",
        source: SET_ALGEBRA_STRINGS,
        function: "main",
        have: &["inplace_add_all_loop", "inplace_claim_loop", "inplace_borrow_str_done"],
        lack: &[],
        why: "set algebra over String elements borrows the other set's payloads",
    },
    Row {
        name: "set_algebra_field",
        source: SET_ALGEBRA_FIELD,
        function: "main",
        have: &["inline_grow_prefix_done", "inplace_add_all_loop"],
        lack: &[],
        why: "set algebra at the last inlined field grows the owner",
    },
    Row {
        name: "field_declines",
        source: FIELD_DECLINES,
        function: "main",
        have: &[],
        lack: &["inline_grow_prefix"],
        why: "every grow at a field that is not last, or whose operand views the owner, declines to the copying form",
    },
    Row {
        name: "field_sets_grow",
        source: FIELD_SETS_GROW,
        function: "main",
        have: &["inline_grow_prefix_done"],
        lack: &[],
        why: "growing self-updates at the last inlined field go through InlineGrow",
    },
    Row {
        name: "with_graph_field",
        source: WITH_GRAPH_FIELD,
        function: "main",
        have: &["graph_value_drop_skip"],
        lack: &[],
        why: "WITH over a record that keeps a cycle-typed field gives the rebuilt record its own copy",
    },
    Row {
        name: "borrowed_field_chain",
        source: BORROWED_FIELD_CHAIN,
        function: "main",
        have: &["list_get_invalid"],
        lack: &[],
        why: "a field chain over get(...) reads through the borrowed element",
    },
    Row {
        name: "thread_send_graphs",
        source: THREAD_SEND_GRAPHS,
        function: "main",
        have: &["runtime_thread_send_ok"],
        lack: &[],
        why: "a collection / union / recursive value crosses a thread by a per-payload copy",
    },
];

#[test]
fn each_program_takes_the_lowering_its_row_names() {
    assert_rows(ROWS);
}
