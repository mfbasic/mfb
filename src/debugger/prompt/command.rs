//! plan-176-L: the `mfb dbg` prompt's command code. The command loop `run` and the help
//! text live in `debugger/prompt/mod.rs`.

use super::*;

/// The thread a bare `step` runs and `list reg` shows when given no thread:
/// the initial thread.
pub(super) const MAIN_THREAD: usize = 0;

/// One parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Command {
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
pub(super) enum Backtrace {
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
pub(super) struct Dump {
    pub(super) block: usize,
    pub(super) offset: u64,
    pub(super) size: u64,
}

/// What a `dis` names, before a location is resolved to an address.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Disassembly {
    Current,
    Count(Window),
    At(Location, Option<Window>),
    Named(String, Option<Window>),
}

impl Disassembly {
    /// `dis <start> [<count>]`: a location (an address, or a register, either
    /// with an offset), else a function name.
    pub(super) fn from(start: &str, count: Option<Window>) -> Result<Disassembly, String> {
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
pub(super) const STRING_LITERAL_PREFIX: &str = "_mfb_str_";

/// How much `show mem` shows when no size is given, and the most it shows.
pub(super) const MEMORY_SIZE: u64 = 64;

pub(super) const MAX_MEMORY: u64 = 64 << 10;

/// The arena a bare `show arena` shows: the main arena's registry slot.
const MAIN_ARENA: usize = 0;

/// How much of a block a dump shows when no size is given.
const DUMP_SIZE: u64 = 1024;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Listing {
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
    pub(super) fn parse(words: &[&str]) -> FunctionFilter {
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
    pub(super) fn admits(&self, origin: Origin, symbol: &str, written: Option<&str>) -> bool {
        (self.types.is_empty() || self.types.contains(&origin))
            && (self.patterns.is_empty()
                || self.patterns.iter().any(|pattern| {
                    glob(pattern, symbol) || written.is_some_and(|name| glob(pattern, name))
                }))
    }

    /// The filter as the user typed it, for a listing that matches nothing.
    pub(super) fn words(&self) -> String {
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
pub(super) fn glob(pattern: &str, text: &str) -> bool {
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
pub(super) enum RegisterPick {
    Group(RegisterGroup),
    Name(String),
}

impl RegisterPick {
    pub(super) fn parse(word: &str) -> RegisterPick {
        match word {
            "gpr" => RegisterPick::Group(RegisterGroup::General),
            "simd" => RegisterPick::Group(RegisterGroup::Simd),
            "sys" => RegisterPick::Group(RegisterGroup::System),
            name => RegisterPick::Name(name.to_string()),
        }
    }
}

pub(super) fn parse(line: &str) -> Result<Command, String> {
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
pub(super) fn after_words(line: &str, count: usize) -> &str {
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
pub(super) const NZCV_FLAGS: u64 = 0xf000_0000;

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
pub(super) fn stop_commands(words: &[&str]) -> Result<Vec<String>, String> {
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
pub(super) fn runs_on(command: &str) -> bool {
    matches!(parse(command), Ok(Command::Run))
}

/// The fault each thread is stopped on, as its stop named it (`a bad memory
/// access at 0x8`), until the thread runs again: what `show error` says when
/// the thread has no error.
pub(super) type Faults = std::collections::BTreeMap<usize, String>;

/// A stop's commands that wait for the prompt to run them, each with the
/// thread that stopped: the ones of a stop that has a `finish` among them.
pub(super) type Queue = std::collections::VecDeque<(usize, String)>;

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
