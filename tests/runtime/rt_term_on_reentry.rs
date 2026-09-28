//! bug-728: `term::off` restores the line discipline the terminal had before the
//! FIRST `term::on`, however many times `term::on` ran in between. A re-entered
//! `term::on` used to save the raw discipline the first call installed, so
//! `term::off` "restored" raw mode and the program left the user's terminal in
//! single-key, no-echo mode.
//!
//! Each case runs the program on a real pty and reads the tty's `ICANON`/`ECHO`
//! bits after the child exits, through a slave handle the parent kept open.

#[path = "../common/mod.rs"]
mod common;
use common::*;
use std::path::Path;
use std::process::Command;

/// Runs `executable` on a pty (canonical mode and echo on, as a login shell
/// leaves it) and reports the tty's modes once it exits: `icanon=<0|1>
/// echo=<0|1>`.
fn tty_modes_after(executable: &Path) -> String {
    let output = Command::new("python3")
        .arg("-c")
        .arg(
            r#"import os, pty, select, subprocess, sys, termios, time
master, slave = pty.openpty()
attrs = termios.tcgetattr(slave)
attrs[3] |= termios.ICANON | termios.ECHO
termios.tcsetattr(slave, termios.TCSANOW, attrs)
keep = os.dup(slave)
proc = subprocess.Popen([sys.argv[1]], stdin=slave, stdout=slave, stderr=slave, close_fds=True)
os.close(slave)
deadline = time.time() + 30.0
while proc.poll() is None:
    ready, _, _ = select.select([master], [], [], 0.1)
    if ready:
        try:
            os.read(master, 4096)
        except OSError:
            pass
    if time.time() > deadline:
        proc.kill()
        sys.stderr.write("timed out\n")
        sys.exit(124)
lflag = termios.tcgetattr(keep)[3]
print("icanon=%d echo=%d" % (1 if lflag & termios.ICANON else 0, 1 if lflag & termios.ECHO else 0))
sys.exit(proc.wait())"#,
        )
        .arg(executable)
        .output()
        .expect("run pty helper");
    assert!(
        output.status.success(),
        "pty run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8")
        .trim()
        .to_string()
}

fn program(calls: &str) -> String {
    format!("IMPORT term\n\nFUNC main AS Integer\n{calls}  RETURN 0\nEND FUNC\n")
}

#[test]
fn a_single_on_off_restores_canonical_mode_and_echo() {
    let exe = build_project(&temp_project(
        "bug728_on_once",
        &program("  term::on()\n  term::off()\n"),
    ));
    assert_eq!(tty_modes_after(&exe), "icanon=1 echo=1");
}

#[test]
fn a_reentered_on_does_not_make_off_restore_raw_mode() {
    let exe = build_project(&temp_project(
        "bug728_on_twice",
        &program("  term::on()\n  term::on()\n  term::off()\n"),
    ));
    assert_eq!(tty_modes_after(&exe), "icanon=1 echo=1");
}

#[test]
fn many_reentries_still_restore_the_original_mode() {
    let exe = build_project(&temp_project(
        "bug728_on_thrice",
        &program("  term::on()\n  term::on()\n  term::on()\n  term::off()\n"),
    ));
    assert_eq!(tty_modes_after(&exe), "icanon=1 echo=1");
}
