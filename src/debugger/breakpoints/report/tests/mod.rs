use super::*;

mod edges;
use crate::debugger::names::C_PREFIX;
use crate::debugger::{
    prompt, Ending, Exit, FunctionRange, FunctionSymbol, Program, RegisterGroup, RunStop, Step,
    StepStop, ThreadFacts,
};

const SHUTDOWN: u64 = 0x1000;
/// Where `_mfb_debug_shutdown` returns to.
const BACK: u64 = 0x2004;
/// An instruction of the program's own, after the report's first line.
const MIDWAY: u64 = 0x1020;
/// An instruction the program reaches once the report is written.
const AFTER: u64 = 0x2008;
const WRITE: u64 = 0x9000;
/// Where a line being written is, and where its `write` returns to.
const LINE: u64 = 0x8000;
const WRITTEN: u64 = 0x1040;

/// What a [`Exiting`] program does next, by thread.
#[derive(Clone, Copy)]
enum Event {
    /// Enter `_mfb_debug_shutdown`.
    Enter(usize),
    /// Call `write` with this file descriptor and text.
    Write(usize, u64, &'static str),
    /// Return from `_mfb_debug_shutdown`.
    Return(usize),
    /// Run the instruction at this address.
    At(usize, u64),
    Exit(i32),
}

/// A program on its way out: it does each of `script` in turn, stopping
/// at an event whose address holds a breakpoint.
struct Exiting {
    script: Vec<Event>,
    next: usize,
    /// Stopped at the next event's breakpoint, the event not yet run.
    sitting: bool,
    /// Moved off it by a `set_registers` of `pc`: the event does not run.
    moved: bool,
    inserted: Vec<u64>,
    /// What the program's `write`s put on the terminal.
    terminal: String,
    exited: Option<i32>,
    /// Whether it is a `--debug` build.
    debug: bool,
    /// Whether it links `write`.
    links_write: bool,
    /// Every `set_registers` fails, as for a thread the system will not
    /// change.
    frozen: bool,
}

impl Exiting {
    fn new(script: Vec<Event>) -> Self {
        Exiting {
            script,
            next: 0,
            sitting: false,
            moved: false,
            inserted: Vec::new(),
            terminal: String::new(),
            exited: None,
            debug: true,
            links_write: true,
            frozen: false,
        }
    }

    fn place(event: Event) -> Option<(usize, u64)> {
        match event {
            Event::Enter(thread) => Some((thread, SHUTDOWN)),
            Event::Write(thread, ..) => Some((thread, WRITE)),
            Event::Return(thread) => Some((thread, BACK)),
            Event::At(thread, address) => Some((thread, address)),
            Event::Exit(_) => None,
        }
    }

    /// Run the next event; `false` when it ends the program.
    fn advance(&mut self) -> bool {
        match self.script[self.next] {
            Event::Exit(status) => {
                self.exited = Some(status);
                return false;
            }
            Event::Write(_, _, text) if !self.moved => self.terminal.push_str(text),
            _ => {}
        }
        self.sitting = false;
        self.moved = false;
        self.next += 1;
        true
    }
}

fn function(name: &str, linked: bool, address: u64) -> FunctionSymbol {
    FunctionSymbol {
        name: name.to_string(),
        linked,
        loaded: false,
        address: Some(address),
        size: None,
    }
}

impl Inferior for Exiting {
    fn pid(&self) -> u32 {
        42
    }
    fn arch(&self) -> &'static crate::debugger::arch::Arch {
        &crate::debugger::arm64::AARCH64
    }
    fn end(&mut self) -> Result<Ending, String> {
        Ok(Ending::AlreadyExited)
    }
    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        let mut functions = vec![function("_main", false, 0x2000)];
        if self.links_write {
            functions.push(function(&format!("{C_PREFIX}write"), true, WRITE));
        }
        if self.debug {
            functions.push(function(DEBUG_SHUTDOWN_SYMBOL, false, SHUTDOWN));
        }
        Ok(functions)
    }
    fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn read_bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        match self.script[self.next] {
            Event::Write(_, _, text) if address == LINE && length == text.len() => {
                Ok(text.as_bytes().to_vec())
            }
            _ => Err(format!("unmapped {address:#x}")),
        }
    }
    fn read_code(&mut self, address: u64, _: usize) -> Result<Vec<u8>, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn function_at(&mut self, _: u64) -> Result<Option<FunctionRange>, String> {
        Ok(None)
    }
    fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
        Ok(Vec::new())
    }
    fn data_symbol_at(&mut self, _: u64) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        let at = Exiting::place(self.script[self.next]).filter(|_| self.sitting);
        Ok((0..2)
            .map(|thread| ThreadFacts {
                id: 10 + thread as u64,
                handle: 0,
                name: None,
                pc: at.filter(|(on, _)| *on == thread).map_or(0, |(_, pc)| pc),
                sp: 0,
                arena: 0,
                stack: None,
            })
            .collect())
    }
    fn registers(&mut self, _: usize) -> Result<Vec<Register>, String> {
        let (x0, x1, x2, lr) = match self.script[self.next] {
            Event::Write(_, fd, text) => (fd, LINE, text.len() as u64, WRITTEN),
            Event::Enter(_) => (0, 0, 0, BACK),
            _ => (0, 0, 0, 0),
        };
        Ok([("x0", x0), ("x1", x1), ("x2", x2), ("lr", lr)]
            .into_iter()
            .map(|(name, value)| Register {
                name: name.to_string(),
                group: RegisterGroup::General,
                value: RegisterValue::Bits64(value),
                role: None,
            })
            .collect())
    }
    fn set_registers(&mut self, _: usize, values: &[(String, u64)]) -> Result<(), String> {
        if self.frozen {
            return Err("cannot set thread 0's registers".to_string());
        }
        let text = match self.script[self.next] {
            Event::Write(_, _, text) => text,
            _ => return Err("a register was set outside write".to_string()),
        };
        assert_eq!(
            values,
            [
                ("x0".to_string(), text.len() as u64),
                ("pc".to_string(), WRITTEN)
            ]
        );
        self.moved = true;
        Ok(())
    }
    fn step(&mut self, _: &[usize], count: u64) -> Result<Vec<Step>, String> {
        for taken in 0..count {
            if !self.advance() {
                return Ok(vec![Step {
                    taken,
                    stop: StepStop::Exited,
                }]);
            }
        }
        Ok(vec![Step {
            taken: count,
            stop: StepStop::Finished,
        }])
    }
    fn restart(&mut self, _: &[(std::ffi::OsString, std::ffi::OsString)]) -> Result<u32, String> {
        self.next = 0;
        self.sitting = false;
        self.moved = false;
        self.inserted.clear();
        self.exited = None;
        Ok(43)
    }
    fn entry(&mut self) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn insert_breakpoint(&mut self, address: u64) -> Result<(), String> {
        if self.inserted.contains(&address) {
            return Err(format!("there is already a breakpoint at {address:#x}"));
        }
        self.inserted.push(address);
        Ok(())
    }
    fn remove_breakpoint(&mut self, address: u64) -> Result<(), String> {
        let before = self.inserted.len();
        self.inserted.retain(|other| *other != address);
        if self.inserted.len() == before {
            return Err(format!("there is no breakpoint at {address:#x}"));
        }
        Ok(())
    }
    fn insert_watchpoint(&mut self, _: u64, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn remove_watchpoint(&mut self, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn run(&mut self) -> Result<RunStop, String> {
        loop {
            if let Some(status) = self.exited {
                return Ok(RunStop::Exited(Exit::Status(status)));
            }
            let stop = Exiting::place(self.script[self.next])
                .filter(|(_, address)| !self.sitting && self.inserted.contains(address));
            if let Some(hit) = stop {
                self.sitting = true;
                return Ok(RunStop::Breakpoints(vec![hit]));
            }
            self.advance();
        }
    }
}

/// A session over `program` fed `input`: what it printed, less the
/// prompts, and the errors.
fn session(program: &mut Exiting, input: &str) -> (Vec<String>, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    prompt::run(
        program,
        Program { app: false },
        Some(Vec::new()),
        &mut input.as_bytes(),
        &mut out,
        &mut err,
        false,
    )
    .expect("session runs");
    let out = String::from_utf8(out).unwrap();
    (
        out.lines()
            .map(|line| line.trim_start_matches(prompt::PROMPT).to_string())
            .filter(|line| !line.is_empty())
            .collect(),
        String::from_utf8(err).unwrap(),
    )
}

/// A report of two lines, written by thread 0, and the exit.
fn report() -> Vec<Event> {
    vec![
        Event::Enter(0),
        Event::Write(0, 2, "mfb.debug.begin 1\n"),
        Event::Write(0, 2, "mfb.debug.end 1\n"),
        Event::Return(0),
        Event::Exit(3),
    ]
}

const EXIT_WITH_REPORT: &str =
    "--[EX] Program exit (3) - type \"show report\" to see the performance and memory report";

#[test]
fn the_report_is_kept_from_the_terminal_and_shown_after_the_exit() {
    let mut program = Exiting::new(report());
    let (out, err) = session(&mut program, "show report\nrun\nshow report\nq\n");
    assert_eq!(
        out,
        [
            NOT_YET,
            EXIT_WITH_REPORT,
            "mfb.debug.begin 1",
            "mfb.debug.end 1",
            "Process 42 had already exited.",
        ]
    );
    assert_eq!(err, "");
    assert_eq!(program.terminal, "");
    // The window's breakpoints went with it.
    assert_eq!(program.inserted, [SHUTDOWN]);
}

#[test]
fn only_the_reporting_threads_writes_to_stderr_are_taken() {
    let mut script = report();
    script.splice(
        2..2,
        [
            Event::Write(1, 2, "a worker's own\n"),
            Event::Write(0, 1, "to stdout\n"),
        ],
    );
    let mut program = Exiting::new(script);
    let (out, _) = session(&mut program, "run\nshow report\nq\n");
    assert_eq!(out[1..3], ["mfb.debug.begin 1", "mfb.debug.end 1"]);
    assert_eq!(program.terminal, "a worker's own\nto stdout\n");
}

#[test]
fn a_write_before_the_report_is_the_programs() {
    let mut script = report();
    script.insert(0, Event::Write(0, 2, "an error message\n"));
    let mut program = Exiting::new(script);
    let (out, _) = session(&mut program, "run\nq\n");
    assert_eq!(out[0], EXIT_WITH_REPORT);
    assert_eq!(program.terminal, "an error message\n");
}

#[test]
fn a_second_pass_through_the_shutdown_does_not_replace_the_report() {
    let mut script = report();
    script.splice(4..4, [Event::Enter(1), Event::Return(1)]);
    let mut program = Exiting::new(script);
    let (out, err) = session(&mut program, "run\nshow report\nq\n");
    assert_eq!(out[1..3], ["mfb.debug.begin 1", "mfb.debug.end 1"]);
    assert_eq!(err, "");
}

#[test]
fn a_stop_inside_the_report_leaves_the_rest_to_the_program() {
    let mut script = report();
    script.insert(2, Event::At(0, MIDWAY));
    let mut program = Exiting::new(script);
    let (out, err) = session(
        &mut program,
        &format!("bp {MIDWAY:#x}\nrun\nshow report\nrun\nshow report\nq\n"),
    );
    assert_eq!(err, "");
    assert_eq!(
        out[2..],
        [
            // Stopped inside it: what was taken, and that there is more.
            "mfb.debug.begin 1",
            INCOMPLETE,
            EXIT_WITH_REPORT,
            "mfb.debug.begin 1",
            INCOMPLETE,
            "Process 42 had already exited.",
        ]
    );
    assert_eq!(program.terminal, "mfb.debug.end 1\n");
    assert_eq!(program.inserted, [SHUTDOWN, MIDWAY]);
}

#[test]
fn a_breakpoint_of_the_users_on_the_shutdown_leaves_the_report_to_the_program() {
    let mut program = Exiting::new(report());
    let (out, err) = session(
        &mut program,
        "bp _mfb_debug_shutdown\nrun\nrun\nshow report\nq\n",
    );
    assert_eq!(err, "");
    assert_eq!(out[2..4], ["--[EX] Program exit (3)", NONE_KEPT]);
    assert_eq!(program.terminal, "mfb.debug.begin 1\nmfb.debug.end 1\n");
}

#[test]
fn a_program_that_ends_inside_the_report_keeps_what_it_wrote() {
    let mut script = report();
    script.truncate(2);
    script.push(Event::Exit(3));
    let mut program = Exiting::new(script);
    let (out, _) = session(&mut program, "run\nshow report\nq\n");
    assert_eq!(
        out[..3],
        [EXIT_WITH_REPORT, "mfb.debug.begin 1", INCOMPLETE]
    );
}

#[test]
fn a_program_that_ends_without_a_report_says_so() {
    let mut program = Exiting::new(vec![Event::Exit(3)]);
    let (out, _) = session(&mut program, "run\nshow report\nq\n");
    assert_eq!(out[..2], ["--[EX] Program exit (3)", NONE_KEPT]);
}

#[test]
fn a_build_without_debug_has_no_report() {
    let mut program = Exiting::new(report());
    program.debug = false;
    let (out, _) = session(&mut program, "show report\nrun\nshow report\nq\n");
    assert_eq!(
        out[..3],
        [
            NOT_A_DEBUG_BUILD,
            "--[EX] Program exit (3)",
            NOT_A_DEBUG_BUILD
        ]
    );
    assert_eq!(program.terminal, "mfb.debug.begin 1\nmfb.debug.end 1\n");
    assert!(program.inserted.is_empty(), "{:?}", program.inserted);
}

#[test]
fn a_restart_forgets_the_report_and_keeps_the_next() {
    let mut program = Exiting::new(report());
    let (out, err) = session(
        &mut program,
        "run\nrestart\nshow report\nrun\nshow report\nq\n",
    );
    assert_eq!(err, "");
    assert_eq!(
        out[2..6],
        [
            NOT_YET,
            EXIT_WITH_REPORT,
            "mfb.debug.begin 1",
            "mfb.debug.end 1"
        ]
    );
    assert_eq!(program.terminal, "");
}

#[test]
fn a_step_that_ends_the_program_says_where_the_report_is() {
    let mut script = report();
    script.insert(4, Event::At(0, AFTER));
    let mut program = Exiting::new(script);
    let (out, err) = session(
        &mut program,
        &format!("bp {AFTER:#x}\nrun\nshow report\nstep 2\nq\n"),
    );
    assert_eq!(err, "");
    assert_eq!(
        out[2..6],
        [
            "mfb.debug.begin 1",
            "mfb.debug.end 1",
            "The program exited after 1 instruction.",
            "Type \"show report\" to see the performance and memory report.",
        ]
    );
}

#[test]
fn a_report_line_is_shown_safe_for_a_terminal() {
    let mut program = Exiting::new(vec![
        Event::Enter(0),
        Event::Write(0, 2, "key \u{1b}[2Jvalue\n"),
        Event::Return(0),
        Event::Exit(0),
    ]);
    let (out, _) = session(&mut program, "run\nshow report\nq\n");
    assert!(!out[1].contains('\u{1b}'), "{:?}", out[1]);
}
