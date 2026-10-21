use super::*;
use crate::debugger::{Ending, Exit, FunctionRange, Register, Step, ThreadFacts};

/// A program whose `_exit` binds when it reaches its entry point, and
/// which then stops at whatever breakpoints `stops` lists, in turn.
struct Program {
    bound: bool,
    inserted: Vec<u64>,
    stops: Vec<RunStop>,
    runs: usize,
    /// The code in `x1` at each entry to the park, in turn.
    raised: Vec<u64>,
    /// The program's memory from [`MEMORY`].
    memory: Vec<u8>,
    /// What each step writes into memory, in turn: (address, byte).
    writes: Vec<Vec<(u64, u8)>>,
    /// Each watchpoint inserted: address, size.
    watched: Vec<(u64, u64)>,
    /// Thread 0's pc.
    pc: u64,
    /// `isPrime` is readable code: `nop`s, with a `ret` at [`RET`].
    code: bool,
    /// Thread 0's `x0`, and the value it takes at each run, in turn.
    x0: u64,
    x0s: Vec<u64>,
}

/// Where [`Program`]'s memory starts.
const MEMORY: u64 = 0x8000;

const PARK: u64 = 0x2000;
/// Where the park returns to, in the function that raised the error.
const RAISER: u64 = 0x1124;

const ENTRY: u64 = 0x1000;
/// A store in `isPrime`.
const STORE: u64 = 0x1110;
const IS_PRIME: u64 = 0x1100;
const EXIT: u64 = 0x9000;
/// Where the linked `abort` binds.
const ABORT: u64 = 0x9200;

/// The C library's `exit` and `abort` as the host's format spells their
/// symbols (`_exit` on Mach-O, `exit` on ELF).
fn exit() -> String {
    format!("{}exit", crate::debugger::names::C_PREFIX)
}
fn abort() -> String {
    format!("{}abort", crate::debugger::names::C_PREFIX)
}
/// `isPrime`'s `ret`, when the program has [`Program::code`].
const RET: u64 = 0x1130;

fn function(name: &str, linked: bool, address: Option<u64>, size: Option<u64>) -> FunctionSymbol {
    FunctionSymbol {
        name: name.to_string(),
        linked,
        loaded: false,
        address,
        size,
    }
}

impl Program {
    fn new(stops: Vec<RunStop>) -> Self {
        Program {
            bound: false,
            inserted: Vec::new(),
            stops,
            runs: 0,
            raised: Vec::new(),
            memory: vec![0; 64],
            writes: Vec::new(),
            watched: Vec::new(),
            pc: STORE,
            code: false,
            x0: 0,
            x0s: Vec::new(),
        }
    }
}

impl Inferior for Program {
    fn pid(&self) -> u32 {
        1
    }
    fn arch(&self) -> &'static crate::debugger::arch::Arch {
        &crate::debugger::arm64::AARCH64
    }
    fn end(&mut self) -> Result<Ending, String> {
        Ok(Ending::Killed)
    }
    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        let (exit_at, abort_at) = (self.bound.then_some(EXIT), self.bound.then_some(ABORT));
        Ok(vec![
            function("_main", false, Some(ENTRY), Some(0x100)),
            function("_mfb_fn_isPrime", false, Some(IS_PRIME), Some(0x40)),
            function("_mfb_fn_isPrime$own1", false, Some(0x1200), Some(0x40)),
            function("_mfb_fn_exit", false, Some(0x1300), Some(0x10)),
            function(&exit(), true, exit_at, None),
            function(&exit(), true, exit_at, None),
            function(&abort(), true, abort_at, None),
            function(PARK_ERROR, false, Some(PARK), Some(0x100)),
        ])
    }
    fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn read_code(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        if !self.code || !(IS_PRIME..IS_PRIME + 0x40).contains(&address) {
            return Err(format!("unmapped {address:#x}"));
        }
        Ok((0..length as u64 / 4)
            .flat_map(|index| {
                let word: u32 = if address + index * 4 == RET {
                    0xd65f_03c0
                } else {
                    0xd503_201f
                };
                word.to_le_bytes()
            })
            .collect())
    }
    fn function_at(&mut self, address: u64) -> Result<Option<FunctionRange>, String> {
        Ok(
            (self.code && (IS_PRIME..IS_PRIME + 0x40).contains(&address)).then(|| FunctionRange {
                image: "prog".to_string(),
                name: Some("_mfb_fn_isPrime".to_string()),
                start: IS_PRIME,
                size: Some(0x40),
            }),
        )
    }
    fn data_symbol_at(&mut self, _: u64) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
        Ok(Vec::new())
    }
    fn read_bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let at = address
            .checked_sub(MEMORY)
            .map(|at| at as usize)
            .filter(|at| at + length <= self.memory.len())
            .ok_or_else(|| format!("unmapped {address:#x}"))?;
        Ok(self.memory[at..at + length].to_vec())
    }
    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        Ok(vec![ThreadFacts {
            id: 1,
            handle: 1,
            name: None,
            pc: self.pc,
            sp: 0,
            arena: 0,
            stack: None,
        }])
    }
    fn registers(&mut self, _: usize) -> Result<Vec<Register>, String> {
        let register = |name: &str, value| Register {
            name: name.to_string(),
            group: crate::debugger::RegisterGroup::General,
            value: crate::debugger::RegisterValue::Bits64(value),
            role: None,
        };
        let code = if self.raised.is_empty() {
            0
        } else {
            self.raised.remove(0)
        };
        Ok(vec![
            register("x0", self.x0),
            register("x1", code),
            register("x3", 0),
            register("lr", RAISER),
        ])
    }
    /// Thread 0's `pc` moves; nothing else is kept.
    fn set_registers(&mut self, _: usize, values: &[(String, u64)]) -> Result<(), String> {
        for (name, value) in values {
            if name == "pc" {
                self.pc = *value;
            }
        }
        Ok(())
    }
    /// Thread 0 runs one instruction: the next of `writes`.
    fn step(&mut self, _: &[usize], _: u64) -> Result<Vec<Step>, String> {
        if !self.writes.is_empty() {
            for (address, byte) in self.writes.remove(0) {
                self.memory[(address - MEMORY) as usize] = byte;
            }
        }
        self.pc += 4;
        Ok(vec![Step {
            taken: 1,
            stop: StepStop::Finished,
        }])
    }
    fn restart(&mut self, _: &[(std::ffi::OsString, std::ffi::OsString)]) -> Result<u32, String> {
        Err("restart needs a program the debugger launched".to_string())
    }
    fn entry(&mut self) -> Result<Option<u64>, String> {
        Ok(Some(ENTRY))
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
    fn insert_watchpoint(&mut self, address: u64, size: u64) -> Result<(), String> {
        self.watched.push((address, size));
        Ok(())
    }
    fn remove_watchpoint(&mut self, address: u64) -> Result<(), String> {
        self.watched.retain(|(other, _)| *other != address);
        Ok(())
    }
    fn run(&mut self) -> Result<RunStop, String> {
        self.runs += 1;
        if !self.x0s.is_empty() {
            self.x0 = self.x0s.remove(0);
        }
        if !self.bound && self.inserted.contains(&ENTRY) {
            self.bound = true;
            return Ok(RunStop::Breakpoints(vec![(0, ENTRY)]));
        }
        self.bound = true;
        Ok(self.stops.remove(0))
    }
}

fn name(word: &str) -> Target {
    Target::parse(word).unwrap()
}

/// Run until the first stop the session hears, which ends the run: the
/// notes heard with it, and the stop; threads at breakpoints neither the
/// user's nor the debugger's own are a `Breakpoints` stop.
fn run(
    breakpoints: &mut Breakpoints,
    program: &mut Program,
) -> Result<(Vec<String>, RunStop), String> {
    let mut heard = None;
    let ended =
        breakpoints.run(program, &mut |_: &mut dyn Inferior,
                                        _: &Breakpoints,
                                        notes,
                                        stop| {
            heard = Some((notes, stop.clone()));
            Ok(false)
        })?;
    Ok(match ended {
        Ended::Heard(_) => heard.expect("a heard stop"),
        Ended::Own(hits) => (breakpoints.take_notes(), RunStop::Breakpoints(hits)),
    })
}

#[test]
fn an_address_or_a_name_is_parsed() {
    assert_eq!(Target::parse("0x1F00"), Ok(Target::Address(0x1f00)));
    assert_eq!(Target::parse("0X10"), Ok(Target::Address(0x10)));
    assert_eq!(
        Target::parse("isPrime"),
        Ok(Target::Name("isPrime".to_string()))
    );
    assert_eq!(
        Target::parse("0xzz"),
        Err("'0xzz' is not an address".to_string())
    );
}

#[test]
fn a_symbol_wins_over_a_written_name_and_every_copy_is_covered() {
    let mut program = Program::new(Vec::new());
    let mut breakpoints = Breakpoints::default();
    // `isPrime` is written; it covers the function and its `$own` copy.
    assert_eq!(
        breakpoints
            .set(&mut program, &name("isPrime"), None, &[])
            .unwrap(),
        [
            "Set a breakpoint at 0x0000000000001100 in isPrime.",
            "Set a breakpoint at 0x0000000000001200 in isPrime.",
        ]
    );
    // `_mfb_fn_exit` is a symbol: only that one function.
    assert_eq!(
        breakpoints
            .set(&mut program, &name("_mfb_fn_exit"), None, &[])
            .unwrap(),
        ["Set a breakpoint at 0x0000000000001300 in exit <mfb>."]
    );
    assert_eq!(program.inserted, [0x1100, 0x1200, 0x1300]);
    assert_eq!(
        breakpoints.set(&mut program, &name("isPrime"), None, &[]),
        Err("there is already a breakpoint on 'isPrime'".to_string())
    );
    assert_eq!(
        breakpoints.set(&mut program, &name("nothing"), None, &[]),
        Err("no function is named 'nothing'; type 'list func' for the functions".to_string())
    );
}

#[test]
fn a_function_and_an_offset_is_a_place_in_each_function_so_named() {
    let mut program = Program::new(Vec::new());
    let mut breakpoints = Breakpoints::default();
    // `isPrime` is two functions, the original and a hand-over copy.
    assert_eq!(
        breakpoints
            .set(&mut program, &name("isPrime+0x24"), None, &[])
            .unwrap(),
        [
            "Set a breakpoint at 0x0000000000001124 in isPrime+0x24.",
            "Set a breakpoint at 0x0000000000001224 in isPrime+0x24.",
        ]
    );
    // A symbol, with a decimal offset.
    assert_eq!(
        breakpoints
            .set(&mut program, &name("_mfb_fn_exit+4"), None, &[])
            .unwrap(),
        ["Set a breakpoint at 0x0000000000001304 in exit+0x4 <mfb>."]
    );
    assert_eq!(
        breakpoints.set(&mut program, &name("_mfb_fn_exit+4"), None, &[]),
        Err("there is already a breakpoint at _mfb_fn_exit+0x4".to_string())
    );
    assert_eq!(
        breakpoints
            .remove(&mut program, &name("_mfb_fn_isPrime+0x24"), None)
            .unwrap(),
        ["Removed the breakpoint at 0x0000000000001124 in isPrime+0x24."]
    );
    assert_eq!(program.inserted, [0x1224, 0x1304]);
    // `abort` is not bound until the program reaches its entry point.
    assert_eq!(
        breakpoints.set(&mut program, &name(&format!("{}+8", abort())), None, &[]),
        Err(format!(
            "'{}' is not loaded yet; run the program until it has loaded its libraries",
            abort()
        ))
    );
    assert_eq!(
        breakpoints.set(&mut program, &name("nothing+8"), None, &[]),
        Err("no function is named 'nothing'; type 'list func' for the functions".to_string())
    );
}

#[test]
fn breakerror_stops_in_the_raiser_only_for_a_watched_code() {
    // Two raises: one not watched (run on), then 77050010, stopped back
    // in the function that raised it.
    let mut program = Program::new(vec![
        RunStop::Breakpoints(vec![(0, PARK)]),
        RunStop::Breakpoints(vec![(0, PARK)]),
        RunStop::Breakpoints(vec![(0, RAISER)]),
    ]);
    program.raised = vec![77_050_004, 77_050_010];
    let mut breakpoints = Breakpoints::default();
    assert_eq!(
        breakpoints
            .watch_errors(&mut program, ErrorCodes::Codes(vec![77_050_010]), &[])
            .unwrap(),
        ["Stopping when a thread raises 77050010 (errorCode::ErrOverflow)."]
    );
    assert_eq!(program.inserted, [PARK]);
    let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert_eq!(
        stop,
        RunStop::Raised {
            thread: 0,
            code: 77_050_010,
            origin: None
        }
    );
    // The stop's own breakpoint is gone; the watch's stays.
    assert_eq!(program.inserted, [PARK]);
    // More codes join the watch; `any` takes them all.
    breakpoints
        .watch_errors(&mut program, ErrorCodes::Codes(vec![42, 77_050_010]), &[])
        .unwrap();
    assert_eq!(
        breakpoints.lines(),
        ["Stopping when a thread raises 77050010 (errorCode::ErrOverflow), 42."]
    );
    breakpoints
        .watch_errors(&mut program, ErrorCodes::Any, &[])
        .unwrap();
    assert_eq!(
        breakpoints.lines(),
        ["Stopping when a thread raises any error."]
    );
    assert_eq!(
        breakpoints.clear_errors(&mut program).unwrap(),
        ["Stopped watching for errors."]
    );
    assert!(program.inserted.is_empty());
    assert_eq!(
        breakpoints.clear_errors(&mut program),
        Err("no errors are being watched for".to_string())
    );
}

#[test]
fn a_breakpoint_of_the_user_s_on_the_park_shares_it_with_breakerror() {
    let mut program = Program::new(vec![RunStop::Breakpoints(vec![(0, PARK)])]);
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .watch_errors(&mut program, ErrorCodes::Any, &[])
        .unwrap();
    breakpoints
        .set(&mut program, &name(PARK_ERROR), None, &[])
        .unwrap();
    assert_eq!(program.inserted, [PARK]);
    // The user's breakpoint is reported as theirs.
    let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert_eq!(stop, RunStop::Breakpoints(vec![(0, PARK)]));
    // Removing it leaves the watch's breakpoint in place.
    breakpoints
        .remove(&mut program, &name(PARK_ERROR), None)
        .unwrap();
    assert_eq!(program.inserted, [PARK]);
    assert_eq!(
        breakpoints.lines(),
        ["Stopping when a thread raises any error."]
    );
}

#[test]
fn an_address_is_described_by_the_function_that_holds_it() {
    let mut program = Program::new(Vec::new());
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .set(&mut program, &Target::Address(0x1124), None, &[])
        .unwrap();
    breakpoints
        .set(&mut program, &Target::Address(0x5000), None, &[])
        .unwrap();
    assert_eq!(
        breakpoints.lines(),
        [
            "Address            | Count | Symbol               | Written name",
            "0x0000000000001124 | 0     | _mfb_fn_isPrime+0x24 | isPrime+0x24",
            "0x0000000000005000 | 0     | -                    | -",
        ]
    );
    assert_eq!(
        breakpoints.set(&mut program, &Target::Address(0x1124), None, &[]),
        Err("there is already a breakpoint at 0x0000000000001124".to_string())
    );
}

#[test]
fn an_unbound_linked_function_is_pending_until_the_entry_point() {
    let mut program = Program::new(vec![RunStop::Breakpoints(vec![(0, EXIT)])]);
    let mut breakpoints = Breakpoints::default();
    // As written, `exit` is both the developer's function and the linked
    // one: on Mach-O, where `exit` is no symbol, that is ambiguous; on ELF
    // the C symbol is spelled `exit`, so the name is that function's.
    if !crate::debugger::names::C_PREFIX.is_empty() {
        assert_eq!(
            breakpoints.set(&mut program, &name("exit"), None, &[]),
            Err(
                "'exit' is ambiguous: it names _exit, _mfb_fn_exit; name one by its symbol"
                    .to_string()
            )
        );
    }
    assert_eq!(
        breakpoints
            .set(&mut program, &name("_mfb_fn_exit"), None, &[])
            .unwrap(),
        ["Set a breakpoint at 0x0000000000001300 in exit <mfb>."]
    );
    // The linked one is not bound yet.
    assert_eq!(
        breakpoints
            .set(&mut program, &name(&exit()), None, &[])
            .unwrap(),
        [format!(
            "Set a pending breakpoint on {}: it is placed when the program has loaded its \
                 libraries.",
            exit()
        )]
    );
    assert_eq!(
        breakpoints.lines(),
        [
            "Address            | Count | Symbol       | Written name".to_string(),
            "0x0000000000001300 | 0     | _mfb_fn_exit | exit <mfb>".to_string(),
            format!(
                "-                  | 0     | {:<12} | exit <native>",
                exit()
            ),
        ]
    );
    let (notes, stop) = run(&mut breakpoints, &mut program).unwrap();
    // The entry-point stop was the debugger's own: placed, and ran on.
    assert_eq!(program.runs, 2);
    assert_eq!(
        notes,
        [format!(
            "Placed the pending breakpoint on {} at 0x0000000000009000.",
            exit()
        )]
    );
    assert_eq!(stop, RunStop::Breakpoints(vec![(0, EXIT)]));
    assert_eq!(program.inserted, [0x1300, EXIT]);
}

#[test]
fn removing_by_name_takes_symbols_first_then_written_names() {
    let mut program = Program::new(Vec::new());
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .set(&mut program, &name("isPrime"), None, &[])
        .unwrap();
    breakpoints
        .set(&mut program, &name(&abort()), None, &[])
        .unwrap();
    assert_eq!(
        breakpoints
            .remove(&mut program, &name("_mfb_fn_isPrime"), None)
            .unwrap(),
        ["Removed the breakpoint at 0x0000000000001100 in isPrime."]
    );
    assert_eq!(
        breakpoints
            .remove(&mut program, &name("isPrime"), None)
            .unwrap(),
        ["Removed the breakpoint at 0x0000000000001200 in isPrime."]
    );
    assert_eq!(
        breakpoints
            .remove(&mut program, &name("abort"), None)
            .unwrap(),
        [format!("Removed the breakpoint at {}.", abort())]
    );
    assert_eq!(breakpoints.lines(), ["no breakpoints"]);
    assert!(program.inserted.is_empty());
    assert_eq!(
        breakpoints.remove(&mut program, &Target::Address(0x1100), None),
        Err("there is no breakpoint at 0x0000000000001100".to_string())
    );
    assert_eq!(
        breakpoints.remove(&mut program, &name("isPrime"), None),
        Err("there is no breakpoint on 'isPrime'".to_string())
    );
}

#[test]
fn a_user_breakpoint_at_the_entry_point_is_reported() {
    let mut program = Program::new(Vec::new());
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .set(&mut program, &name("_main"), None, &[])
        .unwrap();
    breakpoints
        .set(&mut program, &name(&abort()), None, &[])
        .unwrap();
    let (notes, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert_eq!(stop, RunStop::Breakpoints(vec![(0, ENTRY)]));
    assert_eq!(
        notes,
        [format!(
            "Placed the pending breakpoint on {} at 0x0000000000009200.",
            abort()
        )]
    );
    assert_eq!(program.runs, 1);
}

#[test]
fn an_exit_leaves_no_breakpoint_to_remove() {
    let mut program = Program::new(vec![RunStop::Exited(Exit::Status(3))]);
    program.bound = true;
    let mut breakpoints = Breakpoints::default();
    let (notes, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert!(notes.is_empty());
    assert_eq!(stop, RunStop::Exited(Exit::Status(3)));
}

#[test]
fn a_watchpoint_stops_only_when_a_write_changes_its_bytes() {
    // Three writes into the watched word: the same value (unseen), a new
    // one (a stop), and one beside it (unseen).
    let mut program = Program::new(vec![
        RunStop::Watchpoints(vec![0]),
        RunStop::Watchpoints(vec![0]),
        RunStop::Watchpoints(vec![0]),
        RunStop::Exited(Exit::Status(0)),
    ]);
    program.bound = true;
    program.memory[8] = 5;
    program.writes = vec![
        vec![(MEMORY + 8, 5)],
        vec![(MEMORY + 8, 6)],
        vec![(MEMORY + 16, 9)],
    ];
    let mut breakpoints = Breakpoints::default();
    assert_eq!(
        breakpoints.watch(&mut program, MEMORY + 8, 8, &[]).unwrap(),
        ["Set a watchpoint at 0x0000000000008008 (8 bytes): 0x0000000000000005 (5)."]
    );
    assert_eq!(program.watched, [(MEMORY + 8, 8)]);
    let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert_eq!(
        stop,
        RunStop::Changed(vec![Change {
            thread: 0,
            pc: STORE + 4,
            address: MEMORY + 8,
            size: 8,
            old: 5,
            new: 6,
        }])
    );
    assert_eq!(program.runs, 2);
    let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert_eq!(stop, RunStop::Exited(Exit::Status(0)));
    assert_eq!(
        breakpoints.watch_lines(&mut program),
        [
            "Address            | Count | Size | Value",
            "0x0000000000008008 | 1     | 8    | 0x0000000000000006 (6)",
        ]
    );
}

#[test]
fn a_watched_write_under_the_breakpoint_a_thread_is_paused_on_is_a_stop() {
    // Thread 0 is paused on a breakpoint on the store itself: running on
    // runs the store first, and the change it makes is the stop, before
    // the program runs.
    let mut program = Program::new(vec![RunStop::Exited(Exit::Status(0))]);
    program.bound = true;
    program.memory[8] = 5;
    program.writes = vec![vec![(MEMORY + 8, 6)]];
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .set(&mut program, &Target::Address(STORE), None, &[])
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
            old: 5,
            new: 6,
        }])
    );
    assert_eq!(program.runs, 0);
    let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert_eq!(stop, RunStop::Exited(Exit::Status(0)));
}

#[test]
fn a_watched_write_on_the_way_to_the_raiser_that_lands_on_a_breakpoint_stops_there() {
    // A watched raise runs on to the raiser; a watched write on the way
    // changes nothing but leaves the thread on the user's breakpoint,
    // which stops it before the program runs again.
    let mut program = Program::new(vec![
        RunStop::Breakpoints(vec![(0, PARK)]),
        RunStop::Watchpoints(vec![0]),
        RunStop::Breakpoints(vec![(0, RAISER)]),
    ]);
    program.bound = true;
    program.raised = vec![77_050_010];
    program.memory[8] = 5;
    program.writes = vec![vec![(MEMORY + 8, 5)]];
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .watch_errors(&mut program, ErrorCodes::Any, &[])
        .unwrap();
    breakpoints
        .set(&mut program, &Target::Address(STORE + 4), None, &[])
        .unwrap();
    breakpoints.watch(&mut program, MEMORY + 8, 8, &[]).unwrap();
    let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
    assert_eq!(stop, RunStop::Breakpoints(vec![(0, STORE + 4)]));
    assert_eq!(program.runs, 2);
    // The raiser's breakpoint of the run's own is gone.
    assert_eq!(program.inserted, [PARK, STORE + 4]);
}

#[test]
fn a_watchpoint_covers_1_to_8_bytes_within_a_multiple_of_8() {
    let mut program = Program::new(Vec::new());
    let mut breakpoints = Breakpoints::default();
    for (address, size) in [(MEMORY, 0), (MEMORY, 9), (MEMORY + 4, 8), (MEMORY + 7, 2)] {
        assert_eq!(
            breakpoints.watch(&mut program, address, size, &[]),
            Err(format!(
                "cannot watch {size} bytes at 0x{address:016x}: a watchpoint covers 1 to 8 \
                     bytes that do not cross an address that is a multiple of 8"
            ))
        );
    }
    assert_eq!(
        breakpoints.watch(&mut program, MEMORY + 6, 2, &[]).unwrap(),
        ["Set a watchpoint at 0x0000000000008006 (2 bytes): 0x0000."]
    );
    assert_eq!(
        breakpoints.watch(&mut program, MEMORY + 6, 1, &[]),
        Err("there is already a watchpoint at 0x0000000000008006".to_string())
    );
    assert_eq!(
        breakpoints.watch(&mut program, 0x10, 1, &[]),
        Err("unmapped 0x10".to_string())
    );
    assert_eq!(
        breakpoints.unwatch(&mut program, MEMORY + 6).unwrap(),
        ["Removed the watchpoint at 0x0000000000008006 (2 bytes)."]
    );
    assert!(program.watched.is_empty());
    assert_eq!(
        breakpoints.unwatch(&mut program, MEMORY + 6),
        Err("there is no watchpoint at 0x0000000000008006".to_string())
    );
    assert_eq!(breakpoints.watch_lines(&mut program), ["no watchpoints"]);
}

#[test]
fn a_restart_drops_every_watchpoint() {
    let mut program = Program::new(Vec::new());
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .watch(&mut program, MEMORY, 4, &["bt".to_string()])
        .unwrap();
    assert_eq!(
        breakpoints.restarted(&mut program).unwrap(),
        [
            "Dropped the watchpoint at 0x0000000000008000: a restarted program's memory is \
                 laid out anew."
        ]
    );
    assert_eq!(breakpoints.watch_lines(&mut program), ["no watchpoints"]);
}

#[test]
fn a_stop_with_commands_runs_on_and_one_without_ends_the_run() {
    let mut program = Program::new(vec![
        RunStop::Breakpoints(vec![(0, IS_PRIME)]),
        RunStop::Breakpoints(vec![(0, IS_PRIME), (1, 0x1300)]),
        RunStop::Exited(Exit::Status(0)),
    ]);
    program.bound = true;
    let mut breakpoints = Breakpoints::default();
    let commands = ["bt 1".to_string(), "list reg x0".to_string()];
    assert_eq!(
        breakpoints
            .set(&mut program, &name("_mfb_fn_isPrime"), None, &commands)
            .unwrap(),
        [
            "Set a breakpoint at 0x0000000000001100 in isPrime. Each stop runs 'bt 1; list \
                 reg x0'."
        ]
    );
    breakpoints
        .set(&mut program, &name("_mfb_fn_exit"), None, &[])
        .unwrap();
    assert_eq!(
        breakpoints.lines(),
        [
            "Address            | Count | Symbol          | Written name | Commands",
            "0x0000000000001100 | 0     | _mfb_fn_isPrime | isPrime      | bt 1; list reg x0",
            "0x0000000000001300 | 0     | _mfb_fn_exit    | exit <mfb>   | -",
        ]
    );
    // Each stop is heard; the first, whose every breakpoint has
    // commands, runs on; the second has one without and ends the run.
    let mut heard = Vec::new();
    let ended = breakpoints
        .run(
            &mut program,
            &mut |_: &mut dyn Inferior, breakpoints: &Breakpoints, _, stop: &RunStop| {
                let (commands, every) = breakpoints.stop_commands(stop);
                heard.push((stop.clone(), commands));
                Ok(every)
            },
        )
        .unwrap();
    assert_eq!(
        ended,
        Ended::Heard(RunStop::Breakpoints(vec![(0, IS_PRIME), (1, 0x1300)]))
    );
    assert_eq!(
        heard,
        [
            (
                RunStop::Breakpoints(vec![(0, IS_PRIME)]),
                vec![(0, commands.to_vec())]
            ),
            (
                RunStop::Breakpoints(vec![(0, IS_PRIME), (1, 0x1300)]),
                vec![(0, commands.to_vec())]
            ),
        ]
    );
}

fn test(location: &str, test: &str, value: Option<&str>) -> Condition {
    Condition::parse(location, test, value).unwrap()
}

/// Breakpoints with tests share one instruction; a thread stops only
/// for those whose test holds, each running its own commands.
#[test]
fn a_breakpoint_with_a_test_stops_only_when_it_holds() {
    let mut program = Program::new(vec![
        RunStop::Breakpoints(vec![(0, IS_PRIME)]),
        RunStop::Breakpoints(vec![(0, IS_PRIME)]),
        RunStop::Breakpoints(vec![(0, IS_PRIME)]),
        RunStop::Exited(Exit::Status(0)),
    ]);
    program.bound = true;
    program.x0s = vec![1, 5, 7];
    let mut breakpoints = Breakpoints::default();
    let five = test("x0", "eq", Some("5"));
    let big = test("x0", "gt", Some("6"));
    assert_eq!(
        breakpoints
            .set(&mut program, &name("_mfb_fn_isPrime"), Some(&five), &[])
            .unwrap(),
        ["Set a breakpoint at 0x0000000000001100 in isPrime if x0 eq 0x0000000000000005 (5)."]
    );
    breakpoints
        .set(
            &mut program,
            &name("_mfb_fn_isPrime"),
            Some(&big),
            &["bt".to_string()],
        )
        .unwrap();
    assert_eq!(
        breakpoints.set(&mut program, &name("_mfb_fn_isPrime"), Some(&five), &[]),
        Err(
            "there is already a breakpoint on '_mfb_fn_isPrime' if x0 eq 0x0000000000000005 (5)"
                .to_string()
        )
    );
    breakpoints
        .set(&mut program, &name("_mfb_fn_exit"), None, &[])
        .unwrap();
    assert_eq!(program.inserted, [IS_PRIME, 0x1300]);
    assert_eq!(
        breakpoints.lines(),
        [
            "Address            | Count | Symbol          | Written name | If                           | Commands",
            "0x0000000000001100 | 0     | _mfb_fn_isPrime | isPrime      | x0 eq 0x0000000000000005 (5) | -",
            "0x0000000000001100 | 0     | _mfb_fn_isPrime | isPrime      | x0 gt 0x0000000000000006 (6) | bt",
            "0x0000000000001300 | 0     | _mfb_fn_exit    | exit <mfb>   | -                            | -",
        ]
    );
    // x0 = 1 passes neither test and runs on; x0 = 5 stops, with no
    // commands; x0 = 7 runs the other's and runs on to the end.
    let mut heard = Vec::new();
    let mut hear = |_: &mut dyn Inferior, breakpoints: &Breakpoints, _, stop: &RunStop| {
        let (commands, every) = breakpoints.stop_commands(stop);
        heard.push((stop.clone(), commands));
        Ok(every)
    };
    let ended = breakpoints.run(&mut program, &mut hear).unwrap();
    assert_eq!(
        ended,
        Ended::Heard(RunStop::Breakpoints(vec![(0, IS_PRIME)]))
    );
    let ended = breakpoints.run(&mut program, &mut hear).unwrap();
    assert_eq!(ended, Ended::Heard(RunStop::Exited(Exit::Status(0))));
    assert_eq!(
        heard,
        [
            (RunStop::Breakpoints(vec![(0, IS_PRIME)]), Vec::new()),
            (
                RunStop::Breakpoints(vec![(0, IS_PRIME)]),
                vec![(0, vec!["bt".to_string()])]
            ),
            (RunStop::Exited(Exit::Status(0)), Vec::new()),
        ]
    );
    assert_eq!(program.runs, 4);
    // `breakpointif r` takes just its test's; the instruction stays for
    // the other, which `breakpoint r` then takes.
    assert_eq!(
        breakpoints
            .remove(&mut program, &name("isPrime"), Some(&five))
            .unwrap(),
        ["Removed the breakpoint at 0x0000000000001100 in isPrime if x0 eq 0x0000000000000005 (5)."]
    );
    assert_eq!(
        breakpoints.remove(&mut program, &name("isPrime"), Some(&five)),
        Err("there is no breakpoint on 'isPrime' if x0 eq 0x0000000000000005 (5)".to_string())
    );
    assert_eq!(program.inserted, [IS_PRIME, 0x1300]);
    assert_eq!(
        breakpoints
            .remove(&mut program, &Target::Address(IS_PRIME), None)
            .unwrap(),
        ["Removed the breakpoint at 0x0000000000001100 in isPrime if x0 gt 0x0000000000000006 (6)."]
    );
    assert_eq!(program.inserted, [0x1300]);
}

/// A test whose memory cannot be read does not stop the thread; `in`
/// stops just then.
#[test]
fn a_test_that_cannot_be_read_stops_only_in() {
    let mut program = Program::new(vec![RunStop::Breakpoints(vec![(0, IS_PRIME)])]);
    program.bound = true;
    program.memory[8] = 5;
    let mut breakpoints = Breakpoints::default();
    let unreadable = test("[0x10]", "eq", Some("0"));
    let missing = test("[0x10]", "in", None);
    let readable = test("[0x8008]", "eq", Some("5"));
    for (condition, commands) in [
        (&unreadable, "list reg x0"),
        (&missing, "bt"),
        (&readable, "dis"),
    ] {
        breakpoints
            .set(
                &mut program,
                &Target::Address(IS_PRIME),
                Some(condition),
                &[commands.to_string()],
            )
            .unwrap();
    }
    let mut heard = Vec::new();
    breakpoints
        .run(
            &mut program,
            &mut |_: &mut dyn Inferior, breakpoints: &Breakpoints, _, stop: &RunStop| {
                heard.push(breakpoints.stop_commands(stop));
                Ok(false)
            },
        )
        .unwrap();
    assert_eq!(
        heard,
        [(
            vec![(0, vec!["bt".to_string()]), (0, vec!["dis".to_string()])],
            true
        )]
    );
}

/// A pending breakpoint with a test joins one already at its address.
#[test]
fn pending_breakpoints_with_different_tests_share_their_place() {
    let mut program = Program::new(vec![RunStop::Breakpoints(vec![(0, ABORT)])]);
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .set(&mut program, &name(&abort()), None, &[])
        .unwrap();
    assert_eq!(
        breakpoints
            .set(
                &mut program,
                &name(&abort()),
                Some(&test("x0", "ne", Some("0"))),
                &[]
            )
            .unwrap(),
        [format!(
            "Set a pending breakpoint on {} if x0 ne 0x0000000000000000 (0): it is placed \
                 when the program has loaded its libraries.",
            abort()
        )]
    );
    let (notes, stop) = run(&mut breakpoints, &mut program).unwrap();
    let placed = format!(
        "Placed the pending breakpoint on {} at 0x0000000000009200.",
        abort()
    );
    assert_eq!(notes, [placed.clone(), placed]);
    // x0 is 0: only the breakpoint without a test stops.
    assert_eq!(stop, RunStop::Breakpoints(vec![(0, ABORT)]));
    assert_eq!(program.inserted, [ABORT]);
}

/// A breakpoint of the user's on a `ret` with commands runs them, and the
/// finish still returns there.
#[test]
fn a_finish_returns_at_a_ret_holding_a_breakpoint_with_commands() {
    let mut program = Program::new(vec![
        RunStop::Breakpoints(vec![(0, RET)]),
        RunStop::Exited(Exit::Status(0)),
    ]);
    program.bound = true;
    program.code = true;
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .set(
            &mut program,
            &Target::Address(RET),
            None,
            &["bt".to_string()],
        )
        .unwrap();
    let mut heard = Vec::new();
    let finish = breakpoints
        .finish(
            &mut program,
            0,
            &mut |_: &mut dyn Inferior, breakpoints: &Breakpoints, _, stop: &RunStop| {
                heard.push(stop.clone());
                Ok(breakpoints.stop_commands(stop).1)
            },
        )
        .unwrap();
    assert_eq!(
        finish,
        Finish::Returned {
            from: Some("_mfb_fn_isPrime".to_string())
        }
    );
    assert_eq!(heard, [RunStop::Breakpoints(vec![(0, RET)])]);
    assert_eq!(program.inserted, [RET]);
}

/// A restart places the error park's breakpoint once, though one of the
/// user's is there too.
#[test]
fn a_restart_places_a_shared_park_breakpoint_once() {
    let mut program = Program::new(Vec::new());
    let mut breakpoints = Breakpoints::default();
    breakpoints
        .watch_errors(&mut program, ErrorCodes::Any, &[])
        .unwrap();
    breakpoints
        .set(&mut program, &name(PARK_ERROR), None, &[])
        .unwrap();
    // The new process holds none of the old one's breakpoints.
    program.inserted.clear();
    assert_eq!(breakpoints.restarted(&mut program), Ok(Vec::new()));
    assert_eq!(program.inserted, [PARK]);
}

#[test]
fn a_watched_error_with_commands_runs_them_and_keeps_them() {
    let mut program = Program::new(Vec::new());
    let mut breakpoints = Breakpoints::default();
    let commands = ["show error".to_string()];
    assert_eq!(
        breakpoints
            .watch_errors(&mut program, ErrorCodes::Any, &commands)
            .unwrap(),
        ["Stopping when a thread raises any error. Each stop runs 'show error'."]
    );
    // More codes without commands keep the watch's commands.
    breakpoints
        .watch_errors(&mut program, ErrorCodes::Codes(vec![42]), &[])
        .unwrap();
    let raised = RunStop::Raised {
        thread: 0,
        code: 42,
        origin: None,
    };
    assert_eq!(
        breakpoints.stop_commands(&raised),
        (vec![(0, commands.to_vec())], true)
    );
    assert_eq!(
        breakpoints.stop_commands(&RunStop::Interrupted),
        (Vec::new(), false)
    );
}
