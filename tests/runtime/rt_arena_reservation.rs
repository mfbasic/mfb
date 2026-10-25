//! The startup arena reservation (plan-183-B).
//!
//! At startup the program reserves `slices × 20 GiB` of address space — slices = 1 (the
//! entry thread) + 1 (a canvas program's graphics thread) + 2 × `options.maxThreads` —
//! and halves the request until the OS grants it, down to a floor of `slices × 256
//! MiB`. The `--debug` report prints the slice count and the slice size it settled on.
//!
//! Set `MFB_TEST_REMOTE` to run the programs on another box (`tests/common/remote.rs`).

#[path = "../common/mod.rs"]
mod common;

const PROGRAM: &str = "\
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET xs AS List OF Integer = collections::repeat([], 7, 100000)
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
";

const TWENTY_GIB: u64 = 20 << 30;

fn report_value(name: &str, stderr: &str, key: &str) -> u64 {
    let lines = common::debug_report::arena_lines(name, stderr);
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&format!("{key} ")))
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("{name}: no `{key}` in {lines:?}"))
}

/// The default build reserves 21 slices of 20 GiB.
#[test]
fn the_default_reservation_is_twenty_one_slices_of_twenty_gib() {
    let name = "arena_reservation_default";
    let exe = common::debug_report::build_debug(name, PROGRAM);
    let output = common::remote::output(&exe, &[], &[]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{name}: {stdout}\n{stderr}");
    assert_eq!(stdout.trim(), "100000");
    assert_eq!(report_value(name, &stderr, "arena.slices"), 21);
    assert_eq!(report_value(name, &stderr, "arena.slice_size"), TWENTY_GIB);
}

/// Under a 64 GiB address-space limit the 420 GiB request is refused, and the program
/// still starts on a smaller reservation: the halving settles on 52.5 GiB, so each of
/// the 21 slices is 2.5 GiB. Linux only: neither macOS nor Windows enforces
/// `RLIMIT_AS`.
#[test]
fn a_limited_address_space_halves_the_reservation_and_still_starts() {
    if !common::remote::run_target().starts_with("linux") {
        eprintln!("skipped: RLIMIT_AS is enforced only on Linux");
        return;
    }
    let name = "arena_reservation_limited";
    let exe = common::debug_report::build_debug(name, PROGRAM);
    let output = common::remote::output_in_shell(&exe, "ulimit -v 67108864 &&");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{name}: {stdout}\n{stderr}");
    assert_eq!(stdout.trim(), "100000");
    assert_eq!(report_value(name, &stderr, "arena.slices"), 21);
    let slice = report_value(name, &stderr, "arena.slice_size");
    assert!(
        slice < TWENTY_GIB,
        "a 64 GiB address space cannot hold 21 slices of 20 GiB, yet the slice is {slice}"
    );
    assert_eq!(
        slice,
        TWENTY_GIB / 8,
        "420 GiB halved three times is 52.5 GiB"
    );
}
