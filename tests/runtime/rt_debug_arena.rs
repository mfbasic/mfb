//! plan-130-C: the `arena` section of the `--debug` report.
//!
//! A `--debug` build keeps a process-global registry of arena states — the main
//! thread's, each `thread::start` worker's, and the canvas graphics thread's — and the
//! report lists them in registration order: `arena.count <n>`,
//! `arena.registry_overflow <n>`, then per arena `arena.<i>.kind <main|worker|graphics>`
//! and one `arena.<i>.<counter> <value>` line per allocator event counter.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, build_debug, build_debug_project, counter, run_ok};
use std::process::Command;

/// Every counter each registered arena reports, in report order.
const COUNTERS: [&str; 22] = [
    "maps",
    "mapped_bytes",
    "unmaps",
    "unmapped_bytes",
    "alloc_calls",
    "alloc_bytes",
    "free_calls",
    "free_bytes",
    "live_bytes",
    "peak_live_bytes",
    "hit_quick_bin",
    "hit_carve",
    "hit_large_bin",
    "hit_walk",
    "grow",
    "flushes",
    "insert_free_calls",
    "double_free_skips",
    "fill_grow_calls",
    "fill_grow_bytes",
    "fill_free_calls",
    "fill_free_bytes",
];

/// The registration shape: totals and each arena's kind, with every counter of every
/// arena present as an integer.
fn registration(case: &str, lines: &[String]) -> Vec<String> {
    let count: usize = lines
        .iter()
        .find_map(|line| line.strip_prefix("arena.count "))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{case}: no arena.count in {lines:?}"));
    for index in 0..count {
        for counter in COUNTERS {
            let key = format!("arena.{index}.{counter} ");
            let line = lines
                .iter()
                .find(|line| line.starts_with(&key))
                .unwrap_or_else(|| panic!("{case}: missing `{key}<n>` in {lines:?}"));
            assert!(
                line[key.len()..].parse::<u64>().is_ok(),
                "{case}: `{line}` is not an integer counter"
            );
        }
    }
    lines
        .iter()
        .filter(|line| {
            line.starts_with("arena.count ")
                || line.starts_with("arena.registry_overflow ")
                || line.contains(".kind ")
        })
        .cloned()
        .collect()
}

/// Three `thread::start` workers plus the main thread: four arenas, registered in
/// start order, none dropped.
#[test]
fn three_workers_register_four_arenas() {
    let source = concat!(
        "IMPORT io\n",
        "IMPORT thread\n",
        "\n",
        "ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n",
        "  LET echoed AS String = seed & \"!\"\n",
        "  RETURN len(echoed) - 1\n",
        "END FUNC\n",
        "\n",
        "FUNC main AS Integer\n",
        "  LET a AS Thread OF String TO Integer = thread::start(work, \"a\")\n",
        "  LET b AS Thread OF String TO Integer = thread::start(work, \"bb\")\n",
        "  LET c AS Thread OF String TO Integer = thread::start(work, \"ccc\")\n",
        "  LET total AS Integer = thread::waitFor(a) + thread::waitFor(b) + thread::waitFor(c)\n",
        "  io::print(toString(total))\n",
        "  RETURN 0\n",
        "END FUNC\n",
    );
    let exe = build_debug("dbg_arena_workers", source);
    let (stdout, stderr) = run_ok("workers", &exe);
    assert_eq!(stdout, "6\n", "the workers ran");
    let lines = arena_lines("workers", &stderr);
    assert_eq!(
        registration("workers", &lines),
        vec![
            "arena.count 4",
            "arena.registry_overflow 0",
            "arena.0.kind main",
            "arena.1.kind worker",
            "arena.2.kind worker",
            "arena.3.kind worker",
        ],
        "arena section:\n{stderr}"
    );
    // Each worker builds a string, which it allocates in its own arena: the counts land
    // in the worker's slot, not the parent's.
    for index in 1..4 {
        assert!(
            counter("workers", &lines, index, "alloc_calls") >= 1,
            "worker arena {index} counted no allocation:\n{stderr}"
        );
    }
}

/// A canvas app that presents once starts the graphics thread, which registers its own
/// arena: the report lists the program's arena as `main` and the render thread's as
/// `graphics`. Headless, so it opens no window.
#[cfg(target_os = "macos")]
#[test]
fn the_canvas_graphics_thread_registers_a_graphics_arena() {
    let name = "dbg_arena_graphics";
    let source = concat!(
        "IMPORT app\n",
        "IMPORT canvas\n",
        "IMPORT color\n",
        "IMPORT io\n",
        "\n",
        "SUB main()\n",
        "  app::setMode(app::Mode.Canvas)\n",
        "  LET box AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(color::rgb(255, 0, 0))]\n",
        "  canvas::present([box])\n",
        "  io::print(\"done\")\n",
        "END SUB\n",
    );
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--app")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --app --debug");
    assert!(
        output.status.success(),
        "{name} failed to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(common::app_binary(&project, name))
        .env("MFB_MACAPP_HEADLESS", "1")
        .output()
        .expect("run the headless app");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "program failed:\n{stdout}\n{stderr}");
    assert!(stdout.contains("done"), "the program ran:\n{stdout}");
    let lines = arena_lines("graphics", &stderr);
    assert_eq!(
        registration("graphics", &lines),
        vec![
            "arena.count 2",
            "arena.registry_overflow 0",
            "arena.0.kind main",
            "arena.1.kind graphics",
        ],
        "arena section:\n{stderr}"
    );
}

/// A churn loop: each iteration builds a short string that dies with the iteration.
fn churn_source(n: usize) -> String {
    format!(
        "IMPORT io\n\nFUNC main AS Integer\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    LET s AS String = \"k\" & toString(i)\n    total = total + len(s)\n  NEXT\n  io::print(toString(total))\n  RETURN 0\nEND FUNC\n"
    )
}

/// A retaining loop: each iteration appends a short string to a list that lives until
/// the end of `main`.
fn retain_source(n: usize) -> String {
    format!(
        "IMPORT io\nIMPORT collections\n\nFUNC main AS Integer\n  MUT keep AS List OF String = []\n  FOR i = 1 TO {n}\n    keep = collections::append(keep, \"k\" & toString(i))\n  NEXT\n  io::print(toString(len(keep)))\n  RETURN 0\nEND FUNC\n"
    )
}

/// Run a `--debug` build of `source` and return the main arena's report lines.
fn main_arena(name: &str, source: &str) -> Vec<String> {
    let exe = build_debug(name, source);
    let (_, stderr) = run_ok(name, &exe);
    arena_lines(name, &stderr)
}

/// Every successful allocation is attributed to exactly one allocation path.
fn assert_partition(case: &str, lines: &[String]) {
    let paths = [
        "hit_quick_bin",
        "hit_carve",
        "hit_large_bin",
        "hit_walk",
        "grow",
    ]
    .iter()
    .map(|name| counter(case, lines, 0, name))
    .sum::<u64>();
    assert_eq!(
        counter(case, lines, 0, "alloc_calls"),
        paths,
        "{case}: alloc_calls must equal the sum of the five allocation paths: {lines:?}"
    );
}

/// Exact counts under churn: doubling the iterations adds at least one allocation and
/// one free per extra iteration, the extra allocations come from the quick bins, the
/// arena maps no more memory, and every allocation is attributed to exactly one path.
#[test]
fn churn_counts_scale_with_iterations_and_partition_by_path() {
    let small = main_arena("dbg_arena_churn_1000", &churn_source(1000));
    let large = main_arena("dbg_arena_churn_2000", &churn_source(2000));
    assert_partition("churn 1000", &small);
    assert_partition("churn 2000", &large);
    let delta = |name: &str| {
        counter("churn 2000", &large, 0, name) - counter("churn 1000", &small, 0, name)
    };
    assert!(
        delta("alloc_calls") >= 1000,
        "1000 extra iterations must add at least 1000 allocations: {small:?} / {large:?}"
    );
    assert!(
        delta("free_calls") >= 1000,
        "1000 extra iterations must add at least 1000 frees: {small:?} / {large:?}"
    );
    assert!(
        delta("hit_quick_bin") >= 999,
        "a steady churn reuses its quick bins: {small:?} / {large:?}"
    );
    assert_eq!(
        counter("churn 1000", &small, 0, "maps"),
        counter("churn 2000", &large, 0, "maps"),
        "churn reuses memory: doubling the iterations must map no more blocks"
    );
    assert_eq!(counter("churn 2000", &large, 0, "double_free_skips"), 0);
}

/// A single 1 MiB read grows the arena by at least that much.
#[test]
fn a_mebibyte_read_grows_the_arena() {
    let name = "dbg_arena_mebibyte";
    let project = common::temp_project(name, "");
    let data = project.join("big.bin");
    std::fs::write(&data, vec![0x5au8; 1 << 20]).expect("write the 1 MiB input");
    std::fs::write(
        project.join("src/main.mfb"),
        format!(
            "IMPORT fs\nIMPORT io\n\nFUNC main AS Integer\n  LET b AS List OF Byte = fs::readBytes(\"{}\")\n  io::print(toString(len(b)))\n  RETURN 0\nEND FUNC\n",
            common::mfb_path_literal(&data)
        ),
    )
    .expect("write the program");
    let exe = build_debug_project(name, &project);
    let (stdout, stderr) = run_ok(name, &exe);
    assert_eq!(stdout, "1048576\n", "the program read the whole file");
    let lines = arena_lines(name, &stderr);
    assert_partition(name, &lines);
    assert!(
        counter(name, &lines, 0, "grow") >= 1,
        "a 1 MiB allocation must grow the arena:\n{stderr}"
    );
    assert!(
        counter(name, &lines, 0, "mapped_bytes") >= 1 << 20,
        "the arena must map at least 1 MiB:\n{stderr}"
    );
    assert_eq!(
        counter(name, &lines, 0, "maps"),
        counter(name, &lines, 0, "unmaps"),
        "shutdown unmaps every block it mapped:\n{stderr}"
    );
}

/// The leak-counter control (memory `a-leak-counter-must-cover-everything-it-guards`):
/// values the program keeps alive must show up in `peak_live_bytes` in proportion to
/// how many it keeps, while the same number of short-lived values must not.
#[test]
fn retained_values_raise_peak_live_bytes_and_churn_does_not() {
    let retain_small = main_arena("dbg_arena_retain_1000", &retain_source(1000));
    let retain_large = main_arena("dbg_arena_retain_2000", &retain_source(2000));
    let churn_small = main_arena("dbg_arena_flat_1000", &churn_source(1000));
    let churn_large = main_arena("dbg_arena_flat_2000", &churn_source(2000));
    let peak = |case: &str, lines: &[String]| counter(case, lines, 0, "peak_live_bytes");
    let retained_growth = peak("retain 2000", &retain_large) - peak("retain 1000", &retain_small);
    let churn_growth =
        peak("churn 2000", &churn_large).saturating_sub(peak("churn 1000", &churn_small));
    assert!(
        retained_growth >= 1000 * 16,
        "keeping 1000 more strings must raise peak_live_bytes by at least 16 bytes each, got {retained_growth}"
    );
    assert!(
        churn_growth < retained_growth / 4,
        "short-lived values must not accumulate: churn grew {churn_growth}, retained grew {retained_growth}"
    );
}

/// A loop that builds and drops one 8-`Integer` record per iteration: one 64 B chunk freed
/// per iteration (a 6-field record is a 48 B chunk, plan-133-B § Corrections).
fn record_churn_source(n: usize) -> String {
    format!(
        "IMPORT io\n\nTYPE R8\n  a AS Integer\n  b AS Integer\n  c AS Integer\n  d AS Integer\n  e AS Integer\n  f AS Integer\n  g AS Integer\n  h AS Integer\nEND TYPE\n\nFUNC mk(i AS Integer) AS R8\n  RETURN R8[i, i, i, i, i, i, i, i]\nEND FUNC\n\nFUNC main AS Integer\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    LET r AS R8 = mk(i)\n    total = total + r.h\n  NEXT\n  io::print(toString(total))\n  RETURN 0\nEND FUNC\n"
    )
}

/// plan-133-B: the fill counters count exactly the bytes the entropy fill is handed.
/// A fresh block is filled over its usable region (`mapped − 32` per block), so over a
/// run `fill_grow_bytes == mapped_bytes − 32 × grow` with one fill per grow. A freed
/// chunk is scrubbed past its 16-byte node, so each extra 64 B record adds one fill of
/// 48 B.
#[test]
fn fill_counters_match_known_allocations() {
    // Grow path: a 1 MiB read maps at least one block beyond the first.
    let name = "dbg_arena_fill_grow";
    let project = common::temp_project(name, "");
    let data = project.join("big.bin");
    std::fs::write(&data, vec![0x5au8; 1 << 20]).expect("write the 1 MiB input");
    std::fs::write(
        project.join("src/main.mfb"),
        format!(
            "IMPORT fs\nIMPORT io\n\nFUNC main AS Integer\n  LET b AS List OF Byte = fs::readBytes(\"{}\")\n  io::print(toString(len(b)))\n  RETURN 0\nEND FUNC\n",
            common::mfb_path_literal(&data)
        ),
    )
    .expect("write the program");
    let exe = build_debug_project(name, &project);
    let (stdout, stderr) = run_ok(name, &exe);
    assert_eq!(stdout, "1048576\n", "the program read the whole file");
    let lines = arena_lines(name, &stderr);
    let grow = counter(name, &lines, 0, "grow");
    assert!(grow >= 1, "a 1 MiB read must grow the arena:\n{stderr}");
    assert_eq!(
        counter(name, &lines, 0, "fill_grow_calls"),
        grow,
        "one grow-path fill per grow:\n{stderr}"
    );
    assert_eq!(
        counter(name, &lines, 0, "fill_grow_bytes"),
        counter(name, &lines, 0, "mapped_bytes") - 32 * grow,
        "the grow-path fill covers each block's usable bytes (mapped − 32-byte header):\n{stderr}"
    );

    // Free path: 1000 more 64 B records add exactly 1000 scrubs of 48 B.
    let small = main_arena("dbg_arena_fill_free_1000", &record_churn_source(1000));
    let large = main_arena("dbg_arena_fill_free_2000", &record_churn_source(2000));
    let delta =
        |name: &str| counter("fill 2000", &large, 0, name) - counter("fill 1000", &small, 0, name);
    assert_eq!(
        delta("free_calls"),
        1000,
        "each extra iteration frees exactly one block: {small:?} / {large:?}"
    );
    assert_eq!(
        delta("free_bytes"),
        64 * 1000,
        "each freed record is a 64 B chunk: {small:?} / {large:?}"
    );
    assert_eq!(
        delta("fill_free_calls"),
        1000,
        "each freed 64 B chunk is scrubbed once: {small:?} / {large:?}"
    );
    assert_eq!(
        delta("fill_free_bytes"),
        48 * 1000,
        "each scrub covers the chunk past its 16-byte node: {small:?} / {large:?}"
    );
}
