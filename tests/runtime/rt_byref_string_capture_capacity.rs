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
//!
//! plan-146-G gave the by-ref capture the owner's shadow, shared through the
//! closure environment, so the lambda's own self-updates (`s = s & t` and every
//! `String` arm) are in place there too. The cases below are the aliasing that
//! makes dangerous: the lambda grows, shrinks or replaces the owner's block, and
//! the owner then appends past whatever capacity that left.

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
    // plan-146-G: the lambda now GROWS `s` in place through the shared shadow,
    // then the owner appends past the capacity that grow left behind. A shadow
    // the two did not share would send one of them past the block's end.
    (
        "lambda_grows",
        "MUT s AS String = \"a\"
  FOR i = 1 TO 100
    s = s & \"b\"
  NEXT
  collections::forEach([\"x\", \"y\"], LAMBDA(v AS String) -> s = strings::padRight(s, 300))
  LET t1 AS String = \"T\" & toString(len(s))
  LET t2 AS String = \"U\" & toString(len(s))
  LET t3 AS String = \"V\" & toString(len(s))
  FOR i = 1 TO 200
    s = s & \"c\"
  NEXT
  io::print(toString(len(s)))
  io::print(t1 & t2 & t3)",
        "500\nT300U300V300",
    ),
    // The lambda SHRINKS it in place: the bytes it gives up become spare
    // capacity in the owner's shadow, which the owner's next append fills.
    (
        "lambda_shrinks",
        "MUT s AS String = \"a\"
  FOR i = 1 TO 100
    s = s & \"b\"
  NEXT
  collections::forEach([\"x\", \"y\"], LAMBDA(v AS String) -> s = strings::left(s, 10))
  LET t1 AS String = \"T\" & toString(len(s))
  LET t2 AS String = \"U\" & toString(len(s))
  LET t3 AS String = \"V\" & toString(len(s))
  FOR i = 1 TO 200
    s = s & \"c\"
  NEXT
  io::print(toString(len(s)))
  io::print(t1 & t2 & t3)",
        "210\nT10U10V10",
    ),
    // The lambda REASSIGNS it (a fresh tight block) after the owner grew it: the
    // store through the reference frees by the shared shadow and resets it to 0,
    // so the owner's next append does not believe in bytes that are not there.
    (
        "lambda_reassigns_then_owner_grows",
        "MUT s AS String = \"a\"
  FOR i = 1 TO 100
    s = s & \"b\"
  NEXT
  collections::forEach([\"x\"], LAMBDA(v AS String) -> s = strings::repeat(v, 3))
  LET t1 AS String = \"T\" & toString(len(s))
  LET t2 AS String = \"U\" & toString(len(s))
  LET t3 AS String = \"V\" & toString(len(s))
  FOR i = 1 TO 200
    s = s & \"c\"
  NEXT
  io::print(toString(len(s)))
  io::print(t1 & t2 & t3)",
        "203\nT3U3V3",
    ),
    // The owner grows it, then the lambda rewrites it in place with each arm
    // kind in turn (a window, a grow and a rewrite), and the owner appends again.
    (
        "owner_grows_then_lambda_every_arm",
        "MUT s AS String = \"a\"
  FOR i = 1 TO 100
    s = s & \"b\"
  NEXT
  collections::forEach([\"x\"], LAMBDA(v AS String) -> s = strings::left(s, 20))
  collections::forEach([\"x\"], LAMBDA(v AS String) -> s = strings::padRight(s, 40))
  collections::forEach([\"x\"], LAMBDA(v AS String) -> s = strings::upper(s))
  LET t1 AS String = \"T\" & toString(len(s))
  LET t2 AS String = \"U\" & toString(len(s))
  LET t3 AS String = \"V\" & toString(len(s))
  FOR i = 1 TO 200
    s = s & \"c\"
  NEXT
  io::print(toString(len(s)))
  io::print(t1 & t2 & t3)",
        "240\nT40U40V40",
    ),
];

#[test]
fn an_owner_appending_after_a_by_ref_rewrite_stays_in_bounds() {
    let mut failures = Vec::new();
    for (name, body, want) in CASES {
        let source = format!(
            "IMPORT collections\nIMPORT io\nIMPORT strings\n\nFUNC main() AS Integer\n  {body}\n  RETURN 0\nEND FUNC\n"
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
