//! plan-134-F: `_mfb_rt_graph_drop` is the exact inverse of the recursive-value copy.
//!
//! No program calls the drop walker until plan-134-G registers drops, so each case builds
//! one program twice with `--debug`: plain, and with `MFB_TEST_GRAPH_DROP` naming locals.
//! In the hooked build every `LET` or assignment of such a local whose type is a recursive
//! type deep-copies the value and immediately drops the copy through `_mfb_rt_graph_drop`
//! (`src/codegen/memory/arena/graph_drop.rs`). If the drop frees exactly the blocks the copy
//! allocated, the hooked program:
//!
//! * frees exactly the extra bytes it allocated, so it ends with the plain build's
//!   `arena.live_bytes`;
//! * prints what the plain build prints — it reads the source after every drop, and a freed
//!   chunk is scrubbed, so a drop that freed a block of the source changes what it reads;
//! * skips no double free.
//!
//! The hooked build must also allocate more than the plain one, or the hook never ran and
//! the comparison proves nothing.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::Command;

const HOOK_ENV: &str = "MFB_TEST_GRAPH_DROP";

/// Build `source` with `--debug`, the hook naming `hook` (or unset), and return the
/// executable (the host libc's on Linux).
fn build(name: &str, source: &str, hook: Option<&str>) -> PathBuf {
    let project = common::temp_project(name, source);
    let mut command = Command::new(common::mfb_exe());
    command.arg("build").arg("--debug").arg(&project);
    match hook {
        Some(locals) => command.env(HOOK_ENV, locals),
        None => command.env_remove(HOOK_ENV),
    };
    let output = command.output().expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} failed to build:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let chosen = written
        .iter()
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    PathBuf::from(chosen)
}

/// Run `exe`, require success, and return its stdout and the main arena's report lines.
fn run(name: &str, exe: &PathBuf) -> (String, Vec<String>) {
    let output = Command::new(exe).output().expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{name} failed:\n{stdout}\n{stderr}"
    );
    let at = stderr
        .rfind("mfb.debug.begin ")
        .unwrap_or_else(|| panic!("{name}: no report block on stderr:\n{stderr}"));
    let lines = stderr[at..]
        .lines()
        .take_while(|line| *line != "mfb.debug.end 1")
        .filter(|line| line.starts_with("arena.0."))
        .map(str::to_string)
        .collect();
    (stdout, lines)
}

/// `arena.0.<name>` of `lines`.
fn counter(case: &str, lines: &[String], name: &str) -> u64 {
    let key = format!("arena.0.{name} ");
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&key))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{case}: no `{key}<n>` in {lines:?}"))
}

/// The plain and hooked main-arena counters of one case.
struct Pair {
    plain: Vec<String>,
    hooked: Vec<String>,
}

/// Build and run `source` plain and with the hook on `hook`, and assert copy-then-drop is
/// exact (see the module comment).
fn assert_copy_then_drop_is_exact(case: &str, source: &str, hook: &str) -> Pair {
    let plain_exe = build(&format!("{case}_plain"), source, None);
    let hooked_exe = build(&format!("{case}_hooked"), source, Some(hook));
    let (plain_out, plain) = run(&format!("{case} (plain)"), &plain_exe);
    let (hooked_out, hooked) = run(&format!("{case} (hooked)"), &hooked_exe);
    assert_eq!(
        hooked_out, plain_out,
        "{case}: dropping the copies changed what the program read from its values"
    );
    let get = |lines: &[String], name: &str| counter(case, lines, name);
    assert!(
        get(&hooked, "alloc_calls") > get(&plain, "alloc_calls"),
        "{case}: the hook on `{hook}` copied nothing ({} alloc_calls plain, {} hooked) — \
         the comparison would prove nothing",
        get(&plain, "alloc_calls"),
        get(&hooked, "alloc_calls")
    );
    assert_eq!(
        get(&plain, "double_free_skips"),
        0,
        "{case}: plain double_free_skips"
    );
    assert_eq!(
        get(&hooked, "double_free_skips"),
        0,
        "{case}: the drops freed a block twice"
    );
    let extra_allocated = get(&hooked, "alloc_bytes") - get(&plain, "alloc_bytes");
    let extra_freed = get(&hooked, "free_bytes") - get(&plain, "free_bytes");
    assert_eq!(
        extra_freed, extra_allocated,
        "{case}: the drops freed {extra_freed} bytes of the {extra_allocated} the copies allocated"
    );
    assert_eq!(
        get(&hooked, "live_bytes"),
        get(&plain, "live_bytes"),
        "{case}: live_bytes after copy-then-drop differs from the program without it"
    );
    Pair { plain, hooked }
}

const USER_NODE_SOURCE: &str = r#"IMPORT io
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
  name AS String
END TYPE

FUNC chainTotal(top AS Node) AS Integer
  MUT walk AS Node = top
  MUT total AS Integer = 0
  WHILE len(walk.kids) > 0
    total = total + walk.tag + len(walk.name)
    walk = collections::get(walk.kids, 0)
  END WHILE
  RETURN total + walk.tag
END FUNC

SUB main()
  MUT cur AS Node = Node[kids := [], tag := 0, name := "leaf"]
  MUT i AS Integer = 1
  WHILE i <= 40
    LET side AS Node = Node[kids := [], tag := 0 - i, name := "side" & toString(i)]
    cur = Node[kids := [cur, side], tag := i, name := "n" & toString(i)]
    i = i + 1
  END WHILE
  LET probe AS Node = cur
  io::print(probe.name & " " & toString(len(probe.kids)) & " " & toString(chainTotal(cur)))
  io::print(toString(chainTotal(probe)))
END SUB
"#;

/// A user `TYPE` that holds a `List OF` itself, with an inlined `String` field in every block.
#[test]
fn a_user_recursive_record_drops_exactly_its_copy() {
    assert_copy_then_drop_is_exact("drop_user_node", USER_NODE_SOURCE, "probe,side,cur");
}

const USER_UNION_SOURCE: &str = r#"IMPORT io
IMPORT collections

TYPE Leaf
  v AS Integer
  label AS String
END TYPE

TYPE Branch
  kids AS List OF Tree
  weight AS Integer
END TYPE

TYPE Couple
  left AS Tree
  right AS Tree
END TYPE

UNION Tree
  Leaf
  Branch
  Couple
END UNION

FUNC weigh(t AS Tree) AS Integer
  MUT result AS Integer = 0
  MATCH t
    CASE Leaf(l)
      result = l.v + len(l.label)
    CASE Branch(b)
      result = b.weight
      FOR EACH k IN b.kids
        result = result + weigh(k)
      NEXT
    CASE Couple(c)
      result = weigh(c.left) * 3 + weigh(c.right)
  END MATCH
  RETURN result
END FUNC

SUB main()
  MUT t AS Tree = Leaf[v := 1, label := "a"]
  MUT i AS Integer = 0
  WHILE i < 30
    LET leaf AS Tree = Leaf[v := i, label := "leaf" & toString(i)]
    LET branch AS Tree = Branch[kids := [t, leaf], weight := i]
    t = Couple[left := branch, right := leaf]
    i = i + 1
  END WHILE
  LET probeTree AS Tree = t
  io::print(toString(weigh(probeTree)) & " " & toString(weigh(t)))
END SUB
"#;

/// A user recursive `UNION`: a variant holding a `List OF` the union, and a variant whose
/// record fields are the union itself.
#[test]
fn a_user_recursive_union_drops_exactly_its_copy() {
    assert_copy_then_drop_is_exact(
        "drop_user_union",
        USER_UNION_SOURCE,
        "probeTree,leaf,branch,t",
    );
}

const JSON_SOURCE: &str = r#"IMPORT io
IMPORT json

SUB main()
  MUT text AS String = "["
  MUT j AS Integer = 0
  WHILE j < 30
    text = text & "{\u{22}a\u{22}:[1,2.5,true,null],\u{22}b\u{22}:{\u{22}c\u{22}:\u{22}xyz\u{22}}},"
    j = j + 1
  END WHILE
  text = text & "0]"
  LET probeJson AS json::Json = json::parse(text)
  io::print(json::stringify(probeJson))
END SUB
"#;

/// `json::Json`: arrays and objects of itself.
#[test]
fn a_json_value_drops_exactly_its_copy() {
    assert_copy_then_drop_is_exact("drop_json", JSON_SOURCE, "probeJson");
}

const REGEX_SOURCE: &str = r#"IMPORT io
IMPORT regex

SUB main()
  MUT subject AS String = ""
  MUT j AS Integer = 0
  WHILE j < 20
    subject = subject & "abcab1234 aabac99 "
    j = j + 1
  END WHILE
  io::print(toString(len(regex::findAll(subject, "([a-c]+|b)[0-9]{2,3}"))))
  io::print(toString(regex::match(subject, "(ab|a)*c")))
END SUB
"#;

/// The regex engine's private recursive values, through the locals that hold them: the
/// parser's and `__regex_flatten`'s `node AS __regex_Node`, the flatten work stack
/// `work AS List OF __regex_Node`, and the parser's `parts AS List OF __regex_Node`. One
/// build per local, so each is shown to have been copied and dropped. (Until plan-134-I
/// the matcher's `stack AS __regex_Choices` and `cont AS __regex_Cont` were here; the
/// matcher now holds only integers.)
#[test]
fn the_regex_matcher_graphs_drop_exactly_their_copies() {
    for local in ["node", "work", "parts"] {
        assert_copy_then_drop_is_exact(&format!("drop_regex_{local}"), REGEX_SOURCE, local);
    }
}

const DEEP_CHAIN_SOURCE: &str = r#"IMPORT io

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

SUB main()
  MUT cur AS Node = Node[kids := [], tag := 0]
  MUT i AS Integer = 1
  WHILE i <= 1000000
    cur = Node[kids := [cur], tag := i]
    i = i + 1
  END WHILE
  LET probe AS Node = cur
  io::print("top=" & toString(probe.tag) & " " & toString(cur.tag))
END SUB
"#;

/// A 1 000 000-deep chain: the drop keeps its pending edges in the arena, not on the
/// native stack.
#[test]
fn a_million_deep_chain_drops_exactly_its_copy() {
    assert_copy_then_drop_is_exact("drop_deep_chain", DEEP_CHAIN_SOURCE, "probe");
}

const CHURN_SOURCE: &str = r#"IMPORT io
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
  name AS String
END TYPE

SUB main()
  MUT src AS Node = Node[kids := [], tag := 1, name := "leaf"]
  MUT d AS Integer = 0
  WHILE d < 6
    LET other AS Node = Node[kids := [], tag := d, name := "other" & toString(d)]
    src = Node[kids := [src, other], tag := d + 2, name := "level" & toString(d)]
    d = d + 1
  END WHILE
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 20000
    LET probe AS Node = src
    LET first AS Node = collections::get(src.kids, 0)
    total = total + probe.tag + len(src.kids) + len(first.name)
    i = i + 1
  END WHILE
  io::print("total=" & toString(total))
END SUB
"#;

/// 20 000 copy-then-drops, reading the source after each: every copy is freed as it goes,
/// so the hooked build's peak stays within one copy (and the work stack) of the plain
/// build's.
#[test]
fn churn_frees_every_copy_as_it_goes() {
    let pair = assert_copy_then_drop_is_exact("drop_churn", CHURN_SOURCE, "probe");
    let peak = |lines: &[String]| counter("drop_churn", lines, "peak_live_bytes");
    let slack: u64 = 64 * 1024;
    assert!(
        peak(&pair.hooked) <= peak(&pair.plain) + slack,
        "drop_churn: the hooked peak ({}) exceeds the plain peak ({}) by more than {slack} \
         bytes — the copies are not freed as they are dropped",
        peak(&pair.hooked),
        peak(&pair.plain)
    );
}
