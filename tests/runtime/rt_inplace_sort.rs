//! plan-142-C: in-place `sort`/`sortBy` produce exactly the copying result.
//!
//! Each case runs the self-update on `x` (the in-place arm) and prints it beside
//! the same call made as a plain expression on a fresh value (the copying
//! lowering). Covered: `Integer`, `Float`, `Byte`, `String` elements; `sortBy`
//! with `Integer`, `Float` and `String` keys, a key function returning its own
//! argument, a record element with equal keys (stability), a list whose payloads
//! are out of entry order, a single element, and follow-up `append`/`filter`/
//! `drop` on the permuted (now out-of-order) payloads. The program also reports
//! its arena counters: every block it allocates must be freed.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const PROGRAM: &str = "IMPORT collections
IMPORT io

TYPE Pt
  x AS Integer
  name AS String
END TYPE

FUNC keyX(p AS Pt) AS Integer
  RETURN p.x
END FUNC

FUNC keyName(p AS Pt) AS String
  RETURN p.name
END FUNC

FUNC keyLen(s AS String) AS Integer
  RETURN len(s)
END FUNC

FUNC keySelf(s AS String) AS String
  RETURN s
END FUNC

FUNC negFloat(f AS Float) AS Float
  RETURN 0.0 - f
END FUNC

FUNC longish(s AS String) AS Boolean
  RETURN len(s) > 2
END FUNC

FUNC showInts(xs AS List OF Integer) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showFloats(xs AS List OF Float) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showBytes(xs AS List OF Byte) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showStrs(xs AS List OF String) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & v & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showPts(xs AS List OF Pt) AS String
  MUT out AS String = \"\"
  FOR EACH p IN xs
    out = out & toString(p.x) & \":\" & p.name & \",\"
  NEXT
  RETURN out
END FUNC

SUB check(label AS String, got AS String, want AS String)
  IF got = want THEN
    io::print(\"ok   \" & label & \" \" & got)
  ELSE
    io::print(\"FAIL \" & label & \" got=\" & got & \" want=\" & want)
  END IF
END SUB

FUNC ints() AS List OF Integer
  RETURN [5, -1, 3, 3, -7, 8, 5, 0, 2, 9, -1]
END FUNC

FUNC floats() AS List OF Float
  RETURN [2.5, -1.0, 0.0, 7.25, -3.5, 2.5, 1.0]
END FUNC

FUNC bytes() AS List OF Byte
  RETURN [toByte(200), toByte(3), toByte(255), toByte(0), toByte(3)]
END FUNC

FUNC strs() AS List OF String
  RETURN [\"pear\", \"fig\", \"kiwi\", \"date\", \"\", \"fig\", \"apple\"]
END FUNC

FUNC messy() AS List OF String
  MUT s AS List OF String = [\"mid1\", \"zz\", \"mid2\"]
  s = collections::prepend(s, \"front\")
  s = collections::insert(s, 2, \"ins\")
  s = collections::set(s, 1, \"a-much-longer-replacement\")
  s = collections::append(s, \"aa\")
  RETURN s
END FUNC

FUNC pts() AS List OF Pt
  MUT p AS List OF Pt = []
  p = collections::append(p, Pt[x := 3, name := \"c\"])
  p = collections::append(p, Pt[x := 1, name := \"a\"])
  p = collections::append(p, Pt[x := 3, name := \"b\"])
  p = collections::append(p, Pt[x := 2, name := \"d\"])
  p = collections::append(p, Pt[x := 1, name := \"e\"])
  RETURN p
END FUNC

FUNC main() AS Integer
  MUT a AS List OF Integer = ints()
  a = collections::sort(a)
  check(\"sort int\", showInts(a), showInts(collections::sort(ints())))
  MUT f AS List OF Float = floats()
  f = collections::sort(f)
  check(\"sort float\", showFloats(f), showFloats(collections::sort(floats())))
  f = floats()
  f = collections::sortBy(f, negFloat)
  check(\"sortBy float key\", showFloats(f), showFloats(collections::sortBy(floats(), negFloat)))
  MUT b AS List OF Byte = bytes()
  b = collections::sort(b)
  check(\"sort byte\", showBytes(b), showBytes(collections::sort(bytes())))
  MUT s AS List OF String = strs()
  s = collections::sort(s)
  check(\"sort str\", showStrs(s), showStrs(collections::sort(strs())))
  s = strs()
  s = collections::sortBy(s, keyLen)
  check(\"sortBy str len (stable)\", showStrs(s), showStrs(collections::sortBy(strs(), keyLen)))
  s = strs()
  s = collections::sortBy(s, keySelf)
  check(\"sortBy str self\", showStrs(s), showStrs(collections::sortBy(strs(), keySelf)))
  s = messy()
  s = collections::sort(s)
  check(\"sort messy\", showStrs(s), showStrs(collections::sort(messy())))
  s = collections::append(s, \"tail\")
  s = collections::filter(s, longish)
  check(\"sort messy+append+filter\", showStrs(s), showStrs(collections::filter(collections::append(collections::sort(messy()), \"tail\"), longish)))
  MUT p AS List OF Pt = pts()
  p = collections::sortBy(p, keyX)
  check(\"sortBy pt x (stable)\", showPts(p), showPts(collections::sortBy(pts(), keyX)))
  p = pts()
  p = collections::sortBy(p, keyName)
  check(\"sortBy pt name\", showPts(p), showPts(collections::sortBy(pts(), keyName)))
  p = collections::append(p, Pt[x := 0, name := \"z\"])
  p = collections::drop(p, 1)
  check(\"sortBy pt name+append+drop\", showPts(p), showPts(collections::drop(collections::append(collections::sortBy(pts(), keyName), Pt[x := 0, name := \"z\"]), 1)))
  MUT one AS List OF String = [\"solo\"]
  one = collections::sortBy(one, keySelf)
  check(\"sortBy single\", showStrs(one), \"solo,\")
  MUT cycle AS List OF String = []
  FOR i = 1 TO 200
    cycle = collections::append(cycle, \"k\" & toString((i * 37) MOD 101))
    cycle = collections::sort(cycle)
    cycle = collections::sortBy(cycle, keyLen)
  NEXT
  check(\"cycle\", toString(len(cycle)), \"200\")
  RETURN 0
END FUNC
";

#[test]
fn in_place_sort_matches_the_copying_sort() {
    let project = common::temp_project("inplace_sort", PROGRAM);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "build failed:\n{stdout}\n{}",
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
    let out = String::from_utf8_lossy(&run.stdout);
    let err = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "program failed:\n{out}\n{err}");
    let failed: Vec<&str> = out.lines().filter(|l| l.starts_with("FAIL")).collect();
    assert!(
        failed.is_empty(),
        "in-place and copying sorts disagree:\n{}",
        failed.join("\n")
    );
    assert_eq!(
        out.lines().filter(|l| l.starts_with("ok")).count(),
        14,
        "{out}"
    );
    let counter = |name: &str| -> u64 {
        err.lines()
            .find_map(|l| l.strip_prefix(&format!("arena.0.{name} ")))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or_else(|| panic!("no arena.0.{name}:\n{err}"))
    };
    assert_eq!(
        counter("alloc_calls"),
        counter("free_calls"),
        "leaked a block:\n{err}"
    );
    assert_eq!(counter("live_bytes"), 0, "bytes still live at exit:\n{err}");
}
