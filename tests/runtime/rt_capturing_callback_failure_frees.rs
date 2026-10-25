//! A capturing `LAMBDA` passed to a callback position is freed when the call
//! fails, not only when it returns.
//!
//! bug-572 freed the closure object and its environment once the call it was
//! built for had returned. That drain sat on the success path only, so a callback
//! that failed left the closure behind: 32 B per failing call for a one-`Integer`
//! capture, found while probing bug-766 E. Since bug-781 the closure is its
//! statement's temp like any owned value, and the error exits
//! (`emit_call_error_exit`, `emit_inline_error_exit`) free a statement's temps.
//!
//! Each case runs the failing statement `REPS` times, trapped by a function-level
//! `TRAP` one frame up and, separately, by an inline `TRAP` in the failing frame.
//! A `--debug` run must print `-REPS` and end with `alloc_calls = free_calls`
//! and `live_bytes 0`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const REPS: u32 = 50;

const HELPERS: &str = "\
IMPORT collections
IMPORT io

TYPE Item
  id AS Integer
  name AS String
END TYPE

FUNC items() AS List OF Item
  MUT xs AS List OF Item = []
  FOR i = 0 TO 19
    xs = collections::append(xs, Item[id := i, name := \"item\" & toString(i)])
  NEXT
  RETURN xs
END FUNC

FUNC ints() AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO 19
    xs = collections::append(xs, i)
  NEXT
  RETURN xs
END FUNC

FUNC failAt(n AS Integer, k AS Integer) AS Integer
  IF n = k THEN
    FAIL error(77050002, \"boom\")
  END IF
  RETURN n
END FUNC

FUNC applyTo(f AS FUNC(Integer) AS Integer, n AS Integer) AS Integer
  RETURN f(n)
END FUNC
";

/// `(name, statement)`: the failing statement, a `LET` initializer. `xs` is the
/// `Item` list, `ixs` the `Integer` list, `k` a captured `Integer` (10) and
/// `big` a captured `Integer` whose product with an element id overflows.
const CASES: &[(&str, &str)] = &[
    (
        "transform_call",
        "LET r AS List OF Integer = collections::transform(ixs, LAMBDA(n AS Integer) -> failAt(n, k))",
    ),
    (
        "transform_record_update",
        "LET r AS List OF Item = collections::transform(xs, LAMBDA(it AS Item) -> WITH it { id := it.id * big })",
    ),
    (
        "filter",
        "LET r AS List OF Integer = collections::filter(ixs, LAMBDA(n AS Integer) -> failAt(n, k) > 3)",
    ),
    (
        "reduce",
        "LET r AS Integer = collections::reduce(ixs, 0, LAMBDA(acc AS Integer, n AS Integer) -> acc + failAt(n, k))",
    ),
    (
        "user_function",
        "LET r AS Integer = applyTo(LAMBDA(n AS Integer) -> failAt(n, k), 10)",
    ),
];

/// The failing statement, trapped by an inline `TRAP` when `inline`.
fn trapped(statement: &str, inline: bool) -> String {
    if inline {
        format!("{statement} TRAP(e)\n    RETURN -1\n  END TRAP")
    } else {
        statement.to_string()
    }
}

fn program(statement: &str, inline: bool) -> String {
    format!(
        "{HELPERS}
FUNC doOp(xs AS List OF Item, ixs AS List OF Integer) AS Integer
  LET k AS Integer = len(xs) - 10
  LET big AS Integer = 4611686018427387904 + len(ixs) - 20
  LET extra AS List OF Item = items()
  {}
  RETURN len(extra)
END FUNC

FUNC tryOnce(xs AS List OF Item, ixs AS List OF Integer) AS Integer
  RETURN doOp(xs, ixs)
  TRAP(err)
    RETURN -1
  END TRAP
END FUNC

FUNC main() AS Integer
  LET xs AS List OF Item = items()
  LET ixs AS List OF Integer = ints()
  MUT acc AS Integer = 0
  FOR rep = 1 TO {REPS}
    acc = acc + tryOnce(xs, ixs)
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
",
        trapped(statement, inline)
    )
}

/// Build with `--debug`, run, and return `(stdout, alloc_calls, free_calls,
/// live_bytes)` summed over the arenas.
fn run(name: &str, source: &str) -> (String, u64, u64, u64) {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name}: build failed:\n{stdout}\n{}\n--- source ---\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|p| p.ends_with(common::host_executable_suffix()))
        .or_else(|| {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix("Wrote executable to "))
        })
        .expect("an executable")
        .to_string();
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let report = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "{name}: program failed:\n{text}\n{report}"
    );
    let sum = |key: &str| -> u64 {
        report
            .lines()
            .filter_map(|line| line.strip_prefix("arena."))
            .filter_map(|rest| rest.split_once(key))
            .filter(|(arena, _)| !arena.contains('.'))
            .map(|(_, n)| n.trim().parse::<u64>().expect("a count"))
            .sum()
    };
    (
        text,
        sum(".alloc_calls "),
        sum(".free_calls "),
        sum(".live_bytes "),
    )
}

#[test]
fn a_failing_capturing_callback_frees_its_closure() {
    let mut failures = Vec::new();
    for (member, statement) in CASES {
        for inline in [false, true] {
            let mode = if inline { "inline" } else { "function" };
            let name = format!("capcb_{member}_{mode}");
            let (text, alloc, free, live) = run(&name, &program(statement, inline));
            let want = format!("-{REPS}\n");
            if text != want {
                failures.push(format!(
                    "{member} ({mode} TRAP): printed {text:?}, want {want:?}"
                ));
                continue;
            }
            if alloc != free || live != 0 {
                failures.push(format!(
                    "{member} ({mode} TRAP): leaked {} block(s), {live} B live over {REPS} \
                     failing calls",
                    alloc.saturating_sub(free)
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "a failing callback must free the capturing LAMBDA passed to it:\n  {}",
        failures.join("\n  ")
    );
}
