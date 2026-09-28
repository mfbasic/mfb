//! bug-760 A: `groupBy` / `sortBy` over a forwarded record or `STATE` field keep
//! their inline native lowering.
//!
//! plan-149's place forwarding (`optimizer/opt1/placefwd.rs:forward`) rewrites
//! `LET base = rec.xs` + `groupBy(base, …)` into `groupBy(rec.xs, …)`. The native
//! fast paths (`func_group_by.rs:group_by_fast_path`,
//! `func_sort_by.rs:sort_by_fast_path`) accepted only a `Local`-shaped source, so
//! the forwarded `MemberAccess` fell back to the out-of-line `.mfb` body — an
//! O(n·bucket) `groupBy` (675× on the benchmark's `list (Record-Dynamic) groupby`)
//! and a list-copying `sortBy`. The output stayed right, so only the emitted code
//! shows it: at `-O1` the probe function must not call the `.mfb` body.
//!
//! The same programs run at `-O0` and `-O1` and must print the same thing, so a
//! fast path that reads the wrong place fails here too.

#[path = "../common/mod.rs"]
mod common;

use serde_json::Value;
use std::path::Path;
use std::process::Command;

const TARGET: &str = "macos-aarch64";

const PRELUDE: &str = "\
IMPORT io\n\
IMPORT collections\n\
IMPORT fs\n\
\n\
TYPE RecStr\n\
  before AS Integer\n\
  xs AS List OF String\n\
  after AS Integer\n\
END TYPE\n\
\n\
TYPE RecInt\n\
  before AS Integer\n\
  xs AS List OF Integer\n\
  after AS Integer\n\
END TYPE\n\
\n\
FUNC strLen(s AS String) AS Integer\n\
  RETURN len(s)\n\
END FUNC\n\
\n\
FUNC strId(s AS String) AS String\n\
  RETURN s\n\
END FUNC\n\
\n\
FUNC bucket(x AS Integer) AS Integer\n\
  RETURN x MOD 7\n\
END FUNC\n\
\n\
FUNC intId(x AS Integer) AS Integer\n\
  RETURN x\n\
END FUNC\n\
\n\
FUNC strs() AS List OF String\n\
  MUT xs AS List OF String = []\n\
  FOR i = 0 TO 40\n\
    xs = collections::append(xs, \"s\" & toString(i * 37 MOD 41))\n\
  NEXT\n\
  RETURN xs\n\
END FUNC\n\
\n\
FUNC ints() AS List OF Integer\n\
  MUT xs AS List OF Integer = []\n\
  FOR i = 0 TO 40\n\
    xs = collections::append(xs, i * 37 MOD 41)\n\
  NEXT\n\
  RETURN xs\n\
END FUNC\n\
\n\
FUNC joined(xs AS List OF String) AS String\n\
  MUT out AS String = \"\"\n\
  FOR i = 0 TO len(xs) - 1\n\
    out = out & collections::get(xs, i) & \",\"\n\
  NEXT\n\
  RETURN out\n\
END FUNC\n";

const PROBES: &str = "\
FUNC probeRecStrGroupBy(rec AS RecStr) AS Integer\n\
  LET base AS List OF String = rec.xs\n\
  LET g AS Map OF Integer TO List OF String = collections::groupBy(base, strLen, strId)\n\
  RETURN len(g) * 1000 + len(collections::get(g, 2))\n\
END FUNC\n\
\n\
FUNC probeRecIntGroupBy(rec AS RecInt) AS Integer\n\
  LET base AS List OF Integer = rec.xs\n\
  LET g AS Map OF Integer TO List OF Integer = collections::groupBy(base, bucket, intId)\n\
  RETURN len(g) * 1000 + collections::sum(collections::get(g, 3))\n\
END FUNC\n\
\n\
FUNC probeRecStrSortBy(rec AS RecStr) AS String\n\
  LET base AS List OF String = rec.xs\n\
  RETURN joined(collections::sortBy(base, strLen))\n\
END FUNC\n\
\n\
FUNC probeStateStrGroupBy(path AS String) AS Integer\n\
  RES f AS fs::File STATE RecStr = fs::open(path, \"write\")\n\
  fs::deleteFile(path)\n\
  f.state.xs = strs()\n\
  LET base AS List OF String = f.state.xs\n\
  LET g AS Map OF Integer TO List OF String = collections::groupBy(base, strLen, strId)\n\
  RETURN len(g) * 1000 + len(collections::get(g, 3))\n\
END FUNC\n\
\n\
FUNC probeStateStrSortBy(path AS String) AS String\n\
  RES f AS fs::File STATE RecStr = fs::open(path, \"write\")\n\
  fs::deleteFile(path)\n\
  f.state.xs = strs()\n\
  LET base AS List OF String = f.state.xs\n\
  RETURN joined(collections::sortBy(base, strLen))\n\
END FUNC\n\
\n\
FUNC main() AS Integer\n\
  LET rs AS RecStr = RecStr[1, strs(), 2]\n\
  LET ri AS RecInt = RecInt[1, ints(), 2]\n\
  io::print(toString(probeRecStrGroupBy(rs)))\n\
  io::print(toString(probeRecIntGroupBy(ri)))\n\
  io::print(probeRecStrSortBy(rs))\n\
  LET dir AS String = fs::tempDirectory()\n\
  io::print(toString(probeStateStrGroupBy(fs::pathJoin([dir, \"bug760a_g.tmp\"]))))\n\
  io::print(probeStateStrSortBy(fs::pathJoin([dir, \"bug760a_s.tmp\"])))\n\
  RETURN 0\n\
END FUNC\n";

/// Every `bl` target in `function`'s body.
fn call_targets(plan: &Value, function: &str) -> Vec<String> {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["name"].as_str() == Some(function))
        .unwrap_or_else(|| panic!("code plan has no function '{function}'"))["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|i| i["op"] == "bl")
        .filter_map(|i| i["target"].as_str().map(str::to_owned))
        .collect()
}

fn run(project: &Path, level: &str) -> String {
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-O")
        .arg(level)
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "-O{level} build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let exe = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names the executable");
    let run = Command::new(exe).output().expect("run program");
    assert!(
        run.status.success(),
        "-O{level} program failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8(run.stdout).expect("utf8 program output")
}

#[test]
fn a_forwarded_field_keeps_the_native_group_by_and_sort_by() {
    let name = "bug760a_fast_path";
    let project = common::temp_project(name, &format!("{PRELUDE}\n{PROBES}"));
    let plan = common::build_ncode_opt(&project, TARGET, name, "1");
    let cases = [
        ("probeRecStrGroupBy", "collections_5FgroupBy"),
        ("probeRecIntGroupBy", "collections_5FgroupBy"),
        ("probeRecStrSortBy", "collections_5FsortBy"),
        ("probeStateStrGroupBy", "collections_5FgroupBy"),
        ("probeStateStrSortBy", "collections_5FsortBy"),
    ];
    let mut slow = Vec::new();
    for (function, body) in cases {
        let calls = call_targets(&plan, function);
        if let Some(target) = calls.iter().find(|t| t.contains(body)) {
            slow.push(format!("{function} -> {target}"));
        }
    }
    assert!(
        slow.is_empty(),
        "bug-760 A: at -O1 a forwarded field source fell back to the out-of-line `.mfb` \
         body instead of the inline native lowering:\n  {}",
        slow.join("\n  ")
    );
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn the_fast_path_over_a_forwarded_field_changes_no_result() {
    let project = common::temp_project(
        "bug760a_fast_path_run",
        &format!("{PRELUDE}\n{PROBES}"),
    );
    let unoptimized = run(&project, "0");
    assert_eq!(unoptimized.lines().count(), 5, "five probe lines:\n{unoptimized}");
    assert_eq!(run(&project, "1"), unoptimized);
    let _ = std::fs::remove_dir_all(&project);
}
