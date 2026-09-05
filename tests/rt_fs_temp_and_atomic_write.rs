//! bug-544: `fs::createTempFile` and `fs::writeTextAtomic`/`fs::writeBytesAtomic`
//! must actually work — on every platform, at RUNTIME.
//!
//! They did not on Windows, and nothing noticed for as long as they have existed,
//! because nothing ran them there:
//!
//!   * `rt_fs_create_mode_0600.rs` is `#![cfg(unix)]` (it asserts a `0600` mode,
//!     which has no Windows meaning), and
//!   * `rt_fs_atomic_int_return.rs` is a codegen-INSPECTION test — it reads the
//!     emitted instruction stream for the `sxtw`/`movsxd` narrowing and never
//!     executes a thing.
//!
//! So the whole atomic-write path was covered on Unix and, on Windows, only as
//! far as "it compiles". `fs::createTempFile()` raised `7-702-0002 ErrWriteFailed`
//! on every call there, and `fs::writeTextAtomic` with it; the first test to
//! notice was `rt_http_handle_request_serves`, whose MFB server publishes its
//! port with `fs::writeTextAtomic` and therefore hung until its 20-second
//! deadline — a symptom that names neither `fs` nor the real cause.
//!
//! This test is deliberately platform-neutral: no mode bits, no path syntax, no
//! `cfg`. It just does the thing and reads the bytes back, which is the part that
//! was never checked.

mod common;

use std::time::Duration;

const SOURCE: &str = r#"IMPORT fs
IMPORT io
IMPORT strings

FUNC main AS Integer
  RES t = fs::createTempFile()
  io::print("createTempFile=ok")
  fs::close(t)

  fs::writeTextAtomic("atomic.txt", "hello atomic")
  LET text AS String = fs::readText("atomic.txt")
  io::print("text=" & text)

  fs::writeBytesAtomic("atomic.bin", strings::toBytes("hello bytes"))
  LET raw AS String = fs::readText("atomic.bin")
  io::print("bytes=" & raw)

  ' An atomic write REPLACES an existing file — the rename half of the path,
  ' which on Windows is a different call than the create half.
  fs::writeTextAtomic("atomic.txt", "replaced")
  LET again AS String = fs::readText("atomic.txt")
  io::print("replaced=" & again)
  RETURN 0
END FUNC
"#;

#[test]
fn create_temp_file_and_atomic_writes_work() {
    let project = common::temp_project("bug544_atomic", SOURCE);
    let exe = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &exe,
        Duration::from_secs(60),
        "bug-544: the atomic-write program did not finish",
    );
    assert!(
        status.success(),
        "bug-544: fs::createTempFile / fs::writeTextAtomic must succeed on every \
         platform. On Windows both raised 7-702-0002 ErrWriteFailed, because the \
         emitters returned their result in the C register (`rax`) while the shared \
         caller reads the MFB one (`rcx`).\nprogram {}\nstdout:\n{stdout}",
        common::exit_description(&status),
    );
    for expected in [
        "createTempFile=ok",
        "text=hello atomic",
        "bytes=hello bytes",
        "replaced=replaced",
    ] {
        assert!(
            stdout.contains(expected),
            "bug-544: expected `{expected}` in the program's output:\n{stdout}"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}
