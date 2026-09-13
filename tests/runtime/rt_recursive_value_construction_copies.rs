//! plan-134-E: a recursive value built into a record, union, collection or `STATE` field is
//! independent of the value it came from.
//!
//! ## Why this exists
//!
//! plan-134-D made the owning stores (`LET`, assignment, globals, `RETURN`, closure capture)
//! copy a recursive value. The stores that BUILD a value out of others did not: every
//! constructor argument, `WITH` update, union wrap, collection literal element, in-place
//! `append`/`insert`/`set`/`prepend` item and `STATE` replacement lowered its operand with
//! plain `lower_value` (plan-134-E §2 census), so a non-inlined field such as
//! `kids AS List OF Node` held the operand's live block. Growing that list in place
//! afterwards grew — or reallocated and freed — the block every holder still pointed at.
//!
//! Measured on the plan-134-D compiler, the first program printed `record=96`, `union=96`
//! (lengths read from freed memory) and then died with SIGSEGV. The second — a list whose
//! appended element is built from the list itself — stored the list's own block into the
//! element: a self-cycle, or after the growing append a pointer to freed memory; it died with
//! SIGSEGV at the first `collections::get`, on the pre-plan compiler and on plan-134-B's walker
//! alike (plan-134-A "Verified properties").
//!
//! ## What it runs
//!
//! One `MUT xs AS List OF Node` is stored into each construction store, then grown in place;
//! every holder must still see one element. Then the self-referencing construction.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const CONSTRUCTION_SOURCE: &str = r#"IMPORT io
IMPORT fs
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

TYPE Holder
  kids AS List OF Node
  n AS Integer
END TYPE

TYPE Branch
  kids AS List OF Node
END TYPE

TYPE Leaf
  v AS Integer
END TYPE

UNION Shape
  Branch
  Leaf
END UNION

FUNC mk(tag AS Integer) AS Node
  RETURN Node[kids := [], tag := tag]
END FUNC

FUNC kidsIn(s AS Shape) AS Integer
  MATCH s
    CASE Branch(b)
      RETURN len(b.kids)
    CASE ELSE
      RETURN -1
  END MATCH
END FUNC

FUNC main AS Integer
  MUT xs AS List OF Node = [mk(1)]
  LET empty AS List OF Node = []
  LET rec AS Node = Node[kids := xs, tag := 1]
  LET shape AS Shape = Branch[kids := xs]
  LET lit AS List OF List OF Node = [xs]
  MUT appended AS List OF List OF Node = []
  appended = collections::append(appended, xs)
  MUT inserted AS List OF List OF Node = [empty]
  inserted = collections::insert(inserted, 0, xs)
  MUT setted AS List OF List OF Node = [empty]
  setted = collections::set(setted, 0, xs)
  MUT prepended AS List OF List OF Node = [empty]
  prepended = collections::prepend(prepended, xs)
  LET m AS Map OF String TO List OF Node = Map OF String TO List OF Node { "k" := xs }
  LET base AS Holder = Holder[kids := empty, n := 0]
  LET withed AS Holder = WITH base { kids := xs }
  RES f AS fs::File STATE Holder = fs::openFile("{DATA}")
  f.state = Holder[kids := xs, n := 2]
  xs = collections::append(xs, mk(2))
  xs = collections::append(xs, mk(3))
  io::print("record=" & toString(len(rec.kids)))
  io::print("union=" & toString(kidsIn(shape)))
  io::print("literal=" & toString(len(collections::get(lit, 0))))
  io::print("append=" & toString(len(collections::get(appended, 0))))
  io::print("insert=" & toString(len(collections::get(inserted, 0))))
  io::print("set=" & toString(len(collections::get(setted, 0))))
  io::print("prepend=" & toString(len(collections::get(prepended, 0))))
  io::print("map=" & toString(len(collections::get(m, "k"))))
  io::print("with=" & toString(len(withed.kids)))
  io::print("state=" & toString(len(f.state.kids)))
  io::print("source=" & toString(len(xs)))
  fs::close(f)
  RETURN 0
END FUNC
"#;

/// The self-referencing construction: each appended element is built from the list it is
/// appended to, so under value semantics it holds the list as it was BEFORE the append.
const SELF_REFERENCE_SOURCE: &str = r#"IMPORT io
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

FUNC main AS Integer
  MUT xs AS List OF Node = []
  xs = collections::append(xs, Node[kids := xs, tag := 1])
  xs = collections::append(xs, Node[kids := xs, tag := 2])
  io::print("len xs=" & toString(len(xs)))
  LET first AS Node = collections::get(xs, 0)
  io::print("first.kids=" & toString(len(first.kids)))
  LET second AS Node = collections::get(xs, 1)
  io::print("second.kids=" & toString(len(second.kids)))
  RETURN 0
END FUNC
"#;

fn run(name: &str, source: &str) -> (std::process::ExitStatus, String, String) {
    let project = common::temp_project(name, "");
    let data = project.join("state.txt");
    std::fs::write(&data, "plan-134-E state probe\n").expect("write the STATE data file");
    let program = source.replace("{DATA}", &data.to_string_lossy());
    std::fs::write(project.join("src/main.mfb"), program).expect("write the probe source");
    let exe = common::build_project(&project);
    let output = Command::new(&exe).output().expect("run the probe");
    let _ = std::fs::remove_dir_all(&project);
    (
        output.status,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn a_recursive_value_built_into_another_is_independent_of_its_source() {
    let (status, stdout, stderr) = run("p134e_construction_copies", CONSTRUCTION_SOURCE);
    assert!(
        status.success(),
        "plan-134-E: growing a list stored into a construction crashed (status {status:?}): \
         the holder shares the list's block.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        stdout.trim(),
        [
            "record=1",
            "union=1",
            "literal=1",
            "append=1",
            "insert=1",
            "set=1",
            "prepend=1",
            "map=1",
            "with=1",
            "state=1",
            "source=3",
        ]
        .join("\n"),
        "plan-134-E: every holder keeps the value it was built from.\nstderr:\n{stderr}"
    );
}

/// One `Node` stored five ways. Three stores need a copy because `a` is read afterwards —
/// `LET c = a` (plan-134-D), `a` as a list-literal element inside `b`, and `a` as the
/// in-place `append` item (plan-134-E). The fresh constructor appended to `ys` needs none:
/// it is a new graph, so copying it would be a wasted second copy.
const COPY_COUNT_SOURCE: &str = r#"IMPORT io
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

SUB main()
  LET a AS Node = Node[kids := [], tag := 1]
  LET b AS Node = Node[kids := [a], tag := 2]
  MUT xs AS List OF Node = []
  xs = collections::append(xs, a)
  LET c AS Node = a
  MUT ys AS List OF Node = []
  ys = collections::append(ys, Node[kids := [], tag := 9])
  io::print("b=" & toString(len(b.kids)) & " xs=" & toString(len(xs)) & " c=" & toString(c.tag) & " ys=" & toString(len(ys)) & " a=" & toString(a.tag))
END SUB
"#;

/// Every relocation object in an `-ncode` dump whose `from`, `to` prefix and `kind` match.
fn count_calls(value: &serde_json::Value, from: &str, to_prefix: &str, kind: &str) -> usize {
    match value {
        serde_json::Value::Object(map) => {
            let here = map.get("from").and_then(|v| v.as_str()) == Some(from)
                && map
                    .get("to")
                    .and_then(|v| v.as_str())
                    .is_some_and(|to| to.starts_with(to_prefix))
                && map.get("kind").and_then(|v| v.as_str()) == Some(kind);
            usize::from(here)
                + map
                    .values()
                    .map(|child| count_calls(child, from, to_prefix, kind))
                    .sum::<usize>()
        }
        serde_json::Value::Array(items) => items
            .iter()
            .map(|child| count_calls(child, from, to_prefix, kind))
            .sum(),
        _ => 0,
    }
}

#[test]
fn a_construction_store_copies_an_aliased_source_once_and_a_fresh_value_never() {
    let project = common::temp_project("p134e_copy_count", COPY_COUNT_SOURCE);
    let exe = common::build_project(&project);
    let output = Command::new(&exe)
        .output()
        .expect("run the copy-count probe");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "b=1 xs=1 c=1 ys=1 a=1",
        "the copy-count probe's values"
    );
    let ncode = common::build_ncode(&project, "macos-aarch64", "p134e_copy_count");
    let copies = count_calls(&ncode, "_mfb_fn_main", "_mfb_thread_copy_", "branch26");
    assert_eq!(
        copies, 3,
        "plan-134-E: `main` copies `a` at its three reads-after stores and never copies the fresh \
         constructor appended to `ys`"
    );
    let _ = std::fs::remove_dir_all(&project);
}

/// Native collection builtins that build a new collection by byte-copying element payloads out
/// of their input: a rebuilding `append` (the source is not a `MUT` local), `removeAt`,
/// `filter`, and a map's `values`. Each copied `Node` payload's `kids` pointer used to be the
/// source element's own list block. Sharing between two collections cannot be observed through
/// values until something frees one of them (no in-place path reaches a collection nested in an
/// element — `get` copies, `FOR EACH` borrows), so this pins it structurally: each function must
/// now copy its result's element edges. Measured on the plan-134-E build before this fix: zero
/// copy calls in all four functions.
const BUILTIN_COPIES_SOURCE: &str = r#"IMPORT io
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

FUNC keep(n AS Node) AS Boolean
  RETURN n.tag > 0
END FUNC

FUNC viaAppend(xs AS List OF Node, h AS Node) AS Integer
  LET ys AS List OF Node = collections::append(xs, h)
  RETURN len(ys)
END FUNC

FUNC viaRemoveAt(xs AS List OF Node) AS Integer
  LET ys AS List OF Node = collections::removeAt(xs, 0)
  RETURN len(ys)
END FUNC

FUNC viaFilter(xs AS List OF Node) AS Integer
  LET ys AS List OF Node = collections::filter(xs, keep)
  RETURN len(ys)
END FUNC

FUNC viaValues(m AS Map OF String TO Node) AS Integer
  LET ys AS List OF Node = collections::values(m)
  RETURN len(ys)
END FUNC

SUB main()
  LET a AS Node = Node[kids := [], tag := 1]
  LET xs AS List OF Node = [a, a]
  LET m AS Map OF String TO Node = Map OF String TO Node { "k" := a }
  io::print(toString(viaAppend(xs, a)) & " " & toString(viaRemoveAt(xs)) & " " & toString(viaFilter(xs)) & " " & toString(viaValues(m)))
END SUB
"#;

#[test]
fn a_native_collection_builtin_copies_its_result_elements_graphs() {
    let project = common::temp_project("p134e_builtin_copies", BUILTIN_COPIES_SOURCE);
    let exe = common::build_project(&project);
    let output = Command::new(&exe)
        .output()
        .expect("run the builtin-copy probe");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "3 1 2 1",
        "the builtin-copy probe's values"
    );
    let ncode = common::build_ncode(&project, "macos-aarch64", "p134e_builtin_copies");
    for function in [
        "_mfb_fn_viaAppend",
        "_mfb_fn_viaRemoveAt",
        "_mfb_fn_viaFilter",
        "_mfb_fn_viaValues",
    ] {
        let copies = count_calls(&ncode, function, "_mfb_thread_copy_", "branch26");
        assert!(
            copies >= 1,
            "plan-134-E: `{function}` builds a List OF Node from another collection's payloads and \
             must copy their recursive edges (found {copies} copy calls)"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn a_value_built_from_the_list_it_is_appended_to_holds_the_old_list() {
    let (status, stdout, stderr) = run("p134e_self_reference", SELF_REFERENCE_SOURCE);
    assert!(
        status.success(),
        "plan-134-E: appending an element built from its own list crashed (status {status:?}): \
         the element stores the list's block.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        stdout.trim(),
        "len xs=2\nfirst.kids=0\nsecond.kids=1",
        "plan-134-E: an element holds the list as it was before its append.\nstderr:\n{stderr}"
    );
}
