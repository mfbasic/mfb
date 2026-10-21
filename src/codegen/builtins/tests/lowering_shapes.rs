//! Which lowering a program receives, for the collection self-update arms and the
//! shapes around them that no other program in the corpus takes.
//!
//! Every row is a program, the function whose labels are read, the label families
//! that function MUST define, and the families it must NOT. A label family is the
//! stable name of a code shape (`inplace_mapvalues_unwind_*`); the trailing serial
//! churns, so the rows compare stems. The negative column is what turns a row from
//! "this lowered" into a contract: a self-update that declines must fall back to
//! the copying lowering (`replace_list_*`, `transform_call_*`, `merge_*`), never
//! emit a half of the in-place one.
//!
//! The reason on each row is the property it pins; the programs are small and each
//! is aimed at one decision in `collection/assign/**`, `collection/list/**`,
//! `collection/map/**` or `collection/compare/**`.

use std::collections::BTreeSet;

use super::stems::stems_in;

struct Row {
    name: &'static str,
    source: &'static str,
    function: &'static str,
    have: &'static [&'static str],
    lack: &'static [&'static str],
    why: &'static str,
}
const RW_DECLINE_OTHER: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  LET xs AS List OF String = ["a", "bb", "a"]
  MUT ys AS List OF String = ["q"]
  ys = collections::replace(xs, "a", "z")
  io::print(toString(len(ys)))
  RETURN 0
END FUNC
"####;
const RW_FIELD_NOT_LAST: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  names AS List OF String
  tail AS String
END TYPE

FUNC up(s AS String) AS String
  RETURN s & "!"
END FUNC

FUNC main() AS Integer
  MUT r AS Rec = Rec[names := ["a", "bb"], tail := "t"]
  r = WITH r { names := collections::replace(r.names, "a", "a longer replacement") }
  r = WITH r { names := collections::transform(r.names, up) }
  io::print(toString(len(r.names)))
  RETURN 0
END FUNC
"####;
const RW_FIELD_READS_OWNER: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  tail AS String
  names AS List OF String
END TYPE

FUNC main() AS Integer
  MUT r AS Rec = Rec[tail := "t", names := ["a", "bb"]]
  r = WITH r { names := collections::replace(r.names, r.tail, "a longer replacement") }
  r = WITH r { names := collections::replace(r.names, "a", r.tail) }
  io::print(toString(len(r.names)))
  RETURN 0
END FUNC
"####;
const RW_TRANSFORM_RETYPE: &str = r####"IMPORT collections
IMPORT io

FUNC label(n AS Integer) AS String
  RETURN toString(n)
END FUNC

FUNC relabel(xs AS List OF Integer) AS List OF String
  RETURN collections::transform(xs, label)
END FUNC

FUNC main() AS Integer
  io::print(toString(len(relabel([1, 2, 3]))))
  RETURN 0
END FUNC
"####;
const MV_STRING_FAIL: &str = r####"IMPORT collections
IMPORT io

FUNC tag(s AS String) AS String
  IF len(s) = 0 THEN FAIL error(1, "empty")
  RETURN s & "!"
END FUNC

FUNC main() AS Integer
  MUT m AS Map OF String TO String = Map OF String TO String { "a" := "x", "b" := "yy" }
  m = collections::mapValues(m, tag)
  io::print(toString(len(m)))
  RETURN 0
END FUNC
"####;
const MV_INTEGER_FAIL: &str = r####"IMPORT collections
IMPORT io

FUNC halve(n AS Integer) AS Integer
  IF n < 0 THEN FAIL error(1, "neg")
  RETURN n / 2
END FUNC

FUNC main() AS Integer
  MUT m AS Map OF String TO Integer = Map OF String TO Integer { "a" := 4, "b" := 6 }
  m = collections::mapValues(m, halve)
  io::print(toString(len(m)))
  RETURN 0
END FUNC
"####;
const MERGE_ALIAS_AND_STRINGS: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT m AS Map OF String TO String = Map OF String TO String { "a" := "x" }
  LET n AS Map OF String TO String = Map OF String TO String { "a" := "longer", "c" := "zzz" }
  m = collections::merge(m, m, TRUE)
  m = collections::merge(m, n, TRUE)
  m = collections::merge(m, n, FALSE)
  io::print(toString(len(m)))
  RETURN 0
END FUNC
"####;
const MERGE_FIELD: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  m AS Map OF String TO String
  tail AS String
END TYPE

TYPE Rec2
  tail AS String
  m AS Map OF String TO String
END TYPE

FUNC main() AS Integer
  MUT r AS Rec = Rec[m := Map OF String TO String { "a" := "x" }, tail := "t"]
  MUT s AS Rec2 = Rec2[tail := "t", m := Map OF String TO String { "a" := "x" }]
  LET n AS Map OF String TO String = Map OF String TO String { "a" := "longer", "c" := "zzz" }
  r = WITH r { m := collections::merge(r.m, n, TRUE) }
  s = WITH s { m := collections::merge(s.m, n, TRUE) }
  s = WITH s { m := collections::merge(s.m, s.m, TRUE) }
  io::print(toString(len(r.m) + len(s.m)))
  RETURN 0
END FUNC
"####;
const MV_FIELD: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  m AS Map OF String TO String
  tail AS String
END TYPE

TYPE Rec2
  tail AS String
  m AS Map OF String TO String
END TYPE

FUNC tag(s AS String) AS String
  RETURN s & "!"
END FUNC

FUNC main() AS Integer
  MUT r AS Rec = Rec[m := Map OF String TO String { "a" := "x" }, tail := "t"]
  MUT s AS Rec2 = Rec2[tail := "t", m := Map OF String TO String { "a" := "x" }]
  r = WITH r { m := collections::mapValues(r.m, tag) }
  s = WITH s { m := collections::mapValues(s.m, tag) }
  io::print(toString(len(r.m) + len(s.m)))
  RETURN 0
END FUNC
"####;
const REPEAT_STRINGS: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT xs AS List OF String = ["a", "bb"]
  xs = collections::repeat(xs, "zzzz", 3)
  xs = collections::repeat(xs, "q", 2, 1)
  xs = collections::repeat(xs, "tail", 4, 5)
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
"####;
const REPEAT_LISTS: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT xs AS List OF List OF Integer = [[1], [2, 3]]
  xs = collections::repeat(xs, [7, 8, 9], 3)
  xs = collections::repeat(xs, [4], 2, 2)
  xs = collections::repeat(xs, [5, 5], 6, 5)
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
"####;
const REPEAT_FIELD: &str = r####"IMPORT collections
IMPORT io

TYPE Rec
  xs AS List OF String
  tail AS String
END TYPE

TYPE Rec2
  tail AS String
  xs AS List OF String
END TYPE

FUNC main() AS Integer
  MUT r AS Rec = Rec[xs := ["a"], tail := "t"]
  MUT s AS Rec2 = Rec2[tail := "t", xs := ["a"]]
  r = WITH r { xs := collections::repeat(r.xs, "zzzzzzzz", 5) }
  s = WITH s { xs := collections::repeat(s.xs, "zzzzzzzz", 5) }
  s = WITH s { xs := collections::repeat(s.xs, s.tail, 5, 1) }
  io::print(toString(len(r.xs) + len(s.xs)))
  RETURN 0
END FUNC
"####;
const REPEAT_VALUE_FORMS: &str = r####"IMPORT collections
IMPORT io

FUNC grow(xs AS List OF String) AS List OF String
  RETURN collections::repeat(xs, "pad", 3, 0)
END FUNC

FUNC main() AS Integer
  LET a AS List OF String = grow(["x"])
  LET b AS List OF List OF Integer = collections::repeat([[1]], [2, 2], 4)
  io::print(toString(len(a) + len(b)))
  RETURN 0
END FUNC
"####;
const REPEAT_GRAPH: &str = r####"IMPORT collections
IMPORT io

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

FUNC main() AS Integer
  MUT ts AS List OF Tree = [Leaf[v := 1]]
  ts = collections::repeat(ts, Node[kids := [Leaf[v := 2]]], 3)
  ts = collections::repeat(ts, Leaf[v := 9], 2, 1)
  io::print(toString(len(ts)))
  RETURN 0
END FUNC
"####;
const RECORD_TEMPS: &str = r####"IMPORT collections
IMPORT io

TYPE Body
  x AS Float
  vx AS Float
END TYPE

FUNC main() AS Integer
  MUT xs AS List OF Body = [Body[x := 1.0, vx := 2.0], Body[x := 3.0, vx := 4.0]]
  MUT n AS Integer = 0
  FOR j = 0 TO 1
    LET a AS Body = collections::get(xs, 0)
    LET b AS Body = collections::get(xs, 1)
    IF j = 0 THEN
      xs = collections::set(xs, 0, WITH a { x := a.x + b.vx })
    ELSE
      xs = collections::set(xs, 1, WITH b { x := b.x + a.vx })
    END IF
    MATCH j
      CASE 0
        xs = collections::set(xs, 0, WITH a { vx := a.vx + 1.0 })
      CASE ELSE
        xs = collections::set(xs, 1, WITH b { vx := b.vx + 1.0 })
    END MATCH
  NEXT
  WHILE n < 2
    LET a AS Body = collections::get(xs, 0)
    LET b AS Body = collections::get(xs, 1)
    xs = collections::set(xs, 0, WITH a { x := a.x + b.vx })
    n = n + 1
  END WHILE
  DO
    LET a AS Body = collections::get(xs, 0)
    LET b AS Body = collections::get(xs, 1)
    xs = collections::set(xs, 0, WITH a { x := a.x + b.vx })
    n = n + 1
  LOOP UNTIL n > 3
  FOR EACH item IN xs
    LET a AS Body = collections::get(xs, 0)
    LET b AS Body = collections::get(xs, 1)
    xs = collections::set(xs, 1, WITH b { x := b.x + a.vx })
  NEXT
  xs = collections::append(xs, Body[x := 0.0, vx := 0.0]) TRAP(e)
    RECOVER xs
  END TRAP
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
"####;
const SELF_SLICE_APPEND: &str = r####"IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT xs AS List OF Integer = [1, 2, 3, 4, 5]
  LET a AS Integer = 1
  LET b AS Integer = 3
  xs = collections::append(xs, collections::mid(xs, a + 1, b - a))
  xs = collections::append(xs, collections::mid(xs, 0, 2))
  xs = collections::append(xs, collections::mid(xs, a * 2, 1))
  xs = collections::append(xs, collections::mid(xs, len(xs) - 1, 1))
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
"####;
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
        name: "rw_decline_other",
        source: RW_DECLINE_OTHER,
        function: "main",
        have: &["replace_list_copy_loop"],
        lack: &["inplace_replace"],
        why: "a replace whose result is bound to ANOTHER local is not a self-update, so it copies",
    },
    Row {
        name: "rw_field_not_last",
        source: RW_FIELD_NOT_LAST,
        function: "main",
        have: &["replace_list_copy_loop", "transform_call_loop"],
        lack: &["inplace_replace", "inplace_transform"],
        why: "replace/transform may repack the list, which at a field is only sound for the last inlined field",
    },
    Row {
        name: "rw_field_reads_owner",
        source: RW_FIELD_READS_OWNER,
        function: "main",
        have: &["replace_list_copy_loop"],
        lack: &["inplace_replace"],
        why: "an operand that reads the owner may be a view the repack frees, so the arm declines",
    },
    Row {
        name: "rw_transform_retype",
        source: RW_TRANSFORM_RETYPE,
        function: "relabel",
        have: &["transform_call_loop"],
        lack: &["inplace_transform"],
        why: "RETURN transform(x, f) to ANOTHER element type cannot rewrite x in place",
    },
    Row {
        name: "mv_string_fail",
        source: MV_STRING_FAIL,
        function: "main",
        have: &["inplace_mapvalues_unwind", "inplace_mapvalues_room_loop"],
        lack: &[],
        why: "a String-valued mapValues parks its results, reserves room, and unwinds on failure",
    },
    Row {
        name: "mv_integer_fail",
        source: MV_INTEGER_FAIL,
        function: "main",
        have: &["inplace_mapvalues_loop"],
        lack: &["inplace_mapvalues_unwind", "inplace_mapvalues_room"],
        why: "an Integer-valued mapValues owns nothing to unwind and needs no room",
    },
    Row {
        name: "merge_alias_and_strings",
        source: MERGE_ALIAS_AND_STRINGS,
        function: "main",
        have: &["inplace_merge_loop", "inplace_borrow_str_done"],
        lack: &[],
        why: "a String-valued merge borrows the other map's payloads into scratch",
    },
    Row {
        name: "merge_field",
        source: MERGE_FIELD,
        function: "main",
        have: &["inplace_merge_loop", "merge_loop"],
        lack: &[],
        why: "merge at the last inlined field is in place; at an earlier field it copies",
    },
    Row {
        name: "mv_field",
        source: MV_FIELD,
        function: "main",
        have: &["inplace_mapvalues_loop"],
        lack: &[],
        why: "mapValues at the last inlined field is in place",
    },
    Row {
        name: "repeat_strings",
        source: REPEAT_STRINGS,
        function: "main",
        have: &["list_repeat_write", "list_repeat_realloc"],
        lack: &[],
        why: "repeat over variable-width elements reserves once, then writes",
    },
    Row {
        name: "repeat_lists",
        source: REPEAT_LISTS,
        function: "main",
        have: &["list_repeat_write", "list_repeat_realloc"],
        lack: &[],
        why: "repeat over nested lists reserves once, then writes",
    },
    Row {
        name: "repeat_field",
        source: REPEAT_FIELD,
        function: "main",
        have: &["list_repeat_write", "inline_grow_prefix_done"],
        lack: &[],
        why: "repeat at the last inlined field grows the owner",
    },
    Row {
        name: "repeat_value_forms",
        source: REPEAT_VALUE_FORMS,
        function: "grow",
        have: &["list_repeat_write"],
        lack: &[],
        why: "the value form of repeat shares the in-place core",
    },
    Row {
        name: "repeat_graph",
        source: REPEAT_GRAPH,
        function: "main",
        have: &["list_repeat_write", "graph_value_drop_skip", "shallow_block_free_skip"],
        lack: &[],
        why: "repeat of a cycle-typed element builds private copies, then frees only their list block",
    },
    Row {
        name: "record_temps",
        source: RECORD_TEMPS,
        function: "main",
        have: &["su_scratch_alloc_ok", "set_inplace_valid"],
        lack: &[],
        why: "a set(xs, i, WITH ...) loop builds its WITH items in the record-temp scratch",
    },
    Row {
        name: "self_slice_append",
        source: SELF_SLICE_APPEND,
        function: "main",
        have: &["self_slice_append_data_done"],
        lack: &[],
        why: "append(xs, mid(xs, a + 1, b - a)) is a self-slice append over fixed-width elements",
    },
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

fn defines(stems: &BTreeSet<String>, family: &str) -> bool {
    stems.iter().any(|stem| stem.starts_with(family))
}

#[test]
fn each_program_takes_the_lowering_its_row_names() {
    let mut failures = Vec::new();
    for row in ROWS {
        let stems = stems_in(row.source, row.function);
        for family in row.have {
            if !defines(&stems, family) {
                failures.push(format!(
                    "{}: `{}` defines no `{family}*` label ({})",
                    row.name, row.function, row.why
                ));
            }
        }
        for family in row.lack {
            if defines(&stems, family) {
                failures.push(format!(
                    "{}: `{}` defines a `{family}*` label it must not ({})",
                    row.name, row.function, row.why
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
