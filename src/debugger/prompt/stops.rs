//! plan-176-L: the `mfb dbg` prompt's stops code. The command loop `run` and the help
//! text live in `debugger/prompt/mod.rs`.

use super::*;

/// Run the program and describe why it stopped; the flag is set when it ended.
/// The thread a stop names, which becomes the current thread.
fn stop_thread(stop: &RunStop) -> Option<usize> {
    match stop {
        RunStop::Breakpoints(hits) => hits.first().map(|(thread, _)| *thread),
        RunStop::Watchpoints(threads) => threads.first().copied(),
        RunStop::Changed(changes) => changes.first().map(|change| change.thread),
        RunStop::Exception { thread, .. } | RunStop::Raised { thread, .. } => Some(*thread),
        RunStop::Interrupted | RunStop::Exited(_) => None,
    }
}

/// A stop line's `T: <id> P: <pc> F: <function>`: the thread, the address of
/// its instruction (`pc`, else the thread's own), and the function holding it
/// as `(<written name>) <symbol>` with the offset into it — the executable's
/// as `locate_stop` names it, else a library's — `-` where none is known.
fn stop_place(inferior: &mut dyn Inferior, thread: usize, pc: Option<u64>) -> String {
    let threads = inferior.threads().unwrap_or_default();
    let Some(facts) = threads.get(thread) else {
        return "T: - P: - F: -".to_string();
    };
    let pc = pc.unwrap_or(facts.pc);
    let functions = inferior.functions().unwrap_or_default();
    // The executable's functions first; then a library's, by its symbols.
    let function = runtime::locate_stop(&functions, pc)
        .or_else(|| {
            let range = inferior.function_at(pc).ok().flatten()?;
            let symbol = range.name.as_deref()?;
            let written = disassemble::display_name(symbol, &Collisions::default());
            // A stub is named for the function it reaches, not written.
            let written = (!symbol.starts_with("symbol stub for: ")).then_some(written);
            Some(stop_name(written.as_deref(), symbol, pc - range.start))
        })
        .unwrap_or_else(|| "-".to_string());
    format!("T: {thread} P: 0x{pc:016x} F: {function}")
}

/// An error code as a `[BE]` line gives it: `errorCode::ErrOverflow
/// (77050010)`, or the number alone when no constant names it.
fn error_shown(code: i64) -> String {
    match runtime::error_code_name(code) {
        Some(name) => format!("{name} ({code})"),
        None => code.to_string(),
    }
}

/// The lines a stop prints, one per thread it stopped; `report` when the
/// session kept the program's debug report, which the end's line then says.
pub(super) fn stop_lines(inferior: &mut dyn Inferior, stop: &RunStop, report: bool) -> Vec<String> {
    let hint = if report {
        format!(" - {REPORT_HINT}")
    } else {
        String::new()
    };
    match stop {
        RunStop::Breakpoints(hits) => hits
            .iter()
            .map(|(thread, address)| {
                format!("--[BP] {}", stop_place(inferior, *thread, Some(*address)))
            })
            .collect(),
        RunStop::Watchpoints(threads) => threads
            .iter()
            .map(|thread| format!("--[WP] {}", stop_place(inferior, *thread, None)))
            .collect(),
        RunStop::Changed(changes) => changes
            .iter()
            .map(|change| {
                format!(
                    "--[WP] {} M: 0x{:016x}: {} -> {}",
                    stop_place(inferior, change.thread, Some(change.pc)),
                    change.address,
                    breakpoints::value_shown(change.old, change.size),
                    breakpoints::value_shown(change.new, change.size)
                )
            })
            .collect(),
        RunStop::Raised { thread, code, .. } => vec![format!(
            "--[BE] {} E: {}",
            stop_place(inferior, *thread, None),
            error_shown(*code)
        )],
        RunStop::Exception { thread, what } => vec![format!(
            "--[XC] {} X: {what}",
            stop_place(inferior, *thread, None)
        )],
        RunStop::Interrupted => vec!["--[CC] User Break".to_string()],
        RunStop::Exited(Exit::Status(status)) => {
            vec![format!("--[EX] Program exit ({status}){hint}")]
        }
        RunStop::Exited(Exit::Signal(signal)) => {
            vec![format!("--[EX] Program ended by signal ({signal}){hint}")]
        }
        RunStop::Exited(Exit::Unknown) => vec![format!("--[EX] Program exit{hint}")],
    }
}

/// Write a command's lines to `out`, or its error to `err` once `out` is
/// flushed.
pub(super) fn emit(
    out: &mut dyn Write,
    err: &mut dyn Write,
    result: Result<Vec<String>, String>,
) -> Result<(), String> {
    let written = |result: std::io::Result<()>| result.map_err(|e| format!("cannot write: {e}"));
    match result {
        Ok(lines) => {
            for line in lines {
                written(writeln!(out, "{line}"))?;
            }
            Ok(())
        }
        Err(message) => {
            written(out.flush())?;
            written(writeln!(err, "error: {message}"))
        }
    }
}

/// Hear a stop: write the notes and its lines, make its thread current, and
/// run the commands each of its breakpoints, watchpoints or error watch has,
/// each with its own thread current. Answers whether the program runs on:
/// when every stop in it has commands that end in `run`.
///
/// Commands that only read the program run here. A stop with a `finish`
/// among its commands leaves them all in `queue` instead, for the prompt to
/// run as if typed, with a `run` last when the program is to run on; a stop
/// heard while those run takes their place.
#[allow(clippy::too_many_arguments)]
pub(super) fn hear_stop(
    inferior: &mut dyn Inferior,
    program: Program,
    breakpoints: &Breakpoints,
    notes: Vec<String>,
    stop: &RunStop,
    current: &mut usize,
    queue: &mut Queue,
    faults: &mut Faults,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<bool, String> {
    if let Some(thread) = stop_thread(stop) {
        *current = thread;
    }
    if let RunStop::Exception { thread, what } = stop {
        faults.insert(*thread, what.clone());
    }
    let stopped = *current;
    let mut lines = notes;
    lines.extend(stop_lines(inferior, stop, breakpoints.has_report()));
    emit(out, err, Ok(lines))?;
    let (commands, every) = breakpoints.stop_commands(stop);
    let on = every
        && commands.iter().all(|(_, commands)| {
            commands
                .last()
                .is_some_and(|last| runs_on(last, inferior.arch()))
        });
    let finishes = commands.iter().any(|(_, commands)| {
        commands
            .iter()
            .any(|command| matches!(parse(command, inferior.arch()), Ok(Command::Finish)))
    });
    queue.clear();
    for (thread, commands) in commands {
        *current = thread;
        let arch = inferior.arch();
        for command in commands
            .into_iter()
            .filter(|command| !runs_on(command, arch))
        {
            if finishes {
                queue.push_back((thread, command));
                continue;
            }
            let result = parse(&command, arch).and_then(|command| match command {
                Command::Jump { to, registers } => {
                    jump(inferior, thread, &to, &registers).inspect(|_| {
                        faults.remove(&thread);
                    })
                }
                command => inspect(inferior, program, breakpoints, faults, thread, command),
            });
            emit(out, err, result)?;
        }
    }
    if !finishes {
        return Ok(on);
    }
    if on {
        queue.push_back((stopped, "run".to_string()));
    }
    Ok(false)
}

/// Run a command that only reads the program — what a stop's commands may
/// be — with `current` the current thread.
pub(super) fn inspect(
    inferior: &mut dyn Inferior,
    program: Program,
    breakpoints: &Breakpoints,
    faults: &Faults,
    current: usize,
    command: Command,
) -> Result<Vec<String>, String> {
    match command {
        Command::List(listing) => list(inferior, program, breakpoints, listing),
        Command::ShowArena { id, dump } => show_arena(inferior, program, id, dump),
        Command::ShowError(thread) => {
            let thread = thread.unwrap_or(current);
            show_error(inferior, program, thread, faults.get(&thread))
        }
        Command::ErrorWatch => Ok(vec![breakpoints
            .watch_line()
            .unwrap_or_else(|| "No errors are being watched for.".to_string())]),
        // A stop is not the end: only the prompt asks once the program has.
        Command::ShowReport => Ok(breakpoints.report_lines(false)),
        Command::Disassemble(request) => disassemble_at(inferior, current, &request),
        Command::Backtrace(request) => backtrace(inferior, program, current, request),
        Command::ShowType { location, view } => {
            show_type(inferior, program, current, &location, &view)
        }
        Command::ShowMemory { location, size } => show_memory(inferior, current, &location, size),
        // `stop_commands` and the prompt pass only the commands above.
        other => unreachable!("{other:?} is not a command that only reads the program"),
    }
}

/// The lines after a `finish`: the notes, then where the thread returned to
/// and what it returned (MFBASIC returns an error code in the first result
/// register, `x0` on AArch64, and the value in the second), or that the
/// finish did not complete; `true` when the program ended.
pub(super) fn finish_lines(
    inferior: &mut dyn Inferior,
    breakpoints: &mut Breakpoints,
    thread: usize,
    outcome: Finish,
) -> Result<(Vec<String>, bool), String> {
    let mut lines = breakpoints.take_notes();
    let collisions = Collisions::of(&inferior.functions().unwrap_or_default());
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
                    .map(|register| {
                        let (hex, decimal) = register_value(register);
                        format!("{hex} {decimal}")
                    })
                    .unwrap_or_default()
            };
            let from = from
                .as_deref()
                .map(|name| disassemble::display_name(name, &collisions))
                .unwrap_or_else(|| "the function".to_string());
            let [code, result, ..] = inferior.arch().arguments;
            lines.push(format!(
                "Thread {thread} returned from {from} to {at}: {code} = {}, {result} = {}.",
                value(code),
                value(result)
            ));
            Ok((lines, false))
        }
        Finish::Stopped { stop, from } => {
            let exited = matches!(stop, RunStop::Exited(_));
            // A breakpoint, an exception or a pause (a recursive call hitting
            // a breakpoint, most often) ends the finish early: say so, since
            // the stop alone reads like any other.
            if !exited {
                let what = match from.as_deref() {
                    Some(name) => format!(
                        "The finish from {}",
                        disassemble::display_name(name, &collisions)
                    ),
                    None => "The finish".to_string(),
                };
                lines.push(format!(
                    "{what} did not complete; type 'finish' to run it again from here."
                ));
            }
            Ok((lines, exited))
        }
    }
}

/// `restart`: the program again, as the debugger first launched it, with the
/// breakpoints placed anew.
pub(super) fn restart(
    inferior: &mut dyn Inferior,
    breakpoints: &mut Breakpoints,
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
) -> Result<Vec<String>, String> {
    let pid = inferior.restart(environment)?;
    let mut lines = vec![format!(
        "Restarted the program as process {pid}; paused before its first instruction."
    )];
    lines.extend(breakpoints.restarted(inferior)?);
    Ok(lines)
}
