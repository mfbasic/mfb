//! plan-134-H: discarding a recursive element from a collection frees that element's graph.
//!
//! A collection of recursive values owns its elements (`mfb spec language memory-semantics`
//! §14.6). Before this letter the in-place arms that remove or overwrite an element —
//! `set`, `removeAt`, `removeKey`, on a local, a record field or through a rebuild — compact
//! over or overwrite the element's bytes and free nothing of the graph they pointed at
//! (the plan-134-H Phase 1 census: 17 discarding functions, none freeing the element). A
//! collection whose element type only REACHES a cycle has no walker kind at all, so its drop
//! frees nothing either.
//!
//! Each case discards and re-adds one element per iteration, so the collection's size is the
//! same at every count. Built with `--debug`, the program's final `arena.0.live_bytes` must
//! therefore be the same at 1 000 and at 2 000 iterations, with no double free and the same
//! output. A leak shows as `live_bytes` growing with the count; a free of a block still in use
//! shows as a crash or a changed output (freed chunks are scrubbed).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// Build `source` (its `{N}` replaced by `count`) with `--debug`, run it, and return its
/// stdout, `arena.0.live_bytes` and `arena.0.double_free_skips`.
fn run_debug(case: &str, source: &str, count: u64) -> (String, u64, u64) {
    let name = format!("{case}_{count}");
    let project = common::temp_project(&name, &source.replace("{N}", &count.to_string()));
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let build_stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} failed to build:\n{build_stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Vec<&str> = build_stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let exe = written
        .iter()
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{build_stdout}"));
    let output = Command::new(exe).output().expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{name} failed (status {:?}):\n{stdout}\n{stderr}",
        output.status
    );
    let counter = |key: &str| {
        stderr
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|n| n.trim().parse::<u64>().ok())
            .unwrap_or_else(|| panic!("{name}: no `{key}<n>` in the report:\n{stderr}"))
    };
    let live = counter("arena.0.live_bytes ");
    let skips = counter("arena.0.double_free_skips ");
    let _ = std::fs::remove_dir_all(&project);
    (stdout, live, skips)
}

/// The discard loop leaves `arena.live_bytes` where it was: the same at 1 000 and 2 000
/// iterations, no double free, the same output.
fn assert_discards_free_the_element(case: &str, source: &str) {
    let (small_out, small_live, small_skips) = run_debug(case, source, 1_000);
    let (large_out, large_live, large_skips) = run_debug(case, source, 2_000);
    assert_eq!(
        large_out, small_out,
        "{case}: the output changed with the iteration count"
    );
    assert_eq!(small_skips, 0, "{case}: a block was freed twice (1 000 iterations)");
    assert_eq!(large_skips, 0, "{case}: a block was freed twice (2 000 iterations)");
    assert_eq!(
        large_live,
        small_live,
        "{case}: arena.live_bytes grew by {} bytes over 1 000 more discards — the discarded \
         element's graph is not freed",
        large_live.saturating_sub(small_live)
    );
}

/// The shared declarations: `mk(t)` builds a two-level `Node` graph (a separately allocated
/// `kids` list holding one child), and `total` reads a whole graph.
const PRELUDE: &str = r#"IMPORT io
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

TYPE Holder
  items AS List OF Node
  n AS Integer
END TYPE

TYPE Keyed
  byName AS Map OF String TO Node
  n AS Integer
END TYPE

TYPE Rep
  child AS Node
  n AS Integer
END TYPE

FUNC mk(t AS Integer) AS Node
  RETURN Node[kids := [Node[kids := [], tag := t]], tag := t]
END FUNC

FUNC total(n AS Node) AS Integer
  MUT sum AS Integer = n.tag
  FOR EACH k IN n.kids
    sum = sum + total(k)
  NEXT
  RETURN sum
END FUNC
"#;

fn program(body: &str) -> String {
    format!("{PRELUDE}\n{body}")
}

#[test]
fn an_in_place_list_set_frees_the_overwritten_element() {
    assert_discards_free_the_element(
        "h_list_set",
        &program(
            r#"SUB main()
  MUT xs AS List OF Node = [mk(0), mk(1)]
  MUT i AS Integer = 0
  WHILE i < {N}
    xs = collections::set(xs, 0, mk(5))
    i = i + 1
  END WHILE
  io::print("len=" & toString(len(xs)) & " first=" & toString(total(collections::get(xs, 0))))
END SUB"#,
        ),
    );
}

#[test]
fn a_list_remove_at_frees_the_removed_element() {
    assert_discards_free_the_element(
        "h_list_remove_at",
        &program(
            r#"SUB main()
  MUT xs AS List OF Node = [mk(0), mk(1)]
  MUT i AS Integer = 0
  WHILE i < {N}
    xs = collections::removeAt(xs, 0)
    xs = collections::append(xs, mk(5))
    i = i + 1
  END WHILE
  io::print("len=" & toString(len(xs)) & " last=" & toString(total(collections::get(xs, 1))))
END SUB"#,
        ),
    );
}

#[test]
fn a_map_set_over_an_existing_key_frees_the_old_value() {
    assert_discards_free_the_element(
        "h_map_set",
        &program(
            r#"SUB main()
  MUT m AS Map OF String TO Node = Map OF String TO Node { "k" := mk(0) }
  MUT i AS Integer = 0
  WHILE i < {N}
    m = collections::set(m, "k", mk(5))
    i = i + 1
  END WHILE
  io::print("len=" & toString(len(m)) & " k=" & toString(total(collections::get(m, "k"))))
END SUB"#,
        ),
    );
}

#[test]
fn a_map_remove_key_frees_the_removed_value() {
    assert_discards_free_the_element(
        "h_map_remove_key",
        &program(
            r#"SUB main()
  MUT m AS Map OF String TO Node = Map OF String TO Node { "k" := mk(0), "j" := mk(1) }
  MUT i AS Integer = 0
  WHILE i < {N}
    m = collections::removeKey(m, "k")
    m = collections::set(m, "k", mk(5))
    i = i + 1
  END WHILE
  io::print("len=" & toString(len(m)) & " k=" & toString(total(collections::get(m, "k"))))
END SUB"#,
        ),
    );
}

#[test]
fn a_record_field_list_set_frees_the_overwritten_element() {
    assert_discards_free_the_element(
        "h_field_set",
        &program(
            r#"SUB main()
  MUT h AS Holder = Holder[items := [mk(0), mk(1)], n := 2]
  MUT i AS Integer = 0
  WHILE i < {N}
    h = WITH h { items := collections::set(h.items, 0, mk(5)) }
    i = i + 1
  END WHILE
  io::print("len=" & toString(len(h.items)) & " first=" & toString(total(collections::get(h.items, 0))))
END SUB"#,
        ),
    );
}

#[test]
fn a_record_field_list_remove_at_frees_the_removed_element() {
    assert_discards_free_the_element(
        "h_field_remove_at",
        &program(
            r#"SUB main()
  MUT h AS Holder = Holder[items := [mk(0), mk(1)], n := 2]
  MUT i AS Integer = 0
  WHILE i < {N}
    h = WITH h { items := collections::removeAt(h.items, 0) }
    h = WITH h { items := collections::append(h.items, mk(5)) }
    i = i + 1
  END WHILE
  io::print("len=" & toString(len(h.items)) & " last=" & toString(total(collections::get(h.items, 1))))
END SUB"#,
        ),
    );
}

#[test]
fn a_record_field_map_remove_key_frees_the_removed_value() {
    assert_discards_free_the_element(
        "h_field_remove_key",
        &program(
            r#"SUB main()
  MUT h AS Keyed = Keyed[byName := Map OF String TO Node { "k" := mk(0), "j" := mk(1) }, n := 2]
  MUT i AS Integer = 0
  WHILE i < {N}
    h = WITH h { byName := collections::removeKey(h.byName, "k") }
    h = WITH h { byName := collections::set(h.byName, "k", mk(5)) }
    i = i + 1
  END WHILE
  io::print("len=" & toString(len(h.byName)) & " k=" & toString(total(collections::get(h.byName, "k"))))
END SUB"#,
        ),
    );
}

/// A collection whose element type only reaches a cycle (`Rep` holds a `Node`, but nothing in
/// `Rep` leads back to `Rep`) has no walker kind before this letter, so binding one per
/// iteration freed nothing.
#[test]
fn a_collection_of_a_type_that_only_reaches_a_cycle_is_freed() {
    assert_discards_free_the_element(
        "h_reaching_list",
        &program(
            r#"SUB main()
  MUT acc AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET rs AS List OF Rep = [Rep[child := mk(3), n := 1]]
    acc = acc + len(rs)
    i = i + 1
  END WHILE
  io::print("done " & toString(acc > 0))
END SUB"#,
        ),
    );
}
