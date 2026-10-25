//! A `collections::set` or `collections::insert` that fails its index range
//! check frees everything it allocated before the check.
//!
//! bug-782: `set` on a fixed-width element copies the list (bug-365's tight copy)
//! and only then range-checks the index, and `insert` wraps its item in a
//! singleton list before the check in `lower_list_insert_collection`. An
//! out-of-range index left through `ErrIndexOutOfRange` with that block still
//! allocated: 64 B per failing `set(p.trail, 9, 1.0)`. The check now runs first,
//! as bug-147.5 already arranged for `set`'s `removeAt` + `insert` path.
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

TYPE P
  age AS Integer
  trail AS List OF Float
  tags AS List OF String
END TYPE

FUNC people() AS List OF P
  RETURN [P[age := 1, trail := [0.0, 0.0], tags := [\"a\", \"b\"]]]
END FUNC

FUNC setOnField(p AS P, slot AS Integer) AS Integer
  LET r AS List OF Float = collections::set(p.trail, slot, 1.0)
  RETURN len(r)
END FUNC
";

/// `(name, statement)`: the failing statement, a `LET` initializer. `slot` is 9
/// (out of range for every list here), `p` a record copied out of `ps`, `nums` a
/// local `Float` list and `words` a local `String` list.
const CASES: &[(&str, &str)] = &[
    (
        "set_float_field_copy",
        "LET r AS List OF Float = collections::set(p.trail, slot, 1.0)",
    ),
    (
        "set_float_local",
        "LET r AS List OF Float = collections::set(nums, slot, 1.0)",
    ),
    (
        "set_float_param_field",
        "LET r AS Integer = setOnField(p, slot)",
    ),
    (
        "set_string_field_copy",
        "LET r AS List OF String = collections::set(p.tags, slot, \"z\")",
    ),
    (
        "insert_float_field_copy",
        "LET r AS List OF Float = collections::insert(p.trail, slot, 1.0)",
    ),
    (
        "insert_float_local",
        "LET r AS List OF Float = collections::insert(nums, slot, 1.0)",
    ),
    (
        "insert_string_local",
        "LET r AS List OF String = collections::insert(words, slot, \"z\")",
    ),
    (
        "insert_negative_index",
        "LET r AS List OF Float = collections::insert(nums, 0 - slot, 1.0)",
    ),
    (
        "set_string_built_item",
        "LET r AS List OF String = collections::set(p.tags, slot, \"z\" & toString(slot))",
    ),
    (
        "insert_string_built_item",
        "LET r AS List OF String = collections::insert(words, slot, \"z\" & toString(slot))",
    ),
    (
        "set_float_built_list",
        "LET r AS List OF Float = collections::set(collections::append(nums, 2.0), slot, 1.0)",
    ),
    (
        "insert_float_built_list",
        "LET r AS List OF Float = collections::insert(collections::append(nums, 2.0), slot, 1.0)",
    ),
    (
        "remove_at_field_copy",
        "LET r AS List OF Float = collections::removeAt(p.trail, slot)",
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
FUNC doOp(ps AS List OF P, slot AS Integer) AS Integer
  LET p AS P = collections::get(ps, 0)
  LET nums AS List OF Float = [0.0, 0.0]
  LET words AS List OF String = [\"a\", \"b\"]
  {}
  RETURN p.age + len(nums) + len(words)
END FUNC

FUNC tryOnce(ps AS List OF P, slot AS Integer) AS Integer
  RETURN doOp(ps, slot)
  TRAP(err)
    RETURN -1
  END TRAP
END FUNC

FUNC main() AS Integer
  LET ps AS List OF P = people()
  MUT acc AS Integer = 0
  FOR rep = 1 TO {REPS}
    acc = acc + tryOnce(ps, 9)
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
fn a_failing_set_or_insert_frees_what_it_allocated() {
    let mut failures = Vec::new();
    for (member, statement) in CASES {
        for inline in [false, true] {
            let mode = if inline { "inline" } else { "function" };
            let name = format!("listupd_{member}_{mode}");
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
        "an out-of-range set/insert must free what it allocated before the check:\n  {}",
        failures.join("\n  ")
    );
}
