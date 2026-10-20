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
mod tests;
