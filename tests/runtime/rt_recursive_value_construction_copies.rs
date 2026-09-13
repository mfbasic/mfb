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
            "record=1", "union=1", "literal=1", "append=1", "insert=1", "set=1", "prepend=1",
            "map=1", "with=1", "state=1", "source=3",
        ]
        .join("\n"),
        "plan-134-E: every holder keeps the value it was built from.\nstderr:\n{stderr}"
    );
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
