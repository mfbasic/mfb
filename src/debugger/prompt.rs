//! The `DBG>` command prompt: read one command per line, run it against the
//! session's [`Inferior`], repeat until `quit` or end of input.
//!
//! The prompt works the same over a terminal and over a pipe, so a script (or
//! another program) can drive a session by writing commands to stdin. Two
//! choices serve the pipe: the prompt is flushed before every read, so a reader
//! can wait for `DBG> ` to know the debugger is ready; and when stdin is not a
//! terminal each command is echoed after the prompt, so the transcript reads the
//! same as an interactive one. End of input ends the session exactly as `quit`.

use std::io::{BufRead, Write};

use super::breakpoints::{Breakpoints, Finish, Target};
use super::disassemble::{self, Request};
use super::names::{classify_defined, linked_name, Origin};
use super::runtime::{self, Arena, Block, Role, ARENA_STATE_FIELDS};
use super::{
    Ending, Exit, FunctionSymbol, Inferior, Program, Register, RegisterValue, RunStop, StepStop,
    ThreadFacts,
};

pub(crate) const PROMPT: &str = "DBG> ";

/// The `help` listing.
pub(crate) const HELP: &str = "\
Commands:
  help, h         Show this list of commands
  list, l         Show what can be listed
  show            Show memory: show mem <address|register[+offset]> [<size>]
                  An arena's state and blocks: show arena [<id>]
                  Dump a block: show arena <id> <block> [<offset>] [<size>]
  breakpoint, bp  Set a breakpoint: breakpoint <address|function>
                  Remove one: breakpoint r <address|function>
  run, r          Run the program until a breakpoint or its end (Ctrl-C pauses)
  finish, f       Run until the current function returns
  restart, rs     Start the program again (a program the debugger launched)
  disassemble, dis
                  Disassemble the current function, dis <count> instructions
                  from the current one, or dis <function>
  step, s         Run one thread: step [<count>] [<thread>] (default 1, thread 0)
  step-all, sa    Run every thread together: step-all [<count>] (default 1)
  quit, q         End the session: kill the program if the debugger launched
                  it, or detach and let it run on if the debugger attached to it";

/// What a bare `list` prints.
pub(crate) const LIST_USAGE: &str = "\
Usage: list <what>

  func          The program's functions, one per line:
                  <type> <address> <name> <written name>
                <type> is MU for a function the program's author wrote, MR for
                the MFBASIC runtime, LN for a function in a linked library.
                <address> is - for a linked function the program has not
                loaded yet, and <written name> is - where the author wrote no
                name.
  thread        The program's threads: <id> <name> <pc> <task>
                <task> is App, MFB Main, User Thread or Other.
  reg [<id>] [<register>...]
                The registers of thread <id> (default 0, the main thread), or
                just the ones named: <register> <hex value> (<decimal>)
  arena         The program's MFBASIC memory arenas (debug builds):
                  <id> <thread id> <address> <size in KB>
  breakpoint    The breakpoints: <address> <name> <written name>
                <address> is - for one waiting for its library to load.";

/// The thread a bare `step` runs and a bare `list reg` shows: the initial
/// thread.
const MAIN_THREAD: usize = 0;

/// One parsed command line.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Help,
    List(Option<Listing>),
    Step { count: u64, thread: usize },
    StepAll(u64),
    ShowArena { id: usize, dump: Option<Dump> },
    SetBreakpoint(Target),
    RemoveBreakpoint(Target),
    Run,
    Disassemble(Request),
    Finish,
    Restart,
    ShowMemory { location: Location, size: u64 },
    Quit,
    Empty,
}

/// Which bytes of an arena block `show arena` dumps.
#[derive(Debug, PartialEq, Eq)]
struct Dump {
    block: usize,
    offset: u64,
    size: u64,
}

/// Where `show mem` reads: an address, or a register of the current thread
/// plus an offset.
#[derive(Debug, PartialEq, Eq)]
enum Location {
    Address(u64),
    Register { name: String, offset: i64 },
}

impl Location {
    /// `0x…`, or `<register>`, `<register>+<n>`, `<register>-<n>` (`n` decimal
    /// or `0x` hex).
    fn parse(word: &str) -> Result<Location, String> {
        if word.starts_with("0x") || word.starts_with("0X") {
            return number(word).map(Location::Address);
        }
        let (name, offset) = match word.find(['+', '-']) {
            Some(at) => {
                let amount = number(&word[at + 1..])?;
                let amount = i64::try_from(amount).map_err(|_| format!("'{word}' is too far"))?;
                let negative = word[at..].starts_with('-');
                (&word[..at], if negative { -amount } else { amount })
            }
            None => (word, 0),
        };
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(format!("'{word}' is not an address or a register"));
        }
        Ok(Location::Register {
            name: name.to_string(),
            offset,
        })
    }
}

/// How much `show mem` shows when no size is given, and the most it shows.
const MEMORY_SIZE: u64 = 64;
const MAX_MEMORY: u64 = 64 << 10;

/// The arena a bare `show arena` shows: the main arena's registry slot.
const MAIN_ARENA: usize = 0;
/// How much of a block a dump shows when no size is given.
const DUMP_SIZE: u64 = 1024;

#[derive(Debug, PartialEq, Eq)]
enum Listing {
    Functions,
    Threads,
    Registers { thread: usize, names: Vec<String> },
    Arenas,
    Breakpoints,
}

fn parse(line: &str) -> Result<Command, String> {
    let words = line.split_whitespace().collect::<Vec<_>>();
    let Some((&name, arguments)) = words.split_first() else {
        return Ok(Command::Empty);
    };
    match (name, arguments) {
        ("help" | "h", []) => Ok(Command::Help),
        ("quit" | "q", []) => Ok(Command::Quit),
        ("help" | "h" | "quit" | "q", _) => Err(format!("'{name}' takes no arguments")),
        ("list" | "l", []) => Ok(Command::List(None)),
        ("list" | "l", ["func"]) => Ok(Command::List(Some(Listing::Functions))),
        ("list" | "l", ["thread"]) => Ok(Command::List(Some(Listing::Threads))),
        ("list" | "l", ["arena"]) => Ok(Command::List(Some(Listing::Arenas))),
        ("list" | "l", ["breakpoint"]) => Ok(Command::List(Some(Listing::Breakpoints))),
        ("list" | "l", ["reg", rest @ ..]) => {
            let (thread, names) = match rest.first() {
                Some(first) if first.bytes().all(|byte| byte.is_ascii_digit()) => {
                    (thread_id(first)?, &rest[1..])
                }
                _ => (MAIN_THREAD, rest),
            };
            Ok(Command::List(Some(Listing::Registers {
                thread,
                names: names.iter().map(|name| name.to_string()).collect(),
            })))
        }
        ("list" | "l", ["func" | "thread" | "arena" | "breakpoint", ..]) => {
            Err(format!("too many arguments to '{name} {}'", arguments[0]))
        }
        ("list" | "l", [other, ..]) => Err(format!(
            "cannot list '{other}'; type 'list' for what can be listed"
        )),
        ("step" | "s", []) => Ok(Command::Step {
            count: 1,
            thread: MAIN_THREAD,
        }),
        ("step" | "s", [count]) => Ok(Command::Step {
            count: step_count(count)?,
            thread: MAIN_THREAD,
        }),
        ("step" | "s", [count, thread]) => Ok(Command::Step {
            count: step_count(count)?,
            thread: thread_id(thread)?,
        }),
        ("step" | "s", _) => Err(format!("'{name}' takes at most two arguments")),
        ("step-all" | "sa", []) => Ok(Command::StepAll(1)),
        ("step-all" | "sa", [count]) => Ok(Command::StepAll(step_count(count)?)),
        ("step-all" | "sa", _) => Err(format!("'{name}' takes at most one argument")),
        ("breakpoint" | "bp", ["r"]) | ("breakpoint" | "bp", []) => Err(format!(
            "'{name}' needs an address (0x…) or a function name"
        )),
        ("breakpoint" | "bp", ["r", target]) => {
            Ok(Command::RemoveBreakpoint(Target::parse(target)?))
        }
        ("breakpoint" | "bp", [target]) => Ok(Command::SetBreakpoint(Target::parse(target)?)),
        ("breakpoint" | "bp", _) => Err(format!("too many arguments to '{name}'")),
        ("show", ["arena"]) => Ok(Command::ShowArena {
            id: MAIN_ARENA,
            dump: None,
        }),
        ("show", ["arena", id]) => Ok(Command::ShowArena {
            id: arena_id(id)?,
            dump: None,
        }),
        ("show", ["arena", id, block, rest @ ..]) if rest.len() <= 2 => Ok(Command::ShowArena {
            id: arena_id(id)?,
            dump: Some(Dump {
                block: block
                    .parse::<usize>()
                    .map_err(|_| format!("'{block}' is not a block number"))?,
                offset: rest.first().map_or(Ok(0), |word| number(word))?,
                size: rest.get(1).map_or(Ok(DUMP_SIZE), |word| number(word))?,
            }),
        }),
        ("show", ["arena", ..]) => Err("too many arguments to 'show arena'".to_string()),
        ("show", ["mem"]) => {
            Err("'show mem' needs an address or a register: show mem <where> [<size>]".to_string())
        }
        ("show", ["mem", location]) => Ok(Command::ShowMemory {
            location: Location::parse(location)?,
            size: MEMORY_SIZE,
        }),
        ("show", ["mem", location, size]) => Ok(Command::ShowMemory {
            location: Location::parse(location)?,
            size: number(size)?,
        }),
        ("show", ["mem", ..]) => Err("too many arguments to 'show mem'".to_string()),
        ("show", []) => Err(
            "'show' needs what to show: show mem <where> [<size>], or show arena [<id>]"
                .to_string(),
        ),
        ("show", [other, ..]) => Err(format!(
            "cannot show '{other}'; 'show mem' and 'show arena' are what can be shown"
        )),
        ("disassemble" | "dis", []) => Ok(Command::Disassemble(Request::Current)),
        ("disassemble" | "dis", [word]) if word.bytes().all(|byte| byte.is_ascii_digit()) => {
            match word.parse::<u64>() {
                Ok(0) | Err(_) => Err(format!("cannot show {word} instructions; give 1 or more")),
                Ok(count) => Ok(Command::Disassemble(Request::Count(count))),
            }
        }
        ("disassemble" | "dis", [name]) => {
            Ok(Command::Disassemble(Request::Named(name.to_string())))
        }
        ("disassemble" | "dis", _) => Err(format!("'{name}' takes at most one argument")),
        ("finish" | "f", []) => Ok(Command::Finish),
        ("finish" | "f", _) => Err(format!("'{name}' takes no arguments")),
        ("restart" | "rs", []) => Ok(Command::Restart),
        ("restart" | "rs", _) => Err(format!("'{name}' takes no arguments")),
        ("run" | "r", []) => Ok(Command::Run),
        ("run" | "r", _) => Err(format!("'{name}' takes no arguments")),
        _ => Err(format!(
            "unknown command '{name}'; type 'help' for the list of commands"
        )),
    }
}

fn step_count(word: &str) -> Result<u64, String> {
    match word.parse::<u64>() {
        Ok(count) => Ok(count),
        Err(_) if word.parse::<i128>().is_ok_and(|count| count < 0) => Err(format!(
            "cannot step {word} instructions: the count must not be negative"
        )),
        Err(_) => Err(format!("'{word}' is not a number of instructions")),
    }
}

fn arena_id(word: &str) -> Result<usize, String> {
    word.parse::<usize>()
        .map_err(|_| format!("'{word}' is not an arena id; type 'list arena' for the arenas"))
}

/// A byte count or offset: decimal, or hex after `0x`.
fn number(word: &str) -> Result<u64, String> {
    match word.strip_prefix("0x").or_else(|| word.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => word.parse::<u64>(),
    }
    .map_err(|_| format!("'{word}' is not a number"))
}

fn thread_id(word: &str) -> Result<usize, String> {
    word.parse::<usize>()
        .map_err(|_| format!("'{word}' is not a thread id; type 'list thread' for the threads"))
}

/// The `list func` lines: the program's own functions, then the runtime's, then
/// linked ones, each group in address order (a linked function not yet bound
/// last), symbols that tie in symbol-table order, and a name repeated at the
/// same address once.
pub(crate) fn function_lines(symbols: &[FunctionSymbol]) -> Vec<String> {
    let mut rows = symbols
        .iter()
        .map(|symbol| {
            let (origin, written) = if symbol.linked {
                (Origin::Linked, Some(linked_name(&symbol.name)))
            } else {
                classify_defined(&symbol.name)
            };
            (origin, symbol.address, &symbol.name, written)
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|(origin, address, _, _)| (*origin, address.is_none(), *address));
    // The same function named again at the same address (an import's repeated
    // records) is shown once; one name at two addresses is shown twice.
    let mut seen = std::collections::HashSet::new();
    rows.retain(|(_, address, name, _)| seen.insert((*address, name.to_string())));
    let width = rows
        .iter()
        .map(|(_, _, name, _)| name.len())
        .max()
        .unwrap_or(0);
    rows.into_iter()
        .map(|(origin, address, name, written)| {
            let address = address.map_or_else(|| "-".to_string(), |at| format!("0x{at:016x}"));
            format!(
                "{} {address:<18} {name:<width$} {}",
                origin.code(),
                written.as_deref().unwrap_or("-")
            )
        })
        .collect()
}

/// The `list reg` lines: one register per line, the value in full-width hex and
/// then in decimal — signed for a 64-bit register, as MFBASIC's `Integer` is.
pub(crate) fn register_lines(registers: &[Register]) -> Vec<String> {
    let width = registers
        .iter()
        .map(|register| register.name.len())
        .max()
        .unwrap_or(0);
    registers
        .iter()
        .map(|register| {
            let value = match register.value {
                RegisterValue::Bits64(value) => format!("0x{value:016x} ({})", value as i64),
                RegisterValue::Bits128(value) => format!("0x{value:032x} ({value})"),
            };
            format!("{:<width$} {value}", register.name)
        })
        .collect()
}

/// The `list thread` lines: `<id> <name> <pc> <task>`. A thread's name is the
/// one it gave itself, else the function it is in, else its system id.
pub(crate) fn thread_lines(
    threads: &[ThreadFacts],
    roles: &[Role],
    functions: &[FunctionSymbol],
) -> Vec<String> {
    let names = threads
        .iter()
        .map(|thread| {
            thread
                .name
                .clone()
                .or_else(|| runtime::locate(functions, thread.pc))
                .unwrap_or_else(|| format!("thread-{:#x}", thread.id))
        })
        .collect::<Vec<_>>();
    let id_width = threads.len().saturating_sub(1).to_string().len();
    let name_width = names.iter().map(String::len).max().unwrap_or(0);
    threads
        .iter()
        .zip(&names)
        .zip(roles)
        .enumerate()
        .map(|(id, ((thread, name), role))| {
            format!(
                "{id:<id_width$} {name:<name_width$} 0x{:016x} {}",
                thread.pc,
                role.label()
            )
        })
        .collect()
}

/// The `list arena` lines: `<id> <thread id> <address> <size in KB>`, the size
/// rounded up to whole kilobytes and `-` for an arena no thread allocates from.
pub(crate) fn arena_lines(arenas: &[Arena]) -> Vec<String> {
    let threads = arenas
        .iter()
        .map(|arena| {
            arena
                .thread
                .map_or_else(|| "-".to_string(), |id| id.to_string())
        })
        .collect::<Vec<_>>();
    let id_width = arenas
        .iter()
        .map(|arena| arena.id.to_string().len())
        .max()
        .unwrap_or(0);
    let thread_width = threads.iter().map(String::len).max().unwrap_or(0);
    arenas
        .iter()
        .zip(&threads)
        .map(|(arena, thread)| {
            format!(
                "{:<id_width$} {thread:<thread_width$} 0x{:016x} {}",
                arena.id,
                arena.address,
                arena.mapped.div_ceil(1024)
            )
        })
        .collect()
}

/// The lines a `list` prints, or the error it reports.
fn list(
    inferior: &mut dyn Inferior,
    program: Program,
    breakpoints: &Breakpoints,
    listing: Option<Listing>,
) -> Result<Vec<String>, String> {
    match listing {
        None => Ok(LIST_USAGE.lines().map(str::to_string).collect()),
        Some(Listing::Breakpoints) => Ok(breakpoints.lines()),
        Some(Listing::Functions) => Ok(function_lines(&inferior.functions()?)),
        Some(Listing::Threads) => {
            let threads = inferior.threads()?;
            let roles = runtime::roles(inferior, &threads, program.app)?;
            let functions = inferior.functions().unwrap_or_default();
            Ok(thread_lines(&threads, &roles, &functions))
        }
        Some(Listing::Registers { thread, names }) => {
            let count = inferior.threads()?.len();
            if thread >= count {
                return Err(no_thread(thread, count));
            }
            let registers = inferior.registers(thread)?;
            if names.is_empty() {
                return Ok(register_lines(&registers));
            }
            let mut chosen = Vec::new();
            for name in &names {
                let register = registers
                    .iter()
                    .find(|register| &register.name == name)
                    .ok_or_else(|| format!("no register '{name}'; type 'list reg' for them all"))?;
                chosen.push(register.clone());
            }
            Ok(register_lines(&chosen))
        }
        Some(Listing::Arenas) => {
            let threads = inferior.threads()?;
            let roles = runtime::roles(inferior, &threads, program.app)?;
            match runtime::arenas(inferior, &threads, &roles)? {
                None => Err("listing arenas needs a debug build; rebuild the program \
                             with: mfb build --debug -O0 <project>"
                    .to_string()),
                Some(arenas) if arenas.is_empty() => Ok(vec![
                    "no arenas yet: the program's runtime has not started".to_string(),
                ]),
                Some(arenas) => Ok(arena_lines(&arenas)),
            }
        }
    }
}

fn no_thread(id: usize, count: usize) -> String {
    format!(
        "no thread {id}: the program has {count} thread{}; type 'list thread' for them",
        if count == 1 { "" } else { "s" }
    )
}

fn instructions(taken: u64) -> String {
    format!("{taken} instruction{}", if taken == 1 { "" } else { "s" })
}

/// Step `thread` `count` instructions and describe where it stopped; the flag
/// is set when the program ended.
fn step(
    inferior: &mut dyn Inferior,
    thread: usize,
    count: u64,
) -> Result<(Vec<String>, bool), String> {
    let threads = inferior.threads()?.len();
    if thread >= threads {
        return Err(no_thread(thread, threads));
    }
    if count == 0 {
        return Ok((
            vec![format!(
                "Thread {thread} is at {}.",
                places(inferior)?[thread]
            )],
            false,
        ));
    }
    let outcome = inferior
        .step(&[thread], count)?
        .pop()
        .ok_or("the step answered for no thread")?;
    let taken = instructions(outcome.taken);
    let exited = outcome.stop == StepStop::Exited;
    let line = match outcome.stop {
        StepStop::Exited => format!("The program exited after {taken}."),
        stop => {
            let place = places(inferior)?
                .get(thread)
                .cloned()
                .ok_or_else(|| format!("thread {thread} ended during the step"))?;
            match stop {
                StepStop::Finished => format!("Stepped {taken}; thread {thread} is at {place}."),
                StepStop::Exception(what) => {
                    format!("Stopped after {taken} by {what}; thread {thread} is at {place}.")
                }
                _ => format!(
                    "Stopped after {taken}: thread {thread} is waiting in a system call at {place}."
                ),
            }
        }
    };
    Ok((vec![line], exited))
}

/// Step every thread together `count` rounds and describe where each stopped;
/// the flag is set when the program ended.
fn step_all(inferior: &mut dyn Inferior, count: u64) -> Result<(Vec<String>, bool), String> {
    let threads = inferior.threads()?.len();
    let ids = (0..threads).collect::<Vec<_>>();
    let outcomes = if count == 0 {
        Vec::new()
    } else {
        inferior.step(&ids, count)?
    };
    if outcomes
        .iter()
        .any(|outcome| outcome.stop == StepStop::Exited)
    {
        let rounds = outcomes
            .iter()
            .map(|outcome| outcome.taken)
            .max()
            .unwrap_or(0);
        return Ok((
            vec![format!(
                "The program exited after {} of stepping.",
                if rounds == 1 {
                    "1 round".to_string()
                } else {
                    format!("{rounds} rounds")
                }
            )],
            true,
        ));
    }
    let places = places(inferior)?;
    let lines = ids
        .iter()
        .map(|id| {
            let Some(at) = places.get(*id) else {
                return format!("Thread {id} ended during the step.");
            };
            match outcomes.get(*id) {
                None => format!("Thread {id} is at {at}."),
                Some(outcome) => {
                    let taken = instructions(outcome.taken);
                    match &outcome.stop {
                        StepStop::Exception(what) => {
                            format!("Thread {id}: stopped after {taken} by {what}; at {at}.")
                        }
                        StepStop::Waiting => format!(
                            "Thread {id}: stopped after {taken}, waiting in a system call; at {at}."
                        ),
                        _ => format!("Thread {id}: stepped {taken}; at {at}."),
                    }
                }
            }
        })
        .collect();
    Ok((lines, false))
}

/// The live arena with registry slot `id`, or why there is none.
fn find_arena(inferior: &mut dyn Inferior, program: Program, id: usize) -> Result<Arena, String> {
    let threads = inferior.threads()?;
    let roles = runtime::roles(inferior, &threads, program.app)?;
    let Some(arenas) = runtime::arenas(inferior, &threads, &roles)? else {
        return Err(
            "showing an arena needs a debug build; rebuild the program with: \
                    mfb build --debug -O0 <project>"
                .to_string(),
        );
    };
    if arenas.is_empty() {
        return Err("no arenas yet: the program's runtime has not started".to_string());
    }
    arenas
        .into_iter()
        .find(|arena| arena.id == id)
        .ok_or_else(|| format!("no arena {id}; type 'list arena' for the arenas"))
}

/// `show arena`: the arena's state and its blocks' headers, or a dump of one
/// block.
fn show_arena(
    inferior: &mut dyn Inferior,
    program: Program,
    id: usize,
    dump: Option<Dump>,
) -> Result<Vec<String>, String> {
    let arena = find_arena(inferior, program, id)?;
    let blocks = runtime::blocks(inferior, arena.address)?;
    let Some(dump) = dump else {
        let state = runtime::arena_state(inferior, arena.address)?;
        let mut lines = state_lines(&state);
        lines.extend(block_lines(&blocks));
        return Ok(lines);
    };
    let block = blocks.get(dump.block).ok_or_else(|| {
        format!(
            "arena {id} has {} block{}, numbered from 0, the oldest",
            blocks.len(),
            if blocks.len() == 1 { "" } else { "s" }
        )
    })?;
    if dump.size == 0 {
        return Err("a dump needs a size of 1 or more bytes".to_string());
    }
    if dump
        .offset
        .checked_add(dump.size)
        .is_none_or(|end| end > block.size)
    {
        return Err(format!(
            "offset {:#x} + size {:#x} runs past the end of block {}, whose blockSize is {:#x}",
            dump.offset, dump.size, dump.block, block.size
        ));
    }
    let length = usize::try_from(dump.size).map_err(|_| "the dump is too large".to_string())?;
    let bytes = inferior.read_bytes(block.address + dump.offset, length)?;
    let mut lines = vec![format!("Arena Block ({id},{}):", dump.block)];
    lines.extend(dump_lines(dump.offset, &bytes));
    Ok(lines)
}

/// The width of the `show arena` name column: the longest field name.
const FIELD_WIDTH: usize = 17;

/// The `Arena State:` lines: each field, an array as its count of non-empty
/// entries followed by each one, every value at its own width in hex.
pub(crate) fn state_lines(state: &[u8]) -> Vec<String> {
    let value = |at: usize, width: usize| -> String {
        match width {
            16 => format!(
                "0x{:032x}",
                u128::from_le_bytes(state[at..at + 16].try_into().expect("16 bytes"))
            ),
            _ => format!(
                "0x{:016x}",
                u64::from_le_bytes(state[at..at + 8].try_into().expect("8 bytes"))
            ),
        }
    };
    let mut lines = vec!["Arena State:".to_string()];
    for field in ARENA_STATE_FIELDS {
        if field.count == 1 {
            lines.push(format!(
                "  {:<FIELD_WIDTH$} {}",
                field.name,
                value(field.offset, field.width)
            ));
            continue;
        }
        let entries = (0..field.count)
            .map(|index| (index, field.offset + index * field.width))
            .filter(|(_, at)| state[*at..*at + field.width].iter().any(|byte| *byte != 0))
            .collect::<Vec<_>>();
        lines.push(format!(
            "  {:<FIELD_WIDTH$} {} non-empty",
            format!("{}[{}]", field.name, field.count),
            entries.len()
        ));
        for (index, at) in entries {
            lines.push(format!(
                "    {:<width$} {}",
                format!("[{index}]"),
                value(at, field.width),
                width = FIELD_WIDTH - 2
            ));
        }
    }
    lines
}

/// The `Arena Block (#):` sections, oldest block first.
pub(crate) fn block_lines(blocks: &[Block]) -> Vec<String> {
    let mut lines = Vec::new();
    for (number, block) in blocks.iter().enumerate() {
        lines.push(format!("Arena Block ({number}):"));
        for (name, value) in [
            ("prevBlock", block.prev),
            ("blockSize", block.size),
            ("usableCapacity", block.usable),
            ("bumpOffset", block.bump),
        ] {
            lines.push(format!("  {name:<FIELD_WIDTH$} 0x{value:016x}"));
        }
    }
    lines
}

/// Hex-dump lines of 16 bytes: the offset in the block (hex digits in groups
/// of four), the bytes, and the printable ones.
pub(crate) fn dump_lines(offset: u64, bytes: &[u8]) -> Vec<String> {
    hex_dump(offset, bytes, |at| {
        let digits = format!("{at:08x}");
        let grouped = digits
            .as_bytes()
            .rchunks(4)
            .rev()
            .map(|group| std::str::from_utf8(group).expect("hex digits"))
            .collect::<Vec<_>>()
            .join("_");
        format!("0x{grouped}")
    })
}

/// Hex-dump lines of memory, each led by its full address.
pub(crate) fn memory_lines(address: u64, bytes: &[u8]) -> Vec<String> {
    hex_dump(address, bytes, |at| format!("0x{at:016x}"))
}

/// Lines of 16 bytes from `start`, each led by `label` of its first byte's
/// position, then the bytes in hex and the printable ones.
fn hex_dump(start: u64, bytes: &[u8], label: impl Fn(u64) -> String) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(row, chunk)| {
            let label = label(start + row as u64 * 16);
            let hex = (0..16)
                .map(|index| {
                    chunk
                        .get(index)
                        .map_or_else(|| "  ".to_string(), |byte| format!("{byte:02x}"))
                })
                .collect::<Vec<_>>()
                .join(" ");
            let text = chunk
                .iter()
                .map(|byte| {
                    if (0x20..0x7f).contains(byte) {
                        char::from(*byte)
                    } else {
                        '.'
                    }
                })
                .map(String::from)
                .collect::<Vec<_>>()
                .join(" ");
            format!("{label} {hex} | {text}")
        })
        .collect()
}

/// Run the program and describe why it stopped; the flag is set when it ended.
fn run_program(
    inferior: &mut dyn Inferior,
    breakpoints: &mut Breakpoints,
    current: &mut usize,
) -> Result<(Vec<String>, bool), String> {
    let (mut lines, stop) = breakpoints.run(inferior)?;
    let exited = describe_stop(inferior, stop, current, &mut lines)?;
    Ok((lines, exited))
}

/// Describe a stop, and make the thread it names current; `true` when the
/// program ended.
fn describe_stop(
    inferior: &mut dyn Inferior,
    stop: RunStop,
    current: &mut usize,
    lines: &mut Vec<String>,
) -> Result<bool, String> {
    match &stop {
        RunStop::Breakpoints(hits) => {
            if let Some((thread, _)) = hits.first() {
                *current = *thread;
            }
        }
        RunStop::Exception { thread, .. } => *current = *thread,
        _ => {}
    }
    let place = |inferior: &mut dyn Inferior, thread: usize| -> Result<String, String> {
        Ok(places(inferior)?
            .get(thread)
            .cloned()
            .unwrap_or_else(|| "an unknown place".to_string()))
    };
    let exited = matches!(stop, RunStop::Exited(_));
    match stop {
        RunStop::Breakpoints(hits) => {
            for (thread, address) in hits {
                let at = place(inferior, thread).unwrap_or_else(|_| format!("0x{address:016x}"));
                lines.push(format!("Thread {thread} hit a breakpoint at {at}."));
            }
        }
        RunStop::Exception { thread, what } => {
            let at = place(inferior, thread)?;
            lines.push(format!("Thread {thread} stopped by {what} at {at}."));
        }
        RunStop::Interrupted => lines.push(format!(
            "Paused: thread {MAIN_THREAD} is at {}.",
            place(inferior, MAIN_THREAD)?
        )),
        RunStop::Exited(Exit::Status(status)) => {
            lines.push(format!("The program exited with status {status}."))
        }
        RunStop::Exited(Exit::Signal(signal)) => {
            lines.push(format!("The program was ended by signal {signal}."))
        }
        RunStop::Exited(Exit::Unknown) => lines.push("The program exited.".to_string()),
    }
    Ok(exited)
}

/// `finish`: run the current thread until its function returns, then say
/// where it returned to and what it returned (MFBASIC returns an error code in
/// `x0` and the value in `x1`).
fn finish(
    inferior: &mut dyn Inferior,
    breakpoints: &mut Breakpoints,
    current: &mut usize,
) -> Result<(Vec<String>, bool), String> {
    let thread = *current;
    let (mut lines, outcome) = breakpoints.finish(inferior, thread)?;
    match outcome {
        Finish::Returned { from } => {
            let at = places(inferior)?
                .get(thread)
                .cloned()
                .unwrap_or_else(|| "an unknown place".to_string());
            let registers = inferior.registers(thread)?;
            let value = |name: &str| {
                registers
                    .iter()
                    .find(|register| register.name == name)
                    .map(|register| register_lines(std::slice::from_ref(register)).remove(0))
                    .map(|line| {
                        line.split_whitespace()
                            .skip(1)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default()
            };
            let from = from
                .as_deref()
                .map(disassemble::display_name)
                .unwrap_or_else(|| "the function".to_string());
            lines.push(format!(
                "Thread {thread} returned from {from} to {at}: x0 = {}, x1 = {}.",
                value("x0"),
                value("x1")
            ));
            Ok((lines, false))
        }
        Finish::Stopped(stop) => {
            let exited = describe_stop(inferior, stop, current, &mut lines)?;
            Ok((lines, exited))
        }
    }
}

/// `restart`: the program again, as the debugger first launched it, with the
/// breakpoints placed anew.
fn restart(
    inferior: &mut dyn Inferior,
    breakpoints: &mut Breakpoints,
) -> Result<Vec<String>, String> {
    let pid = inferior.restart()?;
    let mut lines = vec![format!(
        "Restarted the program as process {pid}; paused before its first instruction."
    )];
    lines.extend(breakpoints.restarted(inferior)?);
    Ok(lines)
}

/// `show mem`: `size` bytes at an address or at a register of the current
/// thread plus an offset.
fn show_memory(
    inferior: &mut dyn Inferior,
    current: usize,
    location: &Location,
    size: u64,
) -> Result<Vec<String>, String> {
    if size == 0 || size > MAX_MEMORY {
        return Err(format!("cannot show {size} bytes; give 1 to {MAX_MEMORY}"));
    }
    let (address, described) = match location {
        Location::Address(address) => (*address, String::new()),
        Location::Register { name, offset } => {
            let registers = inferior.registers(current)?;
            let register = registers
                .iter()
                .find(|register| &register.name == name)
                .ok_or_else(|| format!("no register '{name}'; type 'list reg' for them all"))?;
            let RegisterValue::Bits64(base) = register.value else {
                return Err(format!("'{name}' is a vector register, not an address"));
            };
            let address = base.wrapping_add(*offset as u64);
            let shown = match offset {
                0 => name.clone(),
                offset if *offset < 0 => format!("{name}-{:#x}", offset.unsigned_abs()),
                offset => format!("{name}+{offset:#x}"),
            };
            (address, format!(" ({shown} of thread {current})"))
        }
    };
    let bytes = inferior.read_bytes(address, size as usize)?;
    let mut lines = vec![format!("Memory at 0x{address:016x}{described}:")];
    lines.extend(memory_lines(address, &bytes));
    Ok(lines)
}

/// `dis` for the current thread: its `pc` is where `dis` starts and what it
/// marks. A current thread that has since ended falls back to thread 0.
fn disassemble_at(
    inferior: &mut dyn Inferior,
    current: usize,
    request: &Request,
) -> Result<Vec<String>, String> {
    let threads = inferior.threads()?;
    let thread = threads
        .get(current)
        .or_else(|| threads.first())
        .ok_or_else(|| no_thread(current, threads.len()))?;
    disassemble::disassemble(inferior, thread.pc, request)
}

/// Where each thread is: its pc, and the function holding it when the
/// executable names one.
fn places(inferior: &mut dyn Inferior) -> Result<Vec<String>, String> {
    let threads = inferior.threads()?;
    let functions = inferior.functions().unwrap_or_default();
    Ok(threads
        .iter()
        .map(|thread| match runtime::locate(&functions, thread.pc) {
            Some(function) => format!("0x{:016x} in {function}", thread.pc),
            None => format!("0x{:016x}", thread.pc),
        })
        .collect())
}

/// Run the prompt until the session ends, then end it. `echo` repeats each
/// command after the prompt (set when the input is not a terminal). Errors from
/// a command go to `err` and the prompt continues; an error ending the session
/// is returned.
pub(crate) fn run(
    inferior: &mut dyn Inferior,
    program: Program,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
    echo: bool,
) -> Result<(), String> {
    let written = |result: std::io::Result<()>| result.map_err(|e| format!("cannot write: {e}"));
    let mut line = String::new();
    let mut breakpoints = Breakpoints::default();
    // The thread `dis` shows: the last one a `run` or `step` stopped.
    let mut current = MAIN_THREAD;
    // Once the program has ended only `help`, `list`, `list breakpoint` and
    // `quit` mean anything.
    let mut exited = false;
    loop {
        write!(out, "{PROMPT}")
            .and_then(|()| out.flush())
            .map_err(|e| format!("cannot write the prompt: {e}"))?;
        line.clear();
        let read = input
            .read_line(&mut line)
            .map_err(|e| format!("cannot read a command: {e}"))?;
        if read == 0 {
            // End of input: finish the prompt's line, then quit.
            written(writeln!(out))?;
            break;
        }
        if echo {
            written(
                write!(out, "{}", line.trim_end_matches(['\r', '\n'])).and_then(|()| writeln!(out)),
            )?;
        }
        let result = match parse(&line) {
            Ok(Command::Empty) => Ok(Vec::new()),
            Ok(Command::Help) => Ok(vec![HELP.to_string()]),
            Ok(Command::Quit) => break,
            Ok(Command::List(listing @ (None | Some(Listing::Breakpoints)))) => {
                list(inferior, program, &breakpoints, listing)
            }
            Ok(Command::Restart) => restart(inferior, &mut breakpoints).inspect(|_| {
                exited = false;
                current = MAIN_THREAD;
            }),
            Ok(_) if exited => {
                Err("the program has exited; type 'quit' to end the session".to_string())
            }
            Ok(Command::List(listing)) => list(inferior, program, &breakpoints, listing),
            Ok(Command::Step { count, thread }) => {
                step(inferior, thread, count).map(|(lines, ended)| {
                    exited = ended;
                    current = thread;
                    lines
                })
            }
            Ok(Command::StepAll(count)) => step_all(inferior, count).map(|(lines, ended)| {
                exited = ended;
                lines
            }),
            Ok(Command::ShowArena { id, dump }) => show_arena(inferior, program, id, dump),
            Ok(Command::SetBreakpoint(target)) => breakpoints.set(inferior, &target),
            Ok(Command::RemoveBreakpoint(target)) => breakpoints.remove(inferior, &target),
            Ok(Command::Run) => {
                run_program(inferior, &mut breakpoints, &mut current).map(|(lines, ended)| {
                    exited = ended;
                    lines
                })
            }
            Ok(Command::Disassemble(request)) => disassemble_at(inferior, current, &request),
            Ok(Command::Finish) => {
                finish(inferior, &mut breakpoints, &mut current).map(|(lines, ended)| {
                    exited = ended;
                    lines
                })
            }
            Ok(Command::ShowMemory { location, size }) => {
                show_memory(inferior, current, &location, size)
            }
            Err(message) => Err(message),
        };
        match result {
            Ok(lines) => {
                for line in lines {
                    written(writeln!(out, "{line}"))?;
                }
            }
            Err(message) => {
                written(out.flush())?;
                written(writeln!(err, "error: {message}"))?;
            }
        }
    }
    let pid = inferior.pid();
    let message = match inferior.end()? {
        Ending::Killed => format!("Killed process {pid}."),
        Ending::Detached => format!("Detached from process {pid}; it continues running."),
        Ending::AlreadyExited => format!("Process {pid} had already exited."),
    };
    written(writeln!(out, "{message}").and_then(|()| out.flush()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::{FunctionRange, Step};

    /// An [`Inferior`] answering from fixed tables: two threads, a few
    /// functions, three registers. It records how often it was ended and every
    /// step it was asked for, and refuses every read when `unreadable`.
    struct Recorder {
        ending: Ending,
        ends: usize,
        unreadable: bool,
        steps: Vec<(Vec<usize>, u64)>,
        stop: StepStop,
        pc: u64,
        inserted: Vec<u64>,
        runs: Vec<RunStop>,
    }

    impl Recorder {
        fn new(ending: Ending) -> Self {
            Recorder {
                ending,
                ends: 0,
                unreadable: false,
                steps: Vec::new(),
                stop: StepStop::Finished,
                pc: 0x1000,
                inserted: Vec::new(),
                runs: Vec::new(),
            }
        }
    }

    fn symbol(name: &str, linked: bool, address: Option<u64>, size: Option<u64>) -> FunctionSymbol {
        FunctionSymbol {
            name: name.to_string(),
            linked,
            address,
            size,
        }
    }

    impl Inferior for Recorder {
        fn pid(&self) -> u32 {
            42
        }
        fn end(&mut self) -> Result<Ending, String> {
            self.ends += 1;
            Ok(self.ending)
        }
        fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
            if self.unreadable {
                return Err("cannot read process 42's memory".to_string());
            }
            Ok(vec![
                symbol("_main", false, Some(0x1000), Some(4)),
                symbol("_mfb_fn_main", false, Some(0x1010), Some(0x10)),
                symbol("_malloc", true, None, None),
                symbol("_mfb_rt_print", false, Some(0x1008), Some(8)),
                symbol("_mfb_fn_next_5Fseed", false, Some(0x1004), Some(4)),
                symbol("_free", true, Some(0x9000), None),
                symbol("_malloc", true, None, None),
            ])
        }
        fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
            Ok(None)
        }
        fn read_u64(&mut self, address: u64) -> Result<u64, String> {
            Err(format!("unmapped {address:#x}"))
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
        fn read_bytes(&mut self, address: u64, _: usize) -> Result<Vec<u8>, String> {
            Err(format!("unmapped {address:#x}"))
        }
        fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
            if self.unreadable {
                return Err("cannot list the threads of process 42".to_string());
            }
            let thread = |id, name: Option<&str>, pc| ThreadFacts {
                id,
                handle: id,
                name: name.map(str::to_string),
                pc,
                sp: 0,
                arena: 0,
                stack: None,
            };
            Ok(vec![
                thread(0x501, None, self.pc),
                thread(0x502, Some("com.apple.worker"), 0x7000),
                thread(0x503, None, 0x7004),
            ])
        }
        fn registers(&mut self, thread: usize) -> Result<Vec<Register>, String> {
            if self.unreadable {
                return Err("cannot read thread 0's registers".to_string());
            }
            let x0 = 0x2a + thread as u64;
            Ok(vec![
                Register {
                    name: "x0".to_string(),
                    value: RegisterValue::Bits64(x0),
                },
                Register {
                    name: "nzcv".to_string(),
                    value: RegisterValue::Bits64(0x6000_0000),
                },
                Register {
                    name: "v31".to_string(),
                    value: RegisterValue::Bits128(1 << 64),
                },
            ])
        }
        /// Thread 0 moves 4 bytes an instruction; the others do not move.
        fn step(&mut self, threads: &[usize], count: u64) -> Result<Vec<Step>, String> {
            self.steps.push((threads.to_vec(), count));
            if threads.contains(&0) {
                self.pc += 4 * count;
            }
            Ok(threads
                .iter()
                .map(|thread| Step {
                    taken: if *thread == 2 { 0 } else { count },
                    stop: if *thread == 2 {
                        StepStop::Waiting
                    } else {
                        self.stop.clone()
                    },
                })
                .collect())
        }
        fn restart(&mut self) -> Result<u32, String> {
            Err("restart needs a program the debugger launched".to_string())
        }
        fn entry(&mut self) -> Result<Option<u64>, String> {
            Ok(Some(0x1000))
        }
        fn insert_breakpoint(&mut self, address: u64) -> Result<(), String> {
            self.inserted.push(address);
            Ok(())
        }
        fn remove_breakpoint(&mut self, address: u64) -> Result<(), String> {
            self.inserted.retain(|other| *other != address);
            Ok(())
        }
        fn run(&mut self) -> Result<RunStop, String> {
            Ok(self.runs.remove(0))
        }
    }

    const CONSOLE: Program = Program { app: false };

    fn session(input: &str, ending: Ending, echo: bool) -> (String, String, usize) {
        let mut inferior = Recorder::new(ending);
        let (out, err) = drive(&mut inferior, input, echo);
        (out, err, inferior.ends)
    }

    fn drive(inferior: &mut Recorder, input: &str, echo: bool) -> (String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run(
            inferior,
            CONSOLE,
            &mut input.as_bytes(),
            &mut out,
            &mut err,
            echo,
        )
        .expect("session runs");
        (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn help_lists_every_command_in_both_spellings() {
        for line in ["help\nq\n", "h\nq\n"] {
            let (out, err, _) = session(line, Ending::Killed, false);
            assert_eq!(
                out,
                format!("{PROMPT}{HELP}\n{PROMPT}Killed process 42.\n"),
                "{line:?}"
            );
            assert_eq!(err, "");
        }
    }

    #[test]
    fn quit_ends_the_session_once_in_both_spellings() {
        for line in ["quit\n", "q\n", "  q  \n"] {
            let (out, _, ends) = session(line, Ending::Detached, false);
            assert_eq!(
                out,
                format!("{PROMPT}Detached from process 42; it continues running.\n")
            );
            assert_eq!(ends, 1, "{line:?}");
        }
    }

    #[test]
    fn end_of_input_quits() {
        let (out, _, ends) = session("", Ending::Killed, false);
        assert_eq!(out, format!("{PROMPT}\nKilled process 42.\n"));
        assert_eq!(ends, 1);
    }

    #[test]
    fn echo_repeats_each_command_after_the_prompt() {
        let (out, _, _) = session("h\r\n\nq\n", Ending::Killed, true);
        assert_eq!(
            out,
            format!("{PROMPT}h\n{HELP}\n{PROMPT}\n{PROMPT}q\nKilled process 42.\n")
        );
    }

    #[test]
    fn a_bad_command_reports_and_the_prompt_continues() {
        let (out, err, ends) = session("frob\nhelp me\nq now\nq\n", Ending::Killed, false);
        assert_eq!(
            out,
            format!("{PROMPT}{PROMPT}{PROMPT}{PROMPT}Killed process 42.\n")
        );
        assert_eq!(
            err,
            "error: unknown command 'frob'; type 'help' for the list of commands\n\
             error: 'help' takes no arguments\n\
             error: 'q' takes no arguments\n"
        );
        assert_eq!(ends, 1);
    }

    #[test]
    fn an_externally_ended_process_is_reported() {
        let (out, _, _) = session("q\n", Ending::AlreadyExited, false);
        assert_eq!(out, format!("{PROMPT}Process 42 had already exited.\n"));
    }

    #[test]
    fn a_bare_list_shows_what_can_be_listed_in_both_spellings() {
        for line in ["list\nq\n", "l\nq\n"] {
            let (out, err, _) = session(line, Ending::Killed, false);
            assert_eq!(
                out,
                format!("{PROMPT}{LIST_USAGE}\n{PROMPT}Killed process 42.\n")
            );
            assert_eq!(err, "");
        }
    }

    #[test]
    fn list_func_groups_by_origin_then_address_and_keeps_repeats() {
        let (out, err, _) = session("l func\nq\n", Ending::Killed, false);
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
MU 0x0000000000001004 _mfb_fn_next_5Fseed next_seed
MU 0x0000000000001010 _mfb_fn_main        main
MR 0x0000000000001000 _main               -
MR 0x0000000000001008 _mfb_rt_print       -
LN 0x0000000000009000 _free               free
LN -                  _malloc             malloc
{PROMPT}Killed process 42.\n"
            )
        );
    }

    #[test]
    fn list_thread_names_each_thread_by_name_frame_or_id() {
        let (out, err, _) = session("list thread\nq\n", Ending::Killed, false);
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
0 _main            0x0000000000001000 MFB Main
1 com.apple.worker 0x0000000000007000 Other
2 thread-0x503     0x0000000000007004 Other
{PROMPT}Killed process 42.\n"
            )
        );
    }

    #[test]
    fn list_reg_shows_the_main_thread_or_the_one_asked_for() {
        let (out, _, _) = session("list reg\nl reg 2\nq\n", Ending::Killed, false);
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
x0   0x000000000000002a (42)
nzcv 0x0000000060000000 (1610612736)
v31  0x00000000000000010000000000000000 (18446744073709551616)
{PROMPT}\
x0   0x000000000000002c (44)
nzcv 0x0000000060000000 (1610612736)
v31  0x00000000000000010000000000000000 (18446744073709551616)
{PROMPT}Killed process 42.\n"
            )
        );
    }

    #[test]
    fn list_reg_refuses_a_thread_or_register_that_does_not_exist() {
        let (_, err, _) = session(
            "list reg 3\nlist reg x9\nlist reg 1 x0 w5\nq\n",
            Ending::Killed,
            false,
        );
        assert_eq!(
            err,
            "error: no thread 3: the program has 3 threads; type 'list thread' for them\n\
             error: no register 'x9'; type 'list reg' for them all\n\
             error: no register 'w5'; type 'list reg' for them all\n"
        );
    }

    #[test]
    fn list_reg_shows_just_the_registers_named() {
        let (out, err, _) = session("list reg 2 nzcv x0\nq\n", Ending::Killed, false);
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
nzcv 0x0000000060000000 (1610612736)
x0   0x000000000000002c (44)
{PROMPT}Killed process 42.\n"
            )
        );
    }

    #[test]
    fn a_negative_64_bit_value_shows_as_a_signed_decimal() {
        let register = Register {
            name: "x8".to_string(),
            value: RegisterValue::Bits64(u64::MAX),
        };
        assert_eq!(register_lines(&[register]), ["x8 0xffffffffffffffff (-1)"]);
    }

    #[test]
    fn show_mem_parses_an_address_or_a_register_and_offset() {
        assert_eq!(
            parse("show mem 0x1000"),
            Ok(Command::ShowMemory {
                location: Location::Address(0x1000),
                size: MEMORY_SIZE
            })
        );
        assert_eq!(
            parse("show mem sp+0x18 8"),
            Ok(Command::ShowMemory {
                location: Location::Register {
                    name: "sp".to_string(),
                    offset: 0x18
                },
                size: 8
            })
        );
        assert_eq!(
            parse("show mem x19-16"),
            Ok(Command::ShowMemory {
                location: Location::Register {
                    name: "x19".to_string(),
                    offset: -16
                },
                size: MEMORY_SIZE
            })
        );
        for (line, message) in [
            (
                "show mem",
                "'show mem' needs an address or a register: show mem <where> [<size>]",
            ),
            ("show mem sp+zz", "'zz' is not a number"),
            ("show mem $x", "'$x' is not an address or a register"),
            ("show mem sp 8 9", "too many arguments to 'show mem'"),
            ("finish now", "'finish' takes no arguments"),
            ("rs again", "'rs' takes no arguments"),
        ] {
            assert_eq!(parse(line), Err(message.to_string()), "{line}");
        }
    }

    #[test]
    fn memory_lines_lead_with_full_addresses() {
        assert_eq!(
            memory_lines(0x16f0_0010, b"MFB!"),
            ["0x0000000016f00010 4d 46 42 21                                     | M F B !"]
        );
    }

    #[test]
    fn restart_needs_a_launched_program() {
        let (_, err, _) = session("restart\nq\n", Ending::Killed, false);
        assert_eq!(
            err,
            "error: restart needs a program the debugger launched\n"
        );
    }

    #[test]
    fn list_arena_without_a_registry_asks_for_a_debug_build() {
        let (_, err, _) = session("list arena\nq\n", Ending::Killed, false);
        assert_eq!(
            err,
            "error: listing arenas needs a debug build; rebuild the program with: \
             mfb build --debug -O0 <project>\n"
        );
    }

    #[test]
    fn arena_lines_round_up_to_kilobytes_and_mark_unowned_arenas() {
        let arena = |id, thread, address, mapped| Arena {
            id,
            thread,
            address,
            mapped,
        };
        assert_eq!(
            arena_lines(&[
                arena(0, Some(0), 0x16f0_0000, 4096),
                arena(12, Some(3), 0x1_0000, 4097),
                arena(13, None, 0x2_0000, 0),
            ]),
            [
                "0  0 0x0000000016f00000 4",
                "12 3 0x0000000000010000 5",
                "13 - 0x0000000000020000 0",
            ]
        );
        assert!(arena_lines(&[]).is_empty());
    }

    #[test]
    fn step_runs_the_main_thread_and_says_where_it_stopped() {
        let mut inferior = Recorder::new(Ending::Killed);
        let (out, err) = drive(&mut inferior, "step\ns 2\nstep 0\nq\n", false);
        assert_eq!(err, "");
        assert_eq!(inferior.steps, [(vec![0], 1), (vec![0], 2)]);
        assert_eq!(
            out,
            format!(
                "{PROMPT}Stepped 1 instruction; thread 0 is at 0x0000000000001004 in next_seed.\n\
                 {PROMPT}Stepped 2 instructions; thread 0 is at 0x000000000000100c in _mfb_rt_print+0x4.\n\
                 {PROMPT}Thread 0 is at 0x000000000000100c in _mfb_rt_print+0x4.\n\
                 {PROMPT}Killed process 42.\n"
            )
        );
    }

    #[test]
    fn step_reports_an_exception_a_wait_or_the_end_of_the_program() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.pc = 0x8000;
        inferior.stop = StepStop::Exception("a breakpoint instruction".to_string());
        let (out, _) = drive(&mut inferior, "s\nq\n", false);
        assert!(
            out.contains("Stopped after 1 instruction by a breakpoint instruction; thread 0 is at 0x0000000000008004.\n"),
            "{out}"
        );
        inferior.stop = StepStop::Waiting;
        let (out, _) = drive(&mut inferior, "s 3\nq\n", false);
        assert!(
            out.contains("Stopped after 3 instructions: thread 0 is waiting in a system call at 0x0000000000008010.\n"),
            "{out}"
        );
        inferior.stop = StepStop::Exited;
        let (out, _) = drive(&mut inferior, "s\nq\n", false);
        assert!(
            out.contains("The program exited after 1 instruction.\n"),
            "{out}"
        );
    }

    #[test]
    fn step_runs_the_thread_asked_for() {
        let mut inferior = Recorder::new(Ending::Killed);
        let (out, err) = drive(&mut inferior, "step 2 1\ns 0 1\nq\n", false);
        assert_eq!(err, "");
        assert_eq!(inferior.steps, [(vec![1], 2)]);
        assert_eq!(
            out,
            format!(
                "{PROMPT}Stepped 2 instructions; thread 1 is at 0x0000000000007000.\n\
                 {PROMPT}Thread 1 is at 0x0000000000007000.\n\
                 {PROMPT}Killed process 42.\n"
            )
        );
    }

    #[test]
    fn step_refuses_a_bad_count_or_thread() {
        let mut inferior = Recorder::new(Ending::Killed);
        let (_, err) = drive(
            &mut inferior,
            "step -1\ns ten\ns 1 x\ns 1 3\ns 1 2 3\nsa -4\nsa 1 2\nq\n",
            false,
        );
        assert_eq!(
            err,
            "error: cannot step -1 instructions: the count must not be negative\n\
             error: 'ten' is not a number of instructions\n\
             error: 'x' is not a thread id; type 'list thread' for the threads\n\
             error: no thread 3: the program has 3 threads; type 'list thread' for them\n\
             error: 's' takes at most two arguments\n\
             error: cannot step -4 instructions: the count must not be negative\n\
             error: 'sa' takes at most one argument\n"
        );
        assert!(inferior.steps.is_empty());
    }

    #[test]
    fn step_all_runs_every_thread_and_reports_each() {
        let mut inferior = Recorder::new(Ending::Killed);
        let (out, err) = drive(&mut inferior, "step-all 2\nsa 0\nq\n", false);
        assert_eq!(err, "");
        assert_eq!(inferior.steps, [(vec![0, 1, 2], 2)]);
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
Thread 0: stepped 2 instructions; at 0x0000000000001008 in _mfb_rt_print.
Thread 1: stepped 2 instructions; at 0x0000000000007000.
Thread 2: stopped after 0 instructions, waiting in a system call; at 0x0000000000007004.
{PROMPT}\
Thread 0 is at 0x0000000000001008 in _mfb_rt_print.
Thread 1 is at 0x0000000000007000.
Thread 2 is at 0x0000000000007004.
{PROMPT}Killed process 42.\n"
            )
        );
    }

    #[test]
    fn step_all_reports_an_exception_or_the_end_of_the_program() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.stop = StepStop::Exception("an illegal instruction".to_string());
        let (out, _) = drive(&mut inferior, "sa\nq\n", false);
        assert!(
            out.contains("Thread 1: stopped after 1 instruction by an illegal instruction; at 0x0000000000007000.\n"),
            "{out}"
        );
        inferior.stop = StepStop::Exited;
        let (out, _) = drive(&mut inferior, "sa 3\nq\n", false);
        assert!(
            out.contains("The program exited after 3 rounds of stepping.\n"),
            "{out}"
        );
    }

    #[test]
    fn a_listing_that_fails_reports_and_the_prompt_continues() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.unreadable = true;
        let (out, err) = drive(
            &mut inferior,
            "list func\nlist reg\nlist thread\nq\n",
            false,
        );
        assert_eq!(
            out,
            format!("{PROMPT}{PROMPT}{PROMPT}{PROMPT}Killed process 42.\n")
        );
        assert_eq!(
            err,
            "error: cannot read process 42's memory\n\
             error: cannot list the threads of process 42\n\
             error: cannot list the threads of process 42\n"
        );
        assert_eq!(inferior.ends, 1);
    }

    #[test]
    fn a_bad_list_argument_is_reported() {
        let (_, err, _) = session("list frames\nlist func reg\nq\n", Ending::Killed, false);
        assert_eq!(
            err,
            "error: cannot list 'frames'; type 'list' for what can be listed\n\
             error: too many arguments to 'list func'\n"
        );
    }

    #[test]
    fn empty_listings_print_nothing() {
        assert!(function_lines(&[]).is_empty());
        assert!(register_lines(&[]).is_empty());
        assert!(thread_lines(&[], &[], &[]).is_empty());
    }

    #[test]
    fn breakpoints_are_set_listed_and_removed() {
        let mut inferior = Recorder::new(Ending::Killed);
        let (out, err) = drive(
            &mut inferior,
            "bp next_seed\nbreakpoint 0x1012\nlist breakpoint\nbp r 0x1012\nl breakpoint\nq\n",
            false,
        );
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed.\n\
                 {PROMPT}Set a breakpoint at 0x0000000000001012 in main+0x2.\n\
                 {PROMPT}\
0x0000000000001004 _mfb_fn_next_5Fseed next_seed
0x0000000000001012 _mfb_fn_main+0x2    main+0x2
{PROMPT}Removed the breakpoint at 0x0000000000001012 in main+0x2.\n\
                 {PROMPT}0x0000000000001004 _mfb_fn_next_5Fseed next_seed\n\
                 {PROMPT}Killed process 42.\n"
            )
        );
        assert_eq!(inferior.inserted, [0x1004]);
    }

    #[test]
    fn a_bad_breakpoint_command_is_refused() {
        let (_, err, _) = session(
            "bp\nbp r\nbp 0xzz\nbp a b c\nbp r nothing\nrun now\nq\n",
            Ending::Killed,
            false,
        );
        assert_eq!(
            err,
            "error: 'bp' needs an address (0x…) or a function name\n\
             error: 'bp' needs an address (0x…) or a function name\n\
             error: '0xzz' is not an address\n\
             error: too many arguments to 'bp'\n\
             error: there is no breakpoint on 'nothing'\n\
             error: 'run' takes no arguments\n"
        );
    }

    #[test]
    fn run_reports_each_kind_of_stop() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.runs = vec![
            RunStop::Breakpoints(vec![(0, 0x1000), (1, 0x7000)]),
            RunStop::Exception {
                thread: 1,
                what: "a bad memory access at 0x8".to_string(),
            },
            RunStop::Interrupted,
            RunStop::Exited(Exit::Signal(9)),
        ];
        let (out, err) = drive(
            &mut inferior,
            "run\nr\nrun\nrun\nstep\nlist breakpoint\nq\n",
            false,
        );
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
Thread 0 hit a breakpoint at 0x0000000000001000 in _main.
Thread 1 hit a breakpoint at 0x0000000000007000.
{PROMPT}Thread 1 stopped by a bad memory access at 0x8 at 0x0000000000007000.
{PROMPT}Paused: thread 0 is at 0x0000000000001000 in _main.
{PROMPT}The program was ended by signal 9.
{PROMPT}{PROMPT}no breakpoints
{PROMPT}Killed process 42.\n"
            )
        );
        assert_eq!(
            err,
            "error: the program has exited; type 'quit' to end the session\n"
        );
    }

    #[test]
    fn run_reports_an_exit_status() {
        let mut inferior = Recorder::new(Ending::AlreadyExited);
        inferior.runs = vec![RunStop::Exited(Exit::Status(3))];
        let (out, _) = drive(&mut inferior, "run\nq\n", false);
        assert!(out.contains("The program exited with status 3.\n"), "{out}");
        let mut inferior = Recorder::new(Ending::AlreadyExited);
        inferior.runs = vec![RunStop::Exited(Exit::Unknown)];
        let (out, _) = drive(&mut inferior, "run\nq\n", false);
        assert!(out.contains("The program exited.\n"), "{out}");
    }

    #[test]
    fn the_arena_state_lists_every_field_and_each_non_empty_entry() {
        let mut state =
            vec![0u8; crate::codegen::error::constants::error_constants::ARENA_STATE_SIZE];
        state[0..8].copy_from_slice(&0x1_0000_4000u64.to_le_bytes());
        // quickBin[2], at 104 + 16.
        state[120..128].copy_from_slice(&0xabcdu64.to_le_bytes());
        // v128Slots[1], a 16-byte entry.
        let v128 = ARENA_STATE_FIELDS
            .iter()
            .find(|field| field.name == "v128Slots")
            .unwrap()
            .offset
            + 16;
        state[v128] = 7;
        let lines = state_lines(&state);
        assert_eq!(lines[0], "Arena State:");
        assert_eq!(lines[1], "  blockHead         0x0000000100004000");
        assert!(lines.contains(&"  quickBin[128]     1 non-empty".to_string()));
        assert!(lines.contains(&"    [2]             0x000000000000abcd".to_string()));
        assert!(lines.contains(&"  largeBin[64]      0 non-empty".to_string()));
        assert!(lines.contains(&"  v128Slots[126]    1 non-empty".to_string()));
        assert!(
            lines.contains(&"    [1]             0x00000000000000000000000000000007".to_string())
        );
        assert_eq!(
            lines.last().unwrap(),
            "  largeFlushCredit  0x0000000000000000"
        );
        // One line per word field, plus the three array headers and two entries.
        assert_eq!(lines.len(), 1 + ARENA_STATE_FIELDS.len() + 2);
    }

    #[test]
    fn blocks_print_their_headers_numbered_from_the_oldest() {
        let block = |address, prev, size| Block {
            address,
            prev,
            size,
            usable: size - 32,
            bump: 0,
        };
        assert_eq!(
            block_lines(&[block(0x2000, 0, 0x1000), block(0x3000, 0x2000, 0x2000)]),
            [
                "Arena Block (0):",
                "  prevBlock         0x0000000000000000",
                "  blockSize         0x0000000000001000",
                "  usableCapacity    0x0000000000000fe0",
                "  bumpOffset        0x0000000000000000",
                "Arena Block (1):",
                "  prevBlock         0x0000000000002000",
                "  blockSize         0x0000000000002000",
                "  usableCapacity    0x0000000000001fe0",
                "  bumpOffset        0x0000000000000000",
            ]
        );
    }

    #[test]
    fn a_dump_shows_offsets_bytes_and_printable_text() {
        let bytes = b"MFBASIC arena!\x00\x01\xffxyz";
        assert_eq!(
            dump_lines(0x1_2340, bytes),
            [
                "0x0001_2340 4d 46 42 41 53 49 43 20 61 72 65 6e 61 21 00 01 | M F B A S I C   a r e n a ! . .",
                "0x0001_2350 ff 78 79 7a                                     | . x y z",
            ]
        );
    }

    #[test]
    fn show_arena_parses_its_forms() {
        assert_eq!(
            parse("show arena"),
            Ok(Command::ShowArena { id: 0, dump: None })
        );
        assert_eq!(
            parse("show arena 3"),
            Ok(Command::ShowArena { id: 3, dump: None })
        );
        assert_eq!(
            parse("show arena 3 1"),
            Ok(Command::ShowArena {
                id: 3,
                dump: Some(Dump {
                    block: 1,
                    offset: 0,
                    size: 1024
                })
            })
        );
        assert_eq!(
            parse("show arena 3 1 0x40 32"),
            Ok(Command::ShowArena {
                id: 3,
                dump: Some(Dump {
                    block: 1,
                    offset: 0x40,
                    size: 32
                })
            })
        );
        for (line, message) in [
            (
                "show",
                "'show' needs what to show: show mem <where> [<size>], or show arena [<id>]",
            ),
            (
                "show it",
                "cannot show 'it'; 'show mem' and 'show arena' are what can be shown",
            ),
            (
                "show arena x",
                "'x' is not an arena id; type 'list arena' for the arenas",
            ),
            ("show arena 0 b", "'b' is not a block number"),
            ("show arena 0 0 zz", "'zz' is not a number"),
            ("show arena 0 0 0 0 0", "too many arguments to 'show arena'"),
        ] {
            assert_eq!(parse(line), Err(message.to_string()), "{line}");
        }
    }

    #[test]
    fn show_arena_without_a_registry_asks_for_a_debug_build() {
        let (_, err, _) = session("show arena\nq\n", Ending::Killed, false);
        assert_eq!(
            err,
            "error: showing an arena needs a debug build; rebuild the program with: mfb build \
             --debug -O0 <project>\n"
        );
    }
}
