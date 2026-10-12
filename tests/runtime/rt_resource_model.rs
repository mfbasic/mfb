//! The resource model's rules, one program per filed bug.
//!
//! Each program is the bug's own repro (`bugs/repro/`). It runs its body `N`
//! times and is measured at two values of `N` from the `--debug` report: the
//! exit code and the total it prints prove every resource is open where the
//! program reads it; live bytes that do not grow with `N` prove every block and
//! every resource record is freed by the scope that owns it (a constant is a
//! module-level value held until exit); `double_free_skips 0` proves nothing is
//! freed twice; and 300 iterations under a 64-descriptor limit prove every
//! descriptor is closed.
//!
//! The rules (`mfb spec language resource-management`, "The owner of a
//! resource"): a resource is closed, and its record freed, by the scope its
//! owner word names; a value made of references is plain data; the scope only
//! floats up, at every store that lets a reference outlive it; a `RETURN` hands
//! what it releases to the caller's statement.

#[path = "../common/mod.rs"]
mod common;

use std::path::Path;
use std::process::Command;

struct Run {
    code: Option<i32>,
    stdout: String,
    live: u64,
    double_frees: u64,
}

fn counter(report: &str, key: &str) -> u64 {
    report
        .lines()
        .find_map(|line| line.strip_prefix(&format!("arena.0.{key} ")))
        .unwrap_or_else(|| panic!("no arena.0.{key} in:\n{report}"))
        .trim()
        .parse()
        .expect("numeric counter")
}

fn run(executable: &Path, n: u32, descriptor_limit: bool) -> Run {
    let mut command = if descriptor_limit && cfg!(unix) {
        let mut shell = Command::new("/bin/sh");
        shell
            .arg("-c")
            .arg("ulimit -n 64; exec \"$0\"")
            .arg(executable);
        shell
    } else {
        Command::new(executable)
    };
    let output = command
        .env("N", n.to_string())
        .output()
        .expect("run the program");
    let report = String::from_utf8_lossy(&output.stderr).to_string();
    Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        live: counter(&report, "live_bytes"),
        double_frees: counter(&report, "double_free_skips"),
    }
}

/// Build `source` with `--debug` and hold it to the rules at `N` = 10 and 40.
fn holds(name: &str, source: &str, at_10: &str, at_40: &str) {
    let data =
        std::env::temp_dir().join(format!("mfb_model_{name}_{}.txt", common::unique_nonce()));
    std::fs::write(&data, "hello\n").expect("write the data file");
    let source = source.replace("/tmp/g933/data.txt", &common::mfb_path_literal(&data));
    let project = common::temp_project(name, &source);
    let built = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    assert!(
        built.status.success(),
        "{name}: build failed:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let executable = project.join("build").join(format!(
        "{name}{}",
        if cfg!(windows) { ".exe" } else { ".out" }
    ));
    let small = run(&executable, 10, false);
    let large = run(&executable, 40, false);
    for (run, expected, n) in [(&small, at_10, 10), (&large, at_40, 40)] {
        assert_eq!(run.code, Some(0), "{name} N={n}: exit code");
        assert_eq!(run.stdout, expected, "{name} N={n}: stdout");
        assert_eq!(run.double_frees, 0, "{name} N={n}: a block was freed twice");
    }
    assert_eq!(
        small.live, large.live,
        "{name}: live bytes grow with the iteration count"
    );
    let limited = run(&executable, 300, true);
    assert_eq!(
        limited.code,
        Some(0),
        "{name}: 300 iterations under 64 descriptors"
    );
    assert_eq!(limited.live, small.live, "{name}: live bytes at N=300");
    let _ = std::fs::remove_file(&data);
}

#[test]
fn bug_1018_list_of_res_argument() {
    holds(
        "bug_1018_list_of_res_argument",
        include_str!("../../bugs/repro/bug-1018-list-of-res-argument.mfb"),
        "n=20",
        "n=80",
    );
}

#[test]
fn bug_1018_list_of_res_bound_from_a_call() {
    holds(
        "bug_1018_list_of_res_bound_from_a_call",
        include_str!("../../bugs/repro/bug-1018-list-of-res-bound-from-a-call.mfb"),
        "n=20",
        "n=80",
    );
}

#[test]
fn bug_1018_map_of_res_argument() {
    holds(
        "bug_1018_map_of_res_argument",
        include_str!("../../bugs/repro/bug-1018-map-of-res-argument.mfb"),
        "n=10",
        "n=40",
    );
}

#[test]
fn bug_1063_state_variant_to_union_bind() {
    holds(
        "bug_1063_state_variant_to_union_bind",
        include_str!("../../bugs/repro/bug-1063-state-variant-to-union-bind.mfb"),
        "50",
        "200",
    );
}

#[test]
fn bug_1063_state_variant_to_union_arg() {
    holds(
        "bug_1063_state_variant_to_union_arg",
        include_str!("../../bugs/repro/bug-1063-state-variant-to-union-arg.mfb"),
        "70",
        "280",
    );
}

#[test]
fn bug_1063_state_variant_to_union_append() {
    holds(
        "bug_1063_state_variant_to_union_append",
        include_str!("../../bugs/repro/bug-1063-state-variant-to-union-append.mfb"),
        "60",
        "240",
    );
}

#[test]
fn bug_1064_record_from_a_call_under_an_inline_trap() {
    holds(
        "bug_1064_record_from_a_call_under_an_inl",
        include_str!("../../bugs/repro/bug-1064-record-from-a-call-under-an-inline-trap.mfb"),
        "120",
        "480",
    );
}

#[test]
fn bug_1065_stateful_list_floated_from_a_loop() {
    holds(
        "bug_1065_stateful_list_floated_from_a_lo",
        include_str!("../../bugs/repro/bug-1065-stateful-list-floated-from-a-loop.mfb"),
        "30",
        "120",
    );
}

#[test]
fn bug_1066_union_map_for_each_match() {
    holds(
        "bug_1066_union_map_for_each_match",
        include_str!("../../bugs/repro/bug-1066-union-map-for-each-match.mfb"),
        "180",
        "720",
    );
}

#[test]
fn bug_1067_list_literal_assigned_in_a_loop() {
    holds(
        "bug_1067_list_literal_assigned_in_a_loop",
        include_str!("../../bugs/repro/bug-1067-list-literal-assigned-in-a-loop.mfb"),
        "120",
        "480",
    );
}

#[test]
fn bug_1069_res_in_constructor_field_appended() {
    holds(
        "bug_1069_res_in_constructor_field_append",
        include_str!("../../bugs/repro/bug-1069-res-in-constructor-field-appended.mfb"),
        "140",
        "560",
    );
}

#[test]
fn bug_1070_list_raised_from_a_loop_copied_by_a_callee() {
    holds(
        "bug_1070_list_raised_from_a_loop_copied_",
        include_str!("../../bugs/repro/bug-1070-list-raised-from-a-loop-copied-by-a-callee.mfb"),
        "140",
        "560",
    );
}

#[test]
fn bug_1071_inner_list_appended_to_a_list_of_lists() {
    holds(
        "bug_1071_inner_list_appended_to_a_list_o",
        include_str!("../../bugs/repro/bug-1071-inner-list-appended-to-a-list-of-lists.mfb"),
        "240",
        "960",
    );
}

#[test]
fn bug_1072_bound_list_of_res_inline_trap() {
    holds(
        "bug_1072_bound_list_of_res_inline_trap",
        include_str!("../../bugs/repro/bug-1072-bound-list-of-res-inline-trap.mfb"),
        "n=20 keep=0",
        "n=80 keep=0",
    );
}

#[test]
fn bug_1074_with_update_from_a_call() {
    holds(
        "bug_1074_with_update_from_a_call",
        include_str!("../../bugs/repro/bug-1074-with-update-from-a-call.mfb"),
        "n=20",
        "n=80",
    );
}

#[test]
fn bug_1075_foreach_lambda_appends_to_captured_outer_list() {
    holds(
        "bug_1075_foreach_lambda_appends_to_captu",
        include_str!("../../bugs/repro/bug-1075-foreach-lambda-appends-to-captured-outer-list.mfb"),
        "80",
        "320",
    );
}

#[test]
fn bug_1076_filter_result_assigned_to_outer_list() {
    holds(
        "bug_1076_filter_result_assigned_to_outer",
        include_str!("../../bugs/repro/bug-1076-filter-result-assigned-to-outer-list.mfb"),
        "70",
        "280",
    );
}

#[test]
fn bug_1077_function_trap_handler_reads_drained_list() {
    holds(
        "bug_1077_function_trap_handler_reads_dra",
        include_str!("../../bugs/repro/bug-1077-function-trap-handler-reads-drained-list.mfb"),
        "70",
        "280",
    );
}

#[test]
fn bug_1078_close_only_drain_kept_beside_an_unraised_outward_store() {
    holds(
        "bug_1078_close_only_drain_kept_beside_an",
        include_str!(
            "../../bugs/repro/bug-1078-close-only-drain-kept-beside-an-unraised-outward-store.mfb"
        ),
        "70",
        "280",
    );
}

#[test]
fn bug_999_call_through_a_function_value() {
    holds(
        "bug_999_call_through_a_function_value",
        include_str!("../../bugs/repro/bug-999-call-through-a-function-value.mfb"),
        "n=20",
        "n=80",
    );
}

#[test]
fn bug_999_callback_result_unbound() {
    holds(
        "bug_999_callback_result_unbound",
        include_str!("../../bugs/repro/bug-999-callback-result-unbound.mfb"),
        "n=20",
        "n=80",
    );
}

#[test]
fn bug_999_match_on_an_element_of_a_call() {
    holds(
        "bug_999_match_on_an_element_of_a_call",
        include_str!("../../bugs/repro/bug-999-match-on-an-element-of-a-call.mfb"),
        "n=10",
        "n=40",
    );
}

#[test]
fn bug_999_record_copy_source_reassigned() {
    holds(
        "bug_999_record_copy_source_reassigned",
        include_str!("../../bugs/repro/bug-999-record-copy-source-reassigned.mfb"),
        "10",
        "40",
    );
}
