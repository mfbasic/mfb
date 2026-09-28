//! Regression tests for bug-745: collections whose element, key or value type is
//! `Nothing`.
//!
//! `Nothing` is a first-class type with one value, `NOTHING` (`mfb spec language
//! types` §4.6; owner decision 2026-09-26), and it is comparable, so it may be a
//! `List` element, a `Map` key or value, and a `Set` element (§12). Before the fix
//! every such collection failed `mfb build` with the internal error "native code
//! collection type 'List OF Nothing' is not supported", because
//! `collection_type_code` had no identifier for `Nothing`. Generic `collections::`
//! members were also rejected in the front end (`TYPE_CALL_ARGUMENT_MISMATCH`),
//! because strict template unification never bound a type variable to `Nothing`.
//!
//! Each test runs one program over a family of operations and checks every
//! observable result: counts, membership, equality with `NOTHING`, and the
//! set/map collapse a unit key implies (all `NOTHING`s are one key).

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::process::Command;

fn build_and_run(name: &str, source: &str) -> String {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_nothingcoll_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create root");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"nothingcoll\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write manifest");
    fs::write(
        root.join("src/main.mfb"),
        format!("IMPORT io\nIMPORT collections\n\n{source}"),
    )
    .expect("write source");
    let out = Command::new(common::mfb_exe())
        .arg("build")
        .arg(&root)
        .output()
        .expect("run mfb build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && !combined.contains("panicked"),
        "expected the `{name}` program to build, but it failed:\n{combined}"
    );
    let exe = combined
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build reported no executable path")
        .trim()
        .to_string();
    let exe = root.join(exe.strip_prefix("./").unwrap_or(&exe));
    let run = Command::new(&exe).output().expect("run built program");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success(),
        "the `{name}` program failed (exit {:?}):\nstdout:\n{stdout}\nstderr:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let _ = fs::remove_dir_all(&root);
    stdout
}

/// `countEq` prints how many elements equal `NOTHING` — every one must.
const HELPERS: &str = "\
FUNC countEq(xs AS List OF Nothing) AS Integer
  MUT n = 0
  FOR EACH v IN xs
    IF v = NOTHING THEN n = n + 1
  NEXT
  RETURN n
END FUNC

FUNC show(label AS String, xs AS List OF Nothing) AS Nothing
  io::print(label & \" \" & toString(len(xs)) & \" \" & toString(countEq(xs)))
END FUNC

";

fn program(body: &str) -> String {
    format!("{HELPERS}FUNC main() AS Nothing\n{body}END FUNC\n")
}

#[test]
fn a_list_literal_of_nothing_builds_and_iterates() {
    let out = build_and_run(
        "literal",
        &program(
            "  LET xs = [NOTHING, NOTHING, NOTHING]\n  show(\"lit\", xs)\n  \
             LET ys AS List OF Nothing = []\n  show(\"empty\", ys)\n  \
             IF collections::get(xs, 2) = NOTHING THEN io::print(\"get\")\n  \
             IF collections::getOr(xs, 9, NOTHING) = NOTHING THEN io::print(\"getOr\")\n",
        ),
    );
    assert_eq!(out, "lit 3 3\nempty 0 0\nget\ngetOr\n");
}

#[test]
fn list_of_nothing_builders_and_slices() {
    let out = build_and_run(
        "builders",
        &program(
            "  LET xs = [NOTHING, NOTHING, NOTHING, NOTHING]\n  \
             show(\"append\", collections::append(xs, NOTHING))\n  \
             show(\"appendList\", collections::append(xs, [NOTHING, NOTHING]))\n  \
             show(\"prepend\", collections::prepend(xs, NOTHING))\n  \
             show(\"insert\", collections::insert(xs, 1, NOTHING))\n  \
             show(\"set\", collections::set(xs, 0, NOTHING))\n  \
             show(\"removeAt\", collections::removeAt(xs, 3))\n  \
             show(\"mid\", collections::mid(xs, 1, 2))\n  \
             show(\"take\", collections::take(xs, 3))\n  \
             show(\"drop\", collections::drop(xs, 3))\n  \
             show(\"repeat\", collections::repeat(xs, NOTHING, 3, 3))\n  \
             show(\"replace\", collections::replace(xs, NOTHING, NOTHING))\n  \
             show(\"distinct\", collections::distinct(xs))\n  \
             show(\"flatten\", collections::flatten([xs, [NOTHING]]))\n  \
             show(\"orig\", xs)\n",
        ),
    );
    assert_eq!(
        out,
        "append 5 5\nappendList 6 6\nprepend 5 5\ninsert 5 5\nset 4 4\nremoveAt 3 3\n\
         mid 2 2\ntake 3 3\ndrop 1 1\nrepeat 6 6\nreplace 4 4\ndistinct 1 1\nflatten 5 5\n\
         orig 4 4\n"
    );
}

#[test]
fn list_of_nothing_searches_and_predicates() {
    let out = build_and_run(
        "search",
        &program(
            "  LET xs = [NOTHING, NOTHING, NOTHING]\n  \
             LET ys AS List OF Nothing = []\n  \
             io::print(toString(collections::contains(xs, NOTHING)) & \" \" & \
             toString(collections::contains(ys, NOTHING)))\n",
        ),
    );
    assert_eq!(out, "TRUE FALSE\n");
    let out = build_and_run(
        "predicates",
        &program(
            "  LET xs = [NOTHING, NOTHING, NOTHING]\n  \
             io::print(toString(collections::find(xs, NOTHING)))\n  \
             io::print(toString(collections::findIndex(xs, LAMBDA(v AS Nothing) -> (v = NOTHING))))\n  \
             io::print(toString(collections::findLastIndex(xs, LAMBDA(v AS Nothing) -> (v = NOTHING))))\n  \
             io::print(toString(collections::all(xs, LAMBDA(v AS Nothing) -> (v = NOTHING))))\n  \
             io::print(toString(collections::any(xs, LAMBDA(v AS Nothing) -> (v <> NOTHING))))\n  \
             show(\"filter\", collections::filter(xs, LAMBDA(v AS Nothing) -> (v = NOTHING)))\n  \
             LET parts = collections::partition(xs, LAMBDA(v AS Nothing) -> (v = NOTHING))\n  \
             show(\"yes\", parts.matched)\n  show(\"no\", parts.unmatched)\n",
        ),
    );
    assert_eq!(out, "0\n0\n2\nTRUE\nFALSE\nfilter 3 3\nyes 3 3\nno 0 0\n");
}

#[test]
fn list_of_nothing_callbacks_and_regrouping() {
    let out = build_and_run(
        "callbacks",
        &program(
            "  LET xs = [NOTHING, NOTHING, NOTHING]\n  \
             LET ones = collections::transform(xs, LAMBDA(v AS Nothing) -> 1)\n  \
             io::print(toString(collections::sum(ones)))\n  \
             io::print(toString(collections::reduce(xs, 10, LAMBDA(acc AS Integer, v AS Nothing) -> acc + 1)))\n  \
             collections::forEach(xs, LAMBDA(v AS Nothing) -> io::print(\"each\"))\n  \
             LET cs = collections::chunks(xs, 2)\n  \
             io::print(toString(len(cs)))\n  FOR EACH c IN cs\n    show(\"chunk\", c)\n  NEXT\n  \
             LET ws = collections::window(xs, 2)\n  \
             io::print(toString(len(ws)))\n  \
             LET zs = collections::zip(xs, [7, 8, 9])\n  \
             FOR EACH z IN zs\n    IF z.first = NOTHING THEN io::print(toString(z.second))\n  NEXT\n  \
             LET s = collections::toSet(xs)\n  io::print(toString(len(s)))\n",
        ),
    );
    assert_eq!(
        out,
        "3\n13\neach\neach\neach\n2\nchunk 2 2\nchunk 1 1\n2\n7\n8\n9\n1\n"
    );
}

#[test]
fn a_mut_list_of_nothing_updates_in_place() {
    let out = build_and_run(
        "inplace",
        &program(
            "  MUT xs AS List OF Nothing = []\n  \
             FOR i = 1 TO 50\n    xs = collections::append(xs, NOTHING)\n  NEXT\n  \
             show(\"grown\", xs)\n  \
             xs = collections::removeAt(xs, 0)\n  xs = collections::set(xs, 3, NOTHING)\n  \
             xs = collections::insert(xs, 2, NOTHING)\n  xs = collections::take(xs, 10)\n  \
             show(\"edited\", xs)\n",
        ),
    );
    assert_eq!(out, "grown 50 50\nedited 10 10\n");
}

#[test]
fn a_map_with_nothing_values() {
    let out = build_and_run(
        "mapvalues",
        &program(
            "  LET m = Map OF String TO Nothing { \"a\" := NOTHING, \"b\" := NOTHING }\n  \
             io::print(toString(len(m)))\n  \
             IF collections::get(m, \"b\") = NOTHING THEN io::print(\"get\")\n  \
             IF collections::getOr(m, \"z\", NOTHING) = NOTHING THEN io::print(\"getOr\")\n  \
             io::print(toString(collections::hasKey(m, \"a\")) & \" \" & toString(collections::hasKey(m, \"z\")))\n  \
             LET m2 = collections::set(m, \"c\", NOTHING)\n  io::print(toString(len(m2)))\n  \
             LET m3 = collections::removeKey(m2, \"a\")\n  io::print(toString(len(m3)))\n  \
             io::print(toString(len(collections::keys(m3))))\n  show(\"values\", collections::values(m3))\n  \
             LET mm = collections::merge(m, m3, TRUE)\n  io::print(toString(len(mm)))\n  \
             LET counts = collections::mapValues(m, LAMBDA(v AS Nothing) -> 5)\n  \
             io::print(toString(collections::get(counts, \"a\")))\n  \
             FOR EACH e IN m\n    IF e.value = NOTHING THEN io::print(e.key)\n  NEXT\n",
        ),
    );
    assert_eq!(
        out,
        "2\nget\ngetOr\nTRUE FALSE\n3\n2\n2\nvalues 2 2\n3\n5\na\nb\n"
    );
}

/// `NOTHING` is the only key a `Map OF Nothing TO V` can have, so every write lands
/// on the same entry.
#[test]
fn a_map_keyed_by_nothing_has_one_entry() {
    let out = build_and_run(
        "mapkey",
        &program(
            "  LET m = Map OF Nothing TO Integer { NOTHING := 1 }\n  \
             LET m2 = collections::set(m, NOTHING, 2)\n  \
             io::print(toString(len(m2)) & \" \" & toString(collections::get(m2, NOTHING)))\n  \
             io::print(toString(collections::hasKey(m2, NOTHING)))\n  \
             MUT w AS Map OF Nothing TO Integer = Map OF Nothing TO Integer {}\n  \
             FOR i = 1 TO 5\n    w = collections::set(w, NOTHING, i)\n  NEXT\n  \
             io::print(toString(len(w)) & \" \" & toString(collections::get(w, NOTHING)))\n  \
             LET e = collections::removeKey(w, NOTHING)\n  io::print(toString(len(e)))\n",
        ),
    );
    assert_eq!(out, "1 2\nTRUE\n1 5\n0\n");
}

/// A `Set OF Nothing` holds at most one element.
#[test]
fn a_set_of_nothing_holds_at_most_one_element() {
    let out = build_and_run(
        "set",
        &program(
            "  LET s = Set OF Nothing { NOTHING, NOTHING }\n  io::print(toString(len(s)))\n  \
             LET e AS Set OF Nothing = Set OF Nothing {}\n  io::print(toString(len(e)))\n  \
             io::print(toString(len(collections::add(e, NOTHING))) & \" \" & toString(len(collections::add(s, NOTHING))))\n  \
             io::print(toString(collections::contains(s, NOTHING)) & \" \" & toString(collections::contains(e, NOTHING)))\n  \
             io::print(toString(len(collections::remove(s, NOTHING))))\n  \
             io::print(toString(len(collections::union(s, e))) & toString(len(collections::intersection(s, e))) & \
             toString(len(collections::difference(s, e))) & toString(len(collections::symmetricDifference(s, s))))\n  \
             io::print(toString(collections::isSubset(e, s)) & \" \" & toString(collections::isSuperset(s, e)) & \" \" & toString(collections::isDisjoint(s, e)))\n  \
             show(\"toList\", collections::toList(s))\n",
        ),
    );
    assert_eq!(
        out,
        "1\n0\n1 1\nTRUE FALSE\n0\n1010\nTRUE TRUE TRUE\ntoList 1 1\n"
    );
}

/// The two collection shapes bug-744 could not test: `FUNC … AS Nothing` calls as
/// literal elements and map values.
#[test]
fn nothing_func_calls_fill_collection_literals() {
    let out = build_and_run(
        "calls",
        &format!(
            "FUNC f(x AS Integer) AS Nothing\n  io::print(\"f\" & toString(x))\n  RETURN NOTHING\nEND FUNC\n\n{}",
            program(
                "  LET xs = [f(1), f(2)]\n  show(\"list\", xs)\n  \
                 LET m = Map OF String TO Nothing { \"a\" := f(3) }\n  \
                 FOR EACH e IN m\n    IF e.value = NOTHING THEN io::print(e.key & \" eq\")\n  NEXT\n  \
                 LET s = Set OF Nothing { f(4), f(5) }\n  io::print(toString(len(s)))\n",
            )
        ),
    );
    assert_eq!(out, "f1\nf2\nlist 2 2\nf3\na eq\nf4\nf5\n1\n");
}

/// Collections of `Nothing` nest, sit in record fields, cross function boundaries,
/// and copy by value like any other collection.
#[test]
fn collections_of_nothing_nest_and_copy() {
    let out = build_and_run(
        "nest",
        &format!(
            "TYPE Bag\n  items AS List OF Nothing\n  tags AS Map OF String TO Nothing\nEND TYPE\n\n\
             FUNC grow(xs AS List OF Nothing) AS List OF Nothing\n  RETURN collections::append(xs, NOTHING)\nEND FUNC\n\n{}",
            program(
                "  LET b = Bag[items := [NOTHING], tags := Map OF String TO Nothing { \"t\" := NOTHING }]\n  \
                 show(\"field\", b.items)\n  io::print(toString(len(b.tags)))\n  \
                 LET g = grow(grow(b.items))\n  show(\"grown\", g)\n  show(\"source\", b.items)\n  \
                 LET nested = [[NOTHING], [NOTHING, NOTHING]]\n  \
                 FOR EACH inner IN nested\n    show(\"inner\", inner)\n  NEXT\n  \
                 MUT copy = g\n  copy = collections::append(copy, NOTHING)\n  \
                 show(\"copy\", copy)\n  show(\"g\", g)\n",
            )
        ),
    );
    assert_eq!(
        out,
        "field 1 1\n1\ngrown 3 3\nsource 1 1\ninner 1 1\ninner 2 2\ncopy 4 4\ng 3 3\n"
    );
}
