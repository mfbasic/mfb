//! `RETURN collections::transform(x, f)` where `f` maps to ANOTHER element type.
//!
//! plan-147-B (site S11) runs `RETURN OP(x, …)` on an owned local at its last use
//! as the in-place arm `x = OP(x, …)` would, then returns `x`. The `transform` arm
//! rewrites `x`'s elements where they are, which needs `f` to return `x`'s element
//! type — guaranteed for an assignment by the type check, but not for a `RETURN`,
//! whose result is only constrained by the function's return type. A mapping from
//! `List OF Integer` to `List OF String` failed the whole build with "native
//! in-place transform of List OF Integer needs a FUNC returning Integer, got
//! String". The arm now declines that shape before emitting anything, and the
//! copying `transform` builds the new list.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

fn build_and_run(name: &str, source: &str) -> String {
    let project = common::temp_project(name, source);
    let exe = common::build_project(&project);
    let output = Command::new(&exe).output().expect("run the program");
    assert!(
        output.status.success(),
        "{name} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_returned_transform_may_change_the_element_type() {
    let stdout = build_and_run(
        "returned_transform_element_type",
        r##"IMPORT io
IMPORT collections
IMPORT strings

FUNC fromParam(xs AS List OF Integer) AS List OF String
  RETURN collections::transform(xs, LAMBDA(v AS Integer) -> toString(v) & "!")
END FUNC

FUNC fromLocal(n AS Integer) AS List OF String
  MUT xs AS List OF Integer = [n, n + 1]
  xs = collections::append(xs, n + 2)
  RETURN collections::transform(xs, LAMBDA(v AS Integer) -> "#" & toString(v))
END FUNC

FUNC sameType(xs AS List OF Integer) AS List OF Integer
  RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v * 10)
END FUNC

FUNC main AS Integer
  io::print(strings::join(fromParam([1, 2, 3]), ","))
  io::print(strings::join(fromLocal(7), ","))
  LET tens AS List OF Integer = sameType([4, 5])
  io::print(toString(collections::get(tens, 0) + collections::get(tens, 1)))
  RETURN 0
END FUNC
"##,
    );
    assert_eq!(stdout, "1!,2!,3!\n#7,#8,#9\n90\n");
}
