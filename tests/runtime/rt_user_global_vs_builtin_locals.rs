//! A user's top-level binding does not collide with a local inside a built-in
//! package member.
//!
//! `collections::union` (and every other `Body::Mfb` member) is MFBASIC source in
//! an internal file, exempt from `SYMBOL_SHADOWS_TOP_LEVEL_BINDING` — the user's
//! bindings are not its own. But the monomorphizer emits each instantiation into
//! the user's first file, so a program with a top-level `MUT x` that called
//! `collections::union` failed to compile:
//! `src/main.mfb:347 error[2-201-0022 SYMBOL_SHADOWS_TOP_LEVEL_BINDING]: `x` is
//! already a top-level binding` — line 347 of the package source, blamed on a
//! user file 11 lines long. Found by plan-142-H's S2 harness, whose global is `x`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const SOURCE: &str = "\
IMPORT collections
IMPORT io

MUT x AS Set OF Integer = Set OF Integer { 1, 2 }

FUNC negated(n AS Integer) AS Integer
  RETURN 0 - n
END FUNC

FUNC main() AS Integer
  LET ys AS Set OF Integer = Set OF Integer { 2, 3 }
  x = collections::union(x, ys)
  LET sorted AS List OF Integer = collections::sortBy([3, 1, 2], negated)
  LET m AS Map OF String TO Integer = collections::mapValues(Map OF String TO Integer { \"a\" := 1 }, negated)
  io::print(toString(len(x)) & \" \" & toString(collections::get(sorted, 0)) & \" \" & toString(collections::get(m, \"a\")))
  RETURN 0
END FUNC
";

#[test]
fn a_top_level_x_builds_beside_builtin_members_with_a_local_x() {
    let project = common::temp_project("user_global_vs_builtin_locals", SOURCE);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "build failed:\n{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix("Wrote executable to "))
        })
        .expect("an executable");
    let run = Command::new(exe).output().expect("run the program");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim_end(), "3 3 -1");
}
