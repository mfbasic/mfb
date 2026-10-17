//! The debug report of a `--debug` build, kept from the terminal for
//! `show report`.
//!
//! A `--debug` build writes its report to stderr as the last thing it does
//! (`_mfb_debug_shutdown`): one `write` to fd 2 per line. The session keeps a
//! breakpoint of its own on that function. A thread entering it opens a
//! window: a breakpoint where the function returns to, and one on each `write`
//! the program links. Inside the window a `write` to fd 2 by that thread is a
//! report line: its bytes are read from the program and kept, and the call is
//! skipped — the thread goes on from where `write` would return, with the
//! length it asked for as the result — so the line never reaches the terminal.
//! The thread's return closes the window and takes its breakpoints away.
//!
//! Only a `run` sees these breakpoints: a `step` runs the instructions they
//! replaced. So a window still open when the program stops for the user is
//! closed there, and the program writes the rest of the report itself; what
//! was kept until then is marked incomplete, as is a report the program ended
//! inside.

use super::Breakpoints;
use crate::codegen::debug::DEBUG_SHUTDOWN_SYMBOL;
use crate::debugger::names::linked_name;
use crate::debugger::{Inferior, Register, RegisterValue};
use crate::terminal_safe::safe;

/// The file descriptor the report is written to.
const STDERR: u64 = 2;

/// The C function each report line is written with.
const WRITE: &str = "write";

/// The most bytes one `write` may ask for and still be taken for a report
/// line; the runtime builds a line in a 128-byte window.
const LINE_LIMIT: u64 = 4096;

/// What ends the `--[EX]` line and follows a step's, when a report was kept.
pub(crate) const REPORT_HINT: &str =
    "type \"show report\" to see the performance and memory report";

const NOT_A_DEBUG_BUILD: &str =
    "No performance and memory report: the program was not built with --debug.";
const NOT_YET: &str = "No performance and memory report yet: the program writes it as it exits.";
const NONE_KEPT: &str =
    "No performance and memory report: the program ended without one being kept.";
const INCOMPLETE: &str = "The report is incomplete: the program stopped or ended while writing it.";

/// The session's hold on the program's debug report.
#[derive(Debug, Default)]
pub(super) struct Report {
    /// The live address of `_mfb_debug_shutdown`, where the debugger keeps a
    /// breakpoint of its own; `None` in a build without `--debug`.
    shutdown: Option<u64>,
    /// The report being written now.
    window: Option<Window>,
    /// The report the program wrote.
    kept: Option<Kept>,
}

/// One thread's pass through `_mfb_debug_shutdown`.
#[derive(Debug)]
struct Window {
    /// The system's identifier for the thread writing the report.
    thread: u64,
    /// Where `_mfb_debug_shutdown` returns to.
    back: u64,
    /// The live address of each `write` the program links.
    writes: Vec<u64>,
    /// The lines taken so far.
    text: Vec<u8>,
}

#[derive(Debug)]
struct Kept {
    text: Vec<u8>,
    /// The thread returned from `_mfb_debug_shutdown` with the window open.
    complete: bool,
}

impl Report {
    /// Whether one of the report's own breakpoints is at `address`.
    pub(super) fn holds(&self, address: u64) -> bool {
        self.shutdown == Some(address)
            || self
                .window
                .as_ref()
                .is_some_and(|window| window.back == address || window.writes.contains(&address))
    }
}

/// The 64-bit register `name` among thread `thread`'s `registers`.
fn register(registers: &[Register], thread: usize, name: &str) -> Result<u64, String> {
    registers
        .iter()
        .find(|register| register.name == name)
        .and_then(|register| match register.value {
            RegisterValue::Bits64(value) => Some(value),
            RegisterValue::Bits128(_) => None,
        })
        .ok_or_else(|| format!("thread {thread} has no register {name}"))
}

/// The system's identifier for thread `thread`, if there is such a thread.
fn thread_id(inferior: &mut dyn Inferior, thread: usize) -> Result<Option<u64>, String> {
    Ok(inferior.threads()?.get(thread).map(|facts| facts.id))
}

impl Breakpoints {
    /// Start keeping the program's debug report: forget any kept so far, and
    /// put a breakpoint of the debugger's own on `_mfb_debug_shutdown`, which
    /// only a `--debug` build has. `placed` is every address that holds an
    /// instruction of the session's already; the report's joins it.
    pub(crate) fn watch_report(
        &mut self,
        inferior: &mut dyn Inferior,
        placed: &mut Vec<u64>,
    ) -> Result<(), String> {
        self.report = Report::default();
        let shutdown = inferior
            .functions()
            .unwrap_or_default()
            .iter()
            .find(|function| function.name == DEBUG_SHUTDOWN_SYMBOL && !function.linked)
            .and_then(|function| function.address);
        if let Some(address) = shutdown {
            if !placed.contains(&address) {
                inferior.insert_breakpoint(address)?;
                placed.push(address);
            }
        }
        self.report.shutdown = shutdown;
        Ok(())
    }

    /// Whether a report was kept.
    pub(crate) fn has_report(&self) -> bool {
        self.report.kept.is_some()
    }

    /// `show report`: the report the program wrote, a line each as it wrote
    /// them, or why there is none; `exited` once the program has ended.
    pub(crate) fn report_lines(&self, exited: bool) -> Vec<String> {
        let Some(kept) = &self.report.kept else {
            let none = if self.report.shutdown.is_none() {
                NOT_A_DEBUG_BUILD
            } else if exited {
                NONE_KEPT
            } else {
                NOT_YET
            };
            return vec![none.to_string()];
        };
        let mut lines = String::from_utf8_lossy(&kept.text)
            .lines()
            .map(|line| safe(line).into_owned())
            .collect::<Vec<_>>();
        if !kept.complete {
            lines.push(INCOMPLETE.to_string());
        }
        lines
    }

    /// Look at breakpoint hits that stop for none of the user's breakpoints:
    /// a thread entering `_mfb_debug_shutdown` opens the window, the thread
    /// that opened it writing a line has the line taken, and its return closes
    /// the window.
    pub(super) fn report_hits(
        &mut self,
        inferior: &mut dyn Inferior,
        hits: &[(usize, u64)],
    ) -> Result<(), String> {
        for &(thread, address) in hits {
            if !self.report.holds(address) {
                continue;
            }
            let Some(id) = thread_id(inferior, thread)? else {
                continue;
            };
            match &self.report.window {
                None => self.open_report(inferior, thread, id)?,
                Some(window) if window.thread != id => {}
                Some(window) if window.back == address => self.close_report(inferior, true)?,
                Some(window) if window.writes.contains(&address) => {
                    self.take_line(inferior, thread)?
                }
                // The thread writing the report, entering it again.
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Thread `thread` (the system's `id`) has just entered
    /// `_mfb_debug_shutdown`: open the window, when the program links a
    /// `write` to stop at.
    fn open_report(
        &mut self,
        inferior: &mut dyn Inferior,
        thread: usize,
        id: u64,
    ) -> Result<(), String> {
        let mut writes = Vec::new();
        for function in inferior.functions()? {
            if let (true, Some(address)) = (function.linked, function.address) {
                if linked_name(&function.name) == WRITE && !writes.contains(&address) {
                    writes.push(address);
                }
            }
        }
        if writes.is_empty() {
            return Ok(());
        }
        let back = register(&inferior.registers(thread)?, thread, "lr")?;
        for address in writes.iter().chain([&back]) {
            if !self.armed(*address) {
                inferior.insert_breakpoint(*address)?;
            }
        }
        self.report.window = Some(Window {
            thread: id,
            back,
            writes,
            text: Vec::new(),
        });
        Ok(())
    }

    /// The thread writing the report is at `write`'s first instruction: when
    /// it writes to stderr, keep the bytes and skip the call.
    fn take_line(&mut self, inferior: &mut dyn Inferior, thread: usize) -> Result<(), String> {
        let registers = inferior.registers(thread)?;
        let value = |name| register(&registers, thread, name);
        let length = value("x2")?;
        if value("x0")? != STDERR || length > LINE_LIMIT {
            return Ok(());
        }
        let bytes = match length {
            0 => Vec::new(),
            _ => inferior.read_bytes(value("x1")?, length as usize)?,
        };
        let back = value("lr")?;
        inferior.set_registers(
            thread,
            &[("x0".to_string(), length), ("pc".to_string(), back)],
        )?;
        if let Some(window) = &mut self.report.window {
            window.text.extend(bytes);
        }
        Ok(())
    }

    /// Close the window, if one is open: take its breakpoints away, and keep
    /// what it took when no report is kept yet. (`_mfb_debug_shutdown` writes
    /// once: a second pass through it — a signal during the exit — takes
    /// nothing, and does not replace the report.)
    pub(super) fn close_report(
        &mut self,
        inferior: &mut dyn Inferior,
        complete: bool,
    ) -> Result<(), String> {
        let Some(window) = self.report.window.take() else {
            return Ok(());
        };
        for address in window.writes.iter().chain([&window.back]) {
            if !self.armed(*address) {
                inferior.remove_breakpoint(*address)?;
            }
        }
        self.keep(window.text, complete);
        Ok(())
    }

    /// The program has ended: a window still open took all it will.
    pub(super) fn report_ended(&mut self) {
        if let Some(window) = self.report.window.take() {
            self.keep(window.text, false);
        }
    }

    fn keep(&mut self, text: Vec<u8>, complete: bool) {
        if self.report.kept.is_none() && !text.is_empty() {
            self.report.kept = Some(Kept { text, complete });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        fn end(&mut self) -> Result<Ending, String> {
            Ok(Ending::AlreadyExited)
        }
        fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
            let mut functions = vec![
                function("_main", false, 0x2000),
                function(&format!("{C_PREFIX}write"), true, WRITE),
            ];
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
        fn restart(
            &mut self,
            _: &[(std::ffi::OsString, std::ffi::OsString)],
        ) -> Result<u32, String> {
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
}
