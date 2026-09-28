//! plan-149: place copy forwarding keeps every program's output.
//!
//! The row deletes `LET b = <place>` and points `b`'s reads at the place, but
//! only inside a window where nothing writes the place's owner. A wrong window
//! is a silent wrong answer: a forwarded read would see the owner's *new*
//! value. So each program here is built at `-O0` (no forwarding) and at `-O1`
//! (the default, where the row runs) and must print the same thing — and the
//! literal expected output, so the two levels cannot agree on a wrong answer.
//!
//! The forwarding shapes mirror the unit tests in
//! `src/optimizer/opt1/placefwd/tests/mod.rs`; the declining shapes (W1, W2) are
//! the ones where forwarding would change the output, so they prove the gate,
//! not just the rewrite.

#[path = "../common/mod.rs"]
mod common;

use std::path::Path;
use std::process::Command;

fn build_and_run(project: &Path, level: u8) -> String {
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(format!("-O{level}"))
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "-O{level} build failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let executable = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names the executable")
        .to_string();
    let run = Command::new(&executable)
        .current_dir(project)
        .output()
        .expect("run program");
    assert!(
        run.status.success(),
        "-O{level} program failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8(run.stdout).expect("utf8 program output")
}

/// Build `body` at `-O0` and `-O1`; both must print `expected`.
fn assert_same_output(name: &str, body: &str, expected: &str) {
    let source = format!("{PRELUDE}{body}");
    let project = common::temp_project(name, &source);
    let o0 = build_and_run(&project, 0);
    let o1 = build_and_run(&project, 1);
    let _ = std::fs::remove_dir_all(&project);
    assert_eq!(o0, expected, "-O0 output");
    assert_eq!(o1, expected, "-O1 output must match -O0");
}

const PRELUDE: &str = "\
IMPORT collections
IMPORT io
TYPE Inner
  xs AS List OF Integer
END TYPE
TYPE R
  before AS Integer
  xs AS List OF Integer
  a AS Inner
  after AS Integer
END TYPE
FUNC mk() AS R
  RETURN R[1, [1, 2, 3], Inner[[10, 20, 30]], 0]
END FUNC
FUNC show(xs AS List OF Integer) AS String
  MUT out AS String = \"\"
  FOR EACH x IN xs
    out = out & toString(x) & \",\"
  NEXT
  RETURN out
END FUNC
";

#[test]
fn the_benchmark_set_shape() {
    assert_same_output(
        "pf_set",
        "FUNC main AS Integer
  MUT rec AS R = mk()
  FOR j = 0 TO 2
    LET cur AS List OF Integer = rec.xs
    LET v AS Integer = collections::get(cur, j)
    rec = WITH rec { xs := collections::set(cur, j, v + 10) }
  NEXT
  io::print(show(rec.xs))
  RETURN 0
END FUNC
",
        "11,12,13,\n",
    );
}

#[test]
fn the_remove_at_shape() {
    assert_same_output(
        "pf_remove_at",
        "FUNC main AS Integer
  MUT rec AS R = mk()
  LET cur AS List OF Integer = rec.xs
  rec = WITH rec { xs := collections::removeAt(cur, 1) }
  io::print(show(rec.xs))
  RETURN 0
END FUNC
",
        "1,3,\n",
    );
}

#[test]
fn a_nested_path() {
    assert_same_output(
        "pf_nested",
        "FUNC main AS Integer
  MUT rec AS R = mk()
  LET cur AS List OF Integer = rec.a.xs
  rec = WITH rec { a := Inner[collections::set(cur, 0, 99)] }
  io::print(show(rec.a.xs) & \" \" & show(rec.xs))
  RETURN 0
END FUNC
",
        "99,20,30, 1,2,3,\n",
    );
}

#[test]
fn a_bare_local() {
    assert_same_output(
        "pf_bare",
        "FUNC main AS Integer
  MUT nums AS List OF Integer = [1, 2, 3]
  FOR j = 1 TO 3
    LET snap AS List OF Integer = nums
    nums = collections::set(snap, 0, collections::get(snap, 0) + j)
  NEXT
  io::print(show(nums))
  RETURN 0
END FUNC
",
        "7,2,3,\n",
    );
}

#[test]
fn a_read_only_window_to_the_block_end() {
    assert_same_output(
        "pf_readonly",
        "FUNC main AS Integer
  LET rec AS R = mk()
  LET cur AS List OF Integer = rec.xs
  io::print(show(cur))
  io::print(toString(len(cur)))
  RETURN 0
END FUNC
",
        "1,2,3,\n3\n",
    );
}

#[test]
fn a_read_in_a_nested_for() {
    assert_same_output(
        "pf_nested_for",
        "FUNC main AS Integer
  MUT rec AS R = mk()
  LET cur AS List OF Integer = rec.xs
  MUT total AS Integer = 0
  FOR k = 0 TO 2
    total = total + collections::get(cur, k)
  NEXT
  rec = WITH rec { xs := collections::removeAt(cur, 0), after := total }
  io::print(show(rec.xs) & \" \" & toString(rec.after))
  RETURN 0
END FUNC
",
        "2,3, 6\n",
    );
}

/// Op L's own value reads the place twice: once as the operand the in-place
/// arm mutates, once as a sibling. The sibling must see the old value.
#[test]
fn sibling_reads_in_the_owner_write() {
    assert_same_output(
        "pf_sibling",
        "FUNC main AS Integer
  MUT rec AS R = mk()
  LET cur AS List OF Integer = rec.xs
  rec = WITH rec { xs := collections::set(cur, 0, 9), before := collections::get(cur, 0) }
  io::print(show(rec.xs) & \" \" & toString(rec.before))
  MUT nums AS List OF Integer = [1, 2, 3]
  LET snap AS List OF Integer = nums
  nums = collections::append(collections::set(snap, 0, 9), collections::get(snap, 0))
  io::print(show(nums))
  RETURN 0
END FUNC
",
        "9,2,3, 1\n9,2,3,1,\n",
    );
}

/// W1: the owner is replaced between the binding and the read; the read must
/// see the value from before the write.
#[test]
fn w1_an_owner_write_before_a_read_keeps_the_old_value() {
    assert_same_output(
        "pf_w1",
        "FUNC main AS Integer
  MUT rec AS R = mk()
  LET cur AS List OF Integer = rec.xs
  rec = WITH rec { xs := collections::set(rec.xs, 0, 50) }
  io::print(show(cur) & \" \" & show(rec.xs))
  RETURN 0
END FUNC
",
        "1,2,3, 50,2,3,\n",
    );
}

/// W2: the owner write is nested in the op holding the last read, so a later
/// iteration's read must still see the value from before the loop.
#[test]
fn w2_an_owner_write_nested_in_the_last_op_keeps_the_old_value() {
    assert_same_output(
        "pf_w2",
        "FUNC main AS Integer
  MUT rec AS R = mk()
  LET cur AS List OF Integer = rec.xs
  FOR k = 0 TO 2
    rec = WITH rec { xs := collections::set(rec.xs, k, collections::get(cur, k) * 100 + collections::get(rec.xs, 0)) }
  NEXT
  io::print(show(rec.xs))
  RETURN 0
END FUNC
",
        "101,301,401,\n",
    );
}
