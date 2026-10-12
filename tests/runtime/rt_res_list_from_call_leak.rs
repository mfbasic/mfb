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

/// bug-753, the user-function half (unblocked by bug-755): a `List OF RES` bound
/// from a user function whose every `RETURN` hands back a block it built — a
/// slice, a literal, or a local it accumulated — is freed at drop. `same` returns
/// its own parameter, the caller's block, so its result must NOT be freed: the
/// caller's `la` is read after the loop.
const USER_SOURCE: &str = r#"IMPORT io
IMPORT collections
IMPORT fs
IMPORT os

FUNC firstTwo(xs AS List OF RES fs::File) AS List OF RES fs::File
  RETURN collections::take(xs, 2)
END FUNC

FUNC pair(h AS fs::File) AS List OF RES fs::File
  RETURN [h, h]
END FUNC

FUNC gather(xs AS List OF RES fs::File) AS List OF RES fs::File
  MUT out AS List OF RES fs::File = []
  out = collections::append(out, collections::get(xs, 0))
  out = collections::append(out, collections::get(xs, 1))
  RETURN out
END FUNC

FUNC same(xs AS List OF RES fs::File) AS List OF RES fs::File
  RETURN xs
END FUNC

FUNC main() AS Nothing
  LET path AS String = os::getEnv("PROBE")
  fs::writeText(path, "abc\n")
  RES a AS fs::File = fs::open(path, "read")
  LET la AS List OF RES fs::File = [a, a, a]
  LET n AS Integer = toInt(os::getEnv("N"))
  MUT i AS Integer = 0
  MUT total AS Integer = 0
  WHILE i < n
    LET p = firstTwo(la)
    LET q = pair(a)
    LET r = gather(la)
    LET s = same(la)
    total = total + len(p) + len(q) + len(r) + len(s)
    i = i + 1
  END WHILE
  io::print(toString(total))
  io::print(toString(len(la)) & " " & fs::readLine(a))
END FUNC
"#;

#[test]
fn a_res_list_bound_from_a_user_function_frees_its_block() {
    let project = common::temp_project("bug753_res_list_from_user_call", USER_SOURCE);
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
        "bug753_res_list_from_user_call{}",
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
            vec![(n * 9).to_string(), "3 abc".to_string()],
            "the lists and the handle are still intact"
        );
        stats
    };
    let small = run(500);
    let large = run(1000);
    assert_eq!(
        counter(&small, "peak_live_bytes"),
        counter(&large, "peak_live_bytes"),
        "peak live bytes grew with the iteration count"
    );
    // Nothing outlives the run. A `List OF RES` is a block of aliases and follows
    // the rules of any flat list: `LET s = same(la)` copies the block `same` hands
    // back (its own parameter, the caller's block), so `s` and `la` each own one
    // and each frees its own.
    for stats in [&small, &large] {
        assert_eq!(
            counter(stats, "alloc_calls"),
            counter(stats, "free_calls"),
            "every block is freed, `la`'s included"
        );
        assert_eq!(
            counter(stats, "double_free_skips"),
            0,
            "no block is freed twice"
        );
    }
}

/// bug-753, the native members: `filter`, `append`, `prepend`, `insert`, `set`,
/// `removeAt` and `mid` each return a block they allocated (audited per lowering,
/// `record_ownership::FRESH_NATIVE_MEMBERS`), and the singleton / removed-copy
/// intermediates they build for a `RES` element are freed too.
const MEMBER_SOURCE: &str = r#"IMPORT io
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
    LET f = collections::filter(la, LAMBDA(x AS fs::File) -> TRUE)
    LET ap = collections::append(la, a)
    LET pp = collections::prepend(la, a)
    LET ins = collections::insert(la, 1, a)
    LET st = collections::set(la, 0, a)
    LET ra = collections::removeAt(la, 0)
    LET m = collections::mid(la, 1, 1)
    total = total + len(f) + len(ap) + len(pp) + len(ins) + len(st) + len(ra) + len(m)
    i = i + 1
  END WHILE
  io::print(toString(total))
  io::print(toString(len(la)) & " " & fs::readLine(a))
END FUNC
"#;

#[test]
fn a_res_list_bound_from_a_native_member_frees_its_block() {
    let project = common::temp_project("bug753_res_list_from_member", MEMBER_SOURCE);
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
        "bug753_res_list_from_member{}",
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
            vec![(n * 21).to_string(), "3 abc".to_string()],
            "the lists and the handle are still intact"
        );
        stats
    };
    let small = run(500);
    let large = run(1000);
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

/// `chunks` and `window` over a `List OF RES` return a `List OF List OF RES`, which
/// holds its inner lists by pointer. A binding of one that nothing else holds frees
/// each inner block and then the outer one; inside the bodies, the copying
/// `result = collections::append(result, piece)` frees the block it replaces even
/// though `result` is returned. On main every iteration leaked seven blocks.
const NESTED_SOURCE: &str = r#"IMPORT io
IMPORT fs
IMPORT os
IMPORT collections

FUNC main AS Integer
  LET path AS String = os::getEnv("PROBE")
  fs::writeText(path, "abc\n")
  MUT xs AS List OF RES fs::File = []
  FOR i = 1 TO 4
    RES f AS fs::File = fs::open(path, "read")
    xs = collections::append(xs, f)
  NEXT
  LET n AS Integer = toInt(os::getEnv("N"))
  MUT total AS Integer = 0
  FOR i = 1 TO n
    LET parts = collections::chunks(xs, 2)
    LET wins = collections::window(xs, 2)
    total = total + len(parts) + len(wins)
  NEXT
  io::print(toString(total))
  RES g AS fs::File = collections::get(xs, 3)
  io::print(fs::readLine(g))
  RETURN 0
END FUNC
"#;

#[test]
fn a_nested_res_list_from_chunks_or_window_frees_every_block() {
    let project = common::temp_project("bug753_res_list_nested", NESTED_SOURCE);
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
        "bug753_res_list_nested{}",
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
            vec![(n * 5).to_string(), "abc".to_string()],
            "the lists and the handles are still intact"
        );
        stats
    };
    let small = run(300);
    let large = run(600);
    assert_eq!(
        counter(&small, "peak_live_bytes"),
        counter(&large, "peak_live_bytes"),
        "peak live bytes grew with the iteration count"
    );
    for stats in [&small, &large] {
        assert_eq!(
            counter(stats, "double_free_skips"),
            0,
            "no block is freed twice"
        );
    }
    assert_eq!(
        counter(&large, "alloc_calls") - counter(&large, "free_calls"),
        counter(&small, "alloc_calls") - counter(&small, "free_calls"),
        "the unfreed count does not grow with the iteration count"
    );
}
