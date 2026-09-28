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
IMPORT fs
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

// ---- plan-149-B3: global and `STATE` owners ----

/// A program with a `MUT` global record `g`.
fn owner_program(body: &str) -> String {
    format!("MUT g AS R = R[1, [1, 2, 3], Inner[[10, 20, 30]], 0]\n{body}")
}

#[test]
fn the_global_set_shape() {
    assert_same_output(
        "pf_global_set",
        &owner_program(
            "FUNC main AS Integer
  FOR j = 0 TO 2
    LET cur AS List OF Integer = g.xs
    LET v AS Integer = collections::get(cur, j)
    g = WITH g { xs := collections::set(cur, j, v + 10) }
  NEXT
  io::print(show(g.xs))
  RETURN 0
END FUNC
",
        ),
        "11,12,13,\n",
    );
}

#[test]
fn the_global_remove_at_shape() {
    assert_same_output(
        "pf_global_remove_at",
        &owner_program(
            "FUNC main AS Integer
  LET cur AS List OF Integer = g.xs
  g = WITH g { xs := collections::removeAt(cur, 0) }
  io::print(show(g.xs))
  RETURN 0
END FUNC
",
        ),
        "2,3,\n",
    );
}

#[test]
fn a_bare_global() {
    assert_same_output(
        "pf_global_bare",
        "MUT gl AS List OF Integer = [1, 2, 3]
FUNC main AS Integer
  LET snap AS List OF Integer = gl
  gl = collections::set(snap, 1, collections::get(snap, 0) + 40)
  io::print(show(gl))
  RETURN 0
END FUNC
",
        "1,41,3,\n",
    );
}

#[test]
fn the_state_set_and_remove_at_shapes() {
    assert_same_output(
        "pf_state_list",
        "FUNC main AS Integer
  RES f AS fs::File STATE R = fs::openFile(\"project.json\")
  f.state = mk()
  FOR j = 0 TO 2
    LET cur AS List OF Integer = f.state.xs
    LET v AS Integer = collections::get(cur, j)
    f.state.xs = collections::set(cur, j, v + 10)
  NEXT
  io::print(show(f.state.xs))
  LET cur2 AS List OF Integer = f.state.xs
  f.state.xs = collections::removeAt(cur2, 1)
  io::print(show(f.state.xs))
  fs::close(f)
  RETURN 0
END FUNC
",
        "11,12,13,\n11,13,\n",
    );
}

#[test]
fn the_state_remove_key_and_remove_shapes() {
    assert_same_output(
        "pf_state_map_set",
        "TYPE Bag
  m AS Map OF Integer TO Integer
  s AS Set OF Integer
END TYPE
FUNC main AS Integer
  RES f AS fs::File STATE Bag = fs::openFile(\"project.json\")
  f.state = Bag[Map OF Integer TO Integer { 1 := 10, 2 := 20, 3 := 30 }, Set OF Integer { 4, 5, 6 }]
  LET m AS Map OF Integer TO Integer = f.state.m
  f.state.m = collections::removeKey(m, 2)
  LET s AS Set OF Integer = f.state.s
  f.state.s = collections::remove(s, 5)
  io::print(toString(len(f.state.m)) & \" \" & toString(collections::hasKey(f.state.m, 2)))
  io::print(toString(len(f.state.s)) & \" \" & toString(collections::contains(f.state.s, 5)))
  fs::close(f)
  RETURN 0
END FUNC
",
        "2 FALSE\n2 FALSE\n",
    );
}

/// W4: a user `SUB` called in the window writes the global; the read after it
/// must still see the value from before the call.
#[test]
fn w4_a_user_sub_writing_the_global_keeps_the_old_value() {
    assert_same_output(
        "pf_w4_sub",
        &owner_program(
            "FUNC bump() AS Nothing
  g = WITH g { xs := collections::set(g.xs, 0, 100) }
END FUNC
FUNC main AS Integer
  LET cur AS List OF Integer = g.xs
  bump()
  g = WITH g { xs := collections::append(cur, 7) }
  io::print(show(g.xs))
  RETURN 0
END FUNC
",
        ),
        "1,2,3,7,\n",
    );
}

/// W4 on op L's own value: `helper()` runs while the owner write's arguments
/// are evaluated. `cur` was taken before it ran.
#[test]
fn w4_a_user_call_inside_the_owner_write_keeps_the_old_value() {
    assert_same_output(
        "pf_w4_inside",
        &owner_program(
            "FUNC bumpAndGive() AS Integer
  g = WITH g { xs := collections::set(g.xs, 0, 100) }
  RETURN 5
END FUNC
FUNC main AS Integer
  LET cur AS List OF Integer = g.xs
  g = WITH g { xs := collections::set(cur, 1, bumpAndGive()) }
  io::print(show(g.xs))
  RETURN 0
END FUNC
",
        ),
        "1,5,3,\n",
    );
}

/// W5: the resource is handed to a user `SUB` that rewrites its `STATE`.
#[test]
fn w5_passing_the_resource_keeps_the_old_value() {
    assert_same_output(
        "pf_w5",
        "FUNC touch(RES f AS fs::File STATE R) AS Nothing
  f.state.xs = collections::set(f.state.xs, 0, 100)
END FUNC
FUNC main AS Integer
  RES f AS fs::File STATE R = fs::openFile(\"project.json\")
  f.state = mk()
  LET cur AS List OF Integer = f.state.xs
  touch(f)
  f.state.xs = collections::append(cur, 7)
  io::print(show(f.state.xs))
  fs::close(f)
  RETURN 0
END FUNC
",
        "1,2,3,7,\n",
    );
}

/// The benchmark row `test_lrf_set` verbatim in shape: `cur` is bound in the
/// loop and bound again after it. Both bindings are forwarded (plan-149-C3).
#[test]
fn a_name_reused_after_the_loop() {
    assert_same_output(
        "pf_reused_name",
        "FUNC main AS Integer
  MUT rec AS R = mk()
  FOR pass = 0 TO 1
    FOR j = 0 TO 2
      LET cur AS List OF Integer = rec.xs
      LET v AS Integer = collections::get(cur, j)
      rec = WITH rec { xs := collections::set(cur, j, v + 1) }
    NEXT
  NEXT
  LET cur AS List OF Integer = rec.xs
  MUT sumv AS Integer = 0
  FOR j = 0 TO 2
    sumv = sumv + collections::get(cur, j)
  NEXT
  io::print(show(cur) & \" \" & toString(sumv))
  RETURN 0
END FUNC
",
        "3,4,5, 12\n",
    );
}
