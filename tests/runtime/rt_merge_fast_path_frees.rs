//! `collections::merge`'s native fast path frees what it materializes.
//!
//! The fast path (a `String`-keyed map, a constant `preferB = TRUE`) rebuilds each
//! of `b`'s keys — and each `String` value — as a fresh length-prefixed block to
//! hand to `lower_map_set_in_place`, which copies the bytes into the result. The
//! blocks were never freed: two entries of `b` leaked four blocks (80 bytes), found
//! while plan-142-D's differential probe compared its in-place `merge` against the
//! copying one (the in-place arm balanced; the copying call did not). The program
//! must end with every allocated block freed.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

fn arena_counters(name: &str, statement: &str) -> (u64, u64, u64) {
    let source = format!(
        "IMPORT collections\nIMPORT io\n\n\
         FUNC main() AS Integer\n  \
         LET a AS Map OF String TO {v} = Map OF String TO {v} {{ \"x\" := {x}, \"y\" := {y} }}\n  \
         LET b AS Map OF String TO {v} = Map OF String TO {v} {{ \"y\" := {y2}, \"w\" := {w} }}\n  \
         {statement}\n  \
         RETURN 0\nEND FUNC\n",
        v = if name.contains("str") {
            "String"
        } else {
            "Integer"
        },
        x = if name.contains("str") { "\"1\"" } else { "1" },
        y = if name.contains("str") { "\"2\"" } else { "2" },
        y2 = if name.contains("str") {
            "\"two-but-longer\""
        } else {
            "22"
        },
        w = if name.contains("str") { "\"4\"" } else { "4" },
    );
    let project = common::temp_project(name, &source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "{name}: build failed:\n{stdout}");
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
    let err = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{name}: program failed:\n{err}");
    let counter = |key: &str| -> u64 {
        err.lines()
            .find_map(|l| l.strip_prefix(&format!("arena.0.{key} ")))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or_else(|| panic!("{name}: no arena.0.{key}:\n{err}"))
    };
    (
        counter("alloc_calls"),
        counter("free_calls"),
        counter("live_bytes"),
    )
}

#[test]
fn the_merge_fast_path_frees_every_key_and_value_it_rebuilds() {
    let mut failures = Vec::new();
    for name in ["merge_fast_str_values", "merge_fast_int_values"] {
        let (alloc, free, live) = arena_counters(
            name,
            "LET m AS Map OF String TO String = collections::merge(a, b, TRUE)\n  io::print(toString(len(m)))"
                .replace(
                    "Map OF String TO String",
                    if name.contains("str") {
                        "Map OF String TO String"
                    } else {
                        "Map OF String TO Integer"
                    },
                )
                .as_str(),
        );
        if alloc != free || live != 0 {
            failures.push(format!(
                "{name}: {alloc} blocks allocated, {free} freed, {live} bytes live at exit"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
