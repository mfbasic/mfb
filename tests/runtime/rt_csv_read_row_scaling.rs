//! bug-809: streaming a CSV document with `csv::readRow` costs memory traffic
//! linear in the document, not quadratic.
//!
//! The old API threaded a `csv::CsvReader` value that carried the whole decoded
//! input inline; every `readRow` returned a fresh `CsvRow` holding a copy of it,
//! so an n-row loop copied O(n) bytes n times. Measured on the bug's reproduction
//! (macos-aarch64, `--debug`): `arena.0.alloc_bytes` was 283,113,744 at 1,000
//! rows and 1,266,416,512 at 2,000 — x4.47 per doubling. `csv::readRow` now reads
//! one record from an `fs::File` whose position the host advances in place.
//!
//! The assertion is on the arena's byte counter, never wall time: the program is
//! run at N, 2N and 4N rows and the INCREMENTS are compared, which cancels every
//! fixed cost (startup, the dialect, the file). A linear loop doubles its
//! increment with the row count (ratio ~2); the quadratic one quadrupled it.
//! Both readers are covered — an `fs::openData` copy and a real file — in one
//! program, so a regression in either moves the ratio.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

const SOURCE: &str = r#"IMPORT io
IMPORT csv
IMPORT fs
IMPORT os
IMPORT strings
IMPORT collections

FUNC makeCsv(rows AS Integer) AS String
  MUT parts AS List OF String = ["id,name,note"]
  MUT i AS Integer = 0
  WHILE i < rows
    parts = collections::append(parts, toString(i) & ",value" & toString(i) & ",\"quoted, with comma\"")
    i = i + 1
  END WHILE
  RETURN strings::join(parts, "\n") & "\n"
END FUNC

FUNC countRows(f AS fs::File) AS Integer
  MUT n AS Integer = 0
  WHILE NOT fs::eof(f)
    LET row AS List OF String = csv::readRow(f)
    IF len(row) <> 3 THEN
      FAIL error(77050003, "wrong field count")
    END IF
    n = n + 1
  END WHILE
  RETURN n
END FUNC

FUNC main() AS Nothing
  LET rows AS Integer = toInt(os::getEnv("ROWS"))
  LET text = makeCsv(rows)
  RES memory = fs::openData(text, "r")
  LET fromMemory AS Integer = countRows(memory)
  LET path AS String = os::getEnv("CSV_PATH")
  fs::writeText(path, text)
  RES disk = fs::openFile(path)
  LET fromDisk AS Integer = countRows(disk)
  fs::close(disk)
  fs::deleteFile(path)
  io::print(toString(fromMemory) & " " & toString(fromDisk))
END FUNC
"#;

fn build_debug(name: &str, source: &str) -> PathBuf {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} failed to build:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let chosen = written
        .iter()
        .find(|path| path.ends_with(common::host_executable_suffix()))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    PathBuf::from(chosen)
}

/// Run the program at `rows` rows and return `arena.0.alloc_bytes`, after checking
/// that both readers saw every record (header included).
fn alloc_bytes(exe: &Path, rows: u64) -> u64 {
    let csv_path = std::env::temp_dir().join(format!(
        "mfb_csv_scaling_{rows}_{}.csv",
        common::unique_nonce()
    ));
    let output = Command::new(exe)
        .env("ROWS", rows.to_string())
        .env("CSV_PATH", &csv_path)
        .output()
        .expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{rows} rows failed:\n{stdout}\n{stderr}"
    );
    let records = rows + 1;
    assert_eq!(
        stdout.lines().next(),
        Some(format!("{records} {records}").as_str()),
        "{rows} rows: wrong record counts"
    );
    stderr
        .lines()
        .find_map(|line| line.strip_prefix("arena.0.alloc_bytes "))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{rows} rows: no arena.0.alloc_bytes in:\n{stderr}"))
}

#[test]
fn read_row_streaming_allocates_linearly_in_the_row_count() {
    let exe = build_debug("csv_read_row_scaling", SOURCE);
    let n = 1_000;
    let at_n = alloc_bytes(&exe, n);
    let at_2n = alloc_bytes(&exe, 2 * n);
    let at_4n = alloc_bytes(&exe, 4 * n);
    let first = at_2n - at_n;
    let second = at_4n - at_2n;
    let ratio = second as f64 / first as f64;
    // Linear: the second increment covers twice the rows of the first (~2.0).
    // Quadratic (the old CsvReader threading): ~4. 3.0 is the midpoint.
    assert!(
        ratio < 3.0,
        "csv::readRow is no longer linear: alloc_bytes {at_n} / {at_2n} / {at_4n} at \
         {n} / {} / {} rows; increment ratio {ratio:.2} (linear ~2, quadratic ~4)",
        2 * n,
        4 * n
    );
    // And per row, the whole stream (both readers) stays far below one copy of
    // the document per record: the old loop copied ~118 KB per row at 1,000 rows.
    let per_row = second / (2 * n);
    assert!(
        per_row < 16_384,
        "csv::readRow allocates {per_row} bytes per row ({at_2n} -> {at_4n})"
    );
}
