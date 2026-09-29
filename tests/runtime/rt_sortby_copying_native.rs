//! bug-767: a copying `sortBy` / `sort` costs what the self-update spelling costs.
//!
//! `LET s = collections::sortBy(xs, keyFn)` had a native lowering only for 8-byte
//! items with `Integer`/`Fixed`/`Money` keys; every record element and every other
//! key type ran the generic `__collections_sortBy` body, which copies each element
//! it moves on every merge pass (10 042 allocations per sort of 1000 records).
//! `MUT s = xs` / `s = sortBy(s, keyFn)` — the same result — already runs the
//! plan-142-C index-permutation merge with one allocation per call. The copying
//! form owes exactly one copy more (`value` → the result), so the two programs
//! below must print the same thing and allocate within a small constant of each
//! other, at any list size.
//!
//! Sub-issue B: `mfb man collections sortBy` says `keyFn` "is called once for a
//! one-element list"; only the native Integer-key path did — the generic body and
//! the in-place arm skipped it.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const N: u32 = 400;
/// The copying spelling may allocate this many blocks more than the self-update
/// one: its one semantic copy of `value`, and slack for a separately-allocated
/// result header. Not proportional to N.
const SLACK: u64 = 4;

const TYPES: &str = "\
IMPORT io
IMPORT collections

TYPE Item
  id AS Integer
  name AS String
  score AS Float
END TYPE

TYPE Body
  x AS Float
  vx AS Float
  mass AS Float
END TYPE

TYPE Tree
  id AS Integer
  kids AS List OF Tree
END TYPE

FUNC items(n AS Integer) AS List OF Item
  MUT xs AS List OF Item = []
  FOR i = 0 TO n - 1
    LET k AS Integer = (i * 7919) MOD n
    xs = collections::append(xs, Item[id := k, name := \"item\" & toString(k MOD 97), score := toFloat(k MOD 50) * 0.5])
  NEXT
  RETURN xs
END FUNC

FUNC bodies(n AS Integer) AS List OF Body
  MUT xs AS List OF Body = []
  FOR i = 0 TO n - 1
    LET k AS Integer = (i * 7919) MOD n
    xs = collections::append(xs, Body[x := toFloat(k), vx := toFloat(k MOD 13) * 0.25, mass := 1.0])
  NEXT
  RETURN xs
END FUNC

FUNC floats(n AS Integer) AS List OF Float
  MUT xs AS List OF Float = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, toFloat((i * 7919) MOD n) * 0.5)
  NEXT
  RETURN xs
END FUNC

FUNC trees(n AS Integer) AS List OF Tree
  MUT xs AS List OF Tree = []
  FOR i = 0 TO n - 1
    LET leaf AS Tree = Tree[id := i, kids := []]
    xs = collections::append(xs, Tree[id := (i * 7919) MOD n, kids := [leaf]])
  NEXT
  RETURN xs
END FUNC

FUNC byScore(it AS Item) AS Float
  RETURN it.score
END FUNC

FUNC byId(it AS Item) AS Integer
  RETURN it.id
END FUNC

FUNC byName(it AS Item) AS String
  RETURN it.name
END FUNC

FUNC byVx(b AS Body) AS Float
  RETURN b.vx
END FUNC

FUNC byTreeId(t AS Tree) AS Integer
  RETURN t.id
END FUNC

' An order-sensitive fold, so stability (ties by score keep index order) shows.
FUNC foldItems(xs AS List OF Item) AS Integer
  MUT h AS Integer = 0
  FOR EACH it IN xs
    h = (h * 31 + it.id + len(it.name)) MOD 1000000007
  NEXT
  RETURN h
END FUNC

FUNC foldBodies(xs AS List OF Body) AS Integer
  MUT h AS Integer = 0
  FOR EACH b IN xs
    h = (h * 31 + toInt(b.x)) MOD 1000000007
  NEXT
  RETURN h
END FUNC

FUNC foldFloats(xs AS List OF Float) AS Integer
  MUT h AS Integer = 0
  FOR EACH f IN xs
    h = (h * 31 + toInt(f * 2.0)) MOD 1000000007
  NEXT
  RETURN h
END FUNC

FUNC foldTrees(xs AS List OF Tree) AS Integer
  MUT h AS Integer = 0
  FOR EACH t IN xs
    h = (h * 31 + t.id + len(t.kids)) MOD 1000000007
  NEXT
  RETURN h
END FUNC
";

/// `(name, builder, list type, copying sort expression over `src`, fold)`.
const CASES: &[(&str, &str, &str, &str, &str)] = &[
    ("record_float_key", "items", "List OF Item", "collections::sortBy(SRC, byScore)", "foldItems"),
    ("record_integer_key", "items", "List OF Item", "collections::sortBy(SRC, byId)", "foldItems"),
    ("record_string_key", "items", "List OF Item", "collections::sortBy(SRC, byName)", "foldItems"),
    ("fixed_width_record", "bodies", "List OF Body", "collections::sortBy(SRC, byVx)", "foldBodies"),
    ("float_sort", "floats", "List OF Float", "collections::sort(SRC)", "foldFloats"),
    ("type_cycle_element", "trees", "List OF Tree", "collections::sortBy(SRC, byTreeId)", "foldTrees"),
];

fn program(builder: &str, list_type: &str, sort: &str, fold: &str, copying: bool) -> String {
    let sorted = if copying {
        format!("  LET s AS {list_type} = {}\n", sort.replace("SRC", "xs"))
    } else {
        format!(
            "  MUT s AS {list_type} = xs\n  s = {}\n",
            sort.replace("SRC", "s")
        )
    };
    format!(
        "{TYPES}
FUNC main() AS Integer
  LET xs AS {list_type} = {builder}({N})
  MUT total AS Integer = 0
  FOR pass = 1 TO 5
{}    total = (total + {fold}(s) + {fold}(xs)) MOD 1000000007
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
",
        sorted.replace("  LET s", "    LET s").replace("  MUT s", "    MUT s").replace("  s = ", "    s = ")
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
        .find(|p| p.ends_with("-glibc.out"))
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
fn a_copying_sort_costs_what_the_self_update_spelling_costs() {
    let mut failures = Vec::new();
    for (name, builder, list_type, sort, fold) in CASES {
        let (copy_out, copy_alloc, copy_free, copy_live) = run(
            &format!("b767_{name}_copy"),
            &program(builder, list_type, sort, fold, true),
        );
        let (self_out, self_alloc, self_free, self_live) = run(
            &format!("b767_{name}_self"),
            &program(builder, list_type, sort, fold, false),
        );
        if copy_out != self_out {
            failures.push(format!(
                "{name}: the copying sort printed {copy_out:?}, the self-update sort {self_out:?}"
            ));
            continue;
        }
        if copy_alloc != copy_free || copy_live != 0 || self_alloc != self_free || self_live != 0 {
            failures.push(format!(
                "{name}: leaked (copy {copy_alloc}/{copy_free}/{copy_live} B, self \
                 {self_alloc}/{self_free}/{self_live} B)"
            ));
            continue;
        }
        if copy_alloc > self_alloc + SLACK {
            failures.push(format!(
                "{name}: the copying sort allocated {copy_alloc} blocks over 5 sorts of {N}, the \
                 self-update spelling {self_alloc} — {} more (≤ {SLACK} allowed)",
                copy_alloc - self_alloc
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "bug-767: a copying sortBy/sort must run the native merge after its one copy:\n  {}",
        failures.join("\n  ")
    );
}

/// A failing `keyFn`: no list is returned, the error reaches the handler, and
/// nothing leaks — the partial copy and keys are freed.
#[test]
fn a_failing_key_fn_in_a_copying_sort_by_frees_everything() {
    let source = format!(
        "{TYPES}
FUNC failAt(it AS Item) AS Float
  IF it.id = 7 THEN
    FAIL error(77050002, \"boom\")
  END IF
  RETURN it.score
END FUNC

FUNC tryOnce(xs AS List OF Item) AS Integer
  LET s AS List OF Item = collections::sortBy(xs, failAt)
  RETURN len(s)
  TRAP(e)
    RETURN -1
  END TRAP
END FUNC

FUNC main() AS Integer
  LET xs AS List OF Item = items(50)
  MUT acc AS Integer = 0
  FOR k = 1 TO 20
    acc = acc + tryOnce(xs)
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
"
    );
    let (text, alloc, free, live) = run("b767_failing_key", &source);
    assert_eq!(text, "-20\n");
    assert_eq!((alloc, live), (free, 0), "bug-767: a failing keyFn leaked");
}

/// Sub-issue B: every lowering calls `keyFn` once for a one-element list, as
/// `mfb man collections sortBy` documents.
#[test]
fn every_sort_by_lowering_calls_key_fn_once_for_one_element() {
    let source = "\
IMPORT io
IMPORT collections
TYPE Item
  id AS Integer
  name AS String
END TYPE
FUNC kf(it AS Item) AS Float
  io::print(\"kf called \" & toString(it.id))
  RETURN toFloat(it.id)
END FUNC
FUNC ki(v AS Integer) AS Integer
  io::print(\"ki called \" & toString(v))
  RETURN v
END FUNC
FUNC main() AS Integer
  LET one AS List OF Item = [Item[id := 7, name := \"a\"]]
  LET s AS List OF Item = collections::sortBy(one, kf)
  io::print(\"copy rec len \" & toString(len(s)))
  MUT m AS List OF Item = [Item[id := 8, name := \"b\"]]
  m = collections::sortBy(m, kf)
  io::print(\"inplace rec len \" & toString(len(m)))
  LET ones AS List OF Integer = [5]
  LET t AS List OF Integer = collections::sortBy(ones, ki)
  io::print(\"native int len \" & toString(len(t)))
  LET none AS List OF Item = []
  LET u AS List OF Item = collections::sortBy(none, kf)
  io::print(\"empty len \" & toString(len(u)))
  RETURN 0
END FUNC
";
    let (text, alloc, free, live) = run("b767_one_element", source);
    assert_eq!(
        text,
        "kf called 7\ncopy rec len 1\nkf called 8\ninplace rec len 1\nki called 5\n\
         native int len 1\nempty len 0\n",
        "bug-767 B: keyFn is called once for a one-element list on every lowering, and not \
         for an empty list"
    );
    assert_eq!((alloc, live), (free, 0));
}
