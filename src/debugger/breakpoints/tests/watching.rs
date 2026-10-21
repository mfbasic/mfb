//! Running with watchpoints set over two threads, a restart that loses or
//! moves breakpoints, a pending breakpoint that binds onto one already there,
//! and a finish whose first `ret` is a deeper call's. Runs and steps answer
//! from scripts; each scripted step may write memory and move its thread.

use std::collections::VecDeque;

use super::*;

/// Where the watched memory starts: 16 bytes, two 8-byte watches.
const WATCHED: u64 = 0x8000;
const WORK: u64 = 0x2000;

/// One scripted step: how it ends, what it writes, where the thread goes
/// (`None`: the thread is gone afterwards).
struct Scripted {
    stop: StepStop,
    writes: Vec<(u64, u8)>,
    to: Option<u64>,
}

fn moves(to: u64) -> Scripted {
    Scripted {
        stop: StepStop::Finished,
        writes: Vec::new(),
        to: Some(to),
    }
}

struct Twin {
    /// Each live thread's pc and sp.
    threads: Vec<(u64, u64)>,
    memory: [u8; 16],
    runs: VecDeque<RunStop>,
    steps: VecDeque<Scripted>,
    inserted: Vec<u64>,
    functions: Vec<FunctionSymbol>,
    /// Thread 0's sp after each breakpoint run, in turn.
    sps: VecDeque<u64>,
}

impl Twin {
    fn new() -> Twin {
        Twin {
            threads: vec![(WORK, 0x7000), (WORK + 8, 0x6000)],
            memory: [0; 16],
            runs: VecDeque::new(),
            steps: VecDeque::new(),
            inserted: Vec::new(),
            functions: vec![
                function("_mfb_fn_work", false, Some(WORK), Some(0x10)),
                function("_mfb_fn_other", false, Some(0x3000), Some(0x10)),
            ],
            sps: VecDeque::new(),
        }
    }
}

impl Inferior for Twin {
    fn pid(&self) -> u32 {
        1
    }
    fn end(&mut self) -> Result<Ending, String> {
        Ok(Ending::Killed)
    }
    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        Ok(self.functions.clone())
    }
    fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn read_bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let at = address
            .checked_sub(WATCHED)
            .map(|at| at as usize)
            .filter(|at| at + length <= self.memory.len())
            .ok_or_else(|| format!("unmapped {address:#x}"))?;
        Ok(self.memory[at..at + length].to_vec())
    }
    /// `work` is `nop`, `nop`, `nop`, `ret`.
    fn read_code(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        if !(WORK..WORK + 0x10).contains(&address) {
            return Err(format!("unmapped {address:#x}"));
        }
        Ok((0..length as u64 / 4)
            .flat_map(|index| {
                let word: u32 = if address + index * 4 == WORK + 0xc {
                    0xd65f_03c0
                } else {
                    0xd503_201f
                };
                word.to_le_bytes()
            })
            .collect())
    }
    fn function_at(&mut self, address: u64) -> Result<Option<FunctionRange>, String> {
        Ok((WORK..WORK + 0x10)
            .contains(&address)
            .then(|| FunctionRange {
                image: "prog".to_string(),
                name: Some("_mfb_fn_work".to_string()),
                start: WORK,
                size: Some(0x10),
            }))
    }
    fn data_symbol_at(&mut self, _: u64) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
        Ok(Vec::new())
    }
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        Ok(self
            .threads
            .iter()
            .enumerate()
            .map(|(index, (pc, sp))| ThreadFacts {
                id: index as u64 + 1,
                handle: index as u64 + 1,
                name: None,
                pc: *pc,
                sp: *sp,
                arena: 0,
                stack: None,
            })
            .collect())
    }
    fn registers(&mut self, _: usize) -> Result<Vec<Register>, String> {
        Ok(Vec::new())
    }
    fn set_registers(&mut self, _: usize, _: &[(String, u64)]) -> Result<(), String> {
        Ok(())
    }
    fn step(&mut self, threads: &[usize], _: u64) -> Result<Vec<Step>, String> {
        let step = self.steps.pop_front().unwrap_or_else(|| moves(0));
        for (address, byte) in &step.writes {
            self.memory[(address - WATCHED) as usize] = *byte;
        }
        let thread = threads[0];
        match step.to {
            Some(to) => self.threads[thread].0 = to,
            None => {
                self.threads.remove(thread);
            }
        }
        Ok(vec![Step {
            taken: 1,
            stop: step.stop,
        }])
    }
    fn restart(&mut self, _: &[(std::ffi::OsString, std::ffi::OsString)]) -> Result<u32, String> {
        Ok(2)
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
    fn run(&mut self) -> Result<RunStop, String> {
        let stop = self
            .runs
            .pop_front()
            .unwrap_or(RunStop::Exited(Exit::Status(0)));
        if let RunStop::Breakpoints(hits) = &stop {
            for (thread, address) in hits {
                self.threads[*thread].0 = *address;
            }
            if let Some(sp) = self.sps.pop_front() {
                self.threads[0].1 = sp;
            }
        }
        Ok(stop)
    }
}

/// Two watchpoints, 8 bytes each, at `WATCHED` and `WATCHED + 8`.
fn watching(twin: &mut Twin) -> Breakpoints {
    let mut breakpoints = Breakpoints::default();
    breakpoints.watch(twin, WATCHED, 8, &[]).unwrap();
    breakpoints.watch(twin, WATCHED + 8, 8, &[]).unwrap();
    breakpoints
}

fn heard(breakpoints: &mut Breakpoints, twin: &mut Twin) -> RunStop {
    run_twin(breakpoints, twin).unwrap().1
}

/// Run until the first stop heard: the notes heard with it, and the stop.
fn run_twin(
    breakpoints: &mut Breakpoints,
    twin: &mut Twin,
) -> Result<(Vec<String>, RunStop), String> {
    let mut heard = None;
    let ended = breakpoints.run(
        twin,
        &mut |_: &mut dyn Inferior, _: &Breakpoints, notes, stop: &RunStop| {
            heard = Some((notes, stop.clone()));
            Ok(false)
        },
    )?;
    Ok(match ended {
        Ended::Heard(_) => heard.expect("a heard stop"),
        Ended::Own(hits) => (breakpoints.take_notes(), RunStop::Breakpoints(hits)),
    })
}

#[test]
fn a_write_counts_against_only_the_watchpoint_it_changed() {
    let mut twin = Twin::new();
    let mut breakpoints = watching(&mut twin);
    twin.runs.push_back(RunStop::Watchpoints(vec![0]));
    twin.steps.push_back(Scripted {
        writes: vec![(WATCHED, 7)],
        ..moves(WORK + 4)
    });
    assert_eq!(
        heard(&mut breakpoints, &mut twin),
        RunStop::Changed(vec![Change {
            thread: 0,
            pc: WORK,
            address: WATCHED,
            size: 8,
            old: 0,
            new: 7,
        }])
    );
    assert_eq!(
        breakpoints.watch_lines(&mut twin)[1..],
        [
            "0x0000000000008000 | 1     | 8    | 0x0000000000000007 (7)",
            "0x0000000000008008 | 0     | 8    | 0x0000000000000000 (0)",
        ]
    );
}

#[test]
fn a_watched_step_that_faults_or_ends_the_program_is_that_stop() {
    let mut twin = Twin::new();
    let mut breakpoints = watching(&mut twin);
    twin.runs.push_back(RunStop::Watchpoints(vec![0]));
    twin.steps.push_back(Scripted {
        stop: StepStop::Exception("EXC_BAD_ACCESS".to_string()),
        ..moves(WORK)
    });
    assert_eq!(
        heard(&mut breakpoints, &mut twin),
        RunStop::Exception {
            thread: 0,
            what: "EXC_BAD_ACCESS".to_string()
        }
    );
    twin.runs.push_back(RunStop::Watchpoints(vec![0]));
    twin.steps.push_back(Scripted {
        stop: StepStop::Exited,
        ..moves(WORK)
    });
    // The ended program answers how it ended.
    assert_eq!(
        heard(&mut breakpoints, &mut twin),
        RunStop::Exited(Exit::Status(0))
    );
}

#[test]
fn a_watched_stop_for_a_thread_that_is_gone_or_goes_changes_nothing() {
    let mut twin = Twin::new();
    let mut breakpoints = watching(&mut twin);
    // Thread 5 does not exist; thread 1 ends during its step. Neither wrote,
    // so the run goes on to the program's exit.
    twin.runs.push_back(RunStop::Watchpoints(vec![5]));
    twin.runs.push_back(RunStop::Watchpoints(vec![1]));
    twin.steps.push_back(Scripted {
        to: None,
        ..moves(0)
    });
    assert_eq!(
        heard(&mut breakpoints, &mut twin),
        RunStop::Exited(Exit::Status(0))
    );
    assert_eq!(twin.threads.len(), 1);
}

#[test]
fn a_thread_stepped_off_one_breakpoint_onto_another_stops_there() {
    let mut twin = Twin::new();
    let mut breakpoints = watching(&mut twin);
    breakpoints
        .set(&mut twin, &Target::Address(WORK), None, &[])
        .unwrap();
    breakpoints
        .set(&mut twin, &Target::Address(WORK + 4), None, &[])
        .unwrap();
    // Thread 0 sits on the breakpoint at WORK; with watchpoints set it is
    // stepped off it first, and lands on the one at WORK + 4.
    twin.steps.push_back(moves(WORK + 4));
    assert_eq!(
        heard(&mut breakpoints, &mut twin),
        RunStop::Breakpoints(vec![(0, WORK + 4)])
    );
}

#[test]
fn a_restart_drops_or_defers_breakpoints_the_new_process_cannot_place() {
    let mut twin = Twin::new();
    let free = format!("{}free", crate::debugger::names::C_PREFIX);
    twin.functions
        .push(function(&free, true, Some(0x9000), None));
    let mut breakpoints = Breakpoints::default();
    // An address no function holds, a linked function, a function the
    // rebuilt program loses, and two tests on one place.
    for target in ["0x5000", &free, "_mfb_fn_other", "work"] {
        breakpoints
            .set(&mut twin, &name(target), None, &[])
            .unwrap();
    }
    let tested = test("x0", "eq", Some("1"));
    breakpoints
        .set(&mut twin, &name("work"), Some(&tested), &[])
        .unwrap();
    twin.functions
        .retain(|function| function.name != "_mfb_fn_other");
    twin.inserted.clear();
    assert_eq!(
        breakpoints.restarted(&mut twin).unwrap(),
        [
            "Dropped the breakpoint at 0x0000000000005000: no function holds it.",
            "Dropped the breakpoint on _mfb_fn_other: the program no longer has it.",
        ]
    );
    // `work` is placed once for both of its breakpoints; `free` waits for
    // its library.
    assert_eq!(twin.inserted, [WORK]);
    assert_eq!(
        breakpoints.lines(),
        [
            "Address            | Count | Symbol       | Written name | If".to_string(),
            "0x0000000000002000 | 0     | _mfb_fn_work | work         | -".to_string(),
            "0x0000000000002000 | 0     | _mfb_fn_work | work         | x0 eq 0x0000000000000001 (1)"
                .to_string(),
            format!("-                  | 0     | {free:<12} | free         | -"),
        ]
    );
}

#[test]
fn a_pending_breakpoint_that_binds_where_one_already_is_folds_into_it() {
    let mut twin = Twin::new();
    let free = format!("{}free", crate::debugger::names::C_PREFIX);
    twin.functions.push(function(&free, true, None, None));
    let mut breakpoints = Breakpoints::default();
    breakpoints.set(&mut twin, &name(&free), None, &[]).unwrap();
    breakpoints
        .set(&mut twin, &Target::Address(0x9000), None, &[])
        .unwrap();
    // The library loads, and `free` binds at 0x9000.
    twin.functions.retain(|function| function.name != free);
    twin.functions
        .push(function(&free, true, Some(0x9000), None));
    let (notes, stop) = run_twin(&mut breakpoints, &mut twin).unwrap();
    assert_eq!(stop, RunStop::Exited(Exit::Status(0)));
    assert_eq!(
        notes,
        [format!(
            "The pending breakpoint on {free} is at 0x0000000000009000, which already has one."
        )]
    );
}

#[test]
fn a_finish_waits_past_a_deeper_call_returning_through_the_same_ret() {
    let mut twin = Twin::new();
    twin.threads[0] = (WORK + 4, 0x7000);
    let mut breakpoints = Breakpoints::default();
    let ret = WORK + 0xc;
    // First a recursive call returns through the ret (its sp below the
    // finishing frame's), then the finishing frame itself.
    twin.runs.push_back(RunStop::Breakpoints(vec![(0, ret)]));
    twin.sps.push_back(0x6f00);
    twin.runs.push_back(RunStop::Breakpoints(vec![(0, ret)]));
    twin.sps.push_back(0x7010);
    let finish = breakpoints.finish(
        &mut twin,
        0,
        &mut |_: &mut dyn Inferior, _: &Breakpoints, _, _: &RunStop| Ok(false),
    );
    assert_eq!(
        finish,
        Ok(Finish::Returned {
            from: Some("_mfb_fn_work".to_string())
        })
    );
    assert!(twin.runs.is_empty());
    assert!(twin.inserted.is_empty());
}
