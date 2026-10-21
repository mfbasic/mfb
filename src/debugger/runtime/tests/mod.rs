use std::collections::HashMap;

use super::*;
use crate::debugger::{Ending, FunctionRange, FunctionSymbol, Register, RunStop, Step};

/// A process made of a word map and a symbol map.
#[derive(Default)]
struct Memory {
    words: HashMap<u64, u64>,
    symbols: HashMap<&'static str, u64>,
}

impl Memory {
    fn put(&mut self, address: u64, value: u64) {
        self.words.insert(address, value);
    }

    /// A `String` at `address`: its length, then its bytes, 8 to a word.
    fn put_text(&mut self, address: u64, text: &str) {
        self.put(address, text.len() as u64);
        for (index, chunk) in text.as_bytes().chunks(8).enumerate() {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.put(address + 8 + index as u64 * 8, u64::from_le_bytes(word));
        }
    }
}

impl Inferior for Memory {
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
        Ok(Vec::new())
    }
    fn data_symbol(&mut self, name: &str) -> Result<Option<u64>, String> {
        Ok(self.symbols.get(name).copied())
    }
    fn read_code(&mut self, address: u64, _: usize) -> Result<Vec<u8>, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn function_at(&mut self, _: u64) -> Result<Option<FunctionRange>, String> {
        Ok(None)
    }
    fn data_symbol_at(&mut self, _: u64) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
        Ok(Vec::new())
    }
    fn read_bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        // Bytes from the words that hold them, each word little-endian.
        (address..address + length as u64)
            .map(|at| {
                let word = self.read_u64(at & !7)?;
                Ok(word.to_le_bytes()[(at & 7) as usize])
            })
            .collect()
    }
    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        self.words
            .get(&address)
            .copied()
            .ok_or_else(|| format!("unmapped {address:#x}"))
    }
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        Ok(Vec::new())
    }
    fn registers(&mut self, _: usize) -> Result<Vec<Register>, String> {
        Ok(Vec::new())
    }
    fn set_registers(&mut self, _: usize, _: &[(String, u64)]) -> Result<(), String> {
        Err("the memory has no threads".to_string())
    }
    fn step(&mut self, _: &[usize], _: u64) -> Result<Vec<Step>, String> {
        Err("not stepped".to_string())
    }
    fn restart(&mut self, _: &[(std::ffi::OsString, std::ffi::OsString)]) -> Result<u32, String> {
        Err("restart needs a program the debugger launched".to_string())
    }
    fn entry(&mut self) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn insert_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn remove_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn insert_watchpoint(&mut self, _: u64, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn remove_watchpoint(&mut self, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn run(&mut self) -> Result<RunStop, String> {
        Err("not run".to_string())
    }
}

fn thread(handle: u64, arena: u64, stack: (u64, u64)) -> ThreadFacts {
    ThreadFacts {
        id: handle,
        handle,
        name: None,
        pc: 0,
        sp: stack.0,
        arena,
        stack: Some(stack),
    }
}

const TABLE: u64 = 0x1000;
const SLOTS: u64 = 0x2000;
const BLOCK: u64 = 0x3000;
const WORKER_ARENA: u64 = 0x4000;

/// A table with one live worker (handle 0x77) and one free slot.
fn with_worker(memory: &mut Memory) {
    memory.symbols.insert(THREAD_TABLE_SYMBOL, TABLE);
    memory.put(TABLE + THREAD_TABLE_SLOTS as u64, SLOTS);
    memory.put(TABLE + THREAD_TABLE_HIGH as u64, 2);
    memory.put(SLOTS, BLOCK);
    memory.put(SLOTS + 8, (2 << 1) | 1);
    memory.put(BLOCK + CONTROL_OFFSET_OS_HANDLE as u64, 0x77);
    memory.put(BLOCK + CONTROL_OFFSET_WORKER_ARENA as u64, WORKER_ARENA);
    memory.put(WORKER_ARENA + ARENA_WORKER_THREAD_OFFSET as u64, BLOCK);
}

#[test]
fn a_console_program_s_initial_thread_runs_main_and_workers_are_user_threads() {
    let mut memory = Memory::default();
    with_worker(&mut memory);
    let threads = [
        thread(0x11, 0, (0x10_0000, 0x20_0000)),
        thread(0x77, 0, (0x30_0000, 0x40_0000)),
        thread(0x99, 0, (0x50_0000, 0x60_0000)),
    ];
    assert_eq!(
        roles(&mut memory, &threads, false).unwrap(),
        [Role::MfbMain, Role::User, Role::Other]
    );
}

#[test]
fn an_app_s_initial_thread_is_the_app_and_main_is_where_the_main_arena_is() {
    let mut memory = Memory::default();
    memory.symbols.insert(MAIN_ARENA_GLOBAL_SYMBOL, 0x500);
    memory.put(0x500, 0x50_1000);
    let threads = [
        thread(0x11, 0, (0x10_0000, 0x20_0000)),
        thread(0x22, 0, (0x30_0000, 0x40_0000)),
        thread(0x33, 0, (0x50_0000, 0x60_0000)),
    ];
    assert_eq!(
        roles(&mut memory, &threads, true).unwrap(),
        [Role::App, Role::Other, Role::MfbMain]
    );
}

/// A main arena state at `state`: no worker, one 4096-byte block.
fn main_arena_at(memory: &mut Memory, state: u64) {
    memory.put(state + ARENA_WORKER_THREAD_OFFSET as u64, 0);
    memory.put(state + STATE_BLOCK_HEAD, 0x90_0000);
    memory.put(0x90_0000 + BLOCK_SIZE, ARENA_DEFAULT_BLOCK_SIZE);
}

#[test]
fn without_symbols_the_registers_decide() {
    let mut memory = Memory::default();
    memory.put(WORKER_ARENA + ARENA_WORKER_THREAD_OFFSET as u64, BLOCK);
    memory.put(BLOCK + CONTROL_OFFSET_WORKER_ARENA as u64, WORKER_ARENA);
    main_arena_at(&mut memory, 0x30_8000);
    // A system thread's register into its own stack, at a word that is not
    // an arena state.
    memory.put(0x90_8000 + ARENA_WORKER_THREAD_OFFSET as u64, 0);
    memory.put(0x90_8000 + STATE_BLOCK_HEAD, 0x1234);
    let threads = [
        thread(0x11, 0, (0x10_0000, 0x20_0000)),
        // Its arena register points into its own stack at a main arena
        // state: the app's main.
        thread(0x22, 0x30_8000, (0x30_0000, 0x40_0000)),
        thread(0x55, 0x90_8000, (0x90_0000, 0xa0_0000)),
        // Its arena register holds a worker arena that names it back.
        thread(0x33, WORKER_ARENA, (0x50_0000, 0x60_0000)),
        // An arena register that reads nothing is no evidence.
        thread(0x44, 0xdead_0000, (0x70_0000, 0x80_0000)),
    ];
    assert_eq!(
        roles(&mut memory, &threads, true).unwrap(),
        [
            Role::App,
            Role::MfbMain,
            Role::Other,
            Role::User,
            Role::Other
        ]
    );
    assert_eq!(arenas(&mut memory, &threads, &[]).unwrap(), None);
}

#[test]
fn without_symbols_two_threads_that_fit_name_neither() {
    let mut memory = Memory::default();
    main_arena_at(&mut memory, 0x30_8000);
    main_arena_at(&mut memory, 0x50_8000);
    let threads = [
        thread(0x11, 0, (0x10_0000, 0x20_0000)),
        thread(0x22, 0x30_8000, (0x30_0000, 0x40_0000)),
        thread(0x33, 0x50_8000, (0x50_0000, 0x60_0000)),
    ];
    assert_eq!(
        roles(&mut memory, &threads, true).unwrap(),
        [Role::App, Role::Other, Role::Other]
    );
}

#[test]
fn the_registry_lists_live_arenas_with_their_owner_and_mapped_bytes() {
    let mut memory = Memory::default();
    with_worker(&mut memory);
    let region = 0x10_0000;
    memory.symbols.insert(ARENA_BASE_SYMBOL, 0x600);
    memory.put(0x600, region);
    memory.put(region + REGION_COUNT_OFFSET as u64, 4);
    let slot = |id: u64| region + REGION_HEADER_SIZE as u64 + id * SLOT_SIZE as u64;
    let main_arena = 0x20_0000;
    // Slot 0: main, two blocks of 4096 and 8192 bytes.
    memory.put(slot(0), main_arena);
    memory.put(slot(0) + SLOT_KIND_OFFSET as u64, 0);
    memory.put(main_arena + STATE_BLOCK_HEAD, 0x21_0000);
    memory.put(0x21_0000 + BLOCK_SIZE, 8192);
    memory.put(0x21_0000 + BLOCK_PREV, 0x22_0000);
    memory.put(0x22_0000 + BLOCK_SIZE, 4096);
    memory.put(0x22_0000 + BLOCK_PREV, 0);
    // Slot 1: a joined worker, skipped.
    memory.put(slot(1), 0);
    // Slot 2: the live worker, no blocks yet.
    memory.put(slot(2), WORKER_ARENA);
    memory.put(slot(2) + SLOT_KIND_OFFSET as u64, 1);
    memory.put(WORKER_ARENA + STATE_BLOCK_HEAD, 0);
    // Slot 3: a block arena, owned by no thread.
    memory.put(slot(3), 0x23_0000);
    memory.put(slot(3) + SLOT_KIND_OFFSET as u64, 3);
    memory.put(0x23_0000 + STATE_BLOCK_HEAD, 0);
    let threads = [
        thread(0x11, 0, (0x1_0000, 0x2_0000)),
        thread(0x77, 0, (0x3_0000, 0x4_0000)),
    ];
    let roles = roles(&mut memory, &threads, false).unwrap();
    assert_eq!(
        arenas(&mut memory, &threads, &roles).unwrap(),
        Some(vec![
            Arena {
                id: 0,
                thread: Some(0),
                address: main_arena,
                mapped: 12288
            },
            Arena {
                id: 2,
                thread: Some(1),
                address: WORKER_ARENA,
                mapped: 0
            },
            Arena {
                id: 3,
                thread: None,
                address: 0x23_0000,
                mapped: 0
            },
        ])
    );
}

#[test]
fn a_registry_not_yet_made_lists_nothing() {
    let mut memory = Memory::default();
    memory.symbols.insert(ARENA_BASE_SYMBOL, 0x600);
    memory.put(0x600, 0);
    assert_eq!(arenas(&mut memory, &[], &[]).unwrap(), Some(Vec::new()));
}

#[test]
fn a_block_chain_that_loops_is_refused() {
    let mut memory = Memory::default();
    memory.put(0x100 + STATE_BLOCK_HEAD, 0x200);
    memory.put(0x200 + BLOCK_SIZE, 16);
    memory.put(0x200 + BLOCK_PREV, 0x200);
    assert!(mapped_bytes(&mut memory, 0x100)
        .unwrap_err()
        .contains("does not end"));
}

#[test]
fn an_unreadable_thread_table_is_reported() {
    let mut memory = Memory::default();
    memory.symbols.insert(THREAD_TABLE_SYMBOL, TABLE);
    assert!(roles(&mut memory, &[], false).is_err());
    memory.put(TABLE + THREAD_TABLE_SLOTS as u64, SLOTS);
    memory.put(TABLE + THREAD_TABLE_HIGH as u64, MAX_THREAD_SLOTS + 1);
    assert!(roles(&mut memory, &[], false)
        .unwrap_err()
        .contains("cannot be read"));
    memory.put(TABLE + THREAD_TABLE_SLOTS as u64, 0);
    assert_eq!(roles(&mut memory, &[], false).unwrap(), []);
}

#[test]
fn a_pc_is_named_by_the_function_that_holds_it() {
    let function = |name: &str, address, size| FunctionSymbol {
        name: name.to_string(),
        linked: false,
        loaded: false,
        address: Some(address),
        size: Some(size),
    };
    let functions = [
        function("_mfb_fn_isPrime", 0x1000, 0x40),
        function("_mfb_rt_print", 0x1040, 0x10),
        FunctionSymbol {
            name: format!("{}malloc", super::super::names::C_PREFIX),
            linked: true,
            loaded: false,
            address: Some(0x9000),
            size: None,
        },
    ];
    assert_eq!(locate(&functions, 0x1000).as_deref(), Some("isPrime"));
    assert_eq!(locate(&functions, 0x1024).as_deref(), Some("isPrime+0x24"));
    assert_eq!(
        locate(&functions, 0x1044).as_deref(),
        Some("_mfb_rt_print+0x4")
    );
    assert_eq!(locate(&functions, 0x1050), None);
    // A linked function is named only at its first instruction.
    assert_eq!(locate(&functions, 0x9000).as_deref(), Some("malloc"));
    assert_eq!(locate(&functions, 0x9004), None);
}

#[test]
fn every_role_has_its_label() {
    assert_eq!(
        [Role::App, Role::MfbMain, Role::User, Role::Other].map(Role::label),
        ["App", "MFB Main", "User Thread", "Other"]
    );
}

#[test]
fn blocks_list_oldest_first() {
    let mut memory = Memory::default();
    memory.put(0x100 + STATE_BLOCK_HEAD, 0x3000);
    for (block, prev, size) in [(0x3000, 0x2000, 0x2000), (0x2000, 0, 0x1000)] {
        memory.put(block + BLOCK_PREV, prev);
        memory.put(block + BLOCK_SIZE, size);
        memory.put(block + BLOCK_USABLE, size - 32);
        memory.put(block + BLOCK_BUMP, 0);
    }
    let blocks = blocks(&mut memory, 0x100).unwrap();
    assert_eq!(
        blocks
            .iter()
            .map(|block| (block.address, block.prev, block.size))
            .collect::<Vec<_>>(),
        [(0x2000, 0, 0x1000), (0x3000, 0x2000, 0x2000)]
    );
    memory.put(0x2000 + BLOCK_PREV, 0x3000);
    assert!(blocks_err(&mut memory).contains("does not end"));
}

fn blocks_err(memory: &mut Memory) -> String {
    blocks(memory, 0x100).unwrap_err()
}

/// The table covers `ArenaState` end to end: fields in order, no gaps or
/// overlaps but `flagRhs`'s 16 bytes, ending at `ARENA_STATE_SIZE`.
#[test]
fn the_state_fields_tile_the_arena_state() {
    let mut end = 0;
    for field in ARENA_STATE_FIELDS {
        assert_eq!(
            field.offset, end,
            "{} starts at {}",
            field.name, field.offset
        );
        let size = field.count * field.width;
        end = field.offset + if field.name == "flagRhs" { 16 } else { size };
    }
    assert_eq!(end, ARENA_STATE_SIZE);
}

#[test]
fn the_error_slot_names_the_parked_error_s_code_message_and_origin() {
    const ARENA: u64 = 0x10_0000;
    const ERROR: u64 = 0x20_0000;
    let slot = ARENA + ARENA_CURRENT_ERROR_OFFSET as u64;
    let mut memory = Memory::default();
    memory.put(slot, 0);
    assert_eq!(current_error(&mut memory, ARENA), Ok(None));
    // An `Error`: code, message at +0x18, `ErrorLoc` at +0x70, whose file
    // name is at +0x18 from it.
    memory.put(slot, ERROR);
    memory.put(ERROR, 77_050_010);
    memory.put(ERROR + 8, 0x18);
    memory.put(ERROR + 16, 0x70);
    memory.put_text(ERROR + 0x18, "Arithmetic overflow.");
    memory.put(ERROR + 0x70, 0x18);
    memory.put(ERROR + 0x78, 11);
    memory.put(ERROR + 0x80, 34);
    memory.put_text(ERROR + 0x88, "src/main.mfb");
    let error = ParkedError {
        address: ERROR,
        code: 77_050_010,
        message: "Arithmetic overflow.".to_string(),
        source: Some("src/main.mfb:11:34".to_string()),
    };
    assert_eq!(current_error(&mut memory, ARENA), Ok(Some(error.clone())));
    // An error that lost its origin has a zero `ErrorLoc` offset.
    memory.put(ERROR + 16, 0);
    assert_eq!(
        current_error(&mut memory, ARENA),
        Ok(Some(ParkedError {
            source: None,
            ..error
        }))
    );
}

#[test]
fn an_error_code_is_named_by_its_errorcode_constant() {
    assert_eq!(
        error_code_name(77_050_010).as_deref(),
        Some("errorCode::ErrOverflow")
    );
    assert_eq!(
        error_code_name(77_010_001).as_deref(),
        Some("errorCode::ErrOutOfMemory")
    );
    assert_eq!(error_code_name(42), None);
    assert_eq!(error_code_by_name("ErrOverflow"), Some(77_050_010));
    assert_eq!(
        error_code_by_name("errorCode::ErrOutOfMemory"),
        Some(77_010_001)
    );
    assert_eq!(error_code_by_name("ErrNothing"), None);
}
