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

use super::arch::Arch;
use super::backtrace;
use super::breakpoints::{self, Breakpoints, Ended, ErrorCodes, Finish, Target, REPORT_HINT};
use super::disassemble::{self, Request, Window};
use super::environment::{self, Environment};
use super::location::{self, Condition, Location};
use super::names::{classify, stop_name, Collisions, Origin};
use super::runtime::{self, Arena, Block, Role, ARENA_STATE_FIELDS};
use super::values::{self, Region, View};
use super::{
    Ending, Exit, FunctionSymbol, Inferior, Program, Register, RegisterGroup, RegisterValue,
    RunStop, StepStop, ThreadFacts,
};

mod actions;
mod command;
mod render;
mod stops;
pub(crate) use actions::*;
pub(crate) use command::*;
pub(crate) use render::*;
use stops::*;

pub(crate) const PROMPT: &str = "DBG> ";

/// The `help` listing: [`help`] fills in the processor's parts, `{arg0}` (the
/// first argument register), `{jump}` (what `jump` sets) and `{watch}` (what
/// `wp` takes).
const HELP: &str = "\
Commands:
  help, h         Show this list of commands
  list, l         Show what can be listed
  <where> is an address, a register or a function, each with an optional
  [+offset]; [<where>] is the 8 bytes stored there, also with an offset:
  sp+0x18 is sp plus 0x18, [sp+0x18] what is stored there, [[{arg0}]+8] what is
  stored 8 past where {arg0}'s own value points
  show            Show memory: show mem <where> [<size>]
                  The error a thread has raised and not yet caught: show error [<thread>]
                  An arena's state and blocks: show arena [<id>]
                  Dump a block: show arena <id> <block> [<offset>] [<size>]
                  An arena value as a type: show type <type> <where> (as show mem)
                  ('show type' alone for the types and their forms)
                  The performance and memory report a --debug build writes as
                  it exits, kept from the terminal: show report
  breakpoint, bp  Set a breakpoint: bp <address|function[+offset]> [<commands>]
                  Remove every one there: bp r <address|function[+offset]>
  breakpointif, bpi
                  Set a breakpoint that stops a thread only when a test holds:
                  bpi <where> eq|ne|gt|lt <value> at <address|function[+offset]>
                  [<commands>]; gt and lt compare signed numbers, and a test
                  whose <where> cannot be read does not stop. bpi <where> in at
                  ... stops just when it cannot be read
                  Remove one: bpi r <where> <test> [<value>] at
                  <address|function[+offset]>
  watchpoint, wp  Stop when a thread changes memory: wp <where> [<size>]
                  {watch}
  breakerror, be  Stop when a thread raises an error:
                  be <code|name>... | any [<commands>]
                  Stop watching: be clear; be alone shows what is watched
                  <commands> run at each stop: list, show, bt, dis, j, finish
                  or a bare be, separated by ';'; the program stays stopped
                  unless run is the last of them (bp fib bt 1; list reg {arg0};
                  run)
  run, r          Run the program until a breakpoint or its end (Ctrl-C pauses)
  finish, f       Run until the current function returns
  restart, rs     Start the program again (a program the debugger launched)
  env             Change the environment the program gets when it restarts
                  (a program the debugger launched; the running one keeps
                  its own): env s|set <name> <value> sets one, its value the
                  rest of the line; env r|remove <name> removes one; env
                  clear removes them all; env reset goes back to the one the
                  session started with; list env shows it
  disassemble, dis
                  Disassemble the current function; dis <count> instructions
                  from the current one (negative: before it), or dis -<back>
                  <count> to show <count> from <back> before it; dis <where>
                  [<count>] from an address, register or function, with an
                  offset (pc-0x20, main+0x110); or
                  dis <function> [<count>]
  backtrace, bt   Show the current thread's calls: bt [<count>] [<thread>]
                  Every thread's: bt all
  step, s         Run one thread: step [<count>] [<thread>] (default 1, thread 0)
  step-all, sa    Run every thread together: step-all [<count>] (default 1)
  jump, j         Move the current thread to <where>, setting up to 8 registers
                  first: j <where> [<register>=<value>]... {jump}
  quit, q         End the session: kill the program if the debugger launched
                  it, or detach and let it run on if the debugger attached to it

Each stop prints one line per thread it stopped, starting --[<kind>]:
  --[BP] T: <thread> P: <pc> F: <function>             a breakpoint
  --[WP] T: <thread> P: <pc> F: <function> M: <address>: <old> -> <new>
                                                        a watched change
  --[BE] T: <thread> P: <pc> F: <function> E: <error>  a watched error
  --[XC] T: <thread> P: <pc> F: <function> X: <what>   another exception
  --[CC] User Break                                     Ctrl-C
  --[EX] Program exit (<status>)                        the end
A --debug build's exit line ends: - type \"show report\" to see the
performance and memory report
<function> is (<written name>) <symbol>, the symbol alone when the function
has no written name; breakpoint takes either, the symbol when several
functions share the written name.";

/// What a bare `list` prints.
pub(crate) const LIST_USAGE: &str = "\
Usage: list <what>

Each listing starts with a line naming its columns; ' | ' separates them.

  func [<filter>...]
                The program's functions (also 'list function'), one per line:
                  <type> <address> <symbol> <written name>
                A <filter> that is a type (MU, MB, MR, SL, SD) keeps that
                type; any other is a name, matched against the symbol and the
                written name, where * stands for any characters and ? for
                one: list func MU, list func fib, list func *norm*.
                <type> is MU for a function the program's author wrote, MB
                for a built-in package's, MR for the MFBASIC runtime, SL for
                a function the program imports from a system library, SD for
                one in a library it loaded while running; a legend follows.
                <address> is - for a linked function the program has not
                loaded yet, and <written name> is - where the author wrote no
                name.
  thread        The program's threads: <id> <name> <pc> <task>
                <task> is App, MFB Main, User Thread or Other.
  reg           A thread's registers: gpr, simd, sys, or the ones named;
                on its own, the argument and result registers and how to
                ask for the rest.
  arena         The program's MFBASIC memory arenas (debug builds):
                  <id> <thread id> <address> <size in KB>
  breakpoint    The breakpoints: <address> <symbol> <written name>
                <address> is - for one waiting for its library to load; an
                <if> column follows when any has a test, and a <commands>
                column when any has commands.
  watchpoint    The watchpoints: <address> <size> <value>
                <value> is the bytes' value now; a <commands> column follows
                when any has commands.
  env           The environment variables, in name order (a program the
                debugger launched): <name> <current> <after restart>
                <current> is the running program's value, <after restart>
                the one 'env' has set for the next restart; a cell is
                empty where the variable is not set.";

/// What a bare `list reg` prints under its registers, and all it prints once
/// the program has ended: [`list_reg_usage`] fills in the processor's
/// `{groups}`, and `{arg0}`, `{arg1}` and `{sp}` in the example.
const LIST_REG_USAGE: &str = "\
Usage: list reg [<id>] [<what>...]

Shows the registers of thread <id> (default 0, the main thread) as
<register> <hex value> (<decimal>). With no <what> it shows the registers
that carry arguments and results, and the ones MFBASIC code keeps for
itself, each with its role. <what> is one or more of:

{groups}
  <register>    One register by name; list several with spaces or commas,
                as in: list reg {arg0}, {arg1}, {sp}";

/// The `help` listing for a program on `arch`.
pub(crate) fn help(arch: &Arch) -> String {
    HELP.replace("{arg0}", arch.arguments[0])
        .replace("{jump}", arch.help.jump)
        .replace("{watch}", arch.help.watch)
}

/// The `list reg` usage for a program on `arch`.
pub(crate) fn list_reg_usage(arch: &Arch) -> String {
    LIST_REG_USAGE
        .replace("{groups}", arch.help.groups)
        .replace("{arg0}", arch.arguments[0])
        .replace("{arg1}", arch.arguments[1])
        .replace("{sp}", arch.sp)
}

/// Run the prompt until the session ends, then end it. `echo` repeats each
/// command after the prompt (set when the input is not a terminal). Errors from
/// a command go to `err` and the prompt continues; an error ending the session
/// is returned.
pub(crate) fn run(
    inferior: &mut dyn Inferior,
    program: Program,
    environment: Option<Environment>,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
    echo: bool,
) -> Result<(), String> {
    let written = |result: std::io::Result<()>| result.map_err(|e| format!("cannot write: {e}"));
    let mut line = String::new();
    let mut breakpoints = Breakpoints::default();
    breakpoints.watch_report(inferior, &mut Vec::new())?;
    // A launched program's environment, which `env` changes for the next
    // restart; none for a program attached to.
    let mut environment = environment.map(environment::Session::new);
    // The thread `dis` shows: the last one a `run` or `step` stopped.
    let mut current = MAIN_THREAD;
    // Once the program has ended only `help`, `list`, `list breakpoint`, a bare
    // `list reg`, `show report`, `restart` and `quit` mean anything.
    let mut exited = false;
    // The commands of a stop that wait to run: they come before the next line.
    let mut queue = Queue::new();
    let mut faults = Faults::new();
    loop {
        line.clear();
        if let Some((thread, command)) = queue.pop_front() {
            current = thread;
            line.push_str(&command);
        } else {
            write!(out, "{PROMPT}")
                .and_then(|()| out.flush())
                .map_err(|e| format!("cannot write the prompt: {e}"))?;
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
                    write!(out, "{}", line.trim_end_matches(['\r', '\n']))
                        .and_then(|()| writeln!(out)),
                )?;
            }
        }
        let result = match parse(&line, inferior.arch()) {
            Ok(Command::Empty) => Ok(Vec::new()),
            Ok(Command::Help) => Ok(vec![help(inferior.arch())]),
            Ok(Command::Quit) => break,
            Ok(Command::TypeUsage) => Ok(values::TYPE_USAGE.lines().map(str::to_string).collect()),
            Ok(Command::List(
                listing @ (None | Some(Listing::Breakpoints | Listing::Watchpoints)),
            )) => list(inferior, program, &breakpoints, listing),
            // An ended program has no registers to show; the usage still reads.
            Ok(Command::List(Some(Listing::CommonRegisters { .. }))) if exited => {
                Ok(list_reg_usage(inferior.arch())
                    .lines()
                    .map(str::to_string)
                    .collect())
            }
            Ok(Command::Restart) => restart(
                inferior,
                &mut breakpoints,
                environment.as_ref().map_or(&[], environment::Session::next),
            )
            .inspect(|_| {
                exited = false;
                current = MAIN_THREAD;
                faults.clear();
                if let Some(environment) = &mut environment {
                    environment.restarted();
                }
            }),
            // The environment is for the next restart: it changes even once
            // the program has ended.
            Ok(Command::List(Some(Listing::Environment))) => environment
                .as_ref()
                .map(environment::Session::lines)
                .ok_or_else(|| launched_only("list env")),
            Ok(Command::Env(change)) => environment
                .as_mut()
                .ok_or_else(|| launched_only("env"))
                .and_then(|environment| environment.change(change)),
            Ok(Command::ShowReport) => Ok(breakpoints.report_lines(exited)),
            Ok(_) if exited => {
                Err("the program has exited; type 'quit' to end the session".to_string())
            }
            Ok(Command::Step { count, thread }) => {
                step(inferior, thread, count, &mut faults).map(|(lines, ended)| {
                    exited = ended;
                    current = thread;
                    ended_lines(lines, ended, &breakpoints)
                })
            }
            Ok(Command::StepAll(count)) => {
                step_all(inferior, count, &mut faults).map(|(lines, ended)| {
                    exited = ended;
                    ended_lines(lines, ended, &breakpoints)
                })
            }
            Ok(Command::SetBreakpoint {
                target,
                condition,
                commands,
            }) => condition
                .as_ref()
                .map_or(Ok(()), |condition| condition.check(inferior, current))
                .and_then(|()| breakpoints.set(inferior, &target, condition.as_ref(), &commands)),
            Ok(Command::RemoveBreakpoint { target, condition }) => {
                breakpoints.remove(inferior, &target, condition.as_ref())
            }
            Ok(Command::Jump { to, registers }) => jump(inferior, current, &to, &registers)
                .inspect(|_| {
                    faults.remove(&current);
                }),
            Ok(Command::SetWatchpoint {
                location,
                size,
                commands,
            }) => resolve(inferior, current, &location)
                .and_then(|(address, _)| breakpoints.watch(inferior, address, size, &commands)),
            Ok(Command::RemoveWatchpoint(location)) => resolve(inferior, current, &location)
                .and_then(|(address, _)| breakpoints.unwatch(inferior, address)),
            Ok(Command::BreakError(codes, commands)) => {
                breakpoints.watch_errors(inferior, codes, &commands)
            }
            Ok(Command::ClearErrors) => breakpoints.clear_errors(inferior),
            Ok(Command::Run) => {
                faults.clear();
                let ended = breakpoints.run(
                    inferior,
                    &mut |inferior: &mut dyn Inferior, breakpoints: &Breakpoints, notes, stop| {
                        hear_stop(
                            inferior,
                            program,
                            breakpoints,
                            notes,
                            stop,
                            &mut current,
                            &mut queue,
                            &mut faults,
                            out,
                            err,
                        )
                    },
                );
                ended.map(|ended| match ended {
                    Ended::Heard(stop) => {
                        exited = matches!(stop, RunStop::Exited(_));
                        Vec::new()
                    }
                    // Breakpoints only a `finish` places: none are left.
                    Ended::Own(hits) => {
                        let mut lines = breakpoints.take_notes();
                        lines.extend(stop_lines(
                            inferior,
                            &RunStop::Breakpoints(hits),
                            breakpoints.has_report(),
                        ));
                        lines
                    }
                })
            }
            Ok(Command::Finish) => {
                faults.clear();
                let thread = current;
                let outcome = breakpoints.finish(
                    inferior,
                    thread,
                    &mut |inferior: &mut dyn Inferior, breakpoints: &Breakpoints, notes, stop| {
                        hear_stop(
                            inferior,
                            program,
                            breakpoints,
                            notes,
                            stop,
                            &mut current,
                            &mut queue,
                            &mut faults,
                            out,
                            err,
                        )
                    },
                );
                outcome
                    .and_then(|outcome| finish_lines(inferior, &mut breakpoints, thread, outcome))
                    .map(|(lines, ended)| {
                        exited = ended;
                        lines
                    })
            }
            Ok(command) => inspect(inferior, program, &breakpoints, &faults, current, command),
            Err(message) => Err(message),
        };
        emit(out, err, result)?;
        if exited {
            queue.clear();
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
mod tests;
