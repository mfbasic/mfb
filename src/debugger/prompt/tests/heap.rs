//! The prompt over a `--debug` program's runtime data: the arena registry,
//! arena regions, a parked error, `String` literals, and real stack frames to
//! unwind. The process is a word map with data symbols, threads with their
//! unwind registers, and functions with their code.

use std::collections::HashMap;
use std::ffi::OsString;

use super::*;
use crate::codegen::debug::arena::{
    ARENA_BASE_SYMBOL, ARENA_KIND_MAIN, REGION_COUNT_OFFSET, REGION_HEADER_SIZE, SLOT_KIND_OFFSET,
};
use crate::codegen::error::constants::error_constants::{
    ARENA_ALLOC_SCALARS_OFFSET, ARENA_CURRENT_ERROR_OFFSET, ARENA_SC_COMMITTED,
    ARENA_SC_HUGE_COMMIT, ARENA_SC_HUGE_FRONT, ARENA_SC_MEDIUM_BASE, ARENA_SC_MEDIUM_COMMIT,
    ARENA_SC_MEDIUM_FRONT, ARENA_SC_SLICE_TOP, ARENA_SC_SMALL_COMMIT, ARENA_SC_SMALL_PAGES,
};
use crate::debugger::{Ending, FunctionRange, Step};

const REGISTRY_GLOBAL: u64 = 0x600;
const REGION: u64 = 0x10_0000;
const MAIN_STATE: u64 = 0x20_0000;
const BLOCK: u64 = 0x21_0000;
const BLOCK_BYTES: u64 = 0x100;

/// `sub sp, sp, #0x10`, `str x30, [sp]`, `nop`.
const FRAME_CODE: [u32; 3] = [0xd100_43ff, 0xf900_03fe, 0xd503_201f];

#[derive(Default)]
struct Heap {
    words: HashMap<u64, u64>,
    symbols: HashMap<String, u64>,
    /// The data symbol each address is the start of.
    data_at: HashMap<u64, String>,
    threads: Vec<ThreadFacts>,
    /// `pc`, `sp`, `fp`, `lr` of each thread.
    unwind: Vec<[u64; 4]>,
    functions: Vec<FunctionSymbol>,
    /// How every stepped thread stops.
    stop: Option<StepStop>,
    /// The last thread ends during a `step-all`.
    last_ends: bool,
}

impl Heap {
    fn put(&mut self, address: u64, value: u64) {
        self.words.insert(address, value);
    }

    /// Bytes at an 8-aligned `address`, packed into words.
    fn put_bytes(&mut self, address: u64, bytes: &[u8]) {
        for (index, chunk) in bytes.chunks(8).enumerate() {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.put(address + index as u64 * 8, u64::from_le_bytes(word));
        }
    }

    /// A `String` at `address`: its length, its bytes, a NUL.
    fn put_text(&mut self, address: u64, text: &str) {
        self.put(address, text.len() as u64);
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        self.put_bytes(address + 8, &bytes);
    }

    fn thread(&mut self, pc: u64, sp: u64, lr: u64) {
        let id = 0x500 + self.threads.len() as u64;
        self.threads.push(ThreadFacts {
            id,
            handle: id,
            name: None,
            pc,
            sp,
            arena: 0,
            stack: None,
        });
        self.unwind.push([pc, sp, 0, lr]);
    }

    /// A `--debug` registry holding the main arena, laid out with `BLOCK_BYTES`
    /// handed out in its medium region (bytes counting up from 0) and nothing in
    /// the small or huge region (plan-183-C).
    fn with_main_arena(&mut self) {
        self.symbols
            .insert(ARENA_BASE_SYMBOL.to_string(), REGISTRY_GLOBAL);
        self.put(REGISTRY_GLOBAL, REGION);
        self.put(REGION + REGION_COUNT_OFFSET as u64, 1);
        let slot = REGION + REGION_HEADER_SIZE as u64;
        self.put(slot, MAIN_STATE);
        self.put(
            slot + SLOT_KIND_OFFSET as u64,
            ARENA_KIND_MAIN.parse().unwrap(),
        );
        self.put_bytes(
            MAIN_STATE,
            &vec![0; crate::codegen::error::constants::error_constants::ARENA_STATE_SIZE],
        );
        let scalar = |offset: usize| MAIN_STATE + (ARENA_ALLOC_SCALARS_OFFSET + offset) as u64;
        for (offset, value) in [
            (ARENA_SC_SMALL_PAGES, 0x1000_0000),
            (ARENA_SC_SMALL_COMMIT, 0x1000_0000),
            (ARENA_SC_MEDIUM_BASE, BLOCK),
            (ARENA_SC_MEDIUM_FRONT, BLOCK + BLOCK_BYTES),
            (ARENA_SC_MEDIUM_COMMIT, BLOCK + 0x1_0000),
            (ARENA_SC_HUGE_FRONT, 0x3000_0000),
            (ARENA_SC_HUGE_COMMIT, 0x3000_0000),
            (ARENA_SC_SLICE_TOP, 0x3000_0000),
            (ARENA_SC_COMMITTED, 0x400),
        ] {
            self.put(scalar(offset), value);
        }
        let bytes: Vec<u8> = (0..BLOCK_BYTES).map(|byte| byte as u8).collect();
        self.put_bytes(BLOCK, &bytes);
    }

    /// Thread 0 runs `_mfb_fn_work` (frame code above), called from `_main`.
    fn with_frames(&mut self) {
        self.functions = vec![
            symbol("_main", false, Some(0x1000), Some(0x10)),
            symbol("_mfb_fn_work", false, Some(0x2000), Some(0xc)),
        ];
        self.thread(0x2008, 0x8000, 0x1008);
        self.put(0x8000, 0x1008);
    }
}

impl Inferior for Heap {
    fn pid(&self) -> u32 {
        42
    }
    fn arch(&self) -> &'static crate::debugger::arch::Arch {
        &crate::debugger::arm64::AARCH64
    }
    fn end(&mut self) -> Result<Ending, String> {
        Ok(Ending::Killed)
    }
    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        Ok(self.functions.clone())
    }
    fn data_symbol(&mut self, name: &str) -> Result<Option<u64>, String> {
        Ok(self.symbols.get(name).copied())
    }
    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        self.words
            .get(&address)
            .copied()
            .ok_or_else(|| format!("unmapped {address:#x}"))
    }
    fn read_bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        (address..address + length as u64)
            .map(|at| Ok(self.read_u64(at & !7)?.to_le_bytes()[(at & 7) as usize]))
            .collect()
    }
    fn read_code(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let start = self
            .functions
            .iter()
            .filter_map(|function| function.address)
            .filter(|start| *start <= address)
            .max()
            .ok_or_else(|| format!("unmapped {address:#x}"))?;
        let first = ((address - start) / 4) as usize;
        Ok(FRAME_CODE
            .iter()
            .cycle()
            .skip(first)
            .take(length / 4)
            .flat_map(|word| word.to_le_bytes())
            .collect())
    }
    fn function_at(&mut self, address: u64) -> Result<Option<FunctionRange>, String> {
        Ok(self.functions.iter().find_map(|function| {
            let (start, size) = (function.address?, function.size?);
            (start <= address && address < start + size).then(|| FunctionRange {
                image: "prog".to_string(),
                name: Some(function.name.clone()),
                start,
                size: Some(size),
            })
        }))
    }
    fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
        Ok(Vec::new())
    }
    fn data_symbol_at(&mut self, address: u64) -> Result<Option<String>, String> {
        Ok(self.data_at.get(&address).cloned())
    }
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        Ok(self.threads.clone())
    }
    fn registers(&mut self, thread: usize) -> Result<Vec<Register>, String> {
        let values = self.unwind[thread];
        Ok(["pc", "sp", "fp", "lr", "x0"]
            .iter()
            .enumerate()
            .map(|(index, name)| Register {
                name: name.to_string(),
                group: RegisterGroup::General,
                value: RegisterValue::Bits64(match *name {
                    "x0" => BLOCK + 0x20,
                    _ => values[index],
                }),
                role: None,
            })
            .collect())
    }
    fn set_registers(&mut self, _: usize, _: &[(String, u64)]) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn step(&mut self, threads: &[usize], count: u64) -> Result<Vec<Step>, String> {
        let stop = self.stop.clone().ok_or("read-only")?;
        if self.last_ends && threads.len() == self.threads.len() {
            self.threads.pop();
            self.unwind.pop();
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
        Err("read-only".to_string())
    }
    fn entry(&mut self) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn insert_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn remove_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn insert_watchpoint(&mut self, _: u64, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn remove_watchpoint(&mut self, _: u64) -> Result<(), String> {
        Err("read-only".to_string())
    }
    fn run(&mut self) -> Result<RunStop, String> {
        Err("read-only".to_string())
    }
}

/// Run `input` at the prompt; the output and errors, prompts removed.
fn session_over(heap: &mut Heap, input: &str) -> (String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    run(
        heap,
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
fn list_arena_lists_the_registry_or_says_the_runtime_has_not_started() {
    let mut heap = Heap::default();
    heap.thread(0x1000, 0x8000, 0);
    heap.with_main_arena();
    let (out, err) = session_over(&mut heap, "list arena\nq\n");
    assert_eq!(err, "");
    assert_eq!(
        out,
        "ID | Thread | Address            | Committed (KB)\n\
         0  | 0      | 0x0000000000200000 | 1\n\
         Killed process 42.\n"
    );
    // The registry exists but the runtime has registered nothing yet.
    heap.put(REGISTRY_GLOBAL, 0);
    let (out, _) = session_over(&mut heap, "list arena\nshow arena\nq\n");
    assert!(out.starts_with("no arenas yet: the program's runtime has not started\n"));
}

#[test]
fn show_arena_shows_its_state_and_regions_and_dumps_a_region() {
    let mut heap = Heap::default();
    heap.thread(0x1000, 0x8000, 0);
    heap.with_main_arena();
    let (out, err) = session_over(&mut heap, "show arena\nq\n");
    assert_eq!(err, "");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "Arena State:");
    let region = lines
        .iter()
        .position(|line| *line == "Arena Region (1 medium):")
        .unwrap();
    assert_eq!(
        lines[region + 1..region + 4],
        [
            "  start             0x0000000000210000",
            "  handedOut         0x0000000000000100",
            "  committed         0x0000000000010000",
        ]
    );
    assert!(lines.contains(&"Free Runs: 0"));
    // A dump of 0x10 bytes from offset 0x30 of region 1, the medium one.
    let (out, err) = session_over(&mut heap, "show arena 0 1 0x30 0x10\nq\n");
    assert_eq!(err, "");
    assert_eq!(
        out.lines().take(2).collect::<Vec<_>>(),
        [
            "Arena Region (0,1 medium):",
            "0x0000_0030 30 31 32 33 34 35 36 37 38 39 3a 3b 3c 3d 3e 3f | 0 1 2 3 4 5 6 7 8 9 : ; < = > ?",
        ]
    );
}

#[test]
fn show_arena_refuses_an_arena_or_dump_that_is_not_there() {
    let mut heap = Heap::default();
    heap.thread(0x1000, 0x8000, 0);
    heap.with_main_arena();
    let (_, err) = session_over(
        &mut heap,
        "show arena 3\nshow arena 0 5\nshow arena 0 1 0 0\nshow arena 0 1 0xf8 0x10\nq\n",
    );
    assert_eq!(
        err,
        "error: no arena 3; type 'list arena' for the arenas\n\
         error: arena 0 has 3 regions: 0 small, 1 medium, 2 huge\n\
         error: a dump needs a size of 1 or more bytes\n\
         error: offset 0xf8 + size 0x10 runs past the end of the medium region's 0x100 \
         handed-out bytes\n"
    );
}

#[test]
fn show_type_reads_a_string_in_an_arena_block_or_a_literal() {
    let mut heap = Heap::default();
    heap.thread(0x1000, 0x8000, 0);
    heap.with_main_arena();
    // x0 points at BLOCK + 0x20.
    heap.put_text(BLOCK + 0x20, "hi");
    let (out, err) = session_over(&mut heap, "show type string x0\nq\n");
    assert_eq!(err, "");
    assert_eq!(out, "Length: 2 Value: \"hi\"\nKilled process 42.\n");
    // A literal in the file's data is shown the same way, and named.
    heap.put_text(0x9000, "lit");
    heap.data_at.insert(0x9000, "_mfb_str_7".to_string());
    let (out, err) = session_over(&mut heap, "show type string 0x9000\nq\n");
    assert_eq!(err, "");
    assert_eq!(
        out.lines().next(),
        Some("Length: 3 Value: \"lit\" (the literal _mfb_str_7)")
    );
    // An arena that has started no block yet holds no values.
    heap.put(REGISTRY_GLOBAL, 0);
    let (_, err) = session_over(&mut heap, "show type string x0\nq\n");
    assert_eq!(
        err,
        "error: no arenas yet: the program's runtime has not started\n"
    );
}

/// The main arena's current-error slot holds an `Error` at 0x30_0000: code,
/// message offset, origin offset.
fn park_error(heap: &mut Heap, code: u64, origin: bool) {
    let error = 0x30_0000;
    heap.put(MAIN_STATE + ARENA_CURRENT_ERROR_OFFSET as u64, error);
    heap.put(error, code);
    heap.put(error + 8, 0x40);
    heap.put_text(error + 0x40, "it \"broke\"");
    if origin {
        heap.put(error + 16, 0x80);
        let location = error + 0x80;
        heap.put(location, 0x40);
        heap.put(location + 8, 12);
        heap.put(location + 16, 5);
        heap.put_text(location + 0x40, "src/main.mfb");
    } else {
        heap.put(error + 16, 0);
    }
}

#[test]
fn show_error_names_the_code_message_and_origin_of_the_error_in_flight() {
    let mut heap = Heap::default();
    heap.thread(0x1000, 0x8000, 0);
    heap.thread(0x1000, 0x9000, 0);
    heap.with_main_arena();
    heap.put(MAIN_STATE + ARENA_CURRENT_ERROR_OFFSET as u64, 0);
    let (out, err) = session_over(&mut heap, "show error\nshow error 1\nshow error 4\nq\n");
    assert_eq!(out.lines().next(), Some("Thread 0 has no error in flight."));
    assert_eq!(
        err,
        "error: thread 1 has no arena: it is running no MFBASIC code\n\
         error: no thread 4: the program has 2 threads; type 'list thread' for them\n"
    );

    let overflow = runtime::error_code_by_name("ErrOverflow").unwrap();
    park_error(&mut heap, overflow as u64, true);
    let (out, err) = session_over(&mut heap, "show error\nq\n");
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "Thread 0's error in flight (the Error at 0x0000000000300000):\n\
             Code: {overflow} (errorCode::ErrOverflow)\n\
             Message: \"it \\\"broke\\\"\"\n\
             Location: src/main.mfb:12:5\n\
             Killed process 42.\n"
        )
    );
    // A code no constant names, and an error with no origin.
    park_error(&mut heap, 987_654, false);
    let (out, _) = session_over(&mut heap, "show error\nq\n");
    assert_eq!(
        out.lines().skip(1).step_by(2).collect::<Vec<_>>(),
        ["Code: 987654", "Location: - (the error has no origin)"]
    );
}

#[test]
fn bt_unwinds_the_current_thread_or_every_thread() {
    let mut heap = Heap::default();
    heap.with_frames();
    // Thread 1 sits at the start of `_main`, called from nowhere known.
    heap.thread(0x1000, 0x9000, 0);
    let (out, err) = session_over(&mut heap, "bt\nbt 1 1\nbt all\nbt 1 7\nq\n");
    assert_eq!(
        err,
        "error: no thread 7: the program has 2 threads; type 'list thread' for them\n"
    );
    assert_eq!(
        out,
        "* thread #0, task = 'MFB Main'\n\
         \x20 * frame #0: 0x0000000000002008 prog`work + 8\n\
         \x20   frame #1: 0x0000000000001008 prog`_main + 8\n\
         \x20 thread #1, task = 'Other'\n\
         \x20   frame #0: 0x0000000000001000 prog`_main\n\
         * thread #0, task = 'MFB Main'\n\
         \x20 * frame #0: 0x0000000000002008 prog`work + 8\n\
         \x20   frame #1: 0x0000000000001008 prog`_main + 8\n\
         \x20 thread #1, task = 'Other'\n\
         \x20   frame #0: 0x0000000000001000 prog`_main\n\
         Killed process 42.\n"
    );
}

#[test]
fn a_thread_that_ends_during_step_all_is_reported_and_stops_being_current() {
    let mut heap = Heap::default();
    heap.with_frames();
    heap.thread(0x1000, 0x9000, 0);
    heap.stop = Some(StepStop::Finished);
    heap.last_ends = true;
    // Thread 1 becomes current, then ends while every thread steps.
    let (out, err) = session_over(&mut heap, "s 1 1\nsa\nbt 1\nj 0x2000\nq\n");
    assert_eq!(
        out,
        "Stepped 1 instruction; thread 1 is at 0x0000000000001000 in _main.\n\
         Thread 0: stepped 1 instruction; at 0x0000000000002008 in work+0x8.\n\
         Thread 1 ended during the step.\n\
         * thread #0, task = 'MFB Main'\n\
         \x20 * frame #0: 0x0000000000002008 prog`work + 8\n\
         Killed process 42.\n"
    );
    assert_eq!(
        err,
        "error: no thread 1: the program has 1 thread; type 'list thread' for them\n"
    );
}

#[test]
fn show_error_on_a_faulted_thread_says_a_fault_is_not_an_error() {
    let mut heap = Heap::default();
    heap.thread(0x1000, 0x8000, 0);
    heap.with_main_arena();
    heap.put(MAIN_STATE + ARENA_CURRENT_ERROR_OFFSET as u64, 0);
    heap.stop = Some(StepStop::Exception("EXC_BAD_ACCESS".to_string()));
    let (out, err) = session_over(&mut heap, "s\nshow error\nq\n");
    assert_eq!(err, "");
    assert_eq!(
        out.lines().nth(1),
        Some(
            "Thread 0 has no error in flight: it stopped on EXC_BAD_ACCESS, which is a fault, \
             not an error a TRAP can catch. Use bt and list reg to see where."
        )
    );
}

#[test]
fn a_function_name_that_names_two_functions_is_refused_as_a_place() {
    let mut heap = Heap::default();
    heap.with_frames();
    heap.functions
        .push(symbol("_mfb_fn_work", false, Some(0x3000), Some(0xc)));
    let (_, err) = session_over(&mut heap, "show mem work 8\nq\n");
    assert_eq!(
        err,
        "error: 'work' names 2 functions, at 0x0000000000002000, 0x0000000000003000; give \
         one by its address\n"
    );
}

#[test]
fn show_type_of_a_data_symbol_that_is_no_literal_reads_the_arena() {
    let mut heap = Heap::default();
    heap.thread(0x1000, 0x8000, 0);
    heap.with_main_arena();
    heap.put_text(BLOCK + 0x20, "hi");
    heap.data_at
        .insert(BLOCK + 0x20, "_mfb_global_name".to_string());
    let (out, err) = session_over(
        &mut heap,
        "show type string x0\nshow type int x0\nshow mem x0 0\nq\n",
    );
    // Read as an Integer, the same bytes are the String's length.
    assert_eq!(
        out,
        "Length: 2 Value: \"hi\"\nValue: 0x0000000000000002 (2)\nKilled process 42.\n"
    );
    assert_eq!(err, "error: cannot show 0 bytes; give 1 to 65536\n");
}

#[test]
fn dis_disassembles_at_the_current_pc_or_a_named_function() {
    let mut heap = Heap::default();
    heap.with_frames();
    let (out, err) = session_over(&mut heap, "dis\ndis work\nq\n");
    assert_eq!(err, "");
    let listing = "prog`work:\n\
                   \x20   0x0000000000002000 <+0>: sub    sp, sp, #0x10\n\
                   \x20   0x0000000000002004 <+4>: str    x30, [sp]\n\
                   ->  0x0000000000002008 <+8>: nop\n";
    // Both start at `work` and mark the current pc.
    assert_eq!(out, format!("{listing}{listing}Killed process 42.\n"));
}

#[test]
fn step_all_that_ends_the_program_after_one_round_says_so() {
    let mut heap = Heap::default();
    heap.thread(0x1000, 0x8000, 0);
    heap.stop = Some(StepStop::Exited);
    let (out, err) = session_over(&mut heap, "list watchpoint\nsa\nq\n");
    assert_eq!(err, "");
    assert_eq!(
        out,
        "no watchpoints\nThe program exited after 1 round of stepping.\nKilled process 42.\n"
    );
}
