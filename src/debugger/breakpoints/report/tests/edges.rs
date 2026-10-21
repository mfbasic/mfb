//! The report's window at its edges: a thread that is gone, the writer
//! entering the shutdown again, an empty write, a program that links no
//! `write`, and a breakpoint of the user's where the report returns.

use super::*;

#[test]
fn a_hit_by_a_thread_that_is_gone_opens_no_report() {
    let mut program = Exiting::new(vec![
        Event::Enter(2),
        Event::Write(2, 2, "mfb.debug.begin 1\n"),
        Event::Return(2),
        Event::Exit(3),
    ]);
    let (out, err) = session(&mut program, "run\nshow report\nq\n");
    assert_eq!(err, "");
    assert_eq!(out[..2], ["--[EX] Program exit (3)", NONE_KEPT]);
    assert_eq!(program.terminal, "mfb.debug.begin 1\n");
}

#[test]
fn the_writer_entering_the_shutdown_again_and_an_empty_write_change_nothing() {
    let mut script = report();
    script.splice(1..1, [Event::Enter(0), Event::Write(0, 2, "")]);
    let mut program = Exiting::new(script);
    let (out, err) = session(&mut program, "run\nshow report\nq\n");
    assert_eq!(err, "");
    assert_eq!(
        out[..3],
        [EXIT_WITH_REPORT, "mfb.debug.begin 1", "mfb.debug.end 1"]
    );
    assert_eq!(program.terminal, "");
}

#[test]
fn a_program_that_links_no_write_writes_its_report_itself() {
    let mut program = Exiting::new(report());
    program.links_write = false;
    let (out, _) = session(&mut program, "run\nshow report\nq\n");
    assert_eq!(out[..2], ["--[EX] Program exit (3)", NONE_KEPT]);
    assert_eq!(program.terminal, "mfb.debug.begin 1\nmfb.debug.end 1\n");
}

#[test]
fn a_breakpoint_of_the_users_where_the_report_returns_stays_after_it() {
    let mut program = Exiting::new(report());
    let (out, err) = session(
        &mut program,
        &format!("bp {BACK:#x}\nrun\nrun\nshow report\nq\n"),
    );
    assert_eq!(err, "");
    assert_eq!(
        out[1..],
        [
            "--[BP] T: 0 P: 0x0000000000002004 F: -",
            EXIT_WITH_REPORT,
            "mfb.debug.begin 1",
            "mfb.debug.end 1",
            INCOMPLETE,
            "Process 42 had already exited.",
        ]
    );
    // The window closed at the stop and left the user's breakpoint.
    assert_eq!(program.inserted, [SHUTDOWN, BACK]);
}

#[test]
fn a_report_line_that_cannot_be_skipped_is_an_error() {
    let mut program = Exiting::new(report());
    program.frozen = true;
    let (_, err) = session(&mut program, "run\nq\n");
    assert_eq!(err, "error: cannot set thread 0's registers\n");
}
