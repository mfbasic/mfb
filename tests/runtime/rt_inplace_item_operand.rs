//! bug-681: a self-update's *item operand* keeps the in-place path when it is a
//! composite expression over a user-declared `FUNC` call.
//!
//! Every `G11` gate in `src/codegen/collection/assign/builder_inplace_assign.rs`
//! asks `static_item_type` for the operand's static type and declines when the
//! answer is `None` — the decline is semantically correct, but the fallback
//! rebuilds the whole collection per statement, so the loop goes quadratic.
//! `static_item_type` resolves a user `FUNC`'s declared `returns` only at its own
//! top level; its composite arms (`Binary`, `Unary`, `MemberAccess`,
//! `ResultValue`) recursed into `static_type_name`, whose call table names only
//! builtins. So `append(xs, f(i))` was in place and `append(xs, f(i) * 2)` was
//! not, one `* 2` apart, with no diagnostic (bug-681: 16 000 appends took 2798 ms
//! against 1 ms, and a 259 920-element decode was killed by the OOM killer).
//!
//! The measure is `rt_inplace_self_update.rs`'s: build the loop at `N` and `2N`
//! with `mfb build --debug` and read the arenas' `alloc_calls`. A copying
//! lowering allocates at least one block per statement, so the extra `N`
//! iterations cost at least `N` more; an in-place one allocates only on
//! geometric growth, so they cost fewer than `N / 8`.
//!
//! The table covers **every** arm that gates on `static_item_type`, not just
//! `append` — the single-element and bulk `collections::append`,
//! `collections::add` and `collections::remove` on a `Set`, and
//! `collections::removeKey` on a `Map` — each with a composite item operand, and
//! alongside them the contrast shapes that were already flat (a bare call, plain
//! arithmetic, arithmetic over a *builtin* call). A contrast row that regresses
//! means the fix traded one shape for another.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::Command;

const N: u64 = 2000;

/// Declarations every program carries: the user `FUNC`s whose return type the
/// gate has to see through a composite operand, and a record for the
/// `MemberAccess` shape.
const PRELUDE: &str = "\
IMPORT collections
IMPORT io

TYPE Box
  n AS Integer
  items AS List OF Integer
END TYPE

FUNC twice(k AS Integer) AS Integer
  RETURN k * 2
END FUNC

FUNC mkBox(k AS Integer) AS Box
  RETURN Box[n := k * 3, items := [k, k]]
END FUNC
";

/// One row: `x`'s declaration, the self-update run in the loop, and the shape of
/// the item operand the row exists to pin.
struct Case {
    /// The gated arm, for the failure message.
    arm: &'static str,
    /// The item operand's shape.
    shape: &'static str,
    /// `<type> = <initializer>` for `x`.
    decl: &'static str,
    /// The self-update statement, run `N` times over loop variable `i`.
    statement: &'static str,
}

const CASES: &[Case] = &[
    // --- the bug: a composite operand over a user `FUNC` call -----------------
    Case {
        arm: "collections::append(list, item)",
        shape: "Binary over a user FUNC call",
        decl: "List OF Integer = [1, 2, 3]",
        statement: "x = collections::append(x, twice(i) * 2)",
    },
    Case {
        arm: "collections::append(list, item)",
        shape: "Unary over a user FUNC call",
        decl: "List OF Integer = [1, 2, 3]",
        statement: "x = collections::append(x, -twice(i))",
    },
    Case {
        arm: "collections::append(list, item)",
        shape: "MemberAccess on a user FUNC call",
        decl: "List OF Integer = [1, 2, 3]",
        statement: "x = collections::append(x, mkBox(i).n)",
    },
    Case {
        arm: "collections::append(list, sublist)",
        shape: "MemberAccess on a user FUNC call",
        decl: "List OF Integer = [1, 2, 3]",
        statement: "x = collections::append(x, mkBox(i).items)",
    },
    Case {
        arm: "collections::add(set, item)",
        shape: "Binary over a user FUNC call",
        decl: "Set OF Integer = Set OF Integer { 1, 2 }",
        statement: "x = collections::add(x, twice(i) * 2)",
    },
    Case {
        arm: "collections::remove(set, item)",
        shape: "Binary over a user FUNC call",
        decl: "Set OF Integer = Set OF Integer { 1, 2 }",
        statement: "x = collections::remove(x, twice(i) * 2)",
    },
    Case {
        arm: "collections::removeKey(map, key)",
        shape: "Binary over a user FUNC call",
        decl: "Map OF Integer TO Integer = Map OF Integer TO Integer { 1 := 1, 2 := 2 }",
        statement: "x = collections::removeKey(x, twice(i) * 2)",
    },
    // --- the contrasts: already flat, and must stay flat -----------------------
    Case {
        arm: "collections::append(list, item)",
        shape: "a user FUNC call at the top level",
        decl: "List OF Integer = [1, 2, 3]",
        statement: "x = collections::append(x, twice(i))",
    },
    Case {
        arm: "collections::append(list, item)",
        shape: "arithmetic with no call in it",
        decl: "List OF Integer = [1, 2, 3]",
        statement: "x = collections::append(x, 3 + i * 2)",
    },
    Case {
        arm: "collections::append(list, item)",
        shape: "Binary over a builtin call",
        decl: "List OF Float = [1.0]",
        statement: "x = collections::append(x, toFloat(i) + 1.0)",
    },
];

/// The whole program: `x` as a `main` local, the statement run `n` times, and the
/// final length printed.
fn program(case: &Case, n: u64) -> String {
    format!(
        "{PRELUDE}\nFUNC main() AS Integer\n  MUT x AS {}\n  FOR i = 1 TO {n}\n    {}\n  NEXT\n  \
         io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n",
        case.decl, case.statement
    )
}

/// Build `source` with `--debug` and return the executable (the host glibc one on
/// Linux).
fn build_debug(name: &str, source: &str) -> Result<(PathBuf, PathBuf), String> {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .map_err(|e| format!("run mfb build: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        return Err(format!(
            "build failed:\n{stdout}\n{}\n--- source ---\n{source}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let exe = written
        .iter()
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| written.first())
        .map(PathBuf::from)
        .ok_or_else(|| format!("no executable in build output:\n{stdout}"))?;
    Ok((exe, project))
}

/// Run the program: `(the printed length, the arenas' summed alloc_calls)`.
fn run(name: &str, source: &str) -> Result<(String, u64), String> {
    let (exe, project) = build_debug(name, source)?;
    let output = Command::new(&exe).output();
    let _ = std::fs::remove_dir_all(&project);
    let output = output.map_err(|e| format!("run: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!(
            "program failed:\n{stdout}\n{stderr}\n--- source ---\n{source}"
        ));
    }
    let mut count = 0u64;
    let mut seen = false;
    for line in stderr.lines() {
        let Some(rest) = line.strip_prefix("arena.") else {
            continue;
        };
        let Some((_, n)) = rest.split_once(".alloc_calls ") else {
            continue;
        };
        count += n
            .trim()
            .parse::<u64>()
            .map_err(|e| format!("bad alloc_calls line `{line}`: {e}"))?;
        seen = true;
    }
    if !seen {
        return Err(format!("no arena.*.alloc_calls in:\n{stderr}"));
    }
    Ok((stdout.lines().next().unwrap_or_default().to_string(), count))
}

fn check(index: usize, case: &Case) -> Result<(), String> {
    let label = format!("{} with {}", case.arm, case.shape);
    let tag = format!("io{index}");
    let (len_n, once) =
        run(&format!("{tag}_n"), &program(case, N)).map_err(|e| format!("{label} (N={N}): {e}"))?;
    let (len_2n, twice) = run(&format!("{tag}_2n"), &program(case, 2 * N))
        .map_err(|e| format!("{label} (N={}): {e}", 2 * N))?;
    let growth = twice.saturating_sub(once);
    let bound = N / 8;
    if growth >= bound {
        return Err(format!(
            "{label}: `{}` allocated {growth} more blocks for the extra {N} iterations \
             ({once} -> {twice}), not under {bound} — the statement rebuilt `x` instead of \
             writing into it, so the loop is O(n^2). Final lengths: {len_n} -> {len_2n}",
            case.statement
        ));
    }
    Ok(())
}

#[test]
fn composite_item_operand_over_a_user_func_stays_in_place() {
    let mut failures = Vec::new();
    for (index, case) in CASES.iter().enumerate() {
        if let Err(message) = check(index, case) {
            failures.push(message);
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} rows rebuilt instead of mutating in place:\n\n{}",
        failures.len(),
        CASES.len(),
        failures.join("\n\n")
    );
}
