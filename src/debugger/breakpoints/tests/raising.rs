//! `breakerror` running on to the raiser: a breakpoint of the user's already
//! there, a write or the end on the way, another raise meanwhile, and the
//! watch's park shared with a breakpoint of the user's.

use super::*;

fn raised(code: i64) -> RunStop {
    RunStop::Raised {
        thread: 0,
        code,
        origin: None,
    }
}

#[test]
fn a_raiser_already_holding_a_breakpoint_keeps_it_after_the_stop() {
    let mut program = Program::new(vec![
        RunStop::Breakpoints(vec![(0, PARK)]),
        RunStop::Breakpoints(vec![(0, RAISER)]),
    ]);
    program.bound = true;
    program.raised = vec![77_050_010];
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .watch_errors(&mut program, ErrorCodes::Any, &[])
        .unwrap();
    breakpoints
        .set(&mut program, &Target::Address(RAISER), None, &[])
        .unwrap();
    let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert_eq!(stop, raised(77_050_010));
    assert_eq!(program.inserted, [PARK, RAISER]);
}

#[test]
fn a_write_that_changes_a_watch_on_the_way_to_the_raiser_is_the_stop() {
    let mut program = Program::new(vec![
        RunStop::Breakpoints(vec![(0, PARK)]),
        RunStop::Watchpoints(vec![0]),
    ]);
    program.bound = true;
    program.raised = vec![77_050_010];
    program.writes = vec![vec![(MEMORY + 8, 9)]];
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .watch_errors(&mut program, ErrorCodes::Any, &[])
        .unwrap();
    breakpoints.watch(&mut program, MEMORY + 8, 8, &[]).unwrap();
    let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert_eq!(
        stop,
        RunStop::Changed(vec![Change {
            thread: 0,
            pc: STORE,
            address: MEMORY + 8,
            size: 8,
            old: 0,
            new: 9,
        }])
    );
    // The run's own breakpoint on the raiser is gone again.
    assert_eq!(program.inserted, [PARK]);
}

#[test]
fn a_program_that_ends_on_the_way_to_the_raiser_ends_the_run() {
    let mut program = Program::new(vec![
        RunStop::Breakpoints(vec![(0, PARK)]),
        RunStop::Exited(Exit::Status(1)),
    ]);
    program.bound = true;
    program.raised = vec![77_050_010];
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .watch_errors(&mut program, ErrorCodes::Any, &[])
        .unwrap();
    let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert_eq!(stop, RunStop::Exited(Exit::Status(1)));
}

#[test]
fn another_raise_on_the_way_to_the_raiser_is_let_go() {
    let mut program = Program::new(vec![
        RunStop::Breakpoints(vec![(0, PARK)]),
        RunStop::Breakpoints(vec![(0, PARK)]),
        RunStop::Breakpoints(vec![(0, RAISER)]),
    ]);
    program.bound = true;
    program.raised = vec![77_050_010];
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .watch_errors(&mut program, ErrorCodes::Any, &[])
        .unwrap();
    let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert_eq!(stop, raised(77_050_010));
    assert_eq!(program.runs, 3);
}

#[test]
fn a_watch_set_over_a_breakpoint_on_the_park_shares_it_both_ways() {
    let mut program = Program::new(Vec::new());
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .set(&mut program, &name(PARK_ERROR), None, &[])
        .unwrap();
    // The park already holds the user's instruction: the watch places none.
    breakpoints
        .watch_errors(&mut program, ErrorCodes::Codes(vec![42]), &[])
        .unwrap();
    // A later watch's commands replace the watch's.
    assert_eq!(
        breakpoints
            .watch_errors(
                &mut program,
                ErrorCodes::Codes(vec![7]),
                &["bt".to_string()]
            )
            .unwrap(),
        ["Stopping when a thread raises 42, 7. Each stop runs 'bt'."]
    );
    assert_eq!(program.inserted, [PARK]);
    // Clearing the watch leaves the user's breakpoint in place.
    breakpoints.clear_errors(&mut program).unwrap();
    assert_eq!(program.inserted, [PARK]);
}
