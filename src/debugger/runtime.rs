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
    ARENA_CURRENT_ERROR_OFFSET, ARENA_DEFAULT_BLOCK_SIZE, ARENA_FLAG_RHS_OFFSET,
    ARENA_FREE_LIST_HEAD_OFFSET, ARENA_LARGE_BIN_BASE_OFFSET, ARENA_LARGE_BIN_COUNT,
    ARENA_LARGE_FLUSH_CREDIT_OFFSET, ARENA_OUT_ENABLED_OFFSET, ARENA_OUT_FILLED_OFFSET,
    ARENA_OUT_PTR_OFFSET, ARENA_QUICK_BIN_BASE_OFFSET, ARENA_QUICK_BIN_COUNT,
    ARENA_RESERVED_16_OFFSET, ARENA_RESERVED_24_OFFSET, ARENA_RNG_STATE_HI_OFFSET,
    ARENA_RNG_STATE_LO_OFFSET, ARENA_ROUNDING_MODE_OFFSET, ARENA_STACK_LIMIT_OFFSET,
    ARENA_STACK_LIMIT_SCRATCH_OFFSET, ARENA_START_TIME_OFFSET, ARENA_STATE_SIZE,
    ARENA_STDIN_LOCAL_BUF_OFFSET, ARENA_STDIN_LOCAL_FILLED_OFFSET, ARENA_STDIN_LOCAL_POS_OFFSET,
    ARENA_STDIN_SUBSCRIBER_OFFSET, ARENA_V128_SLOTS_OFFSET, ARENA_V128_SLOTS_SIZE,
    ARENA_WORKER_THREAD_OFFSET, MAIN_ARENA_GLOBAL_SYMBOL,
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
    word("reserved16", ARENA_RESERVED_16_OFFSET),
    word("reserved24", ARENA_RESERVED_24_OFFSET),
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
    // bug-789: the thread's stack limit and its scratch word, ahead of the v128
    // region.
    word("stackLimit", ARENA_STACK_LIMIT_OFFSET),
    word("stackLimitScratch", ARENA_STACK_LIMIT_SCRATCH_OFFSET),
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
                "the block chain of the arena at 0x{state:016x} does not end; it cannot be read"
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
        "the block chain of the arena at 0x{state:016x} does not end; it cannot be read"
    ))
}

/// The error a thread's arena holds in its current-error slot
/// (`ARENA_CURRENT_ERROR_OFFSET`): the flat `Error` block a failing operation
/// parks there for whoever catches it (`./mfb spec memory fallible-call-abi`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParkedError {
    /// The block's address.
    pub(crate) address: u64,
    pub(crate) code: i64,
    pub(crate) message: String,
    /// `<file>:<line>:<char>`; `None` for an error with no origin.
    pub(crate) source: Option<String>,
}

/// The longest text a parked error's message or file name is read to.
const MAX_ERROR_TEXT: u64 = 4096;

/// The error parked in the arena at `arena`, or `None` when its slot is empty.
/// The block is a flat `Error` (`./mfb spec memory heap-values`): `+0` the code,
/// `+8` the message's offset, `+16` the `ErrorLoc`'s offset (0 for none); an
/// `ErrorLoc` is `+0` its file name's offset from itself, `+8` the line,
/// `+16` the character; each `String` its length, then its bytes.
pub(crate) fn current_error(
    inferior: &mut dyn Inferior,
    arena: u64,
) -> Result<Option<ParkedError>, String> {
    let address = inferior.read_u64(arena + ARENA_CURRENT_ERROR_OFFSET as u64)?;
    if address == 0 {
        return Ok(None);
    }
    let code = inferior.read_u64(address)? as i64;
    let message_at = address + inferior.read_u64(address + 8)?;
    let message = error_text(inferior, message_at)?;
    let source = match inferior.read_u64(address + 16)? {
        0 => None,
        offset => Some(error_origin(inferior, address + offset)?),
    };
    Ok(Some(ParkedError {
        address,
        code,
        message,
        source,
    }))
}

/// A `String` in an error: its length, then its bytes (at most
/// [`MAX_ERROR_TEXT`] of them read).
fn error_text(inferior: &mut dyn Inferior, at: u64) -> Result<String, String> {
    let length = inferior.read_u64(at)?.min(MAX_ERROR_TEXT);
    let bytes = inferior.read_bytes(at + 8, length as usize)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The `ErrorLoc` at `location` as `<file>:<line>:<char>`: `+0` its file
/// name's offset from itself, `+8` the line, `+16` the character.
pub(crate) fn error_origin(inferior: &mut dyn Inferior, location: u64) -> Result<String, String> {
    let file_at = location + inferior.read_u64(location)?;
    let file = error_text(inferior, file_at)?;
    let line = inferior.read_u64(location + 8)?;
    let char = inferior.read_u64(location + 16)?;
    Ok(format!("{file}:{line}:{char}"))
}

/// The code an `errorCode` constant names, written `ErrOverflow` or
/// `errorCode::ErrOverflow`; `None` for any other word.
pub(crate) fn error_code_by_name(name: &str) -> Option<i64> {
    let name = name.strip_prefix("errorCode::").unwrap_or(name);
    crate::codegen::registry::registry()
        .packages()
        .iter()
        .find(|package| package.import_name() == "errorCode")?
        .constants()
        .iter()
        .find(|constant| constant.name == name)?
        .value?
        .parse()
        .ok()
}

/// The `errorCode` constant a code is, as a program names it
/// (`errorCode::ErrOverflow`), or `None` for a code the program made up.
pub(crate) fn error_code_name(code: i64) -> Option<String> {
    let value = code.to_string();
    crate::codegen::registry::registry()
        .packages()
        .iter()
        .find(|package| package.import_name() == "errorCode")?
        .constants()
        .iter()
        .find(|constant| constant.value == Some(value.as_str()))
        .map(|constant| format!("errorCode::{}", constant.name))
}

/// The function `pc` is in and how far in: one the executable defines, or a
/// linked function that starts exactly there.
fn holder(functions: &[super::FunctionSymbol], pc: u64) -> Option<(&super::FunctionSymbol, u64)> {
    functions
        .iter()
        .find(|function| {
            !function.linked
                && function.address.is_some_and(|address| {
                    address <= pc && pc - address < function.size.unwrap_or(0)
                })
        })
        .and_then(|function| Some((function, pc - function.address?)))
        .or_else(|| {
            functions
                .iter()
                .find(|function| function.linked && function.address == Some(pc))
                .map(|function| (function, 0))
        })
}

/// Where `pc` is, as `<name>+0x<offset>` for a function the executable defines
/// (the written name when there is one, else the symbol), or the name of a
/// linked function that starts exactly there; `None` anywhere else. A name an
/// MFBASIC and a native function share carries its tag (`exit <mfb>`).
pub(crate) fn locate(functions: &[super::FunctionSymbol], pc: u64) -> Option<String> {
    let collisions = super::names::Collisions::of(functions);
    let (function, offset) = holder(functions, pc)?;
    let (origin, written) = super::names::classify(function);
    let tag = written
        .as_deref()
        .and_then(|written| collisions.tag(written, origin));
    let name = written.unwrap_or_else(|| function.name.clone());
    let place = if offset == 0 {
        name
    } else {
        format!("{name}+{offset:#x}")
    };
    Some(match tag {
        Some(tag) => format!("{place} {tag}"),
        None => place,
    })
}

/// Where `pc` is as a stop line names it, `(<written name>) <symbol>+0x<offset>`
/// ([`super::names::stop_name`]), for the functions [`locate`] knows.
pub(crate) fn locate_stop(functions: &[super::FunctionSymbol], pc: u64) -> Option<String> {
    let (function, offset) = holder(functions, pc)?;
    let written = super::names::classify(function).1;
    Some(super::names::stop_name(
        written.as_deref(),
        &function.name,
        offset,
    ))
}

#[cfg(test)]
mod tests;
