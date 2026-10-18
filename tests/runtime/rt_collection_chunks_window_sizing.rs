//! The native `collections::chunks` / `collections::window` fast paths (a
//! constant `size`, and for `window` a constant `stride`) size each block from
//! the elements it holds, never from the requested `size`.
//!
//! bug-1007: the fixed-width `chunks` fast path spaced every chunk at
//! `HEADER + size * 8`, so `chunks([1, 2, 3], 1000000)` reserved 8 MB for one
//! three-element chunk (`arena.0.alloc_bytes 8000288`; the same call with a
//! run-time size: 560).
//!
//! bug-1008: the same compile-time `size * 8` overflowed the compiler's own
//! arithmetic for a large valid constant (a panic in a debug compiler), and the
//! String fast paths emitted `size` / `stride` as backend add immediates, which
//! AArch64 rejects past its decomposition limit. A large positive constant must
//! compile and produce what the same call with a run-time value produces.
//!
//! Each case builds the call twice under `--debug`: once with the literal (the
//! fast path) and once with the same value read at run time (the generic body),
//! and compares the printed contents and `arena.0.alloc_bytes`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// Build with `--debug`, run, and return `(stdout, alloc_bytes)`.
fn run(name: &str, source: &str) -> (String, u64) {
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
    let bytes = report
        .lines()
        .find_map(|line| line.strip_prefix("arena.0.alloc_bytes "))
        .and_then(|n| n.trim().parse::<u64>().ok())
        .unwrap_or_else(|| panic!("{name}: no arena.0.alloc_bytes in\n{report}"));
    (text, bytes)
}

/// A program that binds `call` (with `{ARGS}` replaced by `args`) and prints the
/// block count, then each block's length and elements.
fn program(call: &str, args: &str) -> String {
    let call = call.replace("{ARGS}", args);
    format!(
        "IMPORT collections
IMPORT io
IMPORT os

FUNC main() AS Integer
  LET xs = {call}
  io::print(toString(len(xs)))
  FOR EACH c IN xs
    MUT line AS String = toString(len(c))
    FOR EACH v IN c
      line = line & \" \" & toString(v)
    NEXT
    io::print(line)
  NEXT
  RETURN 0
END FUNC
"
    )
}

/// `args` with every integer literal read at run time instead, so the call
/// takes the generic body rather than the constant fast path.
fn dynamic(args: &str) -> String {
    args.split(", ")
        .map(|a| format!("toInt(os::getEnvOr(\"MFB_UNSET_CHUNKS_ARG\", \"{a}\"))"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// bug-1007: a constant chunk size far larger than the input allocates about
/// what the result holds — no more than the same call with a run-time size —
/// and returns the same single three-element chunk.
#[test]
fn constant_chunk_size_allocates_for_the_elements_held() {
    let call = "collections::chunks([1, 2, 3], {ARGS})";
    let (fast_out, fast_bytes) = run("chunks_alloc_const", &program(call, "1000000"));
    let (slow_out, slow_bytes) = run("chunks_alloc_dyn", &program(call, &dynamic("1000000")));
    assert_eq!(fast_out, "1\n3 1 2 3\n");
    assert_eq!(fast_out, slow_out);
    assert!(
        fast_bytes <= slow_bytes,
        "constant-size chunks allocated {fast_bytes} bytes, the run-time-size call {slow_bytes}"
    );
}

/// bug-1007: chunks with a short final block, and windows, from the fast path
/// allocate no more than the generic body and hold the same contents.
#[test]
fn fast_path_blocks_are_sized_by_their_contents() {
    let cases: &[(&str, &str, &str, &str)] = &[
        (
            "chunks_int_short_tail",
            "collections::chunks([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], {ARGS})",
            "4",
            "3\n4 1 2 3 4\n4 5 6 7 8\n2 9 10\n",
        ),
        (
            "chunks_float_short_tail",
            "collections::chunks([1.5, 2.5, 3.5], {ARGS})",
            "2",
            "2\n2 1.50 2.50\n1 3.50\n",
        ),
        (
            "window_int",
            "collections::window([1, 2, 3, 4], {ARGS})",
            "3",
            "2\n3 1 2 3\n3 2 3 4\n",
        ),
        (
            "window_int_too_large",
            "collections::window([1, 2, 3], {ARGS})",
            "1000000",
            "0\n",
        ),
    ];
    for (name, call, args, expected) in cases {
        let (fast_out, fast_bytes) = run(&format!("{name}_const"), &program(call, args));
        let (slow_out, slow_bytes) = run(&format!("{name}_dyn"), &program(call, &dynamic(args)));
        assert_eq!(&fast_out, expected, "{name}: constant-argument contents");
        assert_eq!(fast_out, slow_out, "{name}: constant vs run-time contents");
        assert!(
            fast_bytes <= slow_bytes,
            "{name}: constant arguments allocated {fast_bytes} bytes, run-time {slow_bytes}"
        );
    }
}

/// bug-1008: large valid constant sizes and strides compile and return what
/// the same call with run-time values returns.
#[test]
fn large_constant_sizes_and_strides_compile_and_match_runtime_values() {
    let cases: &[(&str, &str, &str, &str)] = &[
        (
            "chunks_int_huge",
            "collections::chunks([1, 2, 3], {ARGS})",
            "2305843009213693952",
            "1\n3 1 2 3\n",
        ),
        (
            "chunks_float_max",
            "collections::chunks([1.5, 2.5], {ARGS})",
            "9223372036854775807",
            "1\n2 1.50 2.50\n",
        ),
        (
            "window_int_huge",
            "collections::window([1, 2, 3], {ARGS})",
            "2305843009213693952",
            "0\n",
        ),
        (
            "chunks_string_large",
            "collections::chunks([\"a\", \"b\"], {ARGS})",
            "1000000000",
            "1\n2 a b\n",
        ),
        (
            "window_string_large_size",
            "collections::window([\"a\", \"b\"], {ARGS})",
            "1000000000",
            "0\n",
        ),
        (
            "window_string_large_stride",
            "collections::window([\"a\", \"b\"], {ARGS})",
            "1, 1000000000",
            "1\n1 a\n",
        ),
    ];
    for (name, call, args, expected) in cases {
        let (fast_out, _) = run(&format!("{name}_const"), &program(call, args));
        let (slow_out, _) = run(&format!("{name}_dyn"), &program(call, &dynamic(args)));
        assert_eq!(&fast_out, expected, "{name}: constant-argument contents");
        assert_eq!(fast_out, slow_out, "{name}: constant vs run-time contents");
    }
}
