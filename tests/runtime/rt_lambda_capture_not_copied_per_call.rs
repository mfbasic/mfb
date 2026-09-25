//! A lambda reads a collection it captured by value without copying it per call.
//!
//! A closure's env holds its own copy of every captured collection, made once
//! when the closure is created, and nothing can change it: a by-value capture is
//! an immutable binding inside the lambda. The lambda's prologue nevertheless
//! deep-copied it into the binding on every invocation (and freed it on return) —
//! `collections::forEach(src, LAMBDA(v AS Integer) -> total = total + len(ys))`
//! over 1000 elements allocated 1015 blocks, one per call. Found by plan-142-G's
//! S9 harness, where every `forEach` lambda capturing an operand paid it.
//!
//! Each program calls a lambda `N` and `2N` times; the extra `N` calls must
//! allocate (almost) nothing, and the program must print the right result and
//! free every block.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// `upTo(n)`: `[1, 2, …, n]`.
const UP_TO: &str = "\
FUNC upTo(n AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 1 TO n
    xs = collections::append(xs, i)
  NEXT
  RETURN xs
END FUNC
";

/// `(name, main body with {N}, expected output at N = 1000)`.
const CASES: &[(&str, &str, &str)] = &[
    (
        "for_each_list",
        "MUT total AS Integer = 0
  LET ys AS List OF Integer = [1, 2, 3]
  LET src AS List OF Integer = upTo({N})
  collections::forEach(src, LAMBDA(v AS Integer) -> total = total + len(ys))
  io::print(toString(total))",
        "3000",
    ),
    (
        "filter_map",
        "LET m AS Map OF Integer TO String = Map OF Integer TO String { 2 := \"b\", 4 := \"d\" }
  LET src AS List OF Integer = upTo({N})
  LET kept AS List OF Integer = collections::filter(src, LAMBDA(v AS Integer) -> collections::hasKey(m, v MOD 5))
  io::print(toString(len(kept)))",
        "400",
    ),
    (
        "transform_string",
        "LET tag AS String = \"ab\" & toString(7)
  LET src AS List OF Integer = upTo({N})
  LET out AS List OF Integer = collections::transform(src, LAMBDA(v AS Integer) -> v + len(tag))
  io::print(toString(collections::get(out, 0)))",
        "4",
    ),
];

/// Build and run one program: `(stdout, arena.0 alloc_calls, free_calls, live_bytes)`.
fn run(name: &str, source: &str) -> Result<(String, u64, u64, u64), String> {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        return Err(format!(
            "build failed:\n{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
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
    let err = String::from_utf8_lossy(&run.stderr);
    if !run.status.success() {
        return Err(format!("program failed:\n{err}"));
    }
    let counter = |key: &str| -> u64 {
        err.lines()
            .find_map(|l| l.strip_prefix(&format!("arena.0.{key} ")))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(u64::MAX)
    };
    Ok((
        String::from_utf8_lossy(&run.stdout)
            .trim_end_matches('\n')
            .to_string(),
        counter("alloc_calls"),
        counter("free_calls"),
        counter("live_bytes"),
    ))
}

#[test]
fn a_captured_collection_is_read_from_the_env_not_copied_per_call() {
    let mut failures = Vec::new();
    for (name, body, want) in CASES {
        let program = |n: u64| {
            format!(
                "IMPORT collections\nIMPORT io\n\n{UP_TO}\nFUNC main() AS Integer\n  {}\n  RETURN 0\nEND FUNC\n",
                body.replace("{N}", &n.to_string())
            )
        };
        let (at_n, at_2n) = match (
            run(&format!("capture_copy_{name}_n"), &program(1000)),
            run(&format!("capture_copy_{name}_2n"), &program(2000)),
        ) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let (printed, once, freed, live) = at_n;
        let twice = at_2n.1;
        if printed != *want || once != freed || live != 0 || twice.saturating_sub(once) >= 100 {
            failures.push(format!(
                "{name}: printed {printed:?} (want {want:?}); {once} blocks at N=1000, {twice} \
                 at 2N; {freed} freed, {live} B live"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
