//! bug-753: a `List OF RES` bound from a call leaked its collection block on every
//! bind — 144 B per `LET p = collections::take(la, 2)` in a loop, linear in the
//! iteration count. Only a literal or a self-receiving mutator counted as a fresh
//! block the binding owns. The audited builtin bodies that always allocate their
//! result (`take`/`drop` over the native `slice`) now count too, so the binding
//! frees the block at drop. The handles themselves are aliases of `a` and are not
//! closed by that free: the list is still readable and `a` still reads afterwards.
//!
//! Measured with the `--debug` arena counters: peak live bytes must not move
//! between N and 2N iterations, and every allocation must be freed.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const SOURCE: &str = r#"IMPORT io
IMPORT collections
IMPORT fs
IMPORT os

FUNC main() AS Nothing
  LET path AS String = os::getEnv("PROBE")
  fs::writeText(path, "abc\n")
  RES a AS fs::File = fs::open(path, "read")
  LET la AS List OF RES fs::File = [a, a, a]
  LET n AS Integer = toInt(os::getEnv("N"))
  MUT i AS Integer = 0
  MUT total AS Integer = 0
  WHILE i < n
    LET p = collections::take(la, 2)
    LET d = collections::drop(la, 1)
    total = total + len(p) + len(d)
    i = i + 1
  END WHILE
  io::print(toString(total))
  io::print(toString(len(la)) & " " & fs::readLine(a))
END FUNC
"#;

fn counter(stats: &str, key: &str) -> u64 {
    stats
        .lines()
        .find_map(|line| line.strip_prefix(&format!("arena.0.{key} ")))
        .unwrap_or_else(|| panic!("no arena.0.{key} in:\n{stats}"))
        .trim()
        .parse()
        .expect("numeric counter")
}

#[test]
fn a_res_list_bound_from_take_or_drop_frees_its_block() {
    let project = common::temp_project("bug753_res_list_from_call", SOURCE);
    let built = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    assert!(
        built.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let executable = project.join("build").join(format!(
        "bug753_res_list_from_call{}",
        if cfg!(windows) { ".exe" } else { ".out" }
    ));
    let probe = project.join("probe.txt");
    let run = |n: u64| {
        let output = Command::new(&executable)
            .env("N", n.to_string())
            .env("PROBE", &probe)
            .output()
            .expect("run the probe");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stats = format!("{stdout}{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "probe failed:\n{stats}");
        assert_eq!(
            stdout.lines().take(2).collect::<Vec<_>>(),
            vec![(n * 4).to_string(), "3 abc".to_string()],
            "the lists and the handle are still intact"
        );
        stats
    };
    let small = run(1000);
    let large = run(2000);
    assert_eq!(
        counter(&small, "peak_live_bytes"),
        counter(&large, "peak_live_bytes"),
        "peak live bytes grew with the iteration count"
    );
    for stats in [&small, &large] {
        assert_eq!(
            counter(stats, "alloc_calls"),
            counter(stats, "free_calls"),
            "every allocation is freed"
        );
    }
}
