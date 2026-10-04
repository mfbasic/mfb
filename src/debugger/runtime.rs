//! What the MFBASIC runtime's own data says about a paused program: which
//! thread does what, and where its arenas are.
//!
//! Everything is read from the outside through [`Inferior`], so the layouts
//! used here are the compiler's own constants, never copies. The runtime's
//! globals have names only in a `--debug` build; without them the answers
//! fall back to what the threads' registers show.
//!
//! A thread's role:
//! - **App**: the initial thread of an app-mode program, which runs the window
//!   system and never runs MFBASIC code (`./mfb spec app macos-runtime`).
//! - **MFB Main**: the thread running the program's `main` — the initial thread
//!   of a console program; in an app, the thread whose stack holds the main
//!   arena state (`_mfb_rt_main_arena`), or, without symbols, the one thread
//!   whose arena register points into its own stack at something shaped like a
//!   main arena state (the entry keeps the main arena state on its stack; every
//!   other arena state is in mapped memory). Other threads' registers point
//!   into their stacks too, so the shape is checked, and when more than one
//!   thread fits, none is named.
//! - **User Thread**: a `thread::start` worker — its `pthread_t` is the handle a
//!   live control block in `_mfb_rt_thread_table` records, or its arena register
//!   holds a worker arena state whose control block points back at it.
//! - **Other**: anything else (the Canvas graphics thread, the system's own
//!   threads, a `process::detach` reaper).

use super::{Inferior, ThreadFacts};
use crate::codegen::debug::arena::{
    ARENA_BASE_SYMBOL, ARENA_DEBUG_SLOTS, ARENA_KIND_GRAPHICS, ARENA_KIND_MAIN,
    ARENA_KIND_THREAD_BLOCK, ARENA_KIND_WORKER, REGION_COUNT_OFFSET, REGION_HEADER_SIZE,
    SLOT_KIND_OFFSET, SLOT_SIZE,
};
use crate::codegen::error::constants::error_constants::{
    ARENA_CARVE_PTR_OFFSET, ARENA_CARVE_SIZE_OFFSET, ARENA_CLEANUP_FAILURE_CODE_OFFSET,
    ARENA_CLEANUP_FAILURE_COUNT_OFFSET, ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
    ARENA_CURRENT_ERROR_OFFSET, ARENA_DEFAULT_BLOCK_SIZE, ARENA_FILL_RNG_HI_OFFSET,
    ARENA_FILL_RNG_LO_OFFSET, ARENA_FLAG_RHS_OFFSET, ARENA_FREE_LIST_HEAD_OFFSET,
    ARENA_LARGE_BIN_BASE_OFFSET, ARENA_LARGE_BIN_COUNT, ARENA_LARGE_FLUSH_CREDIT_OFFSET,
    ARENA_OUT_ENABLED_OFFSET, ARENA_OUT_FILLED_OFFSET, ARENA_OUT_PTR_OFFSET,
    ARENA_QUICK_BIN_BASE_OFFSET, ARENA_QUICK_BIN_COUNT, ARENA_RNG_STATE_HI_OFFSET,
    ARENA_RNG_STATE_LO_OFFSET, ARENA_ROUNDING_MODE_OFFSET, ARENA_START_TIME_OFFSET,
    ARENA_STATE_SIZE, ARENA_STDIN_LOCAL_BUF_OFFSET, ARENA_STDIN_LOCAL_FILLED_OFFSET,
    ARENA_STDIN_LOCAL_POS_OFFSET, ARENA_STDIN_SUBSCRIBER_OFFSET, ARENA_V128_SLOTS_OFFSET,
    ARENA_V128_SLOTS_SIZE, ARENA_WORKER_THREAD_OFFSET, MAIN_ARENA_GLOBAL_SYMBOL,
};
use crate::codegen::runtime::canvas::{GRAPHICS_OFFSET_TID, GRAPHICS_STATE_SYMBOL};
use crate::codegen::runtime::thread::control_block::{
    CONTROL_OFFSET_OS_HANDLE, CONTROL_OFFSET_WORKER_ARENA, THREAD_TABLE_HIGH, THREAD_TABLE_SLOTS,
    THREAD_TABLE_SYMBOL,
};

/// `ArenaState.blockHead` and `exitStatus`, and a block header's fields
/// (`./mfb spec memory arenas`); the arena code writes these as literals.
const STATE_BLOCK_HEAD: u64 = 0;
const STATE_EXIT_STATUS: usize = 32;
const BLOCK_PREV: u64 = 0;
const BLOCK_SIZE: u64 = 8;
const BLOCK_USABLE: u64 = 16;
const BLOCK_BUMP: u64 = 24;

/// One field of `ArenaState`: a word, or an array of `count` entries of
/// `width` bytes.
pub(crate) struct Field {
    pub(crate) name: &'static str,
    pub(crate) offset: usize,
    pub(crate) count: usize,
    pub(crate) width: usize,
}

const fn word(name: &'static str, offset: usize) -> Field {
    Field {
        name,
        offset,
        count: 1,
        width: 8,
    }
}

/// `ArenaState`, in layout order, from the compiler's own offsets.
pub(crate) const ARENA_STATE_FIELDS: &[Field] = &[
    word("blockHead", STATE_BLOCK_HEAD as usize),
    word("workerThread", ARENA_WORKER_THREAD_OFFSET),
    word("fillRngLo", ARENA_FILL_RNG_LO_OFFSET),
    word("fillRngHi", ARENA_FILL_RNG_HI_OFFSET),
    word("exitStatus", STATE_EXIT_STATUS),
    word("arenaStartTime", ARENA_START_TIME_OFFSET),
    word("freeListHead", ARENA_FREE_LIST_HEAD_OFFSET),
    word("moneyRoundingMode", ARENA_ROUNDING_MODE_OFFSET),
    word("cleanupFailCount", ARENA_CLEANUP_FAILURE_COUNT_OFFSET),
    word("cleanupFailCode", ARENA_CLEANUP_FAILURE_CODE_OFFSET),
    word("cleanupFailMsg", ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET),
    word("rngStateLo", ARENA_RNG_STATE_LO_OFFSET),
    word("rngStateHi", ARENA_RNG_STATE_HI_OFFSET),
    Field {
        name: "quickBin",
        offset: ARENA_QUICK_BIN_BASE_OFFSET,
        count: ARENA_QUICK_BIN_COUNT,
        width: 8,
    },
    word("carvePtr", ARENA_CARVE_PTR_OFFSET),
    word("carveSize", ARENA_CARVE_SIZE_OFFSET),
    word("outPtr", ARENA_OUT_PTR_OFFSET),
    word("outFilled", ARENA_OUT_FILLED_OFFSET),
    word("outEnabled", ARENA_OUT_ENABLED_OFFSET),
    Field {
        name: "largeBin",
        offset: ARENA_LARGE_BIN_BASE_OFFSET,
        count: ARENA_LARGE_BIN_COUNT,
        width: 8,
    },
    Field {
        name: "v128Slots",
        offset: ARENA_V128_SLOTS_OFFSET,
        count: ARENA_V128_SLOTS_SIZE / 16,
        width: 16,
    },
    word("flagRhs", ARENA_FLAG_RHS_OFFSET),
    word("currentError", ARENA_CURRENT_ERROR_OFFSET),
    word("stdinLocalBuf", ARENA_STDIN_LOCAL_BUF_OFFSET),
    word("stdinLocalFilled", ARENA_STDIN_LOCAL_FILLED_OFFSET),
    word("stdinLocalPos", ARENA_STDIN_LOCAL_POS_OFFSET),
    word("stdinSubscriber", ARENA_STDIN_SUBSCRIBER_OFFSET),
    word("largeFlushCredit", ARENA_LARGE_FLUSH_CREDIT_OFFSET),
];

/// One mapped block of an arena, as its header reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    pub(crate) address: u64,
    pub(crate) prev: u64,
    pub(crate) size: u64,
    pub(crate) usable: u64,
    pub(crate) bump: u64,
}

/// The arena state at `state`, all `ARENA_STATE_SIZE` bytes.
pub(crate) fn arena_state(inferior: &mut dyn Inferior, state: u64) -> Result<Vec<u8>, String> {
    inferior.read_bytes(state, ARENA_STATE_SIZE)
}

/// The arena's blocks, oldest first: the chain from `blockHead` through each
/// `prevBlock`, reversed.
pub(crate) fn blocks(inferior: &mut dyn Inferior, state: u64) -> Result<Vec<Block>, String> {
    let mut blocks = Vec::new();
    let mut address = inferior.read_u64(state + STATE_BLOCK_HEAD)?;
    while address != 0 {
        if blocks.len() == MAX_BLOCKS {
            return Err(format!(
                "the block chain of the arena at {state:#x} does not end; it cannot be read"
            ));
        }
        let block = Block {
            address,
            prev: inferior.read_u64(address + BLOCK_PREV)?,
            size: inferior.read_u64(address + BLOCK_SIZE)?,
            usable: inferior.read_u64(address + BLOCK_USABLE)?,
            bump: inferior.read_u64(address + BLOCK_BUMP)?,
        };
        address = block.prev;
        blocks.push(block);
    }
    blocks.reverse();
    Ok(blocks)
}
/// More blocks than any arena maps: a chain this long is a corrupt read.
const MAX_BLOCKS: usize = 1 << 22;
/// More thread-table slots than the table can hold.
const MAX_THREAD_SLOTS: u64 = 1 << 24;

/// What a thread is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    App,
    MfbMain,
    User,
    Other,
}

impl Role {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Role::App => "App",
            Role::MfbMain => "MFB Main",
            Role::User => "User Thread",
            Role::Other => "Other",
        }
    }
}

/// The role of each of `threads`, in order.
pub(crate) fn roles(
    inferior: &mut dyn Inferior,
    threads: &[ThreadFacts],
    app: bool,
) -> Result<Vec<Role>, String> {
    let workers = worker_handles(inferior)?;
    let main_arena = match inferior.data_symbol(MAIN_ARENA_GLOBAL_SYMBOL)? {
        Some(global) => Some(inferior.read_u64(global)?),
        None => None,
    };
    let mut roles = Vec::with_capacity(threads.len());
    for (index, thread) in threads.iter().enumerate() {
        let role = if index == 0 {
            if app {
                Role::App
            } else {
                Role::MfbMain
            }
        } else if workers
            .as_ref()
            .is_some_and(|handles| thread.handle != 0 && handles.contains(&thread.handle))
            || runs_a_worker_arena(inferior, thread)
        {
            Role::User
        } else if app && runs_main(inferior, thread, main_arena) {
            Role::MfbMain
        } else {
            Role::Other
        };
        roles.push(role);
    }
    // Without the main arena's address the shape test can match more than one
    // thread; then it names none rather than guess.
    if main_arena.is_none()
        && roles
            .iter()
            .skip(1)
            .filter(|role| **role == Role::MfbMain)
            .count()
            > 1
    {
        for role in roles.iter_mut().skip(1) {
            if *role == Role::MfbMain {
                *role = Role::Other;
            }
        }
    }
    Ok(roles)
}

/// The `pthread_t` of every live `thread::start` worker, from the thread table;
/// `None` when the program carries no thread table symbol.
fn worker_handles(inferior: &mut dyn Inferior) -> Result<Option<Vec<u64>>, String> {
    let Some(table) = inferior.data_symbol(THREAD_TABLE_SYMBOL)? else {
        return Ok(None);
    };
    let slots = inferior.read_u64(table + THREAD_TABLE_SLOTS as u64)?;
    let high = inferior.read_u64(table + THREAD_TABLE_HIGH as u64)?;
    if slots == 0 {
        return Ok(Some(Vec::new()));
    }
    if high > MAX_THREAD_SLOTS {
        return Err(format!(
            "the thread table names {high} slots; it cannot be read"
        ));
    }
    let mut handles = Vec::new();
    for index in 0..high {
        let block = inferior.read_u64(slots + index * 8)?;
        // A free slot holds an odd link; a used one an 8-aligned block pointer.
        if block == 0 || block & 1 == 1 {
            continue;
        }
        let handle = inferior.read_u64(block + CONTROL_OFFSET_OS_HANDLE as u64)?;
        if handle != 0 {
            handles.push(handle);
        }
    }
    Ok(Some(handles))
}

/// Whether the thread's arena register holds a worker arena state whose
/// control block names it back. A read that fails means it does not.
fn runs_a_worker_arena(inferior: &mut dyn Inferior, thread: &ThreadFacts) -> bool {
    if thread.arena == 0 {
        return false;
    }
    let Ok(block) = inferior.read_u64(thread.arena + ARENA_WORKER_THREAD_OFFSET as u64) else {
        return false;
    };
    if block == 0 || block & 7 != 0 {
        return false;
    }
    inferior
        .read_u64(block + CONTROL_OFFSET_WORKER_ARENA as u64)
        .is_ok_and(|arena| arena == thread.arena)
}

/// Whether the thread runs `main`: its stack holds the main arena state (named
/// by `_mfb_rt_main_arena` once the entry has run), or, without that symbol,
/// its arena register points into its own stack at a main arena state.
fn runs_main(inferior: &mut dyn Inferior, thread: &ThreadFacts, main_arena: Option<u64>) -> bool {
    let Some((start, end)) = thread.stack else {
        return false;
    };
    let within = |address: u64| address != 0 && start <= address && address < end;
    match main_arena {
        Some(arena) => within(arena),
        None => within(thread.arena) && shaped_like_main_arena(inferior, thread.arena),
    }
}

/// Whether `state` reads as the main arena state: no worker control block,
/// and a newest block whose header gives a whole number of default-sized
/// blocks. A failed read means no.
fn shaped_like_main_arena(inferior: &mut dyn Inferior, state: u64) -> bool {
    if inferior.read_u64(state + ARENA_WORKER_THREAD_OFFSET as u64) != Ok(0) {
        return false;
    }
    let Ok(block) = inferior.read_u64(state + STATE_BLOCK_HEAD) else {
        return false;
    };
    if block == 0 || block % 16 != 0 {
        return false;
    }
    inferior
        .read_u64(block + BLOCK_SIZE)
        .is_ok_and(|size| size >= ARENA_DEFAULT_BLOCK_SIZE && size % ARENA_DEFAULT_BLOCK_SIZE == 0)
}

/// One live arena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Arena {
    /// The arena's slot in the debug registry.
    pub(crate) id: usize,
    /// The thread that allocates from it (an index into the threads); `None`
    /// for a thread's block arena, which only carries values between threads.
    pub(crate) thread: Option<usize>,
    /// The arena state's address.
    pub(crate) address: u64,
    /// Bytes of memory the arena has mapped.
    pub(crate) mapped: u64,
}

/// Every live arena in the `--debug` registry, or `None` when the program has
/// no registry (a build without `--debug`). An empty list means the runtime
/// has not started yet.
pub(crate) fn arenas(
    inferior: &mut dyn Inferior,
    threads: &[ThreadFacts],
    roles: &[Role],
) -> Result<Option<Vec<Arena>>, String> {
    let Some(global) = inferior.data_symbol(ARENA_BASE_SYMBOL)? else {
        return Ok(None);
    };
    let region = inferior.read_u64(global)?;
    if region == 0 {
        return Ok(Some(Vec::new()));
    }
    let count = inferior
        .read_u64(region + REGION_COUNT_OFFSET as u64)?
        .min(ARENA_DEBUG_SLOTS as u64);
    let graphics = match inferior.data_symbol(GRAPHICS_STATE_SYMBOL)? {
        Some(state) => Some(inferior.read_u64(state + GRAPHICS_OFFSET_TID as u64)?),
        None => None,
    };
    let by_handle = |handle: u64| {
        threads
            .iter()
            .position(|thread| handle != 0 && thread.handle == handle)
    };
    let mut arenas = Vec::new();
    for id in 0..count as usize {
        let slot = region + (REGION_HEADER_SIZE + id * SLOT_SIZE) as u64;
        let state = inferior.read_u64(slot)?;
        if state == 0 {
            // A worker that has been joined: its slot keeps its counts only.
            continue;
        }
        let kind = inferior
            .read_u64(slot + SLOT_KIND_OFFSET as u64)?
            .to_string();
        let by_register = threads.iter().position(|thread| thread.arena == state);
        let thread = if kind == ARENA_KIND_MAIN {
            roles.iter().position(|role| *role == Role::MfbMain)
        } else if kind == ARENA_KIND_WORKER {
            let block = inferior.read_u64(state + ARENA_WORKER_THREAD_OFFSET as u64)?;
            let handle = if block == 0 {
                0
            } else {
                inferior.read_u64(block + CONTROL_OFFSET_OS_HANDLE as u64)?
            };
            by_handle(handle).or(by_register)
        } else if kind == ARENA_KIND_GRAPHICS {
            graphics.and_then(by_handle).or(by_register)
        } else if kind == ARENA_KIND_THREAD_BLOCK {
            None
        } else {
            by_register
        };
        arenas.push(Arena {
            id,
            thread,
            address: state,
            mapped: mapped_bytes(inferior, state)?,
        });
    }
    Ok(Some(arenas))
}

/// The bytes of every block the arena has mapped: the chain from
/// `blockHead` through each block's `prevBlock`.
fn mapped_bytes(inferior: &mut dyn Inferior, state: u64) -> Result<u64, String> {
    let mut total = 0u64;
    let mut block = inferior.read_u64(state + STATE_BLOCK_HEAD)?;
    for _ in 0..MAX_BLOCKS {
        if block == 0 {
            return Ok(total);
        }
        total = total.saturating_add(inferior.read_u64(block + BLOCK_SIZE)?);
        block = inferior.read_u64(block + BLOCK_PREV)?;
    }
    Err(format!(
        "the block chain of the arena at {state:#x} does not end; it cannot be read"
    ))
}

/// Where `pc` is, as `<name>+0x<offset>` for a function the executable defines
/// (the written name when there is one), or the name of a linked function that
/// starts exactly there; `None` anywhere else.
pub(crate) fn locate(functions: &[super::FunctionSymbol], pc: u64) -> Option<String> {
    if let Some(function) = functions.iter().find(|function| {
        !function.linked
            && function
                .address
                .is_some_and(|address| address <= pc && pc - address < function.size.unwrap_or(0))
    }) {
        let offset = pc - function.address?;
        let (_, written) = super::names::classify_defined(&function.name);
        let name = written.unwrap_or_else(|| function.name.clone());
        return Some(if offset == 0 {
            name
        } else {
            format!("{name}+{offset:#x}")
        });
    }
    functions
        .iter()
        .find(|function| function.linked && function.address == Some(pc))
        .map(|function| super::names::linked_name(&function.name))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::debugger::{Ending, FunctionSymbol, Register, RunStop, Step};

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
    }

    impl Inferior for Memory {
        fn pid(&self) -> u32 {
            1
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
        fn read_bytes(&mut self, address: u64, _: usize) -> Result<Vec<u8>, String> {
            Err(format!("unmapped {address:#x}"))
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
        fn step(&mut self, _: &[usize], _: u64) -> Result<Vec<Step>, String> {
            Err("not stepped".to_string())
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
            address: Some(address),
            size: Some(size),
        };
        let functions = [
            function("_mfb_fn_isPrime", 0x1000, 0x40),
            function("_mfb_rt_print", 0x1040, 0x10),
            FunctionSymbol {
                name: "_malloc".to_string(),
                linked: true,
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
}
