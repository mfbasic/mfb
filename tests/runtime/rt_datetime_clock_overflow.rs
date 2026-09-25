//! A clock reading whose nanosecond count does not fit an `Integer` raises
//! `ErrOverflow` instead of wrapping (bug-640).
//!
//! `emit_libc_clock_nanos` folded `clock_gettime`'s `timespec` into one count as
//! `tv_sec * 1_000_000_000 + tv_nsec` with an unchecked multiply and add, so the
//! first second past `2262-04-11T23:47:16.854775807Z` wrapped to a large negative
//! count and `datetime::now` returned a pre-1970 `Instant` with no error. The
//! monotonic clock shares the same fold.
//!
//! No host clock reads 2262, so **the oracle is a `clock_gettime` interposer**
//! (`LD_PRELOAD` / `DYLD_INSERT_LIBRARIES`) that hands the program a chosen
//! `timespec` when `MFB_FAKE_CLOCK` is set. Unix-only for the same reason as the
//! `close()` interposer in `tests/common`: Windows reads the clock through
//! kernel32 and has no loader-level symbol preload.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ERR_OVERFLOW: i64 = 77050010;

/// Build a `clock_gettime` interposer. With `MFB_FAKE_CLOCK="<sec> <nsec>"` in
/// the environment every clock id reads that `timespec`; without it, the call
/// passes through to the real clock.
fn build_clock_interposer(root: &Path) -> PathBuf {
    let source = root.join("fake_clock.c");
    fs::write(
        &source,
        r#"
#include <stdlib.h>
#include <time.h>
#if !defined(__APPLE__)
#include <sys/syscall.h>
#include <unistd.h>
#endif

static int mfb_fake(struct timespec *ts) {
  const char *fake = getenv("MFB_FAKE_CLOCK");
  if (!fake || !fake[0]) {
    return 0;
  }
  char *end = NULL;
  long long sec = strtoll(fake, &end, 10);
  long long nsec = strtoll(end, NULL, 10);
  ts->tv_sec = (time_t)sec;
  ts->tv_nsec = (long)nsec;
  return 1;
}

#if defined(__APPLE__)
static int mfb_clock_gettime(clockid_t id, struct timespec *ts) {
  if (mfb_fake(ts)) {
    return 0;
  }
  return clock_gettime(id, ts);
}
typedef struct {
  const void *replacement;
  const void *replacee;
} interpose_t;
__attribute__((used)) static const interpose_t interposers[] __attribute__((section("__DATA,__interpose"))) = {
  { (const void *)mfb_clock_gettime, (const void *)clock_gettime }
};
#else
int clock_gettime(clockid_t id, struct timespec *ts) {
  if (mfb_fake(ts)) {
    return 0;
  }
  return (int)syscall(SYS_clock_gettime, id, ts);
}
#endif
"#,
    )
    .expect("write clock interposer source");
    let library = if cfg!(target_os = "macos") {
        root.join("libfake_clock.dylib")
    } else {
        root.join("libfake_clock.so")
    };
    let mut command = Command::new("cc");
    if cfg!(target_os = "macos") {
        command.args(["-dynamiclib", "-o"]);
    } else {
        command.args(["-shared", "-fPIC", "-o"]);
    }
    let output = command
        .arg(&library)
        .arg(&source)
        .output()
        .expect("compile clock interposer");
    assert!(
        output.status.success(),
        "interposer build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    library
}

/// Every clock member, each printed as `<name> ok <value>` or `<name> raised <code>`.
const PROGRAM: &str = r#"IMPORT io
IMPORT datetime

FUNC readNowNanos AS String
  RETURN "ok " & toString(datetime::nowNanos())
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC readNow AS String
  LET i AS datetime::Instant = datetime::now()
  RETURN "ok " & toString(i.seconds) & " " & toString(i.nanos)
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC readMonotonicNanos AS String
  RETURN "ok " & toString(datetime::monotonicNanos())
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC readMonotonic AS String
  LET d AS datetime::Duration = datetime::monotonic()
  RETURN "ok " & toString(d.seconds) & " " & toString(d.nanos)
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  io::print("nowNanos " & readNowNanos())
  io::print("now " & readNow())
  io::print("monotonicNanos " & readMonotonicNanos())
  io::print("monotonic " & readMonotonic())
END SUB
"#;

/// Run the clock program with the clock pinned to `(sec, nsec)`.
fn read_clocks(sec: i64, nsec: i64) -> Vec<String> {
    let project = common::temp_project("datetime_clock_overflow", PROGRAM);
    let executable = common::build_project(&project);
    let interposer = build_clock_interposer(&project);
    let mut envs = vec![("MFB_FAKE_CLOCK", format!("{sec} {nsec}"))];
    if cfg!(target_os = "macos") {
        envs.push(("DYLD_INSERT_LIBRARIES", interposer.display().to_string()));
        envs.push(("DYLD_FORCE_FLAT_NAMESPACE", "1".to_string()));
    } else {
        envs.push(("LD_PRELOAD", interposer.display().to_string()));
    }
    let (status, stdout, stderr) = common::run_capture_with_env(&executable, &envs);
    assert_eq!(
        status, 0,
        "clock program failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let _ = fs::remove_dir_all(&project);
    stdout.lines().map(str::to_string).collect()
}

/// The lines a correct compiler prints for a clock reading of `(sec, nsec)`:
/// the exact count when it fits an `Integer`, otherwise `ErrOverflow` from all four.
fn expected(sec: i64, nsec: i64) -> Vec<String> {
    let exact = i128::from(sec) * 1_000_000_000 + i128::from(nsec);
    match i64::try_from(exact) {
        Ok(ns) => {
            let (s, n) = (ns.div_euclid(1_000_000_000), ns.rem_euclid(1_000_000_000));
            vec![
                format!("nowNanos ok {ns}"),
                format!("now ok {s} {n}"),
                format!("monotonicNanos ok {ns}"),
                format!("monotonic ok {s} {n}"),
            ]
        }
        Err(_) => ["nowNanos", "now", "monotonicNanos", "monotonic"]
            .iter()
            .map(|name| format!("{name} raised {ERR_OVERFLOW}"))
            .collect(),
    }
}

#[test]
fn the_interposer_really_pins_the_clock() {
    // Without this, a program that ignored the interposer would read the real
    // clock and every "in range" expectation below would fail for the wrong reason.
    assert_eq!(
        read_clocks(1_700_000_000, 123_456_789),
        expected(1_700_000_000, 123_456_789)
    );
}

#[test]
fn the_largest_representable_reading_is_returned_exactly() {
    assert_eq!(
        read_clocks(9_223_372_036, 854_775_807),
        expected(9_223_372_036, 854_775_807)
    );
}

#[test]
fn a_reading_past_2262_raises_overflow() {
    // The first nanosecond past the limit overflows the add; the first second past
    // it overflows the multiply.
    for (sec, nsec) in [
        (9_223_372_036, 854_775_808),
        (9_223_372_037, 0),
        (i64::MAX, 999_999_999),
    ] {
        let actual = read_clocks(sec, nsec);
        assert_eq!(
            actual,
            expected(sec, nsec),
            "clock pinned to ({sec}, {nsec})"
        );
    }
}

#[test]
fn a_reading_before_1678_raises_overflow() {
    for (sec, nsec) in [(-9_223_372_037, 0), (i64::MIN, 0)] {
        let actual = read_clocks(sec, nsec);
        assert_eq!(
            actual,
            expected(sec, nsec),
            "clock pinned to ({sec}, {nsec})"
        );
    }
    // Just inside the negative limit still reads exactly.
    assert_eq!(read_clocks(-9_223_372_036, 0), expected(-9_223_372_036, 0));
}

/// `crypto::uuid7` and `crypto::ulid` stamp the clock reading, so they raise the
/// same `ErrOverflow` rather than encoding a wrapped timestamp.
#[test]
fn clock_derived_identifiers_raise_overflow_past_2262() {
    const IDS: &str = r#"IMPORT io
IMPORT crypto

FUNC readUuid7 AS String
  RETURN "ok " & crypto::uuid7()
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC readUlid AS String
  RETURN "ok " & crypto::ulid()
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  io::print("uuid7 " & readUuid7())
  io::print("ulid " & readUlid())
END SUB
"#;
    let project = common::temp_project("datetime_clock_overflow_ids", IDS);
    let executable = common::build_project(&project);
    let interposer = build_clock_interposer(&project);
    let mut envs = vec![("MFB_FAKE_CLOCK", "9223372037 0".to_string())];
    if cfg!(target_os = "macos") {
        envs.push(("DYLD_INSERT_LIBRARIES", interposer.display().to_string()));
        envs.push(("DYLD_FORCE_FLAT_NAMESPACE", "1".to_string()));
    } else {
        envs.push(("LD_PRELOAD", interposer.display().to_string()));
    }
    let (status, stdout, stderr) = common::run_capture_with_env(&executable, &envs);
    assert_eq!(status, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    let _ = fs::remove_dir_all(&project);
    assert_eq!(
        stdout,
        format!("uuid7 raised {ERR_OVERFLOW}\nulid raised {ERR_OVERFLOW}\n")
    );
}
