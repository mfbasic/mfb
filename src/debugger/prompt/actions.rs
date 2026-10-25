//! plan-176-L: the `mfb dbg` prompt's actions code. The command loop `run` and the help
//! text live in `debugger/prompt/mod.rs`.

use super::*;

/// The lines a `list` prints, or the error it reports.
pub(super) fn list(
    inferior: &mut dyn Inferior,
    program: Program,
    breakpoints: &Breakpoints,
    listing: Option<Listing>,
) -> Result<Vec<String>, String> {
    match listing {
        None => Ok(LIST_USAGE.lines().map(str::to_string).collect()),
        Some(Listing::Breakpoints) => Ok(breakpoints.lines()),
        Some(Listing::Watchpoints) => Ok(breakpoints.watch_lines(inferior)),
        // The prompt lists the session's environment itself, and
        // `stop_commands` refuses it at a stop.
        Some(Listing::Environment) => unreachable!("'list env' is the prompt's to answer"),
        Some(Listing::Functions(filter)) => {
            let lines = function_lines(&inferior.functions()?, &filter);
            if lines.is_empty() && filter != FunctionFilter::default() {
                return Err(format!(
                    "no function matches '{}'; type 'list func' for the functions",
                    filter.words()
                ));
            }
            Ok(lines)
        }
        Some(Listing::Threads) => {
            let threads = inferior.threads()?;
            let roles = runtime::roles(inferior, &threads, program.app)?;
            let functions = inferior.functions().unwrap_or_default();
            Ok(thread_lines(&threads, &roles, &functions))
        }
        Some(Listing::CommonRegisters { thread }) => {
            let count = inferior.threads()?.len();
            if thread >= count {
                return Err(no_thread(thread, count));
            }
            let mut lines = common_register_lines(&inferior.registers(thread)?);
            lines.push(String::new());
            lines.extend(list_reg_usage(inferior.arch()).lines().map(str::to_string));
            Ok(lines)
        }
        Some(Listing::Registers { thread, picks }) => {
            let count = inferior.threads()?.len();
            if thread >= count {
                return Err(no_thread(thread, count));
            }
            let registers = inferior.registers(thread)?;
            let mut chosen = Vec::new();
            for pick in &picks {
                match pick {
                    RegisterPick::Group(group) => chosen.extend(
                        registers
                            .iter()
                            .filter(|register| register.group == *group)
                            .cloned(),
                    ),
                    RegisterPick::Name(name) => chosen.push(
                        registers
                            .iter()
                            .find(|register| &register.name == name)
                            .cloned()
                            .ok_or_else(|| {
                                format!("no register '{name}'; type 'list reg' for the groups")
                            })?,
                    ),
                }
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

/// Why `what` refuses a program the debugger attached to.
pub(super) fn launched_only(what: &str) -> String {
    format!("'{what}' needs a program the debugger launched; this one was attached to")
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
pub(super) fn step(
    inferior: &mut dyn Inferior,
    thread: usize,
    count: u64,
    faults: &mut Faults,
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
    faults.remove(&thread);
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
                    faults.insert(thread, what.clone());
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

/// A step's `lines`, with where the debug report is after them when the step
/// `ended` the program and the session kept one.
pub(super) fn ended_lines(
    mut lines: Vec<String>,
    ended: bool,
    breakpoints: &Breakpoints,
) -> Vec<String> {
    if ended && breakpoints.has_report() {
        let mut hint = REPORT_HINT.to_string();
        hint[..1].make_ascii_uppercase();
        lines.push(format!("{hint}."));
    }
    lines
}

/// Step every thread together `count` rounds and describe where each stopped;
/// the flag is set when the program ended.
pub(super) fn step_all(
    inferior: &mut dyn Inferior,
    count: u64,
    faults: &mut Faults,
) -> Result<(Vec<String>, bool), String> {
    let threads = inferior.threads()?.len();
    let ids = (0..threads).collect::<Vec<_>>();
    let outcomes = if count == 0 {
        Vec::new()
    } else {
        faults.clear();
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
                            faults.insert(*id, what.clone());
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
pub(super) fn show_arena(
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

/// `show mem`: `size` bytes at an address or at a register of the current
/// thread plus an offset.
pub(super) fn show_memory(
    inferior: &mut dyn Inferior,
    current: usize,
    location: &Location,
    size: u64,
) -> Result<Vec<String>, String> {
    if size == 0 || size > MAX_MEMORY {
        return Err(format!("cannot show {size} bytes; give 1 to {MAX_MEMORY}"));
    }
    let (address, described) = resolve(inferior, current, location)?;
    let bytes = inferior.read_bytes(address, size as usize)?;
    let mut lines = vec![format!("Memory at 0x{address:016x}{described}:")];
    lines.extend(memory_lines(address, &bytes));
    Ok(lines)
}

/// `show type`: the value at `location` read as the type `view` names. It
/// must be in one of the program's arena blocks, which needs a `--debug`
/// build's arena registry to find.
pub(super) fn show_type(
    inferior: &mut dyn Inferior,
    program: Program,
    current: usize,
    location: &Location,
    view: &View,
) -> Result<Vec<String>, String> {
    let (address, _) = resolve(inferior, current, location)?;
    // A `String` literal is no arena value, but is laid out as one.
    if matches!(view, View::String) {
        if let Some(symbol) = inferior
            .data_symbol_at(address)?
            .filter(|symbol| symbol.starts_with(STRING_LITERAL_PREFIX))
        {
            return values::show_literal(
                &mut |at, length| inferior.read_bytes(at, length),
                address,
                &symbol,
            );
        }
    }
    let threads = inferior.threads()?;
    let roles = runtime::roles(inferior, &threads, program.app)?;
    let Some(arenas) = runtime::arenas(inferior, &threads, &roles)? else {
        return Err(
            "'show type' reads values in the arena, which needs a debug build; \
                    rebuild the program with: mfb build --debug -O0 <project>"
                .to_string(),
        );
    };
    if arenas.is_empty() {
        return Err("no arenas yet: the program's runtime has not started".to_string());
    }
    let mut regions = Vec::new();
    for arena in &arenas {
        for (number, block) in runtime::blocks(inferior, arena.address)?.iter().enumerate() {
            regions.push(Region::of_block(
                arena.id,
                number,
                block.address,
                block.size,
            ));
        }
    }
    values::show(
        &mut |at, length| inferior.read_bytes(at, length),
        &regions,
        address,
        view,
    )
}

/// `show error [<thread>]`: the error parked in the thread's arena — the one in
/// flight, raised and not yet caught — with its code, message and origin.
pub(super) fn show_error(
    inferior: &mut dyn Inferior,
    program: Program,
    thread: usize,
    fault: Option<&String>,
) -> Result<Vec<String>, String> {
    let threads = inferior.threads()?;
    if thread >= threads.len() {
        return Err(no_thread(thread, threads.len()));
    }
    let roles = runtime::roles(inferior, &threads, program.app)?;
    let Some(arenas) = runtime::arenas(inferior, &threads, &roles)? else {
        return Err(
            "'show error' reads the thread's arena, which needs a debug build; rebuild the \
             program with: mfb build --debug -O0 <project>"
                .to_string(),
        );
    };
    let arena = arenas
        .iter()
        .find(|arena| arena.thread == Some(thread))
        .ok_or_else(|| format!("thread {thread} has no arena: it is running no MFBASIC code"))?;
    let Some(error) = runtime::current_error(inferior, arena.address)? else {
        // A fault is no error: nothing raised it and no `TRAP` catches it.
        return Ok(vec![match fault {
            Some(what) => format!(
                "Thread {thread} has no error in flight: it stopped on {what}, which is a \
                 fault, not an error a TRAP can catch. Use bt and list reg to see where."
            ),
            None => format!("Thread {thread} has no error in flight."),
        }]);
    };
    let code = match runtime::error_code_name(error.code) {
        Some(name) => format!("{} ({name})", error.code),
        None => error.code.to_string(),
    };
    Ok(vec![
        format!(
            "Thread {thread}'s error in flight (the Error at 0x{:016x}):",
            error.address
        ),
        format!("Code: {code}"),
        format!("Message: \"{}\"", error.message.escape_debug()),
        format!(
            "Location: {}",
            error
                .source
                .as_deref()
                .unwrap_or("- (the error has no origin)")
        ),
    ])
}

/// The one start of the function `name` names, as `dis <function>` resolves
/// it; an error when it names none, or several (overloads, copies).
pub(crate) fn function_start(inferior: &mut dyn Inferior, name: &str) -> Result<u64, String> {
    let ranges = disassemble::resolve(inferior, name)?;
    match ranges.as_slice() {
        [range] => Ok(range.start),
        _ => Err(format!(
            "'{name}' names {} functions, at {}; give one by its address",
            ranges.len(),
            ranges
                .iter()
                .map(|range| format!("0x{:016x}", range.start))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// The address `location` names for thread `current`, and how to describe it
/// when it is more than a number: ` (<location>)`, with ` of thread <t>` when
/// a register went into it.
pub(super) fn resolve(
    inferior: &mut dyn Inferior,
    current: usize,
    location: &Location,
) -> Result<(u64, String), String> {
    let value = location
        .value(inferior, current)
        .map_err(location::Fault::message)?;
    let described = match (&location.term, value.register) {
        (location::Term::Number(_), _) => String::new(),
        (_, true) => format!(" ({} of thread {current})", location.shown()),
        (_, false) => format!(" ({})", location.shown()),
    };
    Ok((value.value, described))
}

/// `jump <where> [<register>=<value>]...`: put thread `thread` at `<where>`
/// with the registers given, every value worked out before any is set. The
/// thread stays paused.
pub(super) fn jump(
    inferior: &mut dyn Inferior,
    thread: usize,
    to: &Location,
    registers: &[(String, Location)],
) -> Result<Vec<String>, String> {
    let threads = inferior.threads()?.len();
    if thread >= threads {
        return Err(no_thread(thread, threads));
    }
    let (address, _) = resolve(inferior, thread, to)?;
    let alignment = inferior.arch().alignment;
    if address % alignment != 0 {
        return Err(format!(
            "0x{address:016x} is not the start of an instruction: they start every {alignment} \
             bytes"
        ));
    }
    inferior
        .read_code(address, alignment as usize)
        .map_err(|error| format!("cannot jump to 0x{address:016x}: {error}"))?;
    let mut values = vec![("pc".to_string(), address)];
    for (register, location) in registers {
        let value = location
            .value(inferior, thread)
            .map_err(|fault| format!("cannot set {register}: {}", fault.message()))?
            .value;
        (inferior.arch().holds)(register, value)?;
        values.push((register.clone(), value));
    }
    inferior.set_registers(thread, &values)?;
    let place = places(inferior)?
        .get(thread)
        .cloned()
        .unwrap_or_else(|| format!("0x{address:016x}"));
    let set = values[1..]
        .iter()
        .map(|(register, value)| format!("{register} = {}", breakpoints::value_shown(*value, 8)))
        .collect::<Vec<_>>();
    Ok(vec![if set.is_empty() {
        format!("Moved thread {thread} to {place}.")
    } else {
        format!("Moved thread {thread} to {place}; set {}.", set.join(", "))
    }])
}

/// The `backtrace` lines: one thread's frames, or every thread's, the
/// current thread marked. A current thread that has since ended falls back to
/// thread 0.
pub(super) fn backtrace(
    inferior: &mut dyn Inferior,
    program: Program,
    current: usize,
    request: Backtrace,
) -> Result<Vec<String>, String> {
    let threads = inferior.threads()?;
    let roles = runtime::roles(inferior, &threads, program.app)?;
    let current = if current < threads.len() {
        current
    } else {
        MAIN_THREAD
    };
    let (chosen, limit) = match request {
        Backtrace::All => ((0..threads.len()).collect(), backtrace::MAX_FRAMES),
        Backtrace::Thread { count, thread } => {
            let thread = thread.unwrap_or(current);
            if thread >= threads.len() {
                return Err(no_thread(thread, threads.len()));
            }
            (vec![thread], count.unwrap_or(backtrace::MAX_FRAMES))
        }
    };
    let mut lines = Vec::new();
    let collisions = Collisions::of(&inferior.functions().unwrap_or_default());
    for thread in chosen {
        let frames = backtrace::frames(inferior, thread, limit)?;
        lines.extend(backtrace::lines(
            thread,
            threads[thread].name.as_deref(),
            roles[thread].label(),
            &frames,
            thread == current,
            &collisions,
        ));
    }
    Ok(lines)
}

/// `dis` for the current thread: its `pc` is where `dis` starts and what it
/// marks. A current thread that has since ended falls back to thread 0.
pub(super) fn disassemble_at(
    inferior: &mut dyn Inferior,
    current: usize,
    disassembly: &Disassembly,
) -> Result<Vec<String>, String> {
    let threads = inferior.threads()?;
    let thread = threads
        .get(current)
        .or_else(|| threads.first())
        .ok_or_else(|| no_thread(current, threads.len()))?;
    let pc = thread.pc;
    let request = match disassembly {
        Disassembly::Current => Request::Current,
        Disassembly::Count(count) => Request::Count(*count),
        Disassembly::At(location, count) => Request::At {
            address: resolve(inferior, current, location)?.0,
            count: *count,
        },
        Disassembly::Named(name, count) => Request::Named {
            name: name.clone(),
            count: *count,
        },
    };
    disassemble::disassemble(inferior, pc, &request)
}

/// Where each thread is: its pc, and the function holding it when the
/// executable names one.
pub(super) fn places(inferior: &mut dyn Inferior) -> Result<Vec<String>, String> {
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
