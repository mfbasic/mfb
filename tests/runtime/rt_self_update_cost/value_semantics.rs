//! plan-176-C: `rt_self_update_cost`'s value-semantics tests (`VALUE_PROGRAM`, its
//! output and leak check). The linearity and churn tests, and the shared `counter`,
//! live in the root, `rt_self_update_cost.rs`.

use super::*;

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
