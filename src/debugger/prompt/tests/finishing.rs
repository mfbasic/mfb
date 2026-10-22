//! `finish` at the prompt, and the stops a run can make on the way: the
//! thread returns, another breakpoint or an exception ends the finish early,
//! or the program ends. The process is one thread in `work`, called from
//! `_main`; its runs and steps answer from scripts.

use std::collections::VecDeque;
use std::ffi::OsString;

use super::*;
use crate::debugger::{Ending, FunctionRange, Step};

const NOP: u32 = 0xd503_201f;
const RET: u32 = 0xd65f_03c0;
/// `work`'s `ret`.
const WORK_RET: u64 = 0x200c;
/// Where `work` returns to in `_main`.
const RETURN: u64 = 0x1008;

/// Each function: its range and code.
fn ranges() -> Vec<(FunctionRange, Vec<u32>)> {
    let range = |image: &str, name: Option<&str>, start, words: &[u32]| {
        (
            FunctionRange {
                image: image.to_string(),
                name: name.map(str::to_string),
                start,
                size: Some(words.len() as u64 * 4),
            },
            words.to_vec(),
        )
    };
    vec![
        range("prog", Some("_main"), 0x1000, &[NOP, NOP, NOP, RET]),
        range("prog", Some("_mfb_fn_work"), 0x2000, &[NOP, NOP, NOP, RET]),
        // A library's function the executable has no symbol for.
        range("libfoo.dylib", None, 0x3000, &[NOP, RET]),
        range(
            "prog",
            Some("symbol stub for: _malloc"),
            0x4000,
            &[NOP, RET],
        ),
        // A function with no `ret` at all.
        range("prog", Some("_mfb_fn_spin"), 0x5000, &[NOP, NOP]),
    ]
}

struct Stack {
    pc: u64,
    sp: u64,
    inserted: Vec<u64>,
    runs: VecDeque<RunStop>,
    steps: VecDeque<StepStop>,
    /// When set, the thread is gone once it has stepped: `threads` lists none.
    vanishes: bool,
    stepped: bool,
}

impl Stack {
    fn at(pc: u64) -> Stack {
        Stack {
            pc,
            sp: 0x8000,
            inserted: Vec::new(),
            runs: VecDeque::new(),
            steps: VecDeque::new(),
            vanishes: false,
            stepped: false,
        }
    }
}

impl Inferior for Stack {
    fn pid(&self) -> u32 {
        42
    }
    fn end(&mut self) -> Result<Ending, String> {
        Ok(Ending::Killed)
    }
    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        Ok(ranges()
            .into_iter()
            .filter(|(range, _)| {
                range.image == "prog" && !range.name.as_ref().unwrap().contains(' ')
            })
            .map(|(range, _)| {
                symbol(
                    range.name.as_ref().unwrap(),
                    false,
                    Some(range.start),
                    range.size,
                )
            })
            .collect())
    }
    fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        Err(format!("unmapped {address:#x}"))
    }
    /// Zeros at 0x9000..0x9100, and nothing else mapped.
    fn read_bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        if (0x9000..0x9100).contains(&address) {
            return Ok(vec![0; length]);
        }
        Err(format!("unmapped {address:#x}"))
    }
    fn read_code(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let (range, words) = ranges()
            .into_iter()
            .find(|(range, words)| {
                range.start <= address && address < range.start + words.len() as u64 * 4
            })
            .ok_or_else(|| format!("unmapped {address:#x}"))?;
        let first = ((address - range.start) / 4) as usize;
        words
            .get(first..first + length / 4)
            .map(|words| words.iter().flat_map(|word| word.to_le_bytes()).collect())
            .ok_or_else(|| format!("unmapped {address:#x}"))
    }
    fn function_at(&mut self, address: u64) -> Result<Option<FunctionRange>, String> {
        Ok(ranges()
            .into_iter()
            .find(|(range, words)| {
                range.start <= address && address < range.start + words.len() as u64 * 4
            })
            .map(|(range, _)| range))
    }
    fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
        Ok(Vec::new())
    }
    fn data_symbol_at(&mut self, _: u64) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        if self.vanishes && self.stepped {
            return Ok(Vec::new());
        }
        Ok(vec![ThreadFacts {
            id: 0x501,
            handle: 0x501,
            name: None,
            pc: self.pc,
            sp: self.sp,
            arena: 0,
            stack: None,
        }])
    }
    fn registers(&mut self, _: usize) -> Result<Vec<Register>, String> {
        let register = |name: &str, value| Register {
            name: name.to_string(),
            group: RegisterGroup::General,
            value: RegisterValue::Bits64(value),
            role: None,
        };
        Ok(vec![
            register("x0", 0),
            register("x1", 42),
            register("sp", self.sp),
            register("pc", self.pc),
        ])
    }
    fn set_registers(&mut self, _: usize, _: &[(String, u64)]) -> Result<(), String> {
        Err("read-only".to_string())
    }
    /// The scripted stop; a finished step over `work`'s `ret` returns to
    /// `_main` with the frame popped.
    fn step(&mut self, threads: &[usize], count: u64) -> Result<Vec<Step>, String> {
        self.stepped = true;
        let stop = self.steps.pop_front().unwrap_or(StepStop::Finished);
        if stop == StepStop::Finished && self.pc == WORK_RET {
            self.pc = RETURN;
            self.sp += 0x10;
        }
        Ok(threads
            .iter()
            .map(|_| Step {
                taken: count,
                stop: stop.clone(),
            })
            .collect())
    }
    fn restart(&mut self, _: &[(OsString, OsString)]) -> Result<u32, String> {
        self.pc = 0x1000;
        Ok(43)
    }
    fn entry(&mut self) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn insert_breakpoint(&mut self, address: u64) -> Result<(), String> {
        self.inserted.push(address);
        Ok(())
    }
    fn remove_breakpoint(&mut self, address: u64) -> Result<(), String> {
        self.inserted.retain(|other| *other != address);
        Ok(())
    }
    fn insert_watchpoint(&mut self, _: u64, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn remove_watchpoint(&mut self, _: u64) -> Result<(), String> {
        Ok(())
    }
    /// The next scripted stop, the thread moved to the breakpoint it hit;
    /// once the script runs out, the program has exited.
    fn run(&mut self) -> Result<RunStop, String> {
        let stop = self
            .runs
            .pop_front()
            .unwrap_or(RunStop::Exited(Exit::Status(0)));
        if let RunStop::Breakpoints(hits) = &stop {
            self.pc = hits[0].1;
        }
        Ok(stop)
    }
}

fn session_on(stack: &mut Stack, input: &str) -> (String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    run(
        stack,
        CONSOLE,
        None,
        &mut input.as_bytes(),
        &mut out,
        &mut err,
        false,
    )
    .expect("session runs");
    (
        String::from_utf8(out).unwrap().replace(PROMPT, ""),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn finish_runs_to_the_return_and_shows_what_came_back() {
    let mut stack = Stack::at(0x2004);
    stack
        .runs
        .push_back(RunStop::Breakpoints(vec![(0, WORK_RET)]));
    let (out, err) = session_on(&mut stack, "finish\nq\n");
    assert_eq!(err, "");
    assert_eq!(
        out,
        "Thread 0 returned from work to 0x0000000000001008 in _main+0x8: x0 = \
         0x0000000000000000 (0), x1 = 0x000000000000002a (42).\n\
         Killed process 42.\n"
    );
    // The finish's own breakpoint is gone again.
    assert!(stack.inserted.is_empty(), "{:?}", stack.inserted);
}

#[test]
fn a_breakpoint_on_the_way_ends_the_finish_early() {
    let mut stack = Stack::at(0x2000);
    stack
        .runs
        .push_back(RunStop::Breakpoints(vec![(0, 0x2008)]));
    let (out, err) = session_on(&mut stack, "bp 0x2008\nfinish\nq\n");
    assert_eq!(err, "");
    assert_eq!(
        out.lines().skip(1).collect::<Vec<_>>(),
        [
            "--[BP] T: 0 P: 0x0000000000002008 F: (work) _mfb_fn_work+0x8",
            "The finish from work did not complete; type 'finish' to run it again from here.",
            "Killed process 42.",
        ]
    );
}

#[test]
fn a_finish_the_program_ends_during_says_only_that_it_ended() {
    let mut stack = Stack::at(0x2004);
    let (out, err) = session_on(&mut stack, "finish\nfinish\nq\n");
    assert_eq!(out, "--[EX] Program exit (0)\nKilled process 42.\n");
    assert_eq!(
        err,
        "error: the program has exited; type 'quit' to end the session\n"
    );
}

#[test]
fn the_step_over_the_return_can_fault_wait_or_end() {
    let cases = [
        (
            StepStop::Exception("EXC_BAD_ACCESS".to_string()),
            "--[XC] T: 0 P: 0x000000000000200c F: (work) _mfb_fn_work+0xc X: EXC_BAD_ACCESS\n\
             The finish from work did not complete; type 'finish' to run it again from here.\n",
        ),
        (
            StepStop::Waiting,
            "--[CC] User Break\n\
             The finish from work did not complete; type 'finish' to run it again from here.\n",
        ),
        (StepStop::Exited, "--[EX] Program exit\n"),
    ];
    for (stop, shown) in cases {
        let mut stack = Stack::at(0x2004);
        stack
            .runs
            .push_back(RunStop::Breakpoints(vec![(0, WORK_RET)]));
        stack.steps.push_back(stop);
        let (out, err) = session_on(&mut stack, "finish\nq\n");
        assert_eq!(err, "");
        assert_eq!(out, format!("{shown}Killed process 42.\n"));
    }
}

#[test]
fn a_finish_needs_a_function_with_a_ret() {
    let (_, err) = session_on(&mut Stack::at(0x7000), "finish\nq\n");
    assert_eq!(
        err,
        "error: cannot finish: no known function holds 0x0000000000007000\n"
    );
    let (_, err) = session_on(&mut Stack::at(0x5000), "finish\nq\n");
    assert_eq!(
        err,
        "error: cannot finish: no ret instruction found in the function at \
         0x0000000000005000\n"
    );
}

#[test]
fn a_finish_from_a_function_no_symbol_names_says_the_finish() {
    let mut stack = Stack::at(0x3000);
    stack.runs.push_back(RunStop::Exception {
        thread: 0,
        what: "EXC_BREAKPOINT".to_string(),
    });
    let (out, _) = session_on(&mut stack, "finish\nq\n");
    assert_eq!(
        out,
        "--[XC] T: 0 P: 0x0000000000003000 F: - X: EXC_BREAKPOINT\n\
         The finish did not complete; type 'finish' to run it again from here.\n\
         Killed process 42.\n"
    );
}

#[test]
fn a_stop_in_a_stub_or_on_a_thread_that_is_gone_is_placed_as_well_as_it_can_be() {
    let mut stack = Stack::at(0x4004);
    stack.runs.push_back(RunStop::Exception {
        thread: 0,
        what: "EXC_BAD_ACCESS".to_string(),
    });
    stack.runs.push_back(RunStop::Exception {
        thread: 3,
        what: "EXC_CRASH".to_string(),
    });
    let (out, _) = session_on(&mut stack, "run\nrun\nq\n");
    assert_eq!(
        out,
        "--[XC] T: 0 P: 0x0000000000004004 F: symbol stub for: _malloc+0x4 X: EXC_BAD_ACCESS\n\
         --[XC] T: - P: - F: - X: EXC_CRASH\n\
         Killed process 42.\n"
    );
}

#[test]
fn a_stop_whose_commands_finish_queues_them_and_runs_on_after() {
    let mut stack = Stack::at(0x2000);
    stack
        .runs
        .push_back(RunStop::Breakpoints(vec![(0, 0x2004)]));
    stack
        .runs
        .push_back(RunStop::Breakpoints(vec![(0, WORK_RET)]));
    let (out, err) = session_on(&mut stack, "bp 0x2004 finish; r\nr\nq\n");
    assert_eq!(err, "");
    assert_eq!(
        out.lines().skip(1).collect::<Vec<_>>(),
        [
            "--[BP] T: 0 P: 0x0000000000002004 F: (work) _mfb_fn_work+0x4",
            "Thread 0 returned from work to 0x0000000000001008 in _main+0x8: x0 = \
             0x0000000000000000 (0), x1 = 0x000000000000002a (42).",
            "--[EX] Program exit (0)",
            "Killed process 42.",
        ]
    );
}

#[test]
fn a_stop_can_show_the_report_and_the_prompt_sets_watchpoints() {
    let mut stack = Stack::at(0x2000);
    stack
        .runs
        .push_back(RunStop::Breakpoints(vec![(0, 0x2004)]));
    let (out, err) = session_on(
        &mut stack,
        "wp 0x9000 8\nwp r 0x9000\nwp r 0x9000\nbp 0x2004 show report\nr\nrestart\nq\n",
    );
    assert_eq!(err, "error: there is no watchpoint at 0x0000000000009000\n");
    assert_eq!(
        out,
        "Set a watchpoint at 0x0000000000009000 (8 bytes): 0x0000000000000000 (0).\n\
         Removed the watchpoint at 0x0000000000009000 (8 bytes).\n\
         Set a breakpoint at 0x0000000000002004 in work+0x4. Each stop runs 'show report'.\n\
         --[BP] T: 0 P: 0x0000000000002004 F: (work) _mfb_fn_work+0x4\n\
         No performance and memory report: the program was not built with --debug.\n\
         Restarted the program as process 43; paused before its first instruction.\n\
         Killed process 42.\n"
    );
}

#[test]
fn a_stop_whose_commands_finish_without_run_waits_at_the_prompt_after() {
    let mut stack = Stack::at(0x2000);
    stack
        .runs
        .push_back(RunStop::Breakpoints(vec![(0, 0x2004)]));
    stack
        .runs
        .push_back(RunStop::Breakpoints(vec![(0, WORK_RET)]));
    let (out, err) = session_on(&mut stack, "bp 0x2004 finish\nr\nlist thread\nq\n");
    assert_eq!(err, "");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[1..3],
        [
            "--[BP] T: 0 P: 0x0000000000002004 F: (work) _mfb_fn_work+0x4",
            "Thread 0 returned from work to 0x0000000000001008 in _main+0x8: x0 = \
             0x0000000000000000 (0), x1 = 0x000000000000002a (42).",
        ]
    );
    // Not run on: the thread is still where the finish left it.
    assert_eq!(lines[4], "0  | _main+0x8 | 0x0000000000001008 | MFB Main");
}

/// Output that takes the first write (the prompt) and refuses the rest, as
/// a closed pipe does.
struct Closing {
    writes: usize,
}

impl std::io::Write for Closing {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.writes += 1;
        if self.writes > 1 {
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_session_whose_output_closes_stops_with_why() {
    let mut out = Closing { writes: 0 };
    let result = run(
        &mut Stack::at(0x2000),
        CONSOLE,
        None,
        &mut "help\n".as_bytes(),
        &mut out,
        &mut Vec::new(),
        true,
    );
    assert_eq!(result, Err("cannot write: broken pipe".to_string()));
}

/// A finish from a function no symbol names says it returned from "the
/// function"; one whose thread is gone after the step over the `ret` says it
/// returned to "an unknown place".
#[test]
fn a_finish_names_an_unnamed_function_and_a_vanished_thread_as_well_as_it_can() {
    let mut stack = Stack::at(0x3000);
    stack
        .runs
        .push_back(RunStop::Breakpoints(vec![(0, 0x3004)]));
    let (out, err) = session_on(&mut stack, "finish\nq\n");
    assert_eq!(err, "");
    assert!(
        out.starts_with("Thread 0 returned from the function to 0x0000000000003004"),
        "{out}"
    );

    let mut stack = Stack::at(0x2004);
    stack.vanishes = true;
    stack
        .runs
        .push_back(RunStop::Breakpoints(vec![(0, WORK_RET)]));
    let (out, err) = session_on(&mut stack, "finish\nq\n");
    assert_eq!(err, "");
    assert!(
        out.starts_with(
            "Thread 0 returned from work to an unknown place: x0 = 0x0000000000000000 (0), \
             x1 = 0x000000000000002a (42).\n"
        ),
        "{out}"
    );
}

/// A stop heard while the output is closed: writing its line fails, the `run`
/// answers that as its error (on the error stream, which is still open), and
/// the session ends at the next prompt with why. Without echo, the first write
/// after the prompt is the stop's own line.
#[test]
fn a_stop_written_to_a_closed_output_is_the_run_s_error() {
    let mut stack = Stack::at(0x2000);
    stack.runs.push_back(RunStop::Exception {
        thread: 0,
        what: "EXC_BAD_ACCESS".to_string(),
    });
    let mut err = Vec::new();
    let result = run(
        &mut stack,
        CONSOLE,
        None,
        &mut "run\n".as_bytes(),
        &mut Closing { writes: 0 },
        &mut err,
        false,
    );
    assert_eq!(
        String::from_utf8(err).unwrap(),
        "error: cannot write: broken pipe\n"
    );
    assert_eq!(
        result,
        Err("cannot write the prompt: broken pipe".to_string())
    );
}

/// A watchpoint stop that reaches the printer as the raw `Watchpoints` an
/// inferior answers (the session turns it into `Changed` first) names each
/// thread it stopped, without a change.
#[test]
fn a_raw_watchpoint_stop_prints_each_thread_it_stopped() {
    let mut stack = Stack::at(0x2008);
    assert_eq!(
        stop_lines(&mut stack, &RunStop::Watchpoints(vec![0, 3]), false),
        [
            "--[WP] T: 0 P: 0x0000000000002008 F: (work) _mfb_fn_work+0x8",
            "--[WP] T: - P: - F: -",
        ]
    );
}
