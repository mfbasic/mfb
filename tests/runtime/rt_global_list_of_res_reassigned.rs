//! bug-1081: a module-level `List OF RES` that is reassigned frees the block it
//! replaces.
//!
//! `MUT gF AS List OF RES fs::File = []` at module level, given a fresh list on every
//! iteration, kept every list block it had held before: 240 bytes and one block an
//! assignment, so the live bytes at exit grew with the iteration count. The same
//! held for a module-level `List OF RES Handle` assigned a `List OF RES fs::File`
//! (`gH = files`), where each assignment builds a converted block. The resources are
//! the caller's and stay open; only the replaced blocks were kept.
//!
//! A module-level binding holds its current value until exit (bug-1068 owns freeing
//! that value), so the live bytes at exit are not zero here. What this pins is the
//! bug's correct behavior: they do not grow with `N`, nothing is freed twice, and the
//! files are still open (written) after the loop.

#[path = "../common/mod.rs"]
mod common;

/// The bug's reproduction: the global is given a copy built by a callee.
const COPY: &str = "\
IMPORT collections
IMPORT fs
IMPORT io
IMPORT os

MUT gF AS List OF RES fs::File = []

FUNC copyF(files AS List OF RES fs::File) AS List OF RES fs::File
  MUT out AS List OF RES fs::File = []
  FOR EACH f IN files
    out = collections::append(out, f)
  NEXT
  RETURN out
END FUNC

FUNC main AS Integer
  MUT n AS Integer = 0
  MUT files AS List OF RES fs::File = []
  RES f1 AS fs::File = fs::createTempFile()
  files = collections::append(files, f1)
  RES f2 AS fs::File = fs::createTempFile()
  files = collections::append(files, f2)
  FOR i = 1 TO toInt(os::getEnvOr(\"N\", \"10\"))
    gF = copyF(files)
    n = n + len(gF)
  NEXT
  fs::writeAll(f1, \"still open\")
  fs::writeAll(collections::get(files, 1), \"still open\")
  io::print(\"n=\" & toString(n))
  RETURN 0
END FUNC
";

/// The bug's second form: a `List OF RES Handle` global assigned a `List OF RES
/// fs::File`, which builds a converted block on every assignment.
const CONVERTED: &str = "\
IMPORT collections
IMPORT fs
IMPORT io
IMPORT os
IMPORT tcp

UNION Handle
  fs::File
  tcp::Socket
END UNION

MUT gH AS List OF RES Handle = []

FUNC main AS Integer
  MUT n AS Integer = 0
  MUT files AS List OF RES fs::File = []
  RES f1 AS fs::File = fs::createTempFile()
  files = collections::append(files, f1)
  RES f2 AS fs::File = fs::createTempFile()
  files = collections::append(files, f2)
  FOR i = 1 TO toInt(os::getEnvOr(\"N\", \"10\"))
    gH = files
    n = n + len(gH)
  NEXT
  fs::writeAll(f1, \"still open\")
  fs::writeAll(collections::get(files, 1), \"still open\")
  io::print(\"n=\" & toString(n))
  RETURN 0
END FUNC
";

/// `(live_bytes, double_free_skips)` of one run with `N=n`, after checking the total.
fn measure(case: &str, exe: &std::path::Path, n: u64) -> (u64, u64) {
    let output = common::remote::output(exe, &[], &[("N", &n.to_string())]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{case} N={n} failed:\n{stdout}\n{stderr}"
    );
    assert_eq!(
        stdout.trim(),
        format!("n={}", 2 * n),
        "{case} N={n}: wrong total"
    );
    let lines = common::debug_report::arena_lines(case, &stderr);
    let read = |name: &str| common::debug_report::counter(case, &lines, 0, name);
    (read("live_bytes"), read("double_free_skips"))
}

/// Build `source` at `level` (the default when `None`) and require the same live
/// bytes at exit after 10 and after 40 assignments.
fn check(case: &str, source: &str, level: Option<&str>) {
    let name = format!("global_res_reassigned_{case}");
    let project = common::temp_project(&name, source);
    let extra: Vec<&str> = level.into_iter().collect();
    let exe = common::debug_report::build_debug_project_with(&name, &project, &extra);
    let (live10, skips10) = measure(case, &exe, 10);
    let (live40, skips40) = measure(case, &exe, 40);
    assert_eq!(
        (skips10, skips40),
        (0, 0),
        "{case}: a block was freed twice"
    );
    assert_eq!(
        live10, live40,
        "{case}: live bytes at exit grew from {live10} (N=10) to {live40} (N=40): \
         a replaced list block is kept"
    );
}

#[test]
fn a_global_list_of_resources_frees_the_block_it_replaces() {
    check("copy", COPY, None);
    check("copy_o0", COPY, Some("-O0"));
}

#[test]
fn a_global_list_converted_on_assignment_frees_the_block_it_replaces() {
    check("converted", CONVERTED, None);
    check("converted_o0", CONVERTED, Some("-O0"));
}
