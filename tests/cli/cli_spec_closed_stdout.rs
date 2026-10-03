//! bug-787: `mfb spec` panicked (exit 101, "failed printing to stdout: Broken pipe")
//! when its stdout reader had already gone away — `mfb spec architecture --all | head`.
//! A closed reader is a normal way to stop reading, so every rendering mode must end
//! quietly with exit 0 and an empty stderr, as `mfb man` already does.
//!
//! The pipe's read end is closed before the child starts, so the first write fails
//! deterministically instead of depending on how fast `head` exits.

#![cfg(unix)]

use std::os::fd::FromRawFd;
use std::process::{Command, Stdio};

#[path = "../common/mod.rs"]
mod common;
use common::*;

extern "C" {
    fn pipe(fds: *mut i32) -> i32;
    fn close(fd: i32) -> i32;
}

fn run_with_closed_reader(args: &[&str]) -> std::process::Output {
    let mut fds = [0i32; 2];
    // SAFETY: `fds` is a valid two-element buffer `pipe` fills with fresh descriptors.
    assert_eq!(unsafe { pipe(fds.as_mut_ptr()) }, 0, "pipe failed");
    // SAFETY: `fds[0]` is the read end just created and owned by nothing else.
    unsafe { close(fds[0]) };
    // SAFETY: `fds[1]` is the write end just created; `Stdio` takes sole ownership.
    let writer = unsafe { Stdio::from_raw_fd(fds[1]) };
    Command::new(mfb_exe())
        .args(args)
        .stdout(writer)
        .stderr(Stdio::piped())
        .output()
        .expect("run mfb")
}

#[test]
fn spec_ends_quietly_when_its_reader_is_closed() {
    for args in [
        &["spec"][..],
        &["spec", "architecture"],
        &["spec", "architecture", "native"],
        &["spec", "architecture", "--all"],
    ] {
        let output = run_with_closed_reader(args);
        assert_eq!(
            output.status.code(),
            Some(0),
            "mfb {args:?} on a closed reader: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "mfb {args:?} wrote to stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
