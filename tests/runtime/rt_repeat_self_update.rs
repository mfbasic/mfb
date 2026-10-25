//! plan-153-B: `x = collections::repeat(x, …)` over a variable-width element
//! updates `x` in place.
//!
//! `inplace_self_update/cases.tsv` holds one line per signature, and `repeat`'s is a
//! `List OF Integer` (a kind-2, fixed-width list). A `List OF String` takes the other
//! half of the arm — the entry table, the data-tail reservation, the `set`
//! primitive's same-size overwrite and the `append` primitive's tail write — so it
//! is measured here the same way: build the statements run `N` and `2N` times with
//! `mfb build --debug`, and require the extra `N` runs to allocate fewer than `N / 8`
//! more blocks (`arena.0.alloc_calls`). A copy of `x` per statement allocates at
//! least `N` more.
//!
//! The loop overwrites three same-size elements, extends the list by two, and cuts
//! it back with `take`, so every iteration writes the tail without the list
//! growing: after the first reservation the arm never allocates.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const N: u64 = 2000;

fn program(n: u64) -> String {
    format!(
        "IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT x AS List OF String = [\"ab\", \"cd\", \"ef\"]
  LET before AS List OF String = x
  FOR k = 1 TO {n}
    x = collections::repeat(x, \"gh\", 3)
    x = collections::repeat(x, \"ijk\", 2, 3)
    x = collections::take(x, 3)
  NEXT
  io::print(toString(len(x)) & \" \" & collections::get(x, 0) & \" \" & collections::get(before, 0))
  RETURN 0
END FUNC
"
    )
}

/// Build `source` with `--debug`, run it, require it to print `want`, and return
/// the main arena's `alloc_calls`.
fn alloc_calls(name: &str, source: &str, want: &str) -> u64 {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} failed to build:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|path| path.ends_with(common::host_executable_suffix()))
        .or_else(|| {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix("Wrote executable to "))
        })
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"))
        .to_string();
    let run = Command::new(&exe).output().expect("run the program");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(run.status.success(), "{name} failed:\n{text}");
    assert!(
        text.lines().any(|line| line == want),
        "{name}: want the line `{want}`, got:\n{text}"
    );
    text.lines()
        .find_map(|line| line.strip_prefix("arena.0.alloc_calls "))
        .unwrap_or_else(|| panic!("{name}: no arena.0.alloc_calls in the debug report:\n{text}"))
        .trim()
        .parse()
        .expect("alloc_calls is a number")
}

#[test]
fn a_string_list_repeat_self_update_does_not_copy() {
    let once = alloc_calls("repeat_su_str_n", &program(N), "3 gh ab");
    let twice = alloc_calls("repeat_su_str_2n", &program(2 * N), "3 gh ab");
    let extra = twice.saturating_sub(once);
    assert!(
        extra < N / 8,
        "{N} more runs allocated {extra} more blocks ({once} at N={N}, {twice} at 2N), \
         which is not below the bound of {} — the statement copies",
        N / 8
    );
}
