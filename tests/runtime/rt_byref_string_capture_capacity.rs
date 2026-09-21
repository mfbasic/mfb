//! A `String` local captured by reference keeps no stale capacity shadow.
//!
//! An in-place self-append `s = s & t` tracks the spare bytes of `s`'s grown
//! buffer in a frame slot of the function that owns `s` (plan-02 §4.1). A
//! `collections::forEach` lambda that captures `s` by reference and reassigns it
//! replaces the buffer without touching that slot — it cannot see it — so the
//! owner's next self-append believed the new, tight buffer still had the old
//! one's spare bytes and wrote past its end. Measured before the fix: after
//! `forEach(…, LAMBDA(v AS String) -> s = v & "q")`, sixty `s = s & "c"` appends
//! overwrote the blocks allocated after it, and the program died with
//! `Error: 7-701-0001 Allocation failed.` (`alloc_bytes 2361831178293657712`).
//! Found by plan-142-G deciding where a by-ref capture's shadow could live.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// `(name, main body, expected output)`. Each grows `s` in place, lets a lambda
/// rewrite it through the reference, allocates three small neighbours, and then
/// grows `s` again past any spare bytes its new buffer could have.
const CASES: &[(&str, &str, &str)] = &[
    (
        "reassigned",
        "MUT s AS String = \"a\"
  FOR i = 1 TO 100
    s = s & \"b\"
  NEXT
  collections::forEach([\"x\", \"y\"], LAMBDA(v AS String) -> s = v & \"q\")
  LET t1 AS String = \"T\" & toString(len(s))
  LET t2 AS String = \"U\" & toString(len(s))
  LET t3 AS String = \"V\" & toString(len(s))
  FOR i = 1 TO 60
    s = s & \"c\"
  NEXT
  io::print(toString(len(s)))
  io::print(t1 & t2 & t3)",
        "62\nT2U2V2",
    ),
    (
        "self_appended",
        "MUT s AS String = \"a\"
  FOR i = 1 TO 100
    s = s & \"b\"
  NEXT
  collections::forEach([\"x\", \"y\"], LAMBDA(v AS String) -> s = s & v)
  LET t1 AS String = \"T\" & toString(len(s))
  LET t2 AS String = \"U\" & toString(len(s))
  LET t3 AS String = \"V\" & toString(len(s))
  FOR i = 1 TO 200
    s = s & \"c\"
  NEXT
  io::print(toString(len(s)))
  io::print(t1 & t2 & t3)",
        "303\nT103U103V103",
    ),
];

#[test]
fn an_owner_appending_after_a_by_ref_rewrite_stays_in_bounds() {
    let mut failures = Vec::new();
    for (name, body, want) in CASES {
        let source = format!(
            "IMPORT collections\nIMPORT io\n\nFUNC main() AS Integer\n  {body}\n  RETURN 0\nEND FUNC\n"
        );
        let project = common::temp_project(&format!("byref_string_capacity_{name}"), &source);
        let output = Command::new(common::mfb_exe())
            .arg("build")
            .arg(&project)
            .output()
            .expect("run mfb build");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.status.success() {
            failures.push(format!(
                "{name}: build failed:\n{stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            ));
            continue;
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
        let printed = String::from_utf8_lossy(&run.stdout);
        let printed = printed.trim_end_matches('\n');
        if !run.status.success() || printed != *want {
            failures.push(format!(
                "{name}: printed {printed:?} (want {want:?}), exit {:?}, stderr {}",
                run.status.code(),
                String::from_utf8_lossy(&run.stderr).trim()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
