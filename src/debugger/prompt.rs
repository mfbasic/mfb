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

use super::backtrace;
use super::breakpoints::{self, Breakpoints, Ended, ErrorCodes, Finish, Target, REPORT_HINT};
use super::disassemble::{self, Request, Window};
use super::environment::{self, Environment};
use super::location::{self, is_register, Condition, Location};
use super::names::{classify, stop_name, Collisions, Origin};
use super::runtime::{self, Arena, Block, Role, ARENA_STATE_FIELDS};
use super::values::{self, Region, View};
use super::{
    Ending, Exit, FunctionSymbol, Inferior, Program, Register, RegisterGroup, RegisterValue,
    RunStop, StepStop, ThreadFacts,
};

pub(crate) const PROMPT: &str = "DBG> ";

/// The `help` listing.
pub(crate) const HELP: &str = "\
Commands:
  help, h         Show this list of commands
  list, l         Show what can be listed
  <where> is an address, a register or a function, each with an optional
  [+offset]; [<where>] is the 8 bytes stored there, also with an offset:
  sp+0x18 is sp plus 0x18, [sp+0x18] what is stored there, [[x0]+8] what is
  stored 8 past where x0's own value points
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
                  [<commands>], <where> as show mem; <size> is 1 to 8 bytes
                  (default 8) that do not cross a multiple of 8
                  Remove one: wp r <where>
  breakerror, be  Stop when a thread raises an error:
                  be <code|name>... | any [<commands>]
                  Stop watching: be clear; be alone shows what is watched
                  <commands> run at each stop: list, show, bt, dis, j, finish
                  or a bare be, separated by ';'; the program stays stopped
                  unless run is the last of them (bp fib bt 1; list reg x0;
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
                  first: j <where> [<register>=<value>]... (x0-x28, fp, lr, sp,
                  nzcv, fpsr, fpcr; a value is a number or a <where>); the
                  thread stays paused
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
/// the program has ended.
pub(crate) const LIST_REG_USAGE: &str = "\
Usage: list reg [<id>] [<what>...]

Shows the registers of thread <id> (default 0, the main thread) as
<register> <hex value> (<decimal>). With no <what> it shows the registers
that carry arguments and results, and the ones MFBASIC code keeps for
itself, each with its role. <what> is one or more of:

  gpr           The general-purpose registers: x0 to x28, fp (x29), lr (x30)
  simd          The SIMD and floating-point registers: v0 to v31
  sys           The system registers: sp, pc, nzcv, fpsr, fpcr
  <register>    One register by name; list several with spaces or commas,
                as in: list reg x0, x1, sp";

/// The thread a bare `step` runs and `list reg` shows when given no thread:
/// the initial thread.
const MAIN_THREAD: usize = 0;

/// One parsed command line.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Help,
    List(Option<Listing>),
    Step {
        count: u64,
        thread: usize,
    },
    StepAll(u64),
    ShowArena {
        id: usize,
        dump: Option<Dump>,
    },
    /// `show error [<thread>]`: the error parked in a thread's arena.
    ShowError(Option<usize>),
    /// `show report`: the debug report the program wrote as it exited.
    ShowReport,
    /// `breakerror <code>|any [<commands>]`: stop when a thread raises it.
    BreakError(ErrorCodes, Vec<String>),
    /// `breakerror clear`.
    ClearErrors,
    /// `breakerror` alone: what is being watched for.
    ErrorWatch,
    /// `breakpoint <target> [<commands>]`, or `breakpointif <test> at
    /// <target> [<commands>]` with a `condition`.
    SetBreakpoint {
        target: Target,
        condition: Option<Condition>,
        commands: Vec<String>,
    },
    /// `breakpoint r <target>` (every one there), or `breakpointif r <test>
    /// at <target>` (just the ones with that test).
    RemoveBreakpoint {
        target: Target,
        condition: Option<Condition>,
    },
    /// `watchpoint <where> [<size>] [<commands>]`.
    SetWatchpoint {
        location: Location,
        size: u64,
        commands: Vec<String>,
    },
    RemoveWatchpoint(Location),
    Run,
    Disassemble(Disassembly),
    Finish,
    Restart,
    ShowMemory {
        location: Location,
        size: u64,
    },
    ShowType {
        location: Location,
        view: View,
    },
    TypeUsage,
    Backtrace(Backtrace),
    /// `jump <where> [<register>=<value>]...`.
    Jump {
        to: Location,
        registers: Vec<(String, Location)>,
    },
    /// `env s|r|clear|reset ...`: change the environment the next restart
    /// gives.
    Env(environment::Change),
    Quit,
    Empty,
}

/// Which threads' calls `backtrace` shows.
#[derive(Debug, PartialEq, Eq)]
enum Backtrace {
    /// The innermost `count` frames (default all) of `thread` (default the
    /// current thread).
    Thread {
        count: Option<usize>,
        thread: Option<usize>,
    },
    /// Every frame of every thread.
    All,
}

/// Which bytes of an arena block `show arena` dumps.
#[derive(Debug, PartialEq, Eq)]
struct Dump {
    block: usize,
    offset: u64,
    size: u64,
}

/// What a `dis` names, before a location is resolved to an address.
#[derive(Debug, PartialEq, Eq)]
enum Disassembly {
    Current,
    Count(Window),
    At(Location, Option<Window>),
    Named(String, Option<Window>),
}

impl Disassembly {
    /// `dis <start> [<count>]`: a location (an address, or a register, either
    /// with an offset), else a function name.
    fn from(start: &str, count: Option<Window>) -> Result<Disassembly, String> {
        let location = start.starts_with("0x")
            || start.starts_with("0X")
            || start.starts_with('[')
            || start.contains(['+', '-'])
            || is_register(start);
        Ok(if location {
            Disassembly::At(Location::parse(start)?, count)
        } else {
            Disassembly::Named(start.to_string(), count)
        })
    }
}

/// Whether `word` reads as a count: digits, perhaps after a `-`.
fn is_count(word: &str) -> bool {
    let digits = word.strip_prefix('-').unwrap_or(word);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// A `dis` count: any whole number but 0, negative to count back.
fn instruction_count(word: &str) -> Result<i64, String> {
    match word.parse::<i64>() {
        Ok(0) | Err(_) => Err(format!(
            "cannot show {word} instructions; give a count other than 0 (negative counts back)"
        )),
        Ok(count) => Ok(count),
    }
}

/// The instructions one `dis` count picks: see [`Window::count`].
fn instruction_window(word: &str) -> Result<Window, String> {
    instruction_count(word).map(Window::count)
}

/// `dis … -<back> <length>`: `<length>` instructions, from `<back>` before
/// the starting point.
fn instruction_span(back: &str, length: &str) -> Result<Window, String> {
    let back = match instruction_count(back)? {
        count if count < 0 => count.unsigned_abs(),
        _ => {
            return Err(format!(
                "'{back}' does not count back; with two counts the first is negative, as in \
                 dis -6 8"
            ))
        }
    };
    let length = match instruction_count(length)? {
        count if count > 0 => count as u64,
        _ => {
            return Err(format!(
                "cannot show {length} instructions; the count after a negative one must be \
                 positive"
            ))
        }
    };
    Ok(Window { back, length })
}

/// `0x<hex>`, perhaps followed by `+` or `-` and an offset (decimal, or hex
/// after `0x`): the address it names. `None` when `word` does not start with
/// `0x`.
pub(crate) fn address(word: &str) -> Option<Result<u64, String>> {
    let hex = word
        .strip_prefix("0x")
        .or_else(|| word.strip_prefix("0X"))?;
    let bad = || format!("'{word}' is not an address");
    let (base, offset) = match hex.find(['+', '-']) {
        Some(at) => (&hex[..at], Some(&hex[at..])),
        None => (hex, None),
    };
    let Ok(base) = u64::from_str_radix(base, 16) else {
        return Some(Err(bad()));
    };
    let Some(offset) = offset else {
        return Some(Ok(base));
    };
    let Ok(amount) = number(&offset[1..]) else {
        return Some(Err(bad()));
    };
    Some(if offset.starts_with('-') {
        base.checked_sub(amount)
            .ok_or_else(|| format!("'{word}' is below address 0"))
    } else {
        base.checked_add(amount)
            .ok_or_else(|| format!("'{word}' is past the last address"))
    })
}

/// The symbols of the `String` literals the executable carries
/// (`crate::codegen::memory::data::data_objects`).
const STRING_LITERAL_PREFIX: &str = "_mfb_str_";

/// How much `show mem` shows when no size is given, and the most it shows.
const MEMORY_SIZE: u64 = 64;
const MAX_MEMORY: u64 = 64 << 10;

/// The arena a bare `show arena` shows: the main arena's registry slot.
const MAIN_ARENA: usize = 0;
/// How much of a block a dump shows when no size is given.
const DUMP_SIZE: u64 = 1024;

#[derive(Debug, PartialEq, Eq)]
enum Listing {
    Functions(FunctionFilter),
    Threads,
    /// A bare `list reg`: the registers that have a role, then the usage.
    CommonRegisters {
        thread: usize,
    },
    Registers {
        thread: usize,
        picks: Vec<RegisterPick>,
    },
    Arenas,
    Breakpoints,
    Watchpoints,
    /// `list env`: the environment the next restart gives.
    Environment,
}

/// Which functions `list func` shows: those of any of `types` and matching any
/// of `patterns`, either left empty to mean any.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FunctionFilter {
    types: Vec<Origin>,
    patterns: Vec<String>,
}

impl FunctionFilter {
    /// `list func`'s words: a type code (`MU`, `MB`, `MR`, `SL`, `SD`), else a
    /// name pattern.
    fn parse(words: &[&str]) -> FunctionFilter {
        let mut filter = FunctionFilter::default();
        for word in words {
            match Origin::parse(word) {
                Some(origin) => filter.types.push(origin),
                None => filter.patterns.push(word.to_string()),
            }
        }
        filter
    }

    /// Whether a function of type `origin`, spelled `symbol` and written
    /// `written`, is listed: a pattern matches either name.
    fn admits(&self, origin: Origin, symbol: &str, written: Option<&str>) -> bool {
        (self.types.is_empty() || self.types.contains(&origin))
            && (self.patterns.is_empty()
                || self.patterns.iter().any(|pattern| {
                    glob(pattern, symbol) || written.is_some_and(|name| glob(pattern, name))
                }))
    }

    /// The filter as the user typed it, for a listing that matches nothing.
    fn words(&self) -> String {
        self.types
            .iter()
            .map(|origin| origin.code().to_string())
            .chain(self.patterns.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Whether `pattern` matches all of `text`: `*` stands for any run of
/// characters, `?` for any one, and every other character for itself.
fn glob(pattern: &str, text: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let text = text.chars().collect::<Vec<_>>();
    let (mut at, mut from) = (0, 0);
    // The last `*` seen, and where in `text` its run currently ends.
    let mut star: Option<(usize, usize)> = None;
    while from < text.len() {
        match pattern.get(at) {
            Some('*') => {
                star = Some((at, from));
                at += 1;
            }
            Some(&wanted) if wanted == '?' || wanted == text[from] => {
                at += 1;
                from += 1;
            }
            _ => match star {
                // Let the last `*` take one more character and try again.
                Some((star_at, star_end)) => {
                    star = Some((star_at, star_end + 1));
                    at = star_at + 1;
                    from = star_end + 1;
                }
                None => return false,
            },
        }
    }
    pattern[at..].iter().all(|wanted| *wanted == '*')
}

/// What a `list reg` argument asks for: a group, or one register by name.
#[derive(Debug, PartialEq, Eq)]
enum RegisterPick {
    Group(RegisterGroup),
    Name(String),
}

impl RegisterPick {
    fn parse(word: &str) -> RegisterPick {
        match word {
            "gpr" => RegisterPick::Group(RegisterGroup::General),
            "simd" => RegisterPick::Group(RegisterGroup::Simd),
            "sys" => RegisterPick::Group(RegisterGroup::System),
            name => RegisterPick::Name(name.to_string()),
        }
    }
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
        ("list" | "l", ["func" | "function", filter @ ..]) => Ok(Command::List(Some(
            Listing::Functions(FunctionFilter::parse(filter)),
        ))),
        ("list" | "l", ["thread"]) => Ok(Command::List(Some(Listing::Threads))),
        ("list" | "l", ["arena"]) => Ok(Command::List(Some(Listing::Arenas))),
        ("list" | "l", ["breakpoint"]) => Ok(Command::List(Some(Listing::Breakpoints))),
        ("list" | "l", ["watchpoint"]) => Ok(Command::List(Some(Listing::Watchpoints))),
        ("list" | "l", ["env"]) => Ok(Command::List(Some(Listing::Environment))),
        ("list" | "l", ["reg", rest @ ..]) => {
            // Commas separate registers as well as spaces: `x0, x1,x2`.
            let words = rest
                .iter()
                .flat_map(|word| word.split(','))
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>();
            let (thread, picks) = match words.first() {
                Some(first) if first.bytes().all(|byte| byte.is_ascii_digit()) => {
                    (thread_id(first)?, &words[1..])
                }
                _ => (MAIN_THREAD, &words[..]),
            };
            if picks.is_empty() {
                return Ok(Command::List(Some(Listing::CommonRegisters { thread })));
            }
            Ok(Command::List(Some(Listing::Registers {
                thread,
                picks: picks.iter().map(|word| RegisterPick::parse(word)).collect(),
            })))
        }
        ("list" | "l", ["thread" | "arena" | "breakpoint" | "watchpoint" | "env", ..]) => {
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
        ("backtrace" | "bt", []) => Ok(Command::Backtrace(Backtrace::Thread {
            count: None,
            thread: None,
        })),
        ("backtrace" | "bt", ["all"]) => Ok(Command::Backtrace(Backtrace::All)),
        ("backtrace" | "bt", [count]) => Ok(Command::Backtrace(Backtrace::Thread {
            count: Some(frame_count(count)?),
            thread: None,
        })),
        ("backtrace" | "bt", [count, thread]) => Ok(Command::Backtrace(Backtrace::Thread {
            count: Some(frame_count(count)?),
            thread: Some(thread_id(thread)?),
        })),
        ("backtrace" | "bt", _) => Err(format!("'{name}' takes at most two arguments")),
        ("step-all" | "sa", []) => Ok(Command::StepAll(1)),
        ("step-all" | "sa", [count]) => Ok(Command::StepAll(step_count(count)?)),
        ("step-all" | "sa", _) => Err(format!("'{name}' takes at most one argument")),
        ("breakpoint" | "bp", ["r"]) | ("breakpoint" | "bp", []) => Err(format!(
            "'{name}' needs an address (0x…) or a function name"
        )),
        ("breakpoint" | "bp", ["r", target]) => Ok(Command::RemoveBreakpoint {
            target: Target::parse(target)?,
            condition: None,
        }),
        ("breakpoint" | "bp", ["r", ..]) => Err(format!("too many arguments to '{name} r'")),
        ("breakpoint" | "bp", [target, commands @ ..]) => Ok(Command::SetBreakpoint {
            target: Target::parse(target)?,
            condition: None,
            commands: stop_commands(commands)?,
        }),
        ("breakpointif" | "bpi", ["r", words @ ..]) => {
            let (condition, target, rest) = conditional(name, words)?;
            if !rest.is_empty() {
                return Err(format!("too many arguments to '{name} r'"));
            }
            Ok(Command::RemoveBreakpoint {
                target,
                condition: Some(condition),
            })
        }
        ("breakpointif" | "bpi", words) => {
            let (condition, target, commands) = conditional(name, words)?;
            Ok(Command::SetBreakpoint {
                target,
                condition: Some(condition),
                commands: stop_commands(commands)?,
            })
        }
        ("jump" | "j", []) => Err(format!(
            "'{name}' needs a place to go: {name} <where> [<register>=<value>]..."
        )),
        ("jump" | "j", [to, sets @ ..]) => {
            if sets.len() > MAX_JUMP_REGISTERS {
                return Err(format!(
                    "'{name}' sets at most {MAX_JUMP_REGISTERS} registers"
                ));
            }
            let mut registers: Vec<(String, Location)> = Vec::new();
            for set in sets {
                let Some((register, value)) = set.split_once('=') else {
                    return Err(format!(
                        "'{set}' is not a register and its value: <register>=<value>, such as x0=5"
                    ));
                };
                settable(register)?;
                if value.is_empty() {
                    return Err(format!("'{register}' needs a value: {register}=<value>"));
                }
                if registers.iter().any(|(other, _)| other == register) {
                    return Err(format!("'{register}' is set twice"));
                }
                registers.push((register.to_string(), assigned(value)?));
            }
            Ok(Command::Jump {
                to: Location::parse(to)?,
                registers,
            })
        }
        ("watchpoint" | "wp", ["r"] | []) => Err(format!(
            "'{name}' needs an address or a register: {name} <where> [<size>] [<commands>]"
        )),
        ("watchpoint" | "wp", ["r", location]) => {
            Ok(Command::RemoveWatchpoint(Location::parse(location)?))
        }
        ("watchpoint" | "wp", ["r", ..]) => Err(format!("too many arguments to '{name} r'")),
        ("watchpoint" | "wp", [location, rest @ ..]) => {
            // A size is digits or `0x…`; no command starts with either.
            let (size, commands) = match rest.split_first() {
                Some((word, commands)) if word.starts_with(|c: char| c.is_ascii_digit()) => {
                    (number(word)?, commands)
                }
                _ => (WATCH_SIZE, rest),
            };
            Ok(Command::SetWatchpoint {
                location: Location::parse(location)?,
                size,
                commands: stop_commands(commands)?,
            })
        }
        ("breakerror" | "be", words) => {
            // The codes end where a command starts.
            let split = words
                .iter()
                .position(|word| is_command(word))
                .unwrap_or(words.len());
            let (codes, commands) = words.split_at(split);
            let commands = stop_commands(commands)?;
            match codes {
                [] if commands.is_empty() => Ok(Command::ErrorWatch),
                [] => Err(format!(
                    "'{name}' needs an error code, a name or 'any' before its commands"
                )),
                ["clear"] if commands.is_empty() => Ok(Command::ClearErrors),
                ["clear"] => Err(format!("'{name} clear' takes no commands")),
                ["any"] => Ok(Command::BreakError(ErrorCodes::Any, commands)),
                codes => Ok(Command::BreakError(
                    ErrorCodes::Codes(
                        codes
                            .iter()
                            .map(|word| error_code(word))
                            .collect::<Result<_, _>>()?,
                    ),
                    commands,
                )),
            }
        }
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
        ("show", ["error"]) => Ok(Command::ShowError(None)),
        ("show", ["error", thread]) => Ok(Command::ShowError(Some(thread_id(thread)?))),
        ("show", ["error", ..]) => Err("too many arguments to 'show error'".to_string()),
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
        ("show", ["type", type_word, location, rest @ ..]) => {
            let view = values::parse_view(type_word, rest, line)?;
            Ok(Command::ShowType {
                location: Location::parse(location)?,
                view,
            })
        }
        ("show", ["report"]) => Ok(Command::ShowReport),
        ("show", ["report", ..]) => Err("'show report' takes no arguments".to_string()),
        ("show", ["type"]) => Ok(Command::TypeUsage),
        ("show", ["type", type_word]) => Err(format!(
            "'show type {type_word}' needs an address or a register: show type {type_word} <where>"
        )),
        ("show", []) => Err(
            "'show' needs what to show: show mem <where> [<size>], show type <type> <where>, \
             show arena [<id>], show error [<thread>], or show report"
                .to_string(),
        ),
        ("show", [other, ..]) => Err(format!(
            "cannot show '{other}'; 'show mem', 'show type', 'show arena', 'show error' and \
             'show report' are what can be shown"
        )),
        ("disassemble" | "dis", []) => Ok(Command::Disassemble(Disassembly::Current)),
        ("disassemble" | "dis", [word]) if is_count(word) => Ok(Command::Disassemble(
            Disassembly::Count(instruction_window(word)?),
        )),
        ("disassemble" | "dis", [back, length]) if is_count(back) && is_count(length) => Ok(
            Command::Disassemble(Disassembly::Count(instruction_span(back, length)?)),
        ),
        ("disassemble" | "dis", [start]) => {
            Ok(Command::Disassemble(Disassembly::from(start, None)?))
        }
        ("disassemble" | "dis", [start, count]) => Ok(Command::Disassemble(Disassembly::from(
            start,
            Some(instruction_window(count)?),
        )?)),
        ("disassemble" | "dis", [start, back, length]) => Ok(Command::Disassemble(
            Disassembly::from(start, Some(instruction_span(back, length)?))?,
        )),
        ("disassemble" | "dis", _) => Err(format!("'{name}' takes at most three arguments")),
        ("finish" | "f", []) => Ok(Command::Finish),
        ("finish" | "f", _) => Err(format!("'{name}' takes no arguments")),
        ("restart" | "rs", []) => Ok(Command::Restart),
        ("restart" | "rs", _) => Err(format!("'{name}' takes no arguments")),
        ("run" | "r", []) => Ok(Command::Run),
        ("run" | "r", _) => Err(format!("'{name}' takes no arguments")),
        ("env", []) => Err(
            "'env' needs what to do: env s <name> <value>, env r <name>, env clear or env \
             reset; 'list env' shows the environment"
                .to_string(),
        ),
        ("env", [verb @ ("s" | "set")]) => Err(format!(
            "'env {verb}' needs a name and a value: env {verb} <name> <value>"
        )),
        ("env", ["s" | "set", variable, ..]) => {
            environment::check_name(variable)?;
            // The value is the rest of the line, spaces and all.
            let value = after_words(line, 3);
            environment::check_value(value)?;
            Ok(Command::Env(environment::Change::Set {
                name: variable.to_string(),
                value: value.to_string(),
            }))
        }
        ("env", [verb @ ("r" | "remove")]) => {
            Err(format!("'env {verb}' needs a name: env {verb} <name>"))
        }
        ("env", ["r" | "remove", variable]) => {
            environment::check_name(variable)?;
            Ok(Command::Env(environment::Change::Remove(
                variable.to_string(),
            )))
        }
        ("env", [verb @ ("r" | "remove"), ..]) => {
            Err(format!("too many arguments to 'env {verb}'"))
        }
        ("env", ["clear"]) => Ok(Command::Env(environment::Change::Clear)),
        ("env", ["reset"]) => Ok(Command::Env(environment::Change::Reset)),
        ("env", [verb @ ("clear" | "reset"), ..]) => {
            Err(format!("'env {verb}' takes no arguments"))
        }
        ("env", [other, ..]) => Err(format!(
            "cannot 'env {other}'; env s, env r, env clear and env reset are what env does"
        )),
        _ => Err(format!(
            "unknown command '{name}'; type 'help' for the list of commands"
        )),
    }
}

/// What `line` holds after its first `count` words and the spaces after
/// them, less the line's end: an argument that may hold spaces.
fn after_words(line: &str, count: usize) -> &str {
    let mut rest = line.trim_end_matches(['\r', '\n']);
    for _ in 0..count {
        rest = rest.trim_start();
        rest = &rest[rest.find(char::is_whitespace).unwrap_or(rest.len())..];
    }
    rest.trim_start()
}

/// How many bytes a watchpoint covers when no size is given: an `Integer`.
const WATCH_SIZE: u64 = 8;

/// The most registers one `jump` sets besides the `pc`.
const MAX_JUMP_REGISTERS: usize = 8;

/// The bits of `nzcv` that hold anything: the N, Z, C and V flags.
const NZCV_FLAGS: u64 = 0xf000_0000;

/// `<where> <test> [<value>] at <target>`, as `breakpointif` takes them: the
/// test, the place, and the words after it.
#[allow(clippy::type_complexity)]
fn conditional<'a, 'b>(
    name: &str,
    words: &'a [&'b str],
) -> Result<(Condition, Target, &'a [&'b str]), String> {
    let [location, test, rest @ ..] = words else {
        return Err(format!(
            "'{name}' needs a test and a place: {name} <where> eq|ne|gt|lt <value> at \
             <address|function[+offset]> [<commands>], or {name} <where> in at \
             <address|function[+offset]>"
        ));
    };
    let (value, rest) = match rest {
        [value, rest @ ..] if *value != "at" => (Some(*value), rest),
        rest => (None, rest),
    };
    let condition = Condition::parse(location, test, value)?;
    match rest {
        ["at", target, rest @ ..] => Ok((condition, Target::parse(target)?, rest)),
        ["at"] => Err(format!(
            "'{name}' needs a place after 'at': an address (0x…) or a function name"
        )),
        _ => Err(format!(
            "'{name}' needs 'at' and the breakpoint's place after its test: {name} <where> \
             <test> at <address|function[+offset]>"
        )),
    }
}

/// Refuse a register `jump` does not set: it sets `x0`–`x28`, `fp`, `lr`,
/// `sp`, `nzcv`, `fpsr` and `fpcr`, and moves the `pc` itself.
fn settable(name: &str) -> Result<(), String> {
    let general = name.strip_prefix('x').is_some_and(|number| {
        (number == "0" || !number.starts_with('0'))
            && number.parse::<u8>().is_ok_and(|number| number <= 28)
    });
    if general || matches!(name, "fp" | "lr" | "sp" | "nzcv" | "fpsr" | "fpcr") {
        return Ok(());
    }
    let sets = "jump sets x0-x28, fp, lr, sp, nzcv, fpsr and fpcr";
    if name == "pc" {
        return Err(format!(
            "'pc' is where the thread goes: give it as the place, jump <where>; {sets}"
        ));
    }
    if name
        .strip_prefix('v')
        .is_some_and(|number| number.parse::<u8>().is_ok_and(|number| number <= 31))
    {
        return Err(format!("'{name}' is a vector register; {sets}"));
    }
    Err(format!("no register '{name}'; {sets}"))
}

/// A register's new value as `jump` takes it: a number (`5`, `-1`, `0x10`),
/// else a `<where>` worked out as a value (`x1`, `[sp+8]`, `main`).
fn assigned(word: &str) -> Result<Location, String> {
    location::signed(word)
        .map(Location::number)
        .or_else(|_| Location::parse(word))
}

/// Every command name and short form.
const COMMAND_NAMES: &[&str] = &[
    "help",
    "h",
    "quit",
    "q",
    "list",
    "l",
    "show",
    "breakpoint",
    "bp",
    "watchpoint",
    "wp",
    "breakerror",
    "be",
    "run",
    "r",
    "finish",
    "f",
    "restart",
    "rs",
    "disassemble",
    "dis",
    "backtrace",
    "bt",
    "breakpointif",
    "bpi",
    "jump",
    "j",
    "step",
    "s",
    "step-all",
    "sa",
    "env",
];

/// Whether `word` names a command.
fn is_command(word: &str) -> bool {
    COMMAND_NAMES.contains(&word)
}

/// The commands a stop runs, from the words after a breakpoint's,
/// watchpoint's or error watch's own: `;` separates them. Each must be one
/// that reads the program — `list`, `show`, `backtrace`, `disassemble`, or a
/// bare `breakerror` — `jump`, which moves the stopped thread, `finish`, or,
/// as the last of them, `run`, which runs the program on.
fn stop_commands(words: &[&str]) -> Result<Vec<String>, String> {
    let text = words.join(" ");
    let mut commands = Vec::<String>::new();
    for command in text
        .split(';')
        .map(str::trim)
        .filter(|command| !command.is_empty())
    {
        if let Some(run) = commands.last().filter(|last| runs_on(last)) {
            return Err(format!(
                "'{run}' must be the last command of a stop: '{command}' follows it"
            ));
        }
        match parse(command)? {
            // The environment is the next restart's, not the stop's.
            Command::List(Some(Listing::Environment)) => {
                return Err(format!(
                    "'{command}' cannot run at a stop: the environment it lists is the one \
                     the program gets when it restarts"
                ))
            }
            Command::List(_)
            | Command::ShowMemory { .. }
            | Command::ShowType { .. }
            | Command::ShowArena { .. }
            | Command::ShowError(_)
            | Command::ShowReport
            | Command::Backtrace(_)
            | Command::Disassemble(_)
            | Command::Jump { .. }
            | Command::Finish
            | Command::Run
            | Command::ErrorWatch => commands.push(command.to_string()),
            _ => {
                return Err(format!(
                    "'{command}' cannot run at a stop: a stop runs list, show, backtrace, \
                     disassemble, jump, finish, a bare breakerror and, last, run"
                ))
            }
        }
    }
    Ok(commands)
}

/// Whether a stop's command is `run`.
fn runs_on(command: &str) -> bool {
    matches!(parse(command), Ok(Command::Run))
}

/// The fault each thread is stopped on, as its stop named it (`a bad memory
/// access at 0x8`), until the thread runs again: what `show error` says when
/// the thread has no error.
type Faults = std::collections::BTreeMap<usize, String>;

/// A stop's commands that wait for the prompt to run them, each with the
/// thread that stopped: the ones of a stop that has a `finish` among them.
type Queue = std::collections::VecDeque<(usize, String)>;

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
pub(crate) fn number(word: &str) -> Result<u64, String> {
    match word.strip_prefix("0x").or_else(|| word.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => word.parse::<u64>(),
    }
    .map_err(|_| format!("'{word}' is not a number"))
}

fn frame_count(word: &str) -> Result<usize, String> {
    word.parse::<usize>()
        .ok()
        .filter(|count| *count > 0)
        .ok_or_else(|| format!("'{word}' is not a number of frames (1 or more), or 'all'"))
}

/// An error code as `breakerror` takes one: its number, or the name of its
/// `errorCode` constant (`ErrOverflow`, `errorCode::ErrOverflow`).
fn error_code(word: &str) -> Result<i64, String> {
    word.parse::<i64>()
        .ok()
        .or_else(|| runtime::error_code_by_name(word))
        .ok_or_else(|| {
            format!(
                "'{word}' is not an error code: give its number (77050010) or its errorCode \
                 name (ErrOverflow)"
            )
        })
}

fn thread_id(word: &str) -> Result<usize, String> {
    word.parse::<usize>()
        .map_err(|_| format!("'{word}' is not a thread id; type 'list thread' for the threads"))
}

/// What separates a listing's columns.
const COLUMN_SEPARATOR: &str = " | ";

/// Lay out a listing: a line of `headers`, then one line per row (a cell for
/// each header), the columns separated by ` | ` and each but the last
/// left-aligned to its widest entry, header included. A listing with no rows
/// has no lines, not even the header.
pub(crate) fn table(headers: &[&str], rows: Vec<Vec<String>>) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut widths = headers
        .iter()
        .map(|header| header.chars().count())
        .collect::<Vec<_>>();
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let last = headers.len() - 1;
    std::iter::once(headers.iter().map(|header| header.to_string()).collect())
        .chain(rows)
        .map(|row: Vec<String>| {
            row.iter()
                .zip(&widths)
                .enumerate()
                .map(|(at, (cell, width))| {
                    if at == last {
                        cell.clone()
                    } else {
                        format!("{cell:<width$}")
                    }
                })
                .collect::<Vec<_>>()
                .join(COLUMN_SEPARATOR)
        })
        .collect()
}

/// The `list func` lines: the program's own functions, then the runtime's, then
/// linked ones, each group in address order (a linked function not yet bound
/// last), symbols that tie in symbol-table order, and a name repeated at the
/// same address once.
pub(crate) fn function_lines(symbols: &[FunctionSymbol], filter: &FunctionFilter) -> Vec<String> {
    let collisions = Collisions::of(symbols);
    let mut rows = symbols
        .iter()
        .filter_map(|symbol| {
            let (origin, written) = classify(symbol);
            if !filter.admits(origin, &symbol.name, written.as_deref()) {
                return None;
            }
            let written = written.map(|name| collisions.tagged(name, origin));
            Some((origin, symbol.address, &symbol.name, written))
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|(origin, address, _, _)| (*origin, address.is_none(), *address));
    // The same function named again at the same address (an import's repeated
    // records) is shown once; one name at two addresses is shown twice.
    let mut seen = std::collections::HashSet::new();
    rows.retain(|(_, address, name, _)| seen.insert((*address, name.to_string())));
    let mut origins = rows.iter().map(|(origin, ..)| *origin).collect::<Vec<_>>();
    origins.dedup();
    let mut lines = table(
        &["Type", "Address", "Symbol", "Written name"],
        rows.into_iter()
            .map(|(origin, address, name, written)| {
                vec![
                    origin.code().to_string(),
                    address.map_or_else(|| "-".to_string(), |at| format!("0x{at:016x}")),
                    name.clone(),
                    written.unwrap_or_else(|| "-".to_string()),
                ]
            })
            .collect(),
    );
    // A legend for the types the listing shows.
    if !origins.is_empty() {
        lines.push(String::new());
        lines.extend(
            origins
                .into_iter()
                .map(|origin| format!("{}  {}", origin.code(), origin.legend())),
        );
    }
    lines
}

/// The `list reg` lines: one register per line, the value in full-width hex and
/// then in decimal — signed for a 64-bit register, as MFBASIC's `Integer` is.
pub(crate) fn register_lines(registers: &[Register]) -> Vec<String> {
    table(
        &["Register", "Hex", "Decimal"],
        registers
            .iter()
            .map(|register| {
                let (hex, decimal) = register_value(register);
                vec![register.name.clone(), hex, decimal]
            })
            .collect(),
    )
}

/// The lines of a bare `list reg`: the registers that have a role in compiled
/// code, as [`register_lines`] shows them, with the role in a fourth column.
pub(crate) fn common_register_lines(registers: &[Register]) -> Vec<String> {
    table(
        &["Register", "Hex", "Decimal", "Role"],
        registers
            .iter()
            .filter_map(|register| {
                let role = register.role?;
                let (hex, decimal) = register_value(register);
                Some(vec![register.name.clone(), hex, decimal, role.to_string()])
            })
            .collect(),
    )
}

/// A register's value as `list reg` shows it: in full-width hex, and in
/// decimal in parentheses.
fn register_value(register: &Register) -> (String, String) {
    match register.value {
        RegisterValue::Bits64(value) => (format!("0x{value:016x}"), format!("({})", value as i64)),
        RegisterValue::Bits128(value) => (format!("0x{value:032x}"), format!("({value})")),
    }
}

/// The `list thread` lines: `<id> <name> <pc> <task>`. A thread's name is the
/// one it gave itself, else the function it is in, else its system id.
pub(crate) fn thread_lines(
    threads: &[ThreadFacts],
    roles: &[Role],
    functions: &[FunctionSymbol],
) -> Vec<String> {
    table(
        &["ID", "Name", "PC", "Task"],
        threads
            .iter()
            .zip(roles)
            .enumerate()
            .map(|(id, (thread, role))| {
                let name = thread
                    .name
                    .clone()
                    .or_else(|| runtime::locate(functions, thread.pc))
                    .unwrap_or_else(|| format!("thread-{:#x}", thread.id));
                vec![
                    id.to_string(),
                    name,
                    format!("0x{:016x}", thread.pc),
                    role.label().to_string(),
                ]
            })
            .collect(),
    )
}

/// The `list arena` lines: `<id> <thread id> <address> <size in KB>`, the size
/// rounded up to whole kilobytes and `-` for an arena no thread allocates from.
pub(crate) fn arena_lines(arenas: &[Arena]) -> Vec<String> {
    table(
        &["ID", "Thread", "Address", "Size (KB)"],
        arenas
            .iter()
            .map(|arena| {
                vec![
                    arena.id.to_string(),
                    arena
                        .thread
                        .map_or_else(|| "-".to_string(), |id| id.to_string()),
                    format!("0x{:016x}", arena.address),
                    arena.mapped.div_ceil(1024).to_string(),
                ]
            })
            .collect(),
    )
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
            lines.extend(LIST_REG_USAGE.lines().map(str::to_string));
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
fn launched_only(what: &str) -> String {
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
fn step(
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
fn ended_lines(mut lines: Vec<String>, ended: bool, breakpoints: &Breakpoints) -> Vec<String> {
    if ended && breakpoints.has_report() {
        let mut hint = REPORT_HINT.to_string();
        hint[..1].make_ascii_uppercase();
        lines.push(format!("{hint}."));
    }
    lines
}

/// Step every thread together `count` rounds and describe where each stopped;
/// the flag is set when the program ended.
fn step_all(
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
fn stop_lines(inferior: &mut dyn Inferior, stop: &RunStop, report: bool) -> Vec<String> {
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
fn emit(
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
fn hear_stop(
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
        && commands
            .iter()
            .all(|(_, commands)| commands.last().is_some_and(|last| runs_on(last)));
    let finishes = commands.iter().any(|(_, commands)| {
        commands
            .iter()
            .any(|command| matches!(parse(command), Ok(Command::Finish)))
    });
    queue.clear();
    for (thread, commands) in commands {
        *current = thread;
        for command in commands.into_iter().filter(|command| !runs_on(command)) {
            if finishes {
                queue.push_back((thread, command));
                continue;
            }
            let result = parse(&command).and_then(|command| match command {
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
fn inspect(
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
/// and what it returned (MFBASIC returns an error code in `x0` and the value
/// in `x1`), or that the finish did not complete; `true` when the program
/// ended.
fn finish_lines(
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
            lines.push(format!(
                "Thread {thread} returned from {from} to {at}: x0 = {}, x1 = {}.",
                value("x0"),
                value("x1")
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
fn restart(
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
    let (address, described) = resolve(inferior, current, location)?;
    let bytes = inferior.read_bytes(address, size as usize)?;
    let mut lines = vec![format!("Memory at 0x{address:016x}{described}:")];
    lines.extend(memory_lines(address, &bytes));
    Ok(lines)
}

/// `show type`: the value at `location` read as the type `view` names. It
/// must be in one of the program's arena blocks, which needs a `--debug`
/// build's arena registry to find.
fn show_type(
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
fn show_error(
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
fn resolve(
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
fn jump(
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
    if address % 4 != 0 {
        return Err(format!(
            "0x{address:016x} is not the start of an instruction: they start every 4 bytes"
        ));
    }
    inferior
        .read_code(address, 4)
        .map_err(|error| format!("cannot jump to 0x{address:016x}: {error}"))?;
    let mut values = vec![("pc".to_string(), address)];
    for (register, location) in registers {
        let value = location
            .value(inferior, thread)
            .map_err(|fault| format!("cannot set {register}: {}", fault.message()))?
            .value;
        match register.as_str() {
            "nzcv" if value & !NZCV_FLAGS != 0 => {
                return Err(format!(
                    "cannot set nzcv to 0x{value:016x}: it holds only the flags, bits 28 to 31 \
                     (0x00000000f0000000)"
                ))
            }
            "fpsr" | "fpcr" if value > u64::from(u32::MAX) => {
                return Err(format!(
                    "cannot set {register} to 0x{value:016x}: it holds 32 bits"
                ))
            }
            _ => {}
        }
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
fn backtrace(
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
fn disassemble_at(
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
        let result = match parse(&line) {
            Ok(Command::Empty) => Ok(Vec::new()),
            Ok(Command::Help) => Ok(vec![HELP.to_string()]),
            Ok(Command::Quit) => break,
            Ok(Command::TypeUsage) => Ok(values::TYPE_USAGE.lines().map(str::to_string).collect()),
            Ok(Command::List(
                listing @ (None | Some(Listing::Breakpoints | Listing::Watchpoints)),
            )) => list(inferior, program, &breakpoints, listing),
            // An ended program has no registers to show; the usage still reads.
            Ok(Command::List(Some(Listing::CommonRegisters { .. }))) if exited => {
                Ok(LIST_REG_USAGE.lines().map(str::to_string).collect())
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
mod tests {
    use super::*;
    use crate::debugger::names::Origin;
    use crate::debugger::{Change, FunctionRange, Step};

    /// An [`Inferior`] answering from fixed tables: two threads, a few
    /// functions, three registers. It records how often it was ended and every
    /// step it was asked for, and refuses every read when `unreadable`. With
    /// `code` its code is all `nop`s, in functions placed as its symbols say.
    struct Recorder {
        ending: Ending,
        ends: usize,
        unreadable: bool,
        code: bool,
        steps: Vec<(Vec<usize>, u64)>,
        stop: StepStop,
        pc: u64,
        inserted: Vec<u64>,
        watched: Vec<(u64, u64)>,
        runs: Vec<RunStop>,
        /// Every `set_registers`: the thread and its values. Thread 0's `pc`
        /// moves; its `x0` is the last value set, when any is.
        sets: Vec<(usize, Vec<(String, u64)>)>,
        x0: Option<u64>,
        /// Memory a `[…]` can read: 8-byte words by address.
        words: Vec<(u64, u64)>,
        /// The environment a launched program started with; `None` for one
        /// attached to, which cannot restart.
        environment: Option<Environment>,
        /// The environment each restart was given.
        restarts: Vec<Environment>,
    }

    impl Recorder {
        fn new(ending: Ending) -> Self {
            Recorder {
                ending,
                ends: 0,
                unreadable: false,
                code: false,
                steps: Vec::new(),
                stop: StepStop::Finished,
                pc: 0x1000,
                inserted: Vec::new(),
                watched: Vec::new(),
                runs: Vec::new(),
                sets: Vec::new(),
                x0: None,
                words: Vec::new(),
                environment: None,
                restarts: Vec::new(),
            }
        }
    }

    /// A C function's symbol as the host's format spells it.
    fn c(name: &str) -> String {
        format!("{}{name}", crate::debugger::names::C_PREFIX)
    }

    fn symbol(name: &str, linked: bool, address: Option<u64>, size: Option<u64>) -> FunctionSymbol {
        FunctionSymbol {
            name: name.to_string(),
            linked,
            loaded: false,
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
                symbol(&c("malloc"), true, None, None),
                symbol("_mfb_rt_print", false, Some(0x1008), Some(8)),
                symbol("_mfb_fn_next_5Fseed", false, Some(0x1004), Some(4)),
                symbol(
                    "_mfb_ifn_datetime_5FnormInstant",
                    false,
                    Some(0x1020),
                    Some(8),
                ),
                symbol(&c("free"), true, Some(0x9000), None),
                symbol(&c("malloc"), true, None, None),
                FunctionSymbol {
                    loaded: true,
                    ..symbol(&c("vkCreateInstance"), true, Some(0xa000), None)
                },
            ])
        }
        fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
            Ok(None)
        }
        fn read_u64(&mut self, address: u64) -> Result<u64, String> {
            Err(format!("unmapped {address:#x}"))
        }
        fn read_code(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
            if !self.code {
                return Err(format!("unmapped {address:#x}"));
            }
            Ok(0xd503_201f_u32.to_le_bytes().repeat(length / 4))
        }
        fn function_at(&mut self, address: u64) -> Result<Option<FunctionRange>, String> {
            if !self.code {
                return Ok(None);
            }
            Ok(self.functions()?.into_iter().find_map(|function| {
                let (start, size) = (function.address?, function.size?);
                (start <= address && address < start + size).then(|| FunctionRange {
                    image: "prog".to_string(),
                    name: Some(function.name),
                    start,
                    size: Some(size),
                })
            }))
        }
        fn data_symbol_at(&mut self, _: u64) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
            Ok(Vec::new())
        }
        fn read_bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
            match self.words.iter().find(|(at, _)| *at == address) {
                Some((_, word)) if length <= 8 => Ok(word.to_le_bytes()[..length].to_vec()),
                _ => Err(format!("unmapped {address:#x}")),
            }
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
            let x0 = match self.x0 {
                Some(x0) if thread == 0 => x0,
                _ => 0x2a + thread as u64,
            };
            Ok(vec![
                Register {
                    name: "x0".to_string(),
                    group: RegisterGroup::General,
                    value: RegisterValue::Bits64(x0),
                    role: Some("MFB arg, MFB result, C arg, C result"),
                },
                Register {
                    name: "nzcv".to_string(),
                    group: RegisterGroup::System,
                    value: RegisterValue::Bits64(0x6000_0000),
                    role: None,
                },
                Register {
                    name: "v31".to_string(),
                    group: RegisterGroup::Simd,
                    value: RegisterValue::Bits128(1 << 64),
                    role: None,
                },
            ])
        }
        fn set_registers(&mut self, thread: usize, values: &[(String, u64)]) -> Result<(), String> {
            self.sets.push((thread, values.to_vec()));
            for (name, value) in values {
                match name.as_str() {
                    "pc" if thread == 0 => self.pc = *value,
                    "x0" if thread == 0 => self.x0 = Some(*value),
                    _ => {}
                }
            }
            Ok(())
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
        /// A launched program (one with an `environment`) restarts as
        /// process 43, recording the environment it was given.
        fn restart(
            &mut self,
            environment: &[(std::ffi::OsString, std::ffi::OsString)],
        ) -> Result<u32, String> {
            if self.environment.is_none() {
                return Err("restart needs a program the debugger launched".to_string());
            }
            self.restarts.push(environment.to_vec());
            Ok(43)
        }
        fn entry(&mut self) -> Result<Option<u64>, String> {
            Ok(Some(0x1000))
        }
        fn insert_breakpoint(&mut self, address: u64) -> Result<(), String> {
            self.inserted.push(address);
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
        let environment = inferior.environment.clone();
        run(
            inferior,
            CONSOLE,
            environment,
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
    fn list_function_is_list_func() {
        assert_eq!(parse("list function"), parse("list func"));
        assert_eq!(parse("l function MU fib"), parse("l func MU fib"));
    }

    #[test]
    fn list_func_groups_by_origin_then_address_and_keeps_repeats() {
        let (out, err, _) = session("l func\nq\n", Ending::Killed, false);
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
Type | Address            | Symbol                          | Written name
MU   | 0x0000000000001004 | _mfb_fn_next_5Fseed             | next_seed
MU   | 0x0000000000001010 | _mfb_fn_main                    | main
MB   | 0x0000000000001020 | _mfb_ifn_datetime_5FnormInstant | datetime::#normInstant
MR   | 0x0000000000001000 | _main                           | -
MR   | 0x0000000000001008 | _mfb_rt_print                   | -
SL   | 0x0000000000009000 | {free:<31} | free
SL   | -                  | {malloc:<31} | malloc
SD   | 0x000000000000a000 | {vk:<31} | vkCreateInstance

MU  {MU}
MB  {MB}
MR  {MR}
SL  {SL}
SD  {SD}
{PROMPT}Killed process 42.\n",
                MU = Origin::User.legend(),
                free = c("free"),
                malloc = c("malloc"),
                vk = c("vkCreateInstance"),
                MB = Origin::Builtin.legend(),
                MR = Origin::Runtime.legend(),
                SL = Origin::Linked.legend(),
                SD = Origin::Dynamic.legend(),
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
ID | Name             | PC                 | Task
0  | _main            | 0x0000000000001000 | MFB Main
1  | com.apple.worker | 0x0000000000007000 | Other
2  | thread-0x503     | 0x0000000000007004 | Other
{PROMPT}Killed process 42.\n"
            )
        );
    }

    #[test]
    fn list_reg_alone_shows_the_registers_with_a_role_then_its_usage() {
        for (line, value) in [
            ("list reg\nq\n", "2a | (42)   "),
            ("l reg 2\nq\n", "2c | (44)   "),
        ] {
            let (out, err, _) = session(line, Ending::Killed, false);
            assert_eq!(
                out,
                format!(
                    "{PROMPT}\
Register | Hex                | Decimal | Role
x0       | 0x00000000000000{value} | MFB arg, MFB result, C arg, C result

{LIST_REG_USAGE}
{PROMPT}Killed process 42.\n"
                )
            );
            assert_eq!(err, "");
        }
        let (out, err, _) = session("list reg 3\nq\n", Ending::Killed, false);
        assert_eq!(out, format!("{PROMPT}{PROMPT}Killed process 42.\n"));
        assert_eq!(
            err,
            "error: no thread 3: the program has 3 threads; type 'list thread' for them\n"
        );
    }

    #[test]
    fn list_reg_alone_shows_only_its_usage_after_the_program_ends() {
        let mut inferior = Recorder::new(Ending::AlreadyExited);
        inferior.runs = vec![RunStop::Exited(Exit::Status(0))];
        let (out, err) = drive(&mut inferior, "run\nlist reg\nlist reg gpr\nq\n", false);
        assert!(
            out.contains(&format!("{PROMPT}{LIST_REG_USAGE}\n")),
            "{out}"
        );
        assert!(!out.contains("Register |"), "{out}");
        assert_eq!(
            err,
            "error: the program has exited; type 'quit' to end the session\n"
        );
    }

    #[test]
    fn list_reg_shows_each_group_of_the_main_thread_or_the_one_asked_for() {
        let (out, err, _) = session(
            "list reg simd\nl reg 2 gpr sys simd\nq\n",
            Ending::Killed,
            false,
        );
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
Register | Hex                                | Decimal
v31      | 0x00000000000000010000000000000000 | (18446744073709551616)
{PROMPT}\
Register | Hex                                | Decimal
x0       | 0x000000000000002c                 | (44)
nzcv     | 0x0000000060000000                 | (1610612736)
v31      | 0x00000000000000010000000000000000 | (18446744073709551616)
{PROMPT}Killed process 42.\n"
            )
        );
    }

    #[test]
    fn list_reg_shows_the_main_thread_or_the_one_asked_for() {
        let (out, _, _) = session(
            "list reg gpr sys simd\nl reg 2 gpr sys simd\nq\n",
            Ending::Killed,
            false,
        );
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
Register | Hex                                | Decimal
x0       | 0x000000000000002a                 | (42)
nzcv     | 0x0000000060000000                 | (1610612736)
v31      | 0x00000000000000010000000000000000 | (18446744073709551616)
{PROMPT}\
Register | Hex                                | Decimal
x0       | 0x000000000000002c                 | (44)
nzcv     | 0x0000000060000000                 | (1610612736)
v31      | 0x00000000000000010000000000000000 | (18446744073709551616)
{PROMPT}Killed process 42.\n"
            )
        );
    }

    #[test]
    fn list_reg_refuses_a_thread_or_register_that_does_not_exist() {
        let (_, err, _) = session(
            "list reg 3 gpr\nlist reg x9\nlist reg 1 x0, w5\nq\n",
            Ending::Killed,
            false,
        );
        assert_eq!(
            err,
            "error: no thread 3: the program has 3 threads; type 'list thread' for them\n\
             error: no register 'x9'; type 'list reg' for the groups\n\
             error: no register 'w5'; type 'list reg' for the groups\n"
        );
    }

    #[test]
    fn list_reg_shows_just_the_registers_named_by_spaces_or_commas() {
        for line in [
            "list reg 2 nzcv x0\nq\n",
            "list reg 2 nzcv, x0\nq\n",
            "list reg 2,nzcv,x0\nq\n",
        ] {
            let (out, err, _) = session(line, Ending::Killed, false);
            assert_eq!(err, "", "{line}");
            assert_eq!(
                out,
                format!(
                    "{PROMPT}\
Register | Hex                | Decimal
nzcv     | 0x0000000060000000 | (1610612736)
x0       | 0x000000000000002c | (44)
{PROMPT}Killed process 42.\n"
                ),
                "{line}"
            );
        }
    }

    #[test]
    fn a_negative_64_bit_value_shows_as_a_signed_decimal() {
        let register = Register {
            name: "x8".to_string(),
            group: RegisterGroup::General,
            value: RegisterValue::Bits64(u64::MAX),
            role: None,
        };
        assert_eq!(
            register_lines(&[register]),
            [
                "Register | Hex                | Decimal",
                "x8       | 0xffffffffffffffff | (-1)",
            ]
        );
    }

    #[test]
    fn show_mem_parses_an_address_or_a_register_and_offset() {
        assert_eq!(
            parse("show mem 0x1000"),
            Ok(Command::ShowMemory {
                location: Location::number(0x1000),
                size: MEMORY_SIZE
            })
        );
        assert_eq!(
            parse("show mem sp+0x18 8"),
            Ok(Command::ShowMemory {
                location: Location::named("sp", 0x18),
                size: 8
            })
        );
        assert_eq!(
            parse("show mem x19-16"),
            Ok(Command::ShowMemory {
                location: Location::named("x19", -16),
                size: MEMORY_SIZE
            })
        );
        for (line, message) in [
            (
                "show mem",
                "'show mem' needs an address or a register: show mem <where> [<size>]",
            ),
            ("show mem sp+zz", "'zz' is not a number"),
            (
                "show mem @x",
                "'@x' is not an address, a register or a function",
            ),
            ("show mem sp 8 9", "too many arguments to 'show mem'"),
            ("finish now", "'finish' takes no arguments"),
            ("rs again", "'rs' takes no arguments"),
        ] {
            assert_eq!(parse(line), Err(message.to_string()), "{line}");
        }
    }

    /// A place `bt` or a stop shows (`main+0x110`) can be typed back: `show`
    /// and `dis` read a name that is no register as a function, and
    /// `breakpoint` takes one with an offset.
    #[test]
    fn a_function_and_an_offset_name_a_place() {
        assert_eq!(
            parse("show mem _mfb_fn_f$own4+8"),
            Ok(Command::ShowMemory {
                location: Location::named("_mfb_fn_f$own4", 8),
                size: MEMORY_SIZE
            })
        );
        assert_eq!(
            parse("dis datetime::#normInstant+0x10 2"),
            Ok(Command::Disassemble(Disassembly::At(
                Location::named("datetime::#normInstant", 16),
                Some(Window::count(2))
            )))
        );
        assert_eq!(
            parse("bp main+272"),
            Ok(Command::SetBreakpoint {
                target: Target::Offset {
                    name: "main".to_string(),
                    offset: 272,
                },
                condition: None,
                commands: Vec::new()
            })
        );
        // `main` is the program's (`_mfb_fn_main` at 0x1010), so `main+0x10`
        // is 0x1020; a register keeps its name; nothing else is a place.
        let (_, err, _) = session(
            "show mem main+0x10\nshow mem x0+8\nshow mem nothing+4\nq\n",
            Ending::Killed,
            false,
        );
        assert_eq!(
            err,
            "error: unmapped 0x1020\n\
             error: unmapped 0x32\n\
             error: no register or function is named 'nothing'; type 'list reg' or 'list func' \
             for them\n"
        );
    }

    #[test]
    fn breakerror_takes_codes_names_any_or_clear() {
        assert_eq!(parse("be"), Ok(Command::ErrorWatch));
        assert_eq!(parse("breakerror clear"), Ok(Command::ClearErrors));
        assert_eq!(
            parse("be any"),
            Ok(Command::BreakError(ErrorCodes::Any, Vec::new()))
        );
        assert_eq!(
            parse("be 77050010 errorCode::ErrNotFound ErrOutOfMemory"),
            Ok(Command::BreakError(
                ErrorCodes::Codes(vec![77_050_010, 77_050_004, 77_010_001]),
                Vec::new()
            ))
        );
        assert_eq!(
            parse("be ErrNothing"),
            Err(
                "'ErrNothing' is not an error code: give its number (77050010) or its \
                 errorCode name (ErrOverflow)"
                    .to_string()
            )
        );
        // The prompt's fake names no error park, as a build without --debug.
        let (out, err, _) = session("be\nbe any\nbe clear\nq\n", Ending::Killed, false);
        assert!(out.contains("No errors are being watched for."), "{out}");
        assert_eq!(
            err,
            "error: 'breakerror' stops in the runtime's error function, which only a debug \
             build names; rebuild the program with: mfb build --debug -O0 <project>\n\
             error: no errors are being watched for\n"
        );
    }

    #[test]
    fn show_error_takes_a_thread_and_needs_a_debug_build() {
        assert_eq!(parse("show error"), Ok(Command::ShowError(None)));
        assert_eq!(parse("show error 2"), Ok(Command::ShowError(Some(2))));
        assert_eq!(
            parse("show error x"),
            Err("'x' is not a thread id; type 'list thread' for the threads".to_string())
        );
        assert_eq!(
            parse("show error 1 2"),
            Err("too many arguments to 'show error'".to_string())
        );
        let (_, err, _) = session("show error\nshow error 9\nq\n", Ending::Killed, false);
        assert_eq!(
            err,
            "error: 'show error' reads the thread's arena, which needs a debug build; rebuild \
             the program with: mfb build --debug -O0 <project>\n\
             error: no thread 9: the program has 3 threads; type 'list thread' for them\n"
        );
    }

    #[test]
    fn show_type_needs_a_debug_build_s_arenas() {
        let (_, err, _) = session("show type string x0\nq\n", Ending::Killed, false);
        assert_eq!(
            err,
            "error: 'show type' reads values in the arena, which needs a debug build; rebuild \
             the program with: mfb build --debug -O0 <project>\n"
        );
    }

    #[test]
    fn show_type_alone_shows_its_usage() {
        let (out, err, _) = session("show type\nq\n", Ending::Killed, false);
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}{}\n{PROMPT}Killed process 42.\n",
                values::TYPE_USAGE
            )
        );
    }

    #[test]
    fn show_type_parses_a_type_a_location_and_a_view() {
        assert_eq!(
            parse("show type string x1"),
            Ok(Command::ShowType {
                location: Location::named("x1", 0),
                view: View::String
            })
        );
        assert_eq!(
            parse("show type list 0x1000 resource 2..5"),
            Ok(Command::ShowType {
                location: Location::number(0x1000),
                view: View::List {
                    element: Some(values::Kind::Resource),
                    span: Some(values::Span {
                        start: 2,
                        end: Some(5)
                    })
                }
            })
        );
        assert_eq!(
            parse("show type map x3 get Grace Hopper"),
            Ok(Command::ShowType {
                location: Location::named("x3", 0),
                view: View::Map(values::MapView::Get("Grace Hopper".to_string()))
            })
        );
        for (line, message) in [
            (
                "show type string",
                "'show type string' needs an address or a register: show type string <where>",
            ),
            (
                "show type string x1 8",
                "too many arguments to 'show type string'",
            ),
            (
                "show type record x1",
                "cannot show type 'record'; the types are: int, float, fixed, money, bool, \
                 byte, scalar, string, list, set, map, resource",
            ),
            (
                "show type list x1 3..1",
                "'3..1' is not an index or a range (3, 0..10, 2..) or a type",
            ),
            (
                "show type map x1 nope",
                "'nope' is not an index, 'keys', 'values' or 'get <key>'",
            ),
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

    fn variables(pairs: &[(&str, &str)]) -> Environment {
        pairs
            .iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect()
    }

    #[test]
    fn env_changes_what_the_next_restart_gives_and_list_env_shows_it() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.environment = Some(variables(&[("PATH", "/bin"), ("HOME", "/h")]));
        let (out, err) = drive(
            &mut inferior,
            "list env\nenv s GREETING  hello,  world \nenv r HOME\nlist env\nrestart\n\
             list env\nenv clear\nenv reset\nrestart\nq\n",
            false,
        );
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}Name | Current | After restart\n\
                 HOME | /h      | /h\n\
                 PATH | /bin    | /bin\n\
                 {PROMPT}Set GREETING=hello,  world ; the program gets it when it restarts.\n\
                 {PROMPT}Removed HOME; the program goes without it when it restarts.\n\
                 {PROMPT}Name     | Current | After restart\n\
                 GREETING |         | hello,  world \n\
                 HOME     | /h      | \n\
                 PATH     | /bin    | /bin\n\
                 {PROMPT}Restarted the program as process 43; paused before its first \
                 instruction.\n\
                 {PROMPT}Name     | Current        | After restart\n\
                 GREETING | hello,  world  | hello,  world \n\
                 PATH     | /bin           | /bin\n\
                 {PROMPT}Cleared the environment; the program starts with none when it \
                 restarts.\n\
                 {PROMPT}Reset the environment to the one the session started the program \
                 with; the program gets it when it restarts.\n\
                 {PROMPT}Restarted the program as process 43; paused before its first \
                 instruction.\n\
                 {PROMPT}Killed process 42.\n"
            )
        );
        assert_eq!(
            inferior.restarts,
            [
                variables(&[("PATH", "/bin"), ("GREETING", "hello,  world ")]),
                variables(&[("PATH", "/bin"), ("HOME", "/h")]),
            ]
        );
    }

    #[test]
    fn env_works_once_the_program_has_ended() {
        let mut inferior = Recorder::new(Ending::AlreadyExited);
        inferior.environment = Some(Vec::new());
        inferior.runs = vec![RunStop::Exited(Exit::Status(0))];
        let (_, err) = drive(
            &mut inferior,
            "run\nenv s A 1\nlist env\nrestart\nq\n",
            false,
        );
        assert_eq!(err, "");
        assert_eq!(inferior.restarts, [variables(&[("A", "1")])]);
    }

    #[test]
    fn env_needs_a_launched_program() {
        let (_, err, _) = session("env s A 1\nlist env\nq\n", Ending::Detached, false);
        assert_eq!(
            err,
            "error: 'env' needs a program the debugger launched; this one was attached to\n\
             error: 'list env' needs a program the debugger launched; this one was attached to\n"
        );
    }

    #[test]
    fn env_refuses_what_it_cannot_do() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.environment = Some(variables(&[("A", "1")]));
        let (_, err) = drive(
            &mut inferior,
            "env\nenv s\nenv set A=B 1\nenv r\nenv remove A B\nenv r B\nenv clear now\n\
             env reset x\nenv frob\nlist env x\nbp 0x1000 list env\nq\n",
            false,
        );
        assert_eq!(
            err,
            "error: 'env' needs what to do: env s <name> <value>, env r <name>, env clear or \
             env reset; 'list env' shows the environment\n\
             error: 'env s' needs a name and a value: env s <name> <value>\n\
             error: 'A=B' is not a variable name: a name holds no '=' and no NUL\n\
             error: 'env r' needs a name: env r <name>\n\
             error: too many arguments to 'env remove'\n\
             error: no variable 'B' is in the environment; type 'list env' for it\n\
             error: 'env clear' takes no arguments\n\
             error: 'env reset' takes no arguments\n\
             error: cannot 'env frob'; env s, env r, env clear and env reset are what env does\n\
             error: too many arguments to 'list env'\n\
             error: 'list env' cannot run at a stop: the environment it lists is the one the \
             program gets when it restarts\n"
        );
    }

    #[test]
    fn env_s_takes_the_rest_of_the_line_as_the_value() {
        assert_eq!(after_words("env s A  b c \r\n", 3), "b c ");
        assert_eq!(after_words("  env\tset A\n", 3), "");
        assert_eq!(
            parse("env set A x=1 y\n"),
            Ok(Command::Env(environment::Change::Set {
                name: "A".to_string(),
                value: "x=1 y".to_string(),
            }))
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
                "ID | Thread | Address            | Size (KB)",
                "0  | 0      | 0x0000000016f00000 | 4",
                "12 | 3      | 0x0000000000010000 | 5",
                "13 | -      | 0x0000000000020000 | 0",
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
            "list func\nlist reg gpr\nlist thread\nq\n",
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
        let (_, err, _) = session("list frames\nlist thread reg\nq\n", Ending::Killed, false);
        assert_eq!(
            err,
            "error: cannot list 'frames'; type 'list' for what can be listed\n\
             error: too many arguments to 'list thread'\n"
        );
    }

    #[test]
    fn empty_listings_print_nothing() {
        assert!(function_lines(&[], &FunctionFilter::default()).is_empty());
        assert!(register_lines(&[]).is_empty());
        assert!(thread_lines(&[], &[], &[]).is_empty());
        assert!(table(&["A", "B"], Vec::new()).is_empty());
    }

    #[test]
    fn a_table_aligns_every_column_to_its_widest_entry_by_characters() {
        assert_eq!(
            table(
                &["ID", "Name", "Task"],
                vec![
                    vec!["0".to_string(), "héllo".to_string(), "MFB Main".to_string()],
                    vec!["10".to_string(), "x".to_string(), "Other".to_string()],
                ]
            ),
            [
                "ID | Name  | Task",
                "0  | héllo | MFB Main",
                "10 | x     | Other",
            ]
        );
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
Address            | Count | Symbol              | Written name
0x0000000000001004 | 0     | _mfb_fn_next_5Fseed | next_seed
0x0000000000001012 | 0     | _mfb_fn_main+0x2    | main+0x2
{PROMPT}Removed the breakpoint at 0x0000000000001012 in main+0x2.\n\
                 {PROMPT}Address            | Count | Symbol              | Written name\n\
                 0x0000000000001004 | 0     | _mfb_fn_next_5Fseed | next_seed\n\
                 {PROMPT}Killed process 42.\n"
            )
        );
        assert_eq!(inferior.inserted, [0x1004]);
    }

    #[test]
    fn backtrace_parses_its_forms_in_both_spellings() {
        for name in ["backtrace", "bt"] {
            assert_eq!(
                parse(name),
                Ok(Command::Backtrace(Backtrace::Thread {
                    count: None,
                    thread: None
                }))
            );
            assert_eq!(
                parse(&format!("{name} all")),
                Ok(Command::Backtrace(Backtrace::All))
            );
            assert_eq!(
                parse(&format!("{name} 5")),
                Ok(Command::Backtrace(Backtrace::Thread {
                    count: Some(5),
                    thread: None
                }))
            );
            assert_eq!(
                parse(&format!("{name} 5 2")),
                Ok(Command::Backtrace(Backtrace::Thread {
                    count: Some(5),
                    thread: Some(2)
                }))
            );
        }
        for (line, message) in [
            (
                "bt 0",
                "'0' is not a number of frames (1 or more), or 'all'",
            ),
            (
                "bt -1",
                "'-1' is not a number of frames (1 or more), or 'all'",
            ),
            (
                "bt x",
                "'x' is not a number of frames (1 or more), or 'all'",
            ),
            (
                "bt 1 x",
                "'x' is not a thread id; type 'list thread' for the threads",
            ),
            ("bt 1 2 3", "'bt' takes at most two arguments"),
        ] {
            assert_eq!(parse(line), Err(message.to_string()), "{line}");
        }
    }

    #[test]
    fn an_address_may_carry_an_offset_either_way() {
        assert_eq!(address("0x10"), Some(Ok(0x10)));
        assert_eq!(address("0x10+8"), Some(Ok(0x18)));
        assert_eq!(address("0X10-0x8"), Some(Ok(0x8)));
        assert_eq!(
            address("0x10-0x20"),
            Some(Err("'0x10-0x20' is below address 0".to_string()))
        );
        assert_eq!(
            address("0xffffffffffffffff+1"),
            Some(Err(
                "'0xffffffffffffffff+1' is past the last address".to_string()
            ))
        );
        for word in ["0xzz", "0x10+zz", "0x10-", "0x+4"] {
            assert_eq!(
                address(word),
                Some(Err(format!("'{word}' is not an address"))),
                "{word}"
            );
        }
        assert_eq!(address("16"), None);
        assert_eq!(Location::parse("0x1000-16"), Ok(Location::number(0xff0)));
        assert_eq!(Target::parse("0x1000+4"), Ok(Target::Address(0x1004)));
    }

    #[test]
    fn dis_takes_a_count_a_place_or_a_function_and_counts_back_when_negative() {
        let register = |name: &str, offset| Location::named(name, offset);
        for (line, expected) in [
            ("dis", Disassembly::Current),
            ("dis 5", Disassembly::Count(Window::count(5))),
            ("dis -3", Disassembly::Count(Window::count(-3))),
            (
                "dis pc-8 4",
                Disassembly::At(register("pc", -8), Some(Window::count(4))),
            ),
            ("dis x1", Disassembly::At(register("x1", 0), None)),
            (
                "dis lr -2",
                Disassembly::At(register("lr", 0), Some(Window::count(-2))),
            ),
            (
                "dis 0x100+4 -1",
                Disassembly::At(Location::number(0x104), Some(Window::count(-1))),
            ),
            (
                "dis isPrime",
                Disassembly::Named("isPrime".to_string(), None),
            ),
            (
                "dis isPrime -2",
                Disassembly::Named("isPrime".to_string(), Some(Window::count(-2))),
            ),
            ("dis x31", Disassembly::Named("x31".to_string(), None)),
            (
                "dis -6 8",
                Disassembly::Count(Window { back: 6, length: 8 }),
            ),
            (
                "dis isPrime -2 3",
                Disassembly::Named("isPrime".to_string(), Some(Window { back: 2, length: 3 })),
            ),
        ] {
            assert_eq!(parse(line), Ok(Command::Disassemble(expected)), "{line}");
        }
        for (line, message) in [
            (
                "dis 0",
                "cannot show 0 instructions; give a count other than 0 (negative counts back)",
            ),
            (
                "dis pc x",
                "cannot show x instructions; give a count other than 0 (negative counts back)",
            ),
            (
                "dis pc 1 2",
                "'1' does not count back; with two counts the first is negative, as in dis -6 8",
            ),
            (
                "dis -6 -2",
                "cannot show -2 instructions; the count after a negative one must be positive",
            ),
            ("dis pc -6 8 1", "'dis' takes at most three arguments"),
            ("dis 0xzz", "'0xzz' is not an address"),
        ] {
            assert_eq!(parse(line), Err(message.to_string()), "{line}");
        }
    }

    /// A counted `dis` that runs out of the current function heads each other
    /// function's instructions with that function; code no function holds
    /// stays under the starting point.
    #[test]
    fn dis_heads_each_function_s_instructions_with_that_function() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.code = true;
        inferior.pc = 0x1010;
        let (out, err) = drive(&mut inferior, "dis -3 5\ndis 0x1030 -3\nq\n", false);
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
prog`next_seed:
    0x0000000000001004 <+0>: nop
prog`_mfb_rt_print:
    0x0000000000001008 <+0>: nop
    0x000000000000100c <+4>: nop
prog`main:
->  0x0000000000001010 <+0>: nop
    0x0000000000001014 <+4>: nop
{PROMPT}\
prog`datetime::#normInstant:
    0x0000000000001024 <+4>: nop
0x0000000000001030:
    0x0000000000001028:      nop
    0x000000000000102c:      nop
    0x0000000000001030:      nop
{PROMPT}Killed process 42.
"
            )
        );
    }

    #[test]
    fn list_func_keeps_the_types_and_names_asked_for() {
        let (out, err, _) = session(
            "list func next_seed\nlist func MR MB\nlist func *_5F* free\nlist func MU free\nq\n",
            Ending::Killed,
            false,
        );
        assert_eq!(
            err,
            "error: no function matches 'MU free'; type 'list func' for the functions\n"
        );
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
Type | Address            | Symbol              | Written name
MU   | 0x0000000000001004 | _mfb_fn_next_5Fseed | next_seed

MU  {user}
{PROMPT}\
Type | Address            | Symbol                          | Written name
MB   | 0x0000000000001020 | _mfb_ifn_datetime_5FnormInstant | datetime::#normInstant
MR   | 0x0000000000001000 | _main                           | -
MR   | 0x0000000000001008 | _mfb_rt_print                   | -

MB  {builtin}
MR  {runtime}
{PROMPT}\
Type | Address            | Symbol                          | Written name
MU   | 0x0000000000001004 | _mfb_fn_next_5Fseed             | next_seed
MB   | 0x0000000000001020 | _mfb_ifn_datetime_5FnormInstant | datetime::#normInstant
SL   | 0x0000000000009000 | {free:<31} | free

MU  {user}
MB  {builtin}
SL  {linked}
{PROMPT}{PROMPT}Killed process 42.
",
                user = Origin::User.legend(),
                free = c("free"),
                builtin = Origin::Builtin.legend(),
                runtime = Origin::Runtime.legend(),
                linked = Origin::Linked.legend(),
            )
        );
    }

    #[test]
    fn a_glob_matches_the_whole_name() {
        for (pattern, text, matches) in [
            ("fib", "fib", true),
            ("fib", "fibs", false),
            ("fib", "_fib", false),
            ("*fib*", "_mfb_fn_fib", true),
            ("f?b", "fob", true),
            ("f?b", "fb", false),
            ("*", "", true),
            ("a*b*c", "aXbYbZc", true),
            ("a*b*c", "aXbYbZ", false),
            ("**x", "yyx", true),
            ("é?", "éa", true),
        ] {
            assert_eq!(glob(pattern, text), matches, "{pattern} {text}");
        }
    }

    #[test]
    fn a_bad_breakpoint_command_is_refused() {
        let (_, err, _) = session(
            "bp\nbp r\nbp 0xzz\nbp a b c\nbp r a b\nbp r nothing\nrun now\nq\n",
            Ending::Killed,
            false,
        );
        assert_eq!(
            err,
            "error: 'bp' needs an address (0x…) or a function name\n\
             error: 'bp' needs an address (0x…) or a function name\n\
             error: '0xzz' is not an address\n\
             error: unknown command 'b'; type 'help' for the list of commands\n\
             error: too many arguments to 'bp r'\n\
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
--[BP] T: 0 P: 0x0000000000001000 F: _main
--[BP] T: 1 P: 0x0000000000007000 F: -
{PROMPT}--[XC] T: 1 P: 0x0000000000007000 F: - X: a bad memory access at 0x8
{PROMPT}--[CC] User Break
{PROMPT}--[EX] Program ended by signal (9)
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
    fn watchpoint_breakpoint_and_breakerror_take_commands_after_a_semicolon() {
        let commands = |list: &[&str]| list.iter().map(|word| word.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse("bp fib bt 1; list reg x0 ;"),
            Ok(Command::SetBreakpoint {
                target: Target::Name("fib".to_string()),
                condition: None,
                commands: commands(&["bt 1", "list reg x0"])
            })
        );
        // `finish` may be anywhere among a stop's commands, `run` only last.
        assert_eq!(
            parse("bp fib bt 1; f; list reg x0; r"),
            Ok(Command::SetBreakpoint {
                target: Target::Name("fib".to_string()),
                condition: None,
                commands: commands(&["bt 1", "f", "list reg x0", "r"])
            })
        );
        assert_eq!(
            parse("wp sp+0x10"),
            Ok(Command::SetWatchpoint {
                location: Location::named("sp", 16),
                size: 8,
                commands: Vec::new(),
            })
        );
        assert_eq!(
            parse("watchpoint 0x1000 0x4 show mem 0x1000 4;be"),
            Ok(Command::SetWatchpoint {
                location: Location::number(0x1000),
                size: 4,
                commands: commands(&["show mem 0x1000 4", "be"]),
            })
        );
        assert_eq!(
            parse("wp r x19+8"),
            Ok(Command::RemoveWatchpoint(Location::named("x19", 8)))
        );
        assert_eq!(
            parse("be ErrOverflow 42 show error; bt"),
            Ok(Command::BreakError(
                ErrorCodes::Codes(vec![77_050_010, 42]),
                commands(&["show error", "bt"])
            ))
        );
        assert_eq!(
            parse("be any dis 4"),
            Ok(Command::BreakError(ErrorCodes::Any, commands(&["dis 4"])))
        );
        assert_eq!(
            parse("list watchpoint"),
            Ok(Command::List(Some(Listing::Watchpoints)))
        );
        for (line, message) in [
            (
                "bp fib run; bt",
                "'run' must be the last command of a stop: 'bt' follows it",
            ),
            (
                "bp fib r; finish",
                "'r' must be the last command of a stop: 'finish' follows it",
            ),
            (
                "wp 0x1000 8 bt; step",
                "'step' cannot run at a stop: a stop runs list, show, backtrace, disassemble, \
                 jump, finish, a bare breakerror and, last, run",
            ),
            (
                "be any be clear",
                "'be clear' cannot run at a stop: a stop runs list, show, backtrace, \
                 disassemble, jump, finish, a bare breakerror and, last, run",
            ),
            ("wp 0x1000 8x", "'8x' is not a number"),
            (
                "wp",
                "'wp' needs an address or a register: wp <where> [<size>] [<commands>]",
            ),
            (
                "wp r",
                "'wp' needs an address or a register: wp <where> [<size>] [<commands>]",
            ),
            ("wp r 0x10 8", "too many arguments to 'wp r'"),
            (
                "be bt",
                "'be' needs an error code, a name or 'any' before its commands",
            ),
            ("be clear bt", "'be clear' takes no commands"),
            (
                "list watchpoint 2",
                "too many arguments to 'list watchpoint'",
            ),
        ] {
            assert_eq!(parse(line), Err(message.to_string()), "{line}");
        }
    }

    #[test]
    fn a_stop_runs_its_commands_and_the_program_runs_on() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.runs = vec![
            RunStop::Breakpoints(vec![(0, 0x1004)]),
            RunStop::Exited(Exit::Status(0)),
        ];
        let (out, err) = drive(
            &mut inferior,
            "bp next_seed list reg x0; show mem 0x10; run\nrun\nq\n",
            false,
        );
        assert_eq!(err, "error: unmapped 0x10\n");
        assert_eq!(
            out,
            format!(
                "{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed. Each stop runs \
                 'list reg x0; show mem 0x10; run'.
{PROMPT}--[BP] T: 0 P: 0x0000000000001004 F: (next_seed) _mfb_fn_next_5Fseed
Register | Hex                | Decimal
x0       | 0x000000000000002a | (42)
--[EX] Program exit (0)
{PROMPT}Killed process 42.
"
            )
        );
    }

    #[test]
    fn a_stop_whose_commands_do_not_end_in_run_stays_stopped_and_is_counted() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.runs = vec![
            RunStop::Breakpoints(vec![(0, 0x1004)]),
            RunStop::Exited(Exit::Status(0)),
        ];
        let (out, err) = drive(
            &mut inferior,
            "bp next_seed list reg x0\nrun\nlist breakpoint\nq\n",
            false,
        );
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed. Each stop runs \
                 'list reg x0'.
{PROMPT}--[BP] T: 0 P: 0x0000000000001004 F: (next_seed) _mfb_fn_next_5Fseed
Register | Hex                | Decimal
x0       | 0x000000000000002a | (42)
{PROMPT}\
Address            | Count | Symbol              | Written name | Commands
0x0000000000001004 | 1     | _mfb_fn_next_5Fseed | next_seed    | list reg x0
{PROMPT}Killed process 42.
"
            )
        );
    }

    #[test]
    fn run_reports_a_changed_watchpoint_and_a_raised_error() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.runs = vec![
            RunStop::Changed(vec![
                Change {
                    thread: 0,
                    pc: 0x1014,
                    address: 0x2000,
                    size: 8,
                    old: 5,
                    new: u64::MAX,
                },
                Change {
                    thread: 1,
                    pc: 0x7000,
                    address: 0x3001,
                    size: 2,
                    old: 0xab,
                    new: 0x1cd,
                },
            ]),
            RunStop::Raised {
                thread: 0,
                code: 77_050_010,
                origin: Some("main.mfb:3:5".to_string()),
            },
            RunStop::Raised {
                thread: 0,
                code: 42,
                origin: None,
            },
        ];
        let (out, err) = drive(&mut inferior, "run\nrun\nrun\nq\n", false);
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}\
--[WP] T: 0 P: 0x0000000000001014 F: (main) _mfb_fn_main+0x4 M: 0x0000000000002000: 0x0000000000000005 (5) \
-> 0xffffffffffffffff (-1)
--[WP] T: 1 P: 0x0000000000007000 F: - M: 0x0000000000003001: 0x00ab -> 0x01cd
{PROMPT}--[BE] T: 0 P: 0x0000000000001000 F: _main E: errorCode::ErrOverflow (77050010)
{PROMPT}--[BE] T: 0 P: 0x0000000000001000 F: _main E: 42
{PROMPT}Killed process 42.
"
            )
        );
    }

    #[test]
    fn run_reports_an_exit_status() {
        let mut inferior = Recorder::new(Ending::AlreadyExited);
        inferior.runs = vec![RunStop::Exited(Exit::Status(3))];
        let (out, _) = drive(&mut inferior, "run\nq\n", false);
        assert!(out.contains("--[EX] Program exit (3)\n"), "{out}");
        let mut inferior = Recorder::new(Ending::AlreadyExited);
        inferior.runs = vec![RunStop::Exited(Exit::Unknown)];
        let (out, _) = drive(&mut inferior, "run\nq\n", false);
        assert!(out.contains("--[EX] Program exit\n"), "{out}");
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
                "'show' needs what to show: show mem <where> [<size>], show type <type> <where>, \
                 show arena [<id>], show error [<thread>], or show report",
            ),
            (
                "show it",
                "cannot show 'it'; 'show mem', 'show type', 'show arena', 'show error' and \
                 'show report' are what can be shown",
            ),
            ("show report now", "'show report' takes no arguments"),
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
    fn breakpointif_takes_a_test_a_place_and_commands() {
        let condition =
            |location, test, value| Some(Condition::parse(location, test, value).unwrap());
        assert_eq!(
            parse("bpi x0 eq 5 at logPath bt"),
            Ok(Command::SetBreakpoint {
                target: Target::Name("logPath".to_string()),
                condition: condition("x0", "eq", Some("5")),
                commands: vec!["bt".to_string()],
            })
        );
        assert_eq!(
            parse("breakpointif [sp+0x18] in at 0x1000+4"),
            Ok(Command::SetBreakpoint {
                target: Target::Address(0x1004),
                condition: condition("[sp+0x18]", "in", None),
                commands: Vec::new(),
            })
        );
        assert_eq!(
            parse("bpi r [x0]-8 lt -1 at main+0x10"),
            Ok(Command::RemoveBreakpoint {
                target: Target::Offset {
                    name: "main".to_string(),
                    offset: 0x10
                },
                condition: condition("[x0]-8", "lt", Some("-1")),
            })
        );
        for (line, message) in [
            (
                "bpi",
                "'bpi' needs a test and a place: bpi <where> eq|ne|gt|lt <value> at \
                 <address|function[+offset]> [<commands>], or bpi <where> in at \
                 <address|function[+offset]>",
            ),
            (
                "bpi x0 is 5 at f",
                "'is' is not a test: give eq, ne, gt, lt or in",
            ),
            (
                "bpi x0 eq at f",
                "'eq' needs a value to test against: <where> eq <value>",
            ),
            (
                "bpi x0 eq five at f",
                "'five' is not a value: give a number, such as 5, -1 or 0x10",
            ),
            (
                "bpi [x0] in 5 at f",
                "'in' takes no value: <where> in at <address|function[+offset]>",
            ),
            (
                "bpi x0 in at f",
                "'x0' can always be worked out: 'in' needs a memory read ([…])",
            ),
            (
                "bpi x0 eq 5 f",
                "'bpi' needs 'at' and the breakpoint's place after its test: bpi <where> \
                 <test> at <address|function[+offset]>",
            ),
            (
                "bpi x0 eq 5 at",
                "'bpi' needs a place after 'at': an address (0x…) or a function name",
            ),
            ("bpi r x0 eq 5 at f bt", "too many arguments to 'bpi r'"),
            (
                "bpi x0 eq 5 at f restart",
                "'restart' cannot run at a stop: a stop runs list, show, backtrace, \
                 disassemble, jump, finish, a bare breakerror and, last, run",
            ),
        ] {
            assert_eq!(parse(line), Err(message.to_string()), "{line}");
        }
    }

    #[test]
    fn a_breakpoint_with_a_test_is_listed_with_it_and_bp_r_takes_every_one_there() {
        let mut inferior = Recorder::new(Ending::Killed);
        let (out, err) = drive(
            &mut inferior,
            "bp next_seed bt
bpi x0 eq 5 at next_seed
bpi [0x12348593] eq 5 at next_seed bt
\
             bpi x9 eq 5 at next_seed
l breakpoint
bpi r x0 eq 5 at next_seed
\
             bp r next_seed
l breakpoint
q
",
            false,
        );
        assert_eq!(
            err,
            "error: no register or function is named 'x9'; type 'list reg' or 'list func' for \
             them
"
        );
        assert_eq!(
            out,
            format!(
                "{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed. Each stop runs 'bt'.
{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed if x0 eq 0x0000000000000005 (5).
{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed if [0x0000000012348593] eq \
                 0x0000000000000005 (5). Each stop runs 'bt'.
{PROMPT}{PROMPT}\
Address            | Count | Symbol              | Written name | If                                             | Commands
0x0000000000001004 | 0     | _mfb_fn_next_5Fseed | next_seed    | -                                              | bt
0x0000000000001004 | 0     | _mfb_fn_next_5Fseed | next_seed    | x0 eq 0x0000000000000005 (5)                   | -
0x0000000000001004 | 0     | _mfb_fn_next_5Fseed | next_seed    | [0x0000000012348593] eq 0x0000000000000005 (5) | bt
{PROMPT}Removed the breakpoint at 0x0000000000001004 in next_seed if x0 eq 0x0000000000000005 (5).
{PROMPT}Removed the breakpoint at 0x0000000000001004 in next_seed.
Removed the breakpoint at 0x0000000000001004 in next_seed if [0x0000000012348593] eq \
                 0x0000000000000005 (5).
{PROMPT}no breakpoints
{PROMPT}Killed process 42.
"
            )
        );
        assert!(inferior.inserted.is_empty());
    }

    #[test]
    fn jump_moves_the_current_thread_and_sets_its_registers() {
        assert_eq!(
            parse("j lr x0=0 x1=-1 sp=[sp+8]"),
            Ok(Command::Jump {
                to: Location::named("lr", 0),
                registers: vec![
                    ("x0".to_string(), Location::number(0)),
                    ("x1".to_string(), Location::number(u64::MAX)),
                    ("sp".to_string(), Location::parse("[sp+8]").unwrap()),
                ],
            })
        );
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.code = true;
        inferior.words = vec![(0x2a, 0x77)];
        let (out, err) = drive(
            &mut inferior,
            "jump 0x1010
j next_seed x0=[x0] nzcv=0x60000000 fpcr=0
list reg x0
q
",
            false,
        );
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}Moved thread 0 to 0x0000000000001010 in main.
{PROMPT}Moved thread 0 to 0x0000000000001004 in next_seed; set x0 = 0x0000000000000077 \
                 (119), nzcv = 0x0000000060000000 (1610612736), fpcr = 0x0000000000000000 (0).
{PROMPT}\
Register | Hex                | Decimal
x0       | 0x0000000000000077 | (119)
{PROMPT}Killed process 42.
"
            )
        );
        assert_eq!(
            inferior.sets,
            [
                (0, vec![("pc".to_string(), 0x1010)]),
                (
                    0,
                    vec![
                        ("pc".to_string(), 0x1004),
                        ("x0".to_string(), 0x77),
                        ("nzcv".to_string(), 0x6000_0000),
                        ("fpcr".to_string(), 0),
                    ]
                ),
            ]
        );
    }

    #[test]
    fn jump_refuses_a_bad_place_register_or_value() {
        let sets = "jump sets x0-x28, fp, lr, sp, nzcv, fpsr and fpcr";
        for (line, message) in [
            (
                "j".to_string(),
                "'j' needs a place to go: j <where> [<register>=<value>]...".to_string(),
            ),
            (
                "j lr pc=4".to_string(),
                format!(
                    "'pc' is where the thread goes: give it as the place, jump <where>; {sets}"
                ),
            ),
            (
                "j lr v0=1".to_string(),
                format!("'v0' is a vector register; {sets}"),
            ),
            (
                "j lr x29=1".to_string(),
                format!("no register 'x29'; {sets}"),
            ),
            (
                "j lr x0=1 x0=2".to_string(),
                "'x0' is set twice".to_string(),
            ),
            (
                "j lr x0".to_string(),
                "'x0' is not a register and its value: <register>=<value>, such as x0=5"
                    .to_string(),
            ),
            (
                "j lr x0=".to_string(),
                "'x0' needs a value: x0=<value>".to_string(),
            ),
            (
                format!(
                    "j lr {}",
                    (0..9)
                        .map(|n| format!("x{n}=0"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                "'j' sets at most 8 registers".to_string(),
            ),
        ] {
            assert_eq!(parse(&line), Err(message), "{line}");
        }
        let mut inferior = Recorder::new(Ending::Killed);
        let (_, err) = drive(
            &mut inferior,
            "j 0x1002
j 0x1010
",
            false,
        );
        assert_eq!(
            err,
            "error: 0x0000000000001002 is not the start of an instruction: they start every 4 \
             bytes
\
             error: cannot jump to 0x0000000000001010: unmapped 0x1010
"
        );
        inferior.code = true;
        let (_, err) = drive(
            &mut inferior,
            "j 0x1010 nzcv=1
j 0x1010 fpsr=0x100000000
j 0x1010 x0=[0x10]
j 0x1010 x0=nothing
",
            false,
        );
        assert_eq!(
            err,
            "error: cannot set nzcv to 0x0000000000000001: it holds only the flags, bits 28 to 31 \
             (0x00000000f0000000)
\
             error: cannot set fpsr to 0x0000000100000000: it holds 32 bits
\
             error: cannot set x0: unmapped 0x10
\
             error: cannot set x0: no register or function is named 'nothing'; type 'list reg' or \
             'list func' for them
"
        );
        assert!(inferior.sets.is_empty());
    }

    #[test]
    fn a_stop_can_jump_the_stopped_thread_and_run_on() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.code = true;
        inferior.runs = vec![
            RunStop::Breakpoints(vec![(0, 0x1004)]),
            RunStop::Exited(Exit::Status(0)),
        ];
        let (out, err) = drive(
            &mut inferior,
            "bp next_seed j 0x1010 x0=7; run
run
q
",
            false,
        );
        assert_eq!(err, "");
        assert_eq!(
            out,
            format!(
                "{PROMPT}Set a breakpoint at 0x0000000000001004 in next_seed. Each stop runs 'j \
                 0x1010 x0=7; run'.
{PROMPT}--[BP] T: 0 P: 0x0000000000001004 F: (next_seed) _mfb_fn_next_5Fseed
Moved thread 0 to 0x0000000000001010 in main; set x0 = 0x0000000000000007 (7).
--[EX] Program exit (0)
{PROMPT}Killed process 42.
"
            )
        );
    }

    #[test]
    fn show_mem_reads_through_memory_in_brackets() {
        let mut inferior = Recorder::new(Ending::Killed);
        inferior.words = vec![(0x2000, 0x3000), (0x3000, 0x4847_4645_4443_4241)];
        let (out, err) = drive(
            &mut inferior,
            "show mem [0x2000] 8
show mem [x0]+8 8
show mem [0x2000 8
",
            false,
        );
        assert_eq!(
            out,
            format!(
                "{PROMPT}Memory at 0x0000000000003000 ([0x0000000000002000]):
0x0000000000003000 41 42 43 44 45 46 47 48                         | A B C D E F G H
{PROMPT}{PROMPT}{PROMPT}
Killed process 42.
"
            )
        );
        assert_eq!(
            err,
            "error: unmapped 0x2a
\
             error: '[0x2000' opens a '[' it does not close
"
        );
        assert_eq!(
            parse("dis [sp+8]"),
            Ok(Command::Disassemble(Disassembly::At(
                Location::parse("[sp+8]").unwrap(),
                None
            )))
        );
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
