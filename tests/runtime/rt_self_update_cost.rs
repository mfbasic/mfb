//! bug-732: six in-place self-updates cost the size of the collection, not the
//! work they do.
//!
//! bug-652 made the owner's rule explicit: a `MUT` self-update `x = f(x, …)` costs
//! the elements it adds, removes or moves (or, for an operation that must test
//! every element, the elements it tests) — never the size of the part of `x` it
//! leaves alone. Its audit found six forms that did O(1) work per call and still
//! walked all of `x`, so every loop over them was O(n²):
//!
//! 1. `removeAt` on a variable-width list fixed up every entry's offset;
//! 2. `removeKey` / a `Set`'s `remove` scanned for the key, and dropped the hash
//!    index so the next probe rebuilt it;
//! 3. `difference` marked every element of `x`;
//! 4. `symmetricDifference` marked every element of `x`;
//! 5. the `String` window family copied the kept bytes onto themselves, and
//!    `trimEnd` scanned the whole string forward;
//! 6. `take`/`mid` truncating a variable-width list whose payloads are out of
//!    entry order walked every entry to count live bytes.
//!
//! **The measure** is bug-652's (`rt_list_take_drop_in_place.rs`): each shape runs
//! the same total work as `ROUNDS` builds of `n` and as one build of `n * ROUNDS`.
//! Linear work costs the same both ways (×1); a per-call cost proportional to the
//! collection's size costs ×`ROUNDS` more on the one long build. The ×3 bound sits
//! between. Time is the child's own CPU time (`wait4`), never wall time — this box
//! runs at load averages where wall time is meaningless. Every program prints a
//! result derived from the collection, so a fast path that loses or corrupts
//! elements fails too.
//!
//! The value program pins what the fixes must not change: the results (checked by
//! hand, and equal to the pre-fix compiler's), iteration order, the hash index
//! after removals (every surviving key is probed back), and copies/aliases staying
//! untouched. Built with `--debug` it must free every block it allocates. The
//! churn program pins the one memory bound the list fixes rely on: a list that
//! keeps removing payloads out of the data tail while it keeps adding others must
//! not grow without bound.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

const ROUNDS: u64 = 8;
const MAX_RATIO: f64 = 3.0;
const SLACK_SECONDS: f64 = 0.05;

/// One function per shape. Each takes `n` and returns `"<a> <b>"`, which
/// [`expected`] predicts; `main` runs a shape `rounds` times.
const SCALE_PROGRAM: &str = r#"IMPORT io
IMPORT os
IMPORT collections
IMPORT strings

FUNC removeAtLast(n AS Integer) AS String
  MUT x AS List OF String = []
  FOR i = 1 TO n
    x = collections::append(x, toString(i))
    x = collections::append(x, "gone")
    x = collections::removeAt(x, len(x) - 1)
  NEXT
  RETURN toString(len(x)) & " " & collections::get(x, len(x) - 1)
END FUNC

FUNC removeAtSorted(n AS Integer) AS String
  MUT x AS List OF String = []
  FOR i = 1 TO n
    x = collections::append(x, toString(n - i + 1))
  NEXT
  x = collections::sort(x)
  MUT total AS Integer = 0
  WHILE len(x) > 0
    total = total + toInt(collections::get(x, len(x) - 1))
    x = collections::removeAt(x, len(x) - 1)
  END WHILE
  RETURN toString(total) & " " & toString(n)
END FUNC

FUNC removeKeyLoop(n AS Integer) AS String
  MUT m AS Map OF String TO Integer = Map OF String TO Integer {}
  FOR i = 1 TO n
    m = collections::set(m, toString(i), i)
    m = collections::set(m, "tmp", 0)
    m = collections::removeKey(m, "tmp")
  NEXT
  RETURN toString(len(m)) & " " & toString(collections::get(m, toString(n)))
END FUNC

FUNC setRemoveLoop(n AS Integer) AS String
  MUT s AS Set OF Integer = Set OF Integer {}
  FOR i = 1 TO n
    s = collections::add(s, i)
    s = collections::add(s, -1)
    s = collections::remove(s, -1)
  NEXT
  RETURN toString(len(s)) & " " & toString(n)
END FUNC

FUNC differenceLoop(n AS Integer) AS String
  MUT s AS Set OF Integer = Set OF Integer {}
  LET miss AS Set OF Integer = Set OF Integer { -5 }
  LET hit AS Set OF Integer = Set OF Integer { -1 }
  FOR i = 1 TO n
    s = collections::add(s, i)
    s = collections::add(s, -1)
    s = collections::difference(s, miss)
    s = collections::difference(s, hit)
  NEXT
  RETURN toString(len(s)) & " " & toString(n)
END FUNC

FUNC symDiffLoop(n AS Integer) AS String
  MUT s AS Set OF Integer = Set OF Integer {}
  LET t AS Set OF Integer = Set OF Integer { -5 }
  FOR i = 1 TO n
    s = collections::add(s, i)
    s = collections::symmetricDifference(s, t)
  NEXT
  RETURN toString(len(s)) & " " & toString(n)
END FUNC

FUNC stripLoop(n AS Integer) AS String
  MUT s AS String = ""
  FOR i = 1 TO n
    s = s & "x"
    s = strings::stripSuffix(s, "q")
    s = strings::stripPrefix(s, "q")
  NEXT
  RETURN toString(len(s)) & " " & toString(n)
END FUNC

FUNC trimLoop(n AS Integer) AS String
  MUT s AS String = ""
  FOR i = 1 TO n
    s = s & "x "
    s = strings::trimEnd(s)
    s = strings::trim(s)
  NEXT
  RETURN toString(len(s)) & " " & toString(n)
END FUNC

FUNC popSorted(n AS Integer) AS String
  MUT x AS List OF String = []
  FOR i = 1 TO n
    x = collections::append(x, toString(n - i + 1))
  NEXT
  x = collections::sort(x)
  MUT total AS Integer = 0
  WHILE len(x) > 1
    total = total + toInt(collections::get(x, len(x) - 1))
    x = collections::take(x, len(x) - 1)
    total = total + toInt(collections::get(x, len(x) - 1))
    x = collections::mid(x, 0, len(x) - 1)
  END WHILE
  RETURN toString(total) & " " & toString(n)
END FUNC

FUNC popInserted(n AS Integer) AS String
  MUT x AS List OF String = []
  FOR i = 1 TO n
    x = collections::append(x, "a" & toString(i))
    x = collections::insert(x, len(x) - 1, toString(i))
    x = collections::take(x, len(x) - 1)
  NEXT
  RETURN toString(len(x)) & " " & collections::get(x, len(x) - 1)
END FUNC

FUNC appendLoop(n AS Integer) AS String
  MUT x AS List OF String = []
  FOR i = 1 TO n
    x = collections::append(x, toString(i))
  NEXT
  RETURN toString(len(x)) & " " & collections::get(x, len(x) - 1)
END FUNC

FUNC run(shape AS String, n AS Integer) AS String
  IF shape = "removeAtLast" THEN
    RETURN removeAtLast(n)
  ELSEIF shape = "removeAtSorted" THEN
    RETURN removeAtSorted(n)
  ELSEIF shape = "removeKey" THEN
    RETURN removeKeyLoop(n)
  ELSEIF shape = "setRemove" THEN
    RETURN setRemoveLoop(n)
  ELSEIF shape = "difference" THEN
    RETURN differenceLoop(n)
  ELSEIF shape = "symDiff" THEN
    RETURN symDiffLoop(n)
  ELSEIF shape = "strip" THEN
    RETURN stripLoop(n)
  ELSEIF shape = "trim" THEN
    RETURN trimLoop(n)
  ELSEIF shape = "popSorted" THEN
    RETURN popSorted(n)
  ELSEIF shape = "popInserted" THEN
    RETURN popInserted(n)
  ELSEIF shape = "append" THEN
    RETURN appendLoop(n)
  END IF
  RETURN "?"
END FUNC

FUNC main AS Integer
  LET args = os::args()
  LET shape = collections::get(args, 0)
  LET n = toInt(collections::get(args, 1))
  LET rounds = toInt(collections::get(args, 2))
  MUT last AS String = ""
  FOR r = 1 TO rounds
    last = run(shape, n)
  NEXT
  io::print(shape & " " & last)
  RETURN 0
END FUNC
"#;

/// Removals and windows over every path the fixes touch, with the copying path and
/// an alias beside each self-update.
const VALUE_PROGRAM: &str = r#"IMPORT io
IMPORT collections
IMPORT strings

TYPE Element
  name AS String
  children AS List OF Node
END TYPE

TYPE Text
  value AS String
END TYPE

UNION Node
  Element
  Text
END UNION

TYPE Holder
  tag AS Integer
  xs AS List OF String
END TYPE

TYPE Tagged
  xs AS List OF String
  name AS String
END TYPE

'GLOBAL-BEGIN
MUT gList AS List OF String = []
'GLOBAL-END

FUNC el(name AS String) AS Node
  RETURN Element[name := name, children := [Text[value := name & "-kid"]]]
END FUNC

FUNC tx(value AS String) AS Node
  RETURN Text[value := value]
END FUNC

FUNC showN(xs AS List OF Node) AS String
  MUT out AS List OF String = []
  FOR EACH n IN xs
    MATCH n
      CASE Element(e)
        out = collections::append(out, e.name & "/" & toString(len(e.children)))
      CASE Text(t)
        out = collections::append(out, "'" & t.value & "'")
    END MATCH
  NEXT
  RETURN "[" & strings::join(out, " ") & "]"
END FUNC

FUNC showS(xs AS List OF String) AS String
  RETURN "[" & strings::join(xs, " ") & "]"
END FUNC

FUNC showSet(s AS Set OF Integer) AS String
  MUT out AS List OF String = []
  FOR EACH v IN s
    out = collections::append(out, toString(v))
  NEXT
  RETURN "{" & strings::join(out, " ") & "}"
END FUNC

FUNC probeSet(s AS Set OF Integer, lo AS Integer, hi AS Integer) AS String
  MUT out AS String = ""
  FOR i = lo TO hi
    IF collections::contains(s, i) THEN
      out = out & "1"
    ELSE
      out = out & "0"
    END IF
  NEXT
  RETURN out
END FUNC

FUNC showM(m AS Map OF String TO Integer) AS String
  MUT out AS List OF String = []
  FOR EACH k IN collections::keys(m)
    out = collections::append(out, k & "=" & toString(collections::get(m, k)))
  NEXT
  RETURN "{" & strings::join(out, " ") & "}"
END FUNC

FUNC showMN(m AS Map OF String TO Node) AS String
  MUT out AS List OF String = []
  FOR EACH k IN collections::keys(m)
    out = collections::append(out, k & "=" & showN([collections::get(m, k)]))
  NEXT
  RETURN "{" & strings::join(out, " ") & "}"
END FUNC

FUNC showMI(m AS Map OF Integer TO String) AS String
  MUT out AS List OF String = []
  FOR EACH k IN collections::keys(m)
    out = collections::append(out, toString(k) & "=" & collections::get(m, k))
  NEXT
  RETURN "{" & strings::join(out, " ") & "}"
END FUNC

FUNC removeAtStrings() AS Nothing
  MUT x AS List OF String = ["a", "bb", "ccc", "dddd", "eeeee", "f"]
  x = collections::removeAt(x, len(x) - 1)
  io::print("ra last " & showS(x))
  x = collections::removeAt(x, 2)
  io::print("ra mid " & showS(x))
  x = collections::removeAt(x, 0)
  io::print("ra first " & showS(x))
  x = collections::append(x, "gg")
  x = collections::sort(x)
  x = collections::removeAt(x, len(x) - 1)
  io::print("ra sorted-last " & showS(x))
  x = collections::prepend(x, "front")
  x = collections::removeAt(x, 1)
  io::print("ra prepend-1 " & showS(x))
  x = collections::insert(x, 1, "ins")
  x = collections::removeAt(x, len(x) - 1)
  io::print("ra insert-last " & showS(x))
  x = collections::set(x, 0, "a-much-longer-front")
  x = collections::removeAt(x, 1)
  io::print("ra set-1 " & showS(x))
  FOR i = 1 TO 40
    x = collections::append(x, "n" & toString(i))
    x = collections::removeAt(x, len(x) - 2)
  NEXT
  io::print("ra churn " & showS(x))
  LET keep = x
  x = collections::removeAt(x, 0)
  io::print("ra alias " & showS(keep) & " " & showS(x))
  x = collections::append(x, "c1")
  x = collections::append(x, "c2")
  LET copy = collections::removeAt(x, 1)
  io::print("ra copy " & showS(x) & " " & showS(copy))
  WHILE len(x) > 0
    x = collections::removeAt(x, 0)
  END WHILE
  x = collections::append(x, "reuse")
  io::print("ra empty " & showS(x))
END FUNC

FUNC removeAtNodes() AS Nothing
  MUT x AS List OF Node = [el("a"), tx("b"), el("c"), tx("d"), el("e")]
  x = collections::removeAt(x, len(x) - 1)
  x = collections::removeAt(x, 1)
  io::print("rn " & showN(x))
  x = collections::prepend(x, el("front"))
  x = collections::insert(x, 2, tx("mid"))
  x = collections::removeAt(x, len(x) - 1)
  x = collections::removeAt(x, 0)
  io::print("rn reordered " & showN(x))
  FOR i = 1 TO 30
    x = collections::append(x, el("n" & toString(i)))
    x = collections::removeAt(x, len(x) - 2)
  NEXT
  io::print("rn churn " & showN(x))
  LET keep = x
  x = collections::removeAt(x, 0)
  io::print("rn alias " & showN(keep) & " " & showN(x))
END FUNC

FUNC truncations() AS Nothing
  MUT x AS List OF String = ["d", "bb", "a", "ccc", "eeeee", "ff"]
  x = collections::sort(x)
  x = collections::take(x, len(x) - 1)
  io::print("tr sort-pop " & showS(x))
  x = collections::mid(x, 0, len(x) - 2)
  io::print("tr sort-mid " & showS(x))
  x = collections::append(x, "zz")
  x = collections::insert(x, len(x) - 1, "yy")
  x = collections::take(x, len(x) - 1)
  io::print("tr insert-pop " & showS(x))
  FOR i = 1 TO 50
    x = collections::append(x, "p" & toString(i))
    x = collections::insert(x, len(x) - 1, "q" & toString(i))
    x = collections::take(x, len(x) - 1)
  NEXT
  io::print("tr churn " & toString(len(x)) & " " & collections::get(x, len(x) - 1) & " " & collections::get(x, 3))
  x = collections::sort(x)
  WHILE len(x) > 3
    x = collections::take(x, len(x) - 1)
  END WHILE
  io::print("tr pop-down " & showS(x))
  LET keep = x
  x = collections::take(x, 1)
  io::print("tr alias " & showS(keep) & " " & showS(x))
  MUT n AS List OF Node = [el("a"), tx("b"), el("c")]
  n = collections::prepend(n, el("front"))
  n = collections::insert(n, 1, tx("i"))
  n = collections::take(n, len(n) - 1)
  n = collections::take(n, 2)
  io::print("tr nodes " & showN(n))
  FOR i = 1 TO 20
    n = collections::append(n, el("x" & toString(i)))
    n = collections::insert(n, len(n) - 1, tx("y" & toString(i)))
    n = collections::take(n, len(n) - 1)
  NEXT
  io::print("tr nodes churn " & toString(len(n)) & " " & showN(collections::drop(n, len(n) - 2)))
END FUNC

FUNC maps() AS Nothing
  MUT m AS Map OF String TO Integer = Map OF String TO Integer {}
  FOR EACH k IN ["a", "b", "c", "d", "e", "f", "g", "h"]
    m = collections::set(m, k, len(k) + len(showS([k])))
  NEXT
  io::print("m built " & toString(collections::get(m, "a")) & " " & showM(m))
  m = collections::removeKey(m, "c")
  io::print("m -c " & showM(m) & " " & toString(collections::hasKey(m, "c")))
  m = collections::removeKey(m, "a")
  m = collections::removeKey(m, "h")
  m = collections::removeKey(m, "zz")
  io::print("m -a-h-zz " & showM(m))
  m = collections::set(m, "i", 9)
  m = collections::set(m, "d", 40)
  io::print("m +i d=40 " & showM(m))
  LET keep = m
  m = collections::removeKey(m, "b")
  io::print("m alias " & showM(keep) & " " & showM(m))
  MUT big AS Map OF String TO Integer = Map OF String TO Integer {}
  FOR i = 1 TO 300
    big = collections::set(big, "k" & toString(i), i)
  NEXT
  LET warm = collections::get(big, "k1")
  FOR i = 1 TO 300
    IF i MOD 3 = 0 THEN
      big = collections::removeKey(big, "k" & toString(i))
    END IF
  NEXT
  FOR i = 301 TO 330
    big = collections::set(big, "k" & toString(i), i)
  NEXT
  MUT sum AS Integer = 0
  MUT present AS Integer = 0
  FOR i = 1 TO 330
    IF collections::hasKey(big, "k" & toString(i)) THEN
      present = present + 1
      sum = sum + collections::get(big, "k" & toString(i))
    END IF
  NEXT
  LET ks = collections::keys(big)
  io::print("m big " & toString(len(big)) & " " & toString(present) & " " & toString(sum) & " " & collections::get(ks, 0) & " " & collections::get(ks, 5) & " " & collections::get(ks, len(ks) - 1))
  MUT mi AS Map OF Integer TO String = Map OF Integer TO String {}
  FOR i = 1 TO 6
    mi = collections::set(mi, i * 7, "v" & toString(i))
  NEXT
  mi = collections::removeKey(mi, 21)
  mi = collections::removeKey(mi, 7)
  io::print("mi " & showMI(mi) & " " & toString(collections::hasKey(mi, 21)))
  MUT mn AS Map OF String TO Node = Map OF String TO Node {}
  mn = collections::set(mn, "x", el("x"))
  mn = collections::set(mn, "y", tx("y"))
  mn = collections::set(mn, "z", el("z"))
  mn = collections::removeKey(mn, "x")
  mn = collections::removeKey(mn, "z")
  mn = collections::set(mn, "w", el("w"))
  io::print("mn " & showMN(mn))
END FUNC

FUNC sets() AS Nothing
  MUT s AS Set OF Integer = Set OF Integer {}
  FOR i = 1 TO 10
    s = collections::add(s, i)
  NEXT
  s = collections::remove(s, 5)
  s = collections::remove(s, 1)
  s = collections::remove(s, 10)
  s = collections::remove(s, 99)
  s = collections::add(s, 11)
  io::print("s remove " & showSet(s) & " " & probeSet(s, 0, 12))
  LET keep = s
  s = collections::remove(s, 2)
  io::print("s alias " & showSet(keep) & " " & showSet(s))
  MUT d AS Set OF Integer = Set OF Integer {1, 2, 3, 4, 5, 6, 7, 8}
  d = collections::difference(d, Set OF Integer {3, 99, 1})
  io::print("d " & showSet(d) & " " & probeSet(d, 0, 9))
  d = collections::difference(d, Set OF Integer {})
  d = collections::difference(d, Set OF Integer {8})
  io::print("d last " & showSet(d) & " " & probeSet(d, 0, 9))
  d = collections::add(d, 20)
  io::print("d add " & showSet(d) & " " & probeSet(d, 18, 21))
  LET dk = d
  d = collections::difference(d, Set OF Integer {2, 4, 5, 6, 7, 20, 30, 40, 50, 60})
  io::print("d all " & showSet(dk) & " " & showSet(d) & " " & toString(len(d)))
  d = collections::add(d, 3)
  d = collections::difference(d, d)
  io::print("d self " & showSet(d))
  MUT x AS Set OF Integer = Set OF Integer {1, 2, 3, 4}
  x = collections::symmetricDifference(x, Set OF Integer {3, 5, 1, 6})
  io::print("x " & showSet(x) & " " & probeSet(x, 0, 7))
  x = collections::symmetricDifference(x, Set OF Integer {6})
  x = collections::symmetricDifference(x, Set OF Integer {9})
  io::print("x toggle " & showSet(x) & " " & probeSet(x, 0, 10))
  LET xk = x
  x = collections::symmetricDifference(x, Set OF Integer {2, 9, 100})
  io::print("x alias " & showSet(xk) & " " & showSet(x) & " " & probeSet(x, 0, 10))
  MUT big AS Set OF Integer = Set OF Integer {}
  FOR i = 1 TO 200
    big = collections::add(big, i)
  NEXT
  LET warm = collections::contains(big, 1)
  MUT t AS Set OF Integer = Set OF Integer {}
  FOR i = 1 TO 200
    IF i MOD 7 = 0 THEN
      t = collections::add(t, i)
    END IF
  NEXT
  big = collections::difference(big, t)
  big = collections::symmetricDifference(big, Set OF Integer {1, 2, 500, 7})
  MUT c AS Integer = 0
  FOR i = 0 TO 600
    IF collections::contains(big, i) THEN
      c = c + i
    END IF
  NEXT
  io::print("big " & toString(len(big)) & " " & toString(c) & " " & showSet(collections::toSet(collections::take(collections::toList(big), 4))))
END FUNC

FUNC stringsWin() AS Nothing
  MUT s AS String = "hello world"
  s = strings::stripSuffix(s, "world")
  io::print("st <" & s & ">")
  s = strings::stripSuffix(s, "zz")
  s = strings::stripPrefix(s, "q")
  io::print("st nomatch <" & s & ">")
  s = strings::stripPrefix(s, "he")
  io::print("st prefix <" & s & ">")
  MUT t AS String = "  a b " & "\t" & "\n"
  t = strings::trimEnd(t)
  io::print("te <" & t & ">")
  t = strings::trimStart(t)
  io::print("ts <" & t & ">")
  MUT u AS String = "x" & "  　    "
  u = strings::trimEnd(u)
  io::print("tu <" & u & "> " & toString(len(u)))
  MUT v AS String = "  é ü 　"
  v = strings::trim(v)
  io::print("tv <" & v & "> " & toString(len(v)))
  MUT w AS String = "   "
  w = strings::trimEnd(w)
  io::print("tw <" & w & "> " & toString(len(w)))
  MUT y AS String = "abcdef"
  y = strings::left(y, 4)
  y = strings::mid(y, 0, 3)
  io::print("lm <" & y & ">")
  LET orig AS String = "keep this  "
  MUT z AS String = orig
  z = strings::trimEnd(z)
  LET c = strings::trimEnd(orig)
  io::print("alias <" & orig & "> <" & z & "> <" & c & ">")
  MUT g AS String = ""
  FOR i = 1 TO 30
    g = g & "ab "
    g = strings::trimEnd(g)
    g = strings::stripSuffix(g, "q")
  NEXT
  io::print("grow " & toString(len(g)) & " <" & strings::right(g, 6) & ">")
END FUNC

FUNC fields() AS Nothing
  MUT h AS Holder = Holder[tag := 1, xs := ["s1", "s2", "s3", "s4"]]
  FOR i = 1 TO 200
    h = WITH h { xs := collections::insert(h.xs, len(h.xs) - 1, "i" & toString(i)) }
    h = WITH h { xs := collections::take(h.xs, len(h.xs) - 1) }
  NEXT
  io::print("f insert-take " & toString(h.tag) & " " & showS(h.xs))
  FOR i = 1 TO 200
    h = WITH h { xs := collections::append(h.xs, "a" & toString(i)) }
    h = WITH h { xs := collections::removeAt(h.xs, len(h.xs) - 2) }
  NEXT
  io::print("f append-removeAt " & showS(h.xs))
  FOR i = 1 TO 100
    h = WITH h { xs := collections::append(h.xs, ["b" & toString(i), "c" & toString(i)]) }
    h = WITH h { xs := collections::removeAt(h.xs, len(h.xs) - 3) }
    h = WITH h { xs := collections::removeAt(h.xs, len(h.xs) - 3) }
  NEXT
  io::print("f bulk-removeAt " & showS(h.xs))
  MUT t AS Tagged = Tagged[xs := ["t1", "t2", "t3", "t4"], name := "tag"]
  FOR i = 1 TO 150
    t = WITH t { xs := collections::append(t.xs, "c" & toString(i)) }
    t = WITH t { xs := collections::removeAt(t.xs, len(t.xs) - 2) }
  NEXT
  io::print("f copy-grow " & t.name & " " & showS(t.xs))
  MUT hx AS List OF String = ["h1", "h2", "h3", "h4", "h5", "h6"]
  FOR i = 1 TO 4
    hx = collections::removeAt(hx, 0)
  NEXT
  LET hy = collections::insert(hx, 1, "mid")
  LET hz = collections::prepend(hx, "front")
  LET hw = collections::append(hx, ["x1", "x2"])
  io::print("f packed-copy " & showS(hx) & " " & showS(hy) & " " & showS(hz) & " " & showS(hw))
END FUNC

'GLOBAL-BEGIN
FUNC globals() AS Nothing
  gList = ["g1", "g2", "g3"]
  FOR i = 1 TO 200
    gList = collections::insert(gList, len(gList) - 1, "j" & toString(i))
    gList = collections::take(gList, len(gList) - 1)
    gList = collections::prepend(gList, "p" & toString(i))
    gList = collections::removeAt(gList, 1)
  NEXT
  io::print("g churn " & showS(gList))
END FUNC
'GLOBAL-END

FUNC mapReclaim() AS Nothing
  MUT m AS Map OF String TO Node = Map OF String TO Node {}
  MUT s AS Set OF String = Set OF String {}
  MUT w AS Map OF Integer TO String = Map OF Integer TO String {}
  FOR i = 1 TO 600
    m = collections::set(m, "k" & toString(i), el("n" & toString(i)))
    s = collections::add(s, "e" & toString(i))
    w = collections::set(w, i, strings::repeat("w", i MOD 13))
    IF i > 8 THEN
      m = collections::removeKey(m, "k" & toString(i - 8))
      s = collections::remove(s, "e" & toString(i - 8))
      w = collections::removeKey(w, i - 8)
    END IF
    IF i MOD 5 = 0 THEN
      w = collections::set(w, i - 1, strings::repeat("long", 5))
    END IF
  NEXT
  MUT ws AS List OF String = []
  FOR EACH k IN collections::keys(w)
    ws = collections::append(ws, toString(k) & ":" & collections::get(w, k))
  NEXT
  io::print("mr map " & showMN(m))
  io::print("mr set " & strings::join(collections::toList(s), " ") & " " & toString(collections::contains(s, "e600")) & toString(collections::contains(s, "e592")))
  io::print("mr int " & strings::join(ws, " ") & " " & toString(collections::hasKey(w, 592)))
END FUNC

FUNC main() AS Nothing
  mapReclaim()
  fields()
  globals() 'GLOBAL-CALL
  removeAtStrings()
  removeAtNodes()
  truncations()
  maps()
  sets()
  stringsWin()
END FUNC
"#;

const VALUE_OUTPUT: &str = "\
mr map {k593=[n593/1] k594=[n594/1] k595=[n595/1] k596=[n596/1] k597=[n597/1] k598=[n598/1] k599=[n599/1] k600=[n600/1]}
mr set e593 e594 e595 e596 e597 e598 e599 e600 TRUEFALSE
mr int 593:wwwwwwww 594:longlonglonglonglong 595:wwwwwwwwww 596:wwwwwwwwwww 597:wwwwwwwwwwww 598: 599:longlonglonglonglong 600:ww FALSE
f insert-take 1 [s1 s2 s3 i200]
f append-removeAt [s1 s2 s3 a200]
f bulk-removeAt [s1 s2 b100 c100]
f copy-grow tag [t1 t2 t3 c150]
f packed-copy [h5 h6] [h5 mid h6] [front h5 h6] [h5 h6 x1 x2]
g churn [p200 g2 j200]
ra last [a bb ccc dddd eeeee]
ra mid [a bb dddd eeeee]
ra first [bb dddd eeeee]
ra sorted-last [bb dddd eeeee]
ra prepend-1 [front dddd eeeee]
ra insert-last [front ins dddd]
ra set-1 [a-much-longer-front dddd]
ra churn [a-much-longer-front n40]
ra alias [a-much-longer-front n40] [n40]
ra copy [n40 c1 c2] [n40 c2]
ra empty [reuse]
rn [a/1 c/1 'd']
rn reordered [a/1 'mid' c/1]
rn churn [a/1 'mid' n30/1]
rn alias [a/1 'mid' n30/1] ['mid' n30/1]
tr sort-pop [a bb ccc d eeeee]
tr sort-mid [a bb ccc]
tr insert-pop [a bb ccc yy]
tr churn 54 q50 yy
tr pop-down [a bb ccc]
tr alias [a bb ccc] [a]
tr nodes [front/1 'i']
tr nodes churn 22 ['y19' 'y20']
m built 4 {a=4 b=4 c=4 d=4 e=4 f=4 g=4 h=4}
m -c {a=4 b=4 d=4 e=4 f=4 g=4 h=4} FALSE
m -a-h-zz {b=4 d=4 e=4 f=4 g=4}
m +i d=40 {b=4 d=40 e=4 f=4 g=4 i=9}
m alias {b=4 d=40 e=4 f=4 g=4 i=9} {d=40 e=4 f=4 g=4 i=9}
m big 230 230 39465 k1 k8 k330
mi {14=v2 28=v4 35=v5 42=v6} FALSE
mn {y=['y'] w=[w/1]}
s remove {2 3 4 6 7 8 9 11} 0011101111010
s alias {2 3 4 6 7 8 9 11} {3 4 6 7 8 9 11}
d {2 4 5 6 7 8} 0010111110
d last {2 4 5 6 7} 0010111100
d add {2 4 5 6 7 20} 0010
d all {2 4 5 6 7 20} {} 0
d self {}
x {2 4 5 6} 00101110
x toggle {2 4 5 9} 00101100010
x alias {2 4 5 9} {4 5 100} 00001100000
big 172 17762 {3 4 5 6}
st <hello >
st nomatch <hello >
st prefix <llo >
te <  a b>
ts <a b>
tu <x> 1
tv <é ü> 3
tw <> 0
lm <abc>
alias <keep this  > <keep this> <keep this>
grow 60 <ababab>
";

/// A fixed-size list that keeps losing a payload that is not the data tail while
/// gaining a new one: `insert` before the last element then pop it, or `append`
/// then remove the element before it — in a local, in a record field (the grow
/// reallocates the record), in a module-level global, and in a field that is not
/// its record's last inlined field, where `removeAt` runs in place but `append`
/// is a copying rebuild (which must not carry the dead bytes forward).
const CHURN_PROGRAM: &str = r#"IMPORT io
IMPORT os
IMPORT collections

FUNC seed() AS List OF String
  MUT x AS List OF String = []
  FOR i = 1 TO 64
    x = collections::append(x, "seed-" & toString(i))
  NEXT
  RETURN x
END FUNC

FUNC item(i AS Integer) AS String
  RETURN "item-" & toString(100000 + i MOD 90000)
END FUNC

FUNC churnInsertPop(n AS Integer) AS String
  MUT x = seed()
  FOR i = 1 TO n
    x = collections::insert(x, len(x) - 1, item(i))
    x = collections::take(x, len(x) - 1)
  NEXT
  RETURN toString(len(x)) & " " & collections::get(x, len(x) - 1)
END FUNC

FUNC churnRemoveAt(n AS Integer) AS String
  MUT x = seed()
  FOR i = 1 TO n
    x = collections::append(x, item(i))
    x = collections::removeAt(x, len(x) - 2)
  NEXT
  RETURN toString(len(x)) & " " & collections::get(x, len(x) - 1)
END FUNC

TYPE Holder
  tag AS Integer
  xs AS List OF String
END TYPE

TYPE Tagged
  xs AS List OF String
  name AS String
END TYPE

MUT gList AS List OF String = []

FUNC churnField(n AS Integer) AS String
  MUT h AS Holder = Holder[tag := 7, xs := seed()]
  FOR i = 1 TO n
    h = WITH h { xs := collections::insert(h.xs, len(h.xs) - 1, item(i)) }
    h = WITH h { xs := collections::take(h.xs, len(h.xs) - 1) }
    h = WITH h { xs := collections::append(h.xs, [item(i), item(i + 1)]) }
    h = WITH h { xs := collections::removeAt(h.xs, len(h.xs) - 3) }
    h = WITH h { xs := collections::removeAt(h.xs, len(h.xs) - 3) }
  NEXT
  RETURN toString(len(h.xs)) & " " & collections::get(h.xs, len(h.xs) - 1)
END FUNC

FUNC churnCopy(n AS Integer) AS String
  MUT t AS Tagged = Tagged[xs := seed(), name := "t"]
  FOR i = 1 TO n
    t = WITH t { xs := collections::append(t.xs, item(i)) }
    t = WITH t { xs := collections::removeAt(t.xs, len(t.xs) - 2) }
  NEXT
  RETURN toString(len(t.xs)) & " " & collections::get(t.xs, len(t.xs) - 1)
END FUNC

FUNC churnGlobal(n AS Integer) AS String
  gList = seed()
  FOR i = 1 TO n
    gList = collections::append(gList, item(i))
    gList = collections::removeAt(gList, len(gList) - 2)
  NEXT
  RETURN toString(len(gList)) & " " & collections::get(gList, len(gList) - 1)
END FUNC

FUNC main AS Integer
  LET args = os::args()
  LET shape = collections::get(args, 0)
  LET n = toInt(collections::get(args, 1))
  MUT r AS String = "?"
  IF shape = "insertPop" THEN
    r = churnInsertPop(n)
  ELSEIF shape = "removeAt" THEN
    r = churnRemoveAt(n)
  ELSEIF shape = "field" THEN
    r = churnField(n)
  ELSEIF shape = "global" THEN
    r = churnGlobal(n)
  ELSEIF shape = "copy" THEN
    r = churnCopy(n)
  END IF
  io::print(shape & " " & r)
  RETURN 0
END FUNC
"#;

/// bug-733: a `Map`/`Set` that keeps 64 entries while it keeps adding new keys and
/// removing old ones — through `set`/`removeKey`, `add`/`remove`, `union`/
/// `difference`, values that outgrow their slots (`valueChurn`: each is moved to
/// the data tail, out of entry order), and a record field (last inlined field: in
/// place; not last: rebuilt by copying).
const MAP_CHURN_PROGRAM: &str = r#"IMPORT io
IMPORT os
IMPORT collections
IMPORT strings

TYPE Bag
  m AS Map OF String TO Integer
  name AS String
END TYPE

TYPE Tail
  tag AS Integer
  m AS Map OF String TO String
END TYPE

FUNC key(i AS Integer) AS String
  RETURN "key-" & toString(100000 + i MOD 90000)
END FUNC

FUNC mapChurn(n AS Integer) AS String
  MUT m AS Map OF String TO Integer = Map OF String TO Integer {}
  FOR i = 1 TO n
    m = collections::set(m, key(i), i)
    IF i > 64 THEN
      m = collections::removeKey(m, key(i - 64))
    END IF
  NEXT
  RETURN toString(len(m)) & " " & toString(collections::get(m, key(n)))
END FUNC

FUNC intMapChurn(n AS Integer) AS String
  MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer {}
  FOR i = 1 TO n
    m = collections::set(m, i, i * 2)
    IF i > 64 THEN
      m = collections::removeKey(m, i - 64)
    END IF
  NEXT
  RETURN toString(len(m)) & " " & toString(collections::get(m, n))
END FUNC

FUNC setChurn(n AS Integer) AS String
  MUT s AS Set OF Integer = Set OF Integer {}
  FOR i = 1 TO n
    s = collections::add(s, i)
    IF i > 64 THEN
      s = collections::remove(s, i - 64)
    END IF
  NEXT
  RETURN toString(len(s)) & " " & toString(collections::contains(s, n))
END FUNC

FUNC valueGrow(n AS Integer) AS String
  MUT m AS Map OF String TO String = Map OF String TO String {}
  FOR k = 1 TO 64
    m = collections::set(m, key(k), "v")
  NEXT
  FOR i = 1 TO n
    LET k = key(1 + i MOD 64)
    IF i MOD 2 = 0 THEN
      m = collections::set(m, k, "a longer value " & toString(i))
    ELSE
      m = collections::set(m, k, "v")
    END IF
  NEXT
  RETURN toString(len(m)) & " " & collections::get(m, key(1 + n MOD 64))
END FUNC

FUNC valueChurn(n AS Integer) AS String
  MUT m AS Map OF String TO String = Map OF String TO String {}
  FOR k = 1 TO 64
    m = collections::set(m, key(k), "v")
  NEXT
  FOR i = 1 TO n
    m = collections::set(m, key(1 + i MOD 64), strings::repeat("v", (i * 7) MOD 97))
  NEXT
  RETURN toString(len(m)) & " " & collections::get(m, key(1 + n MOD 64))
END FUNC

FUNC setAlgebra(n AS Integer) AS String
  MUT s AS Set OF String = Set OF String {}
  FOR i = 1 TO n
    s = collections::union(s, Set OF String {key(i), key(i + 1)})
    IF i > 64 THEN
      s = collections::difference(s, Set OF String {key(i - 64)})
    END IF
  NEXT
  RETURN toString(len(s)) & " " & toString(collections::contains(s, key(n + 1)))
END FUNC

FUNC fieldCopy(n AS Integer) AS String
  MUT b AS Bag = Bag[m := Map OF String TO Integer {}, name := "bag"]
  FOR i = 1 TO n
    b = WITH b { m := collections::set(b.m, key(i), i) }
    IF i > 64 THEN
      b = WITH b { m := collections::removeKey(b.m, key(i - 64)) }
    END IF
  NEXT
  RETURN toString(len(b.m)) & " " & toString(collections::get(b.m, key(n)))
END FUNC

FUNC fieldLast(n AS Integer) AS String
  MUT t AS Tail = Tail[tag := 1, m := Map OF String TO String {}]
  FOR i = 1 TO n
    t = WITH t { m := collections::set(t.m, key(i), "value " & toString(i)) }
    IF i > 64 THEN
      t = WITH t { m := collections::removeKey(t.m, key(i - 64)) }
    END IF
  NEXT
  RETURN toString(len(t.m)) & " " & collections::get(t.m, key(n))
END FUNC

FUNC main AS Integer
  LET args = os::args()
  LET shape = collections::get(args, 0)
  LET n = toInt(collections::get(args, 1))
  MUT r AS String = "?"
  IF shape = "map" THEN
    r = mapChurn(n)
  ELSEIF shape = "intMap" THEN
    r = intMapChurn(n)
  ELSEIF shape = "set" THEN
    r = setChurn(n)
  ELSEIF shape = "valueGrow" THEN
    r = valueGrow(n)
  ELSEIF shape = "valueChurn" THEN
    r = valueChurn(n)
  ELSEIF shape = "setAlgebra" THEN
    r = setAlgebra(n)
  ELSEIF shape = "fieldCopy" THEN
    r = fieldCopy(n)
  ELSEIF shape = "fieldLast" THEN
    r = fieldLast(n)
  END IF
  io::print(shape & " " & r)
  RETURN 0
END FUNC
"#;

/// A map with 8-byte-aligned `Integer` keys whose `String` values keep outgrowing
/// their slots (the shape of `rt-behavior/collections/list-set-shared-offset-rt`).
const MAP_MOVED_VALUES_PROGRAM: &str = r#"IMPORT io
IMPORT collections

' bug-733 regression: values that outgrow their slots in a map with 8-byte
' aligned keys. The in-place reclaim packs payloads in offset order (moved
' values last), which needs fewer alignment pads than the entry-order total it
' compares against, so dataLength - live underflowed and the reclaim repeated
' forever.
FUNC main AS Integer
  MUT m AS Map OF Integer TO String = Map OF Integer TO String {}
  FOR i = 0 TO 39
    m = collections::set(m, i, "")
  NEXT
  FOR k = 0 TO 1999
    LET row AS Integer = (k * 17 + k / 3) MOD 40
    m = collections::set(m, row, collections::getOr(m, row, "") & "ab")
  NEXT
  MUT total AS Integer = 0
  FOR EACH k IN collections::keys(m)
    total = total + len(collections::get(m, k))
  NEXT
  io::print("map grow " & toString(len(m)) & " " & toString(total))
  RETURN 0
END FUNC
"#;

/// A two-element list whose elements keep changing length (a `set` that moves a
/// longer payload to the data tail, or leaves a hole) while an element is added
/// and one removed every round — `String` and record elements, through `append`,
/// `prepend` and `insert`, and `removeAt` and `take`.
const SMALL_CHURN_PROGRAM: &str = r#"IMPORT io
IMPORT os
IMPORT collections
IMPORT strings

TYPE Rec
  n AS Integer
  s AS String
END TYPE

FUNC lcg(x AS Integer) AS Integer
  RETURN (x * 1103515245 + 12345) MOD 2147483648
END FUNC

FUNC main AS Integer
  LET shape = collections::get(os::args(), 0)
  LET n = toInt(collections::get(os::args(), 1))
  MUT x AS List OF Rec = [Rec[n := 1, s := ""], Rec[n := 2, s := ""]]
  MUT y AS List OF String = ["", ""]
  MUT r AS Integer = 5
  FOR i = 1 TO n
    r = lcg(r)
    LET k = r MOD 2
    LET text = strings::repeat("z", (r / 101) MOD 23)
    IF shape = "strAppendRemoveFirst" THEN
      y = collections::set(y, k, text)
      y = collections::append(y, "p")
      y = collections::removeAt(y, 0)
    ELSEIF shape = "recAppendRemoveFirst" THEN
      x = collections::set(x, k, Rec[n := i, s := text])
      x = collections::append(x, Rec[n := i, s := "p"])
      x = collections::removeAt(x, 0)
    ELSEIF shape = "recPrependRemoveLast" THEN
      x = collections::set(x, k, Rec[n := i, s := text])
      x = collections::prepend(x, Rec[n := i, s := "p"])
      x = collections::removeAt(x, len(x) - 1)
    ELSEIF shape = "recPrependPop" THEN
      x = collections::set(x, k, Rec[n := i, s := text])
      x = collections::prepend(x, Rec[n := i, s := "p"])
      x = collections::take(x, len(x) - 1)
    ELSEIF shape = "recInsertRemoveLast" THEN
      x = collections::set(x, k, Rec[n := i, s := text])
      x = collections::insert(x, 1, Rec[n := i, s := "p"])
      x = collections::removeAt(x, len(x) - 1)
    END IF
  NEXT
  MUT total AS Integer = 0
  FOR EACH e IN x
    total = total + len(e.s)
  NEXT
  FOR EACH e IN y
    total = total + len(e)
  NEXT
  io::print(shape & " " & toString(len(x) + len(y)) & " " & toString(total))
  RETURN 0
END FUNC
"#;

fn scale_exe() -> &'static PathBuf {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let project = common::temp_project("b732_scale", SCALE_PROGRAM);
        common::build_project(&project)
    })
}

/// The line each shape prints for a build of `n`.
fn expected(shape: &str, n: u64) -> String {
    let triangle = n * (n + 1) / 2;
    let result = match shape {
        "removeAtLast" | "removeKey" | "popInserted" | "append" => format!("{n} {n}"),
        "removeAtSorted" => format!("{triangle} {n}"),
        // Two pops per round while more than one element is left: an odd `n`
        // keeps the smallest, `1`.
        "popSorted" => format!("{} {n}", triangle - n % 2),
        "setRemove" | "difference" | "strip" => format!("{n} {n}"),
        // `trim` keeps one `x` per round (the space it adds is trimmed).
        "trim" => format!("{n} {n}"),
        // `-5` toggles in and out: present after an odd number of rounds.
        "symDiff" => format!("{} {n}", n + n % 2),
        other => panic!("no expectation for {other}"),
    };
    format!("{shape} {result}")
}

/// Run `exe shape n rounds`, check its line, and return the child's CPU seconds.
fn cpu_run(exe: &Path, shape: &str, n: u64, rounds: u64) -> f64 {
    let mut child = Command::new(exe)
        .args([shape, &n.to_string(), &rounds.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run the program");
    let mut stdout_pipe = child.stdout.take().expect("child stdout");
    let mut stderr_pipe = child.stderr.take().expect("child stderr");
    let pid = child.id() as libc::pid_t;
    let mut status: libc::c_int = 0;
    // SAFETY: `rusage` is plain old data; `wait4` fills it for the reaped child.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // `wait4` is the only call that reports THIS child's CPU time. The program
    // writes one short line, far below a pipe buffer, so it cannot block on a
    // write while nobody reads; the pipes are drained after it exits.
    let reaped = unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
    assert_eq!(
        reaped,
        pid,
        "wait4 failed: {}",
        std::io::Error::last_os_error()
    );
    let mut stdout = String::new();
    let mut stderr = String::new();
    stdout_pipe
        .read_to_string(&mut stdout)
        .expect("read stdout");
    stderr_pipe
        .read_to_string(&mut stderr)
        .expect("read stderr");
    assert!(
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
        "{shape} n={n} rounds={rounds} failed (status {status}):\n{stdout}\n{stderr}"
    );
    assert_eq!(
        stdout.lines().next(),
        Some(expected(shape, n).as_str()),
        "bug-732: {shape} n={n}: wrong result"
    );
    let seconds = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    seconds(usage.ru_utime) + seconds(usage.ru_stime)
}

/// Assert `shape` costs the same for `ROUNDS` builds of `n` as for one build of
/// `n * ROUNDS`.
fn assert_linear(shape: &str, n: u64) {
    let exe = scale_exe();
    // An untimed first run of each: a freshly written executable's first launch
    // pays a one-off start-up cost that would inflate only one side.
    cpu_run(exe, shape, n, ROUNDS);
    cpu_run(exe, shape, n * ROUNDS, 1);
    let short = (0..3)
        .map(|_| cpu_run(exe, shape, n, ROUNDS))
        .fold(f64::INFINITY, f64::min);
    let bound = short * MAX_RATIO + SLACK_SECONDS;
    let mut long = cpu_run(exe, shape, n * ROUNDS, 1);
    if long > bound {
        for _ in 0..2 {
            long = long.min(cpu_run(exe, shape, n * ROUNDS, 1));
        }
    }
    eprintln!(
        "{shape}: {ROUNDS}×{n} = {short:.3} s CPU; 1×{} = {long:.3} s CPU; ×{:.2}",
        n * ROUNDS,
        long / short.max(1e-9)
    );
    assert!(
        long <= bound,
        "bug-732: {shape}: {ROUNDS} runs of {n} took {short:.3} s CPU, one run of {} took \
         {long:.3} s: ×{:.1} for the same work. Linear is ×1; a self-update that walks the \
         whole collection on every call is ×{ROUNDS}.",
        n * ROUNDS,
        long / short.max(1e-9)
    );
}

/// Finding 1: `x = removeAt(x, len(x) - 1)` on a `List OF String` built by
/// `append` (the removed payload is the data tail), and on one reordered by `sort`
/// (it is not).
#[test]
fn remove_at_near_the_end_of_a_variable_width_list_is_linear() {
    assert_linear("removeAtLast", 6_000);
    assert_linear("removeAtSorted", 6_000);
}

/// Finding 2: `m = removeKey(m, k)` and a `Set`'s `s = remove(s, e)`, between
/// inserts that keep the hash index in use.
#[test]
fn remove_key_and_set_remove_are_linear() {
    assert_linear("removeKey", 2_000);
    assert_linear("setRemove", 2_000);
}

/// Finding 3: `s = difference(s, t)` with a one-element `t`, both a miss and a hit.
#[test]
fn difference_with_a_small_argument_is_linear() {
    assert_linear("difference", 2_000);
}

/// Finding 4: `s = symmetricDifference(s, t)` with a one-element `t`.
#[test]
fn symmetric_difference_with_a_small_argument_is_linear() {
    assert_linear("symDiff", 2_000);
}

/// Finding 5: `stripSuffix` / a no-match `stripPrefix` (the window already starts
/// at the block), and `trimEnd` / `trim` over a growing string.
#[test]
fn string_window_self_updates_that_keep_the_start_are_linear() {
    assert_linear("strip", 25_000);
    assert_linear("trim", 3_000);
}

/// Finding 6: `take`/`mid` truncating a list whose payloads are out of entry order
/// — after `sort`, and after an `insert` before the last element.
#[test]
fn truncating_an_out_of_order_variable_width_list_is_linear() {
    assert_linear("popSorted", 6_000);
    assert_linear("popInserted", 5_000);
}

/// The contrast shape stays linear.
#[test]
fn append_stays_linear() {
    assert_linear("append", 50_000);
}

/// Every removal and window keeps the elements, order and index a copy would.
#[test]
fn removals_and_windows_keep_value_semantics() {
    let project = common::temp_project("b732_values", VALUE_PROGRAM);
    let exe = common::build_project(&project);
    let output = Command::new(&exe).output().expect("run the program");
    assert!(
        output.status.success(),
        "the value program failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), VALUE_OUTPUT);
    let _ = std::fs::remove_dir_all(&project);
}

/// The value program without its module-level global: the text between each
/// `'GLOBAL-BEGIN` / `'GLOBAL-END` pair, and the line calling `globals()`.
fn without_globals(program: &str) -> String {
    let mut out = String::new();
    let mut cutting = false;
    for line in program.lines() {
        match line {
            "'GLOBAL-BEGIN" => cutting = true,
            "'GLOBAL-END" => cutting = false,
            _ if cutting || line.ends_with("'GLOBAL-CALL") => {}
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    assert!(
        !out.contains("gList"),
        "the leak variant must drop the global churn"
    );
    out
}

fn counter(stderr: &str, key: &str) -> u64 {
    stderr
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .and_then(|n| n.trim().parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no `{key}<n>` in the report:\n{stderr}"))
}

/// Built with `--debug`, the value program frees every block exactly once: each
/// removed graph-owning element (a `List OF Node` element, a `Map OF String TO
/// Node` value) is freed, and no reclaimed or repacked block leaks. The
/// module-level global's churn is left out: a global still holds its last value
/// at exit, which the report counts as live.
#[test]
fn removals_free_every_removed_element_once() {
    let source = without_globals(VALUE_PROGRAM);
    let expected: String = VALUE_OUTPUT
        .lines()
        .filter(|line| !line.starts_with("g churn "))
        .map(|line| format!("{line}\n"))
        .collect();
    let project = common::temp_project("b732_leaks", &source);
    let exe = common::debug_report::build_debug_project("b732_leaks", &project);
    let (stdout, stderr) = common::debug_report::run_ok("b732_leaks", &exe);
    assert_eq!(stdout, expected, "the --debug build changed the output");
    let allocs = counter(&stderr, "arena.0.alloc_calls ");
    let frees = counter(&stderr, "arena.0.free_calls ");
    let live = counter(&stderr, "arena.0.live_bytes ");
    let skips = counter(&stderr, "arena.0.double_free_skips ");
    assert_eq!(skips, 0, "bug-732: a block was freed twice");
    assert_eq!(
        (allocs, live),
        (frees, 0),
        "bug-732: {} block(s) / {live} byte(s) never freed",
        allocs.saturating_sub(frees)
    );
    let _ = std::fs::remove_dir_all(&project);
}

/// A list whose count stays at 64 while it keeps losing a payload that is not the
/// data tail must not grow: its peak live bytes are the same after 8× the rounds.
/// (Removing without reclaiming the bytes would leave the next grow to copy them.)
#[test]
fn a_churning_list_does_not_grow_without_bound() {
    let project = common::temp_project("b732_churn", CHURN_PROGRAM);
    let exe = common::debug_report::build_debug_project("b732_churn", &project);
    for shape in ["insertPop", "removeAt", "field", "global", "copy"] {
        let peak = |n: u64| {
            let output = Command::new(&exe)
                .args([shape, &n.to_string()])
                .output()
                .expect("run the program");
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            assert!(
                output.status.success(),
                "{shape} {n} failed:\n{}\n{stderr}",
                String::from_utf8_lossy(&output.stdout)
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                stdout.starts_with(&format!("{shape} 64 ")),
                "{shape} {n}: wrong element count: {stdout}"
            );
            counter(&stderr, "arena.0.peak_live_bytes ")
        };
        let small = peak(2_000);
        let large = peak(16_000);
        eprintln!("{shape}: peak_live_bytes {small} at 2000, {large} at 16000");
        assert!(
            large <= small + small / 4,
            "bug-732: {shape}: peak live bytes grew from {small} to {large} for 8× the rounds \
             of a list that never holds more than 65 elements"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

/// bug-733: a map or set whose size stays at 64 entries under churn must not
/// grow: its peak live bytes are the same after 8× the rounds. In-place removals
/// leave the removed key/value bytes as dead slack, and before bug-733 no map grow
/// reclaimed them — `map` peaked at 421 KB after 2,000 rounds and 3.3 MB after
/// 16,000.
#[test]
fn a_churning_map_or_set_does_not_grow_without_bound() {
    let project = common::temp_project("b733_churn", MAP_CHURN_PROGRAM);
    let exe = common::debug_report::build_debug_project("b733_churn", &project);
    let mut grew = Vec::new();
    for shape in [
        "map",
        "intMap",
        "set",
        "valueGrow",
        "valueChurn",
        "setAlgebra",
        "fieldCopy",
        "fieldLast",
    ] {
        let peak = |n: u64| {
            let output = Command::new(&exe)
                .args([shape, &n.to_string()])
                .output()
                .expect("run the program");
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            assert!(
                output.status.success(),
                "{shape} {n} failed:\n{stdout}\n{stderr}"
            );
            let size = if shape == "setAlgebra" { 65 } else { 64 };
            assert!(
                stdout.starts_with(&format!("{shape} {size} ")),
                "{shape} {n}: wrong entry count: {stdout}"
            );
            counter(&stderr, "arena.0.peak_live_bytes ")
        };
        let small = peak(2_000);
        let large = peak(16_000);
        eprintln!("{shape}: peak_live_bytes {small} at 2000, {large} at 16000");
        if large > small + small / 4 {
            grew.push(format!("{shape}: {small} -> {large}"));
        }
    }
    assert!(
        grew.is_empty(),
        "bug-733: peak live bytes grew for 8x the rounds of a map/set that never holds \
         more than 65 entries: {grew:?}"
    );
    let _ = std::fs::remove_dir_all(&project);
}

/// The reclaim must terminate and report the right values when moved values
/// leave the packed layout with fewer alignment pads than the entry-order live
/// total (bug-733's first reclaim computed `dataLength - live`, which then
/// underflowed, and the grow retried the repack forever — the program hung).
#[test]
fn a_map_reclaim_terminates_when_moved_values_pack_tighter() {
    let project = common::temp_project("b733_moved", MAP_MOVED_VALUES_PROGRAM);
    let exe = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &exe,
        std::time::Duration::from_secs(30),
        "bug-733: the map reclaim repeated forever",
    );
    assert!(status.success(), "the program failed: {stdout}");
    assert_eq!(stdout, "map grow 40 4000\n");
    let _ = std::fs::remove_dir_all(&project);
}

/// Run `exe args` with a deadline; return its stderr, or panic naming a hang.
fn run_with_deadline(exe: &Path, args: &[String], seconds: u64) -> (String, String) {
    let mut child = Command::new(exe)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run the program");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    loop {
        if child.try_wait().expect("poll the program").is_some() {
            break;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{args:?} did not finish within {seconds}s");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let output = child.wait_with_output().expect("collect the output");
    assert!(output.status.success(), "{args:?} failed");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// bug-732 C7: a list that keeps a few elements whose payloads keep changing length
/// must stay small. Every in-place grow of the data region stepped the ENTRY
/// capacity too, and the dead-byte reclaim (bug-732) repacked into a data region
/// sized from the live bytes without shrinking the entries — so a list of two
/// strings alternated repack and grow and doubled its entry capacity on every
/// grow: 1 KB after 150 rounds, 933 KB after 1,200 (3.1 GB for records, and an
/// `insert` shape that never finished).
#[test]
fn a_small_list_under_length_changing_churn_stays_small() {
    let project = common::temp_project("b732_small", SMALL_CHURN_PROGRAM);
    let exe = common::debug_report::build_debug_project("b732_small", &project);
    let mut grew = Vec::new();
    for shape in [
        "strAppendRemoveFirst",
        "recAppendRemoveFirst",
        "recPrependRemoveLast",
        "recPrependPop",
        "recInsertRemoveLast",
    ] {
        let peak = |n: u64| {
            let (stdout, stderr) = run_with_deadline(&exe, &[shape.to_string(), n.to_string()], 60);
            assert!(
                stdout.starts_with(&format!("{shape} 4 ")),
                "{shape} {n}: wrong element count: {stdout}"
            );
            counter(&stderr, "arena.0.peak_live_bytes ")
        };
        let small = peak(150);
        let large = peak(1_200);
        eprintln!("{shape}: peak_live_bytes {small} at 150, {large} at 1200");
        if large > small + small / 4 {
            grew.push(format!("{shape}: {small} -> {large}"));
        }
    }
    assert!(
        grew.is_empty(),
        "bug-732 C7: a list holding two elements grew with the rounds: {grew:?}"
    );
    let _ = std::fs::remove_dir_all(&project);
}
