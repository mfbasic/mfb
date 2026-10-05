//! The session's breakpoints and `run`.
//!
//! A breakpoint is set by address or by function name. A name is looked up as
//! a symbol first (`_mfb_rt_print`, `_exit`) and, only when no symbol has it,
//! as a written name (`isPrime`, `exit`); every distinct address it matches
//! gets a breakpoint. A linked function the loader has not bound yet has no
//! address: its breakpoint is pending, and `run` places it once the program
//! reaches its entry point, by which time every library is loaded — stopping
//! there on a breakpoint of its own when no other is in the way.
//!
//! Each breakpoint remembers the function and offset it is at, so a `restart`
//! (a new process, loaded at a new address) places it again. `finish` runs the
//! current thread to the end of its function with breakpoints of its own on the
//! function's `ret`s, then steps the `ret`.
//!
//! A watchpoint covers 1 to 8 bytes at an address. A thread about to write
//! them is stepped over the write; when the bytes changed, that is a stop, and
//! otherwise the program runs on unseen. A `restart` drops every watchpoint:
//! the new process's memory is laid out anew.
//!
//! A breakpoint, a watchpoint and the `breakerror` watch may carry commands:
//! a stop at one runs them, and when every stop at once has commands the
//! program runs on. `run` hands each stop to the session as it happens.

use super::disassemble::returns;
use super::names::{classify, Collisions};
use super::prompt::table;
use super::{Change, FunctionSymbol, Inferior, RunStop, StepStop};

/// One breakpoint the session holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Breakpoint {
    /// Where it is written; `None` while its function is not loaded.
    pub(crate) address: Option<u64>,
    /// The symbol it is on (with `+0x<offset>` inside a function), if any.
    pub(crate) symbol: Option<String>,
    /// The name the developer wrote (likewise), if any.
    pub(crate) written: Option<String>,
    /// `<mfb>` or `<native>` when an MFBASIC and a native function share the
    /// written name; shown after it, never matched.
    pub(crate) tag: Option<&'static str>,
    /// The function symbol it is in and its offset there, by which a restart
    /// finds it again; `None` for an address no function holds.
    pub(crate) anchor: Option<(String, u64)>,
    /// The commands a stop here runs before the program runs on.
    pub(crate) commands: Vec<String>,
}

/// One watchpoint the session holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Watch {
    pub(crate) address: u64,
    pub(crate) size: u64,
    /// The commands a change here runs before the program runs on.
    pub(crate) commands: Vec<String>,
}

/// The most bytes a watchpoint covers, and the boundary it cannot cross.
pub(crate) const WATCH_SPAN: u64 = 8;

/// Refuse a watchpoint the processor cannot hold: one covers 1 to 8 bytes
/// that do not cross a multiple of 8.
pub(crate) fn watchable(address: u64, size: u64) -> Result<(), String> {
    if size == 0 || size > WATCH_SPAN || address % WATCH_SPAN + size > WATCH_SPAN {
        return Err(format!(
            "cannot watch {size} bytes at 0x{address:016x}: a watchpoint covers 1 to 8 bytes \
             that do not cross an address that is a multiple of 8"
        ));
    }
    Ok(())
}

/// The `size` bytes at `address`, little-endian.
fn watched_value(inferior: &mut dyn Inferior, address: u64, size: u64) -> Result<u64, String> {
    let bytes = inferior.read_bytes(address, size as usize)?;
    let mut word = [0u8; 8];
    word[..bytes.len().min(8)].copy_from_slice(&bytes[..bytes.len().min(8)]);
    Ok(u64::from_le_bytes(word))
}

/// A watched value as a message shows it: hex at the watchpoint's width, and
/// for 8 bytes the signed decimal too (`0x05`, `0x0000000000000005 (5)`).
pub(crate) fn value_shown(value: u64, size: u64) -> String {
    let digits = size as usize * 2;
    if size == WATCH_SPAN {
        format!("0x{value:0digits$x} ({})", value as i64)
    } else {
        format!("0x{value:0digits$x}")
    }
}

/// Commands as a message shows them: `bt 1; list reg x0`.
pub(crate) fn commands_shown(commands: &[String]) -> String {
    commands.join("; ")
}

/// Commands as a listing's `Commands` column shows them: `-` for none.
fn listed(commands: &[String]) -> String {
    if commands.is_empty() {
        "-".to_string()
    } else {
        commands_shown(commands)
    }
}

/// The sentence after a breakpoint or watchpoint that has commands.
fn runs(commands: &[String]) -> String {
    if commands.is_empty() {
        String::new()
    } else {
        format!(
            " Each stop runs '{}' and continues.",
            commands_shown(commands)
        )
    }
}

/// What a `run` ends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ended {
    /// A stop the session heard and did not run on from.
    Heard(RunStop),
    /// Threads at breakpoints the caller placed itself (`finish`'s), unheard.
    Own(Vec<(usize, u64)>),
}

/// The session's ear: hears each stop with the notes made since the last one,
/// and answers whether the program runs on.
pub(crate) type Hear<'a> =
    dyn FnMut(&mut dyn Inferior, &Breakpoints, Vec<String>, &RunStop) -> Result<bool, String> + 'a;

/// What a `breakpoint` command names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Address(u64),
    Name(String),
    /// `<function>+<offset>`: that many bytes into each function the name
    /// names, as `bt` and stops show a place (`main+0x110`, `main+272`).
    Offset {
        name: String,
        offset: u64,
    },
}

impl Target {
    /// `0x` and hex digits, perhaps with `+`/`-` an offset, is an address;
    /// `<name>+<n>` (`n` decimal or `0x` hex) a place in a function; anything
    /// else is a name.
    pub(crate) fn parse(word: &str) -> Result<Target, String> {
        if let Some(address) = super::prompt::address(word) {
            return address.map(Target::Address);
        }
        match word.split_once('+') {
            Some((name, offset)) if !name.is_empty() => Ok(Target::Offset {
                name: name.to_string(),
                offset: super::prompt::number(offset)?,
            }),
            _ => Ok(Target::Name(word.to_string())),
        }
    }

    /// How the target reads in a message: `0x…`, `isPrime`, `main+0x110`.
    fn shown(&self) -> String {
        match self {
            Target::Address(address) => format!("0x{address:016x}"),
            Target::Name(name) => name.clone(),
            Target::Offset { name, offset } => format!("{name}+{offset:#x}"),
        }
    }
}

/// The addresses `offset` bytes into each loaded function `name` names.
fn offsets(functions: &[FunctionSymbol], name: &str, offset: u64) -> Result<Vec<u64>, String> {
    let found = find(functions, name);
    if found.is_empty() {
        return Err(format!(
            "no function is named '{name}'; type 'list func' for the functions"
        ));
    }
    let starts = found
        .iter()
        .filter_map(|breakpoint| breakpoint.address)
        .collect::<Vec<_>>();
    if starts.is_empty() {
        return Err(format!(
            "'{name}' is not loaded yet; run the program until it has loaded its libraries"
        ));
    }
    starts
        .into_iter()
        .map(|start| {
            start
                .checked_add(offset)
                .ok_or_else(|| format!("'{name}+{offset:#x}' is past the last address"))
        })
        .collect()
}

#[derive(Debug, Default)]
pub(crate) struct Breakpoints {
    list: Vec<Breakpoint>,
    watches: Vec<Watch>,
    /// The program has been seen at its entry point: every library is bound.
    entry_passed: bool,
    /// The errors `breakerror` stops on, if any.
    errors: Option<ErrorWatch>,
    /// Notes made since the session last heard a stop.
    notes: Vec<String>,
    /// Threads a step over a watched write left on one of the user's
    /// breakpoints: the next `run` stops there before it runs anything.
    landed: Vec<(usize, u64)>,
}

/// The runtime function every newly raised error goes through: it parks the
/// `Error` block (code in `x1`, origin `ErrorLoc` in `x3`) and returns to the
/// function that raised it.
pub(crate) const PARK_ERROR: &str = "_mfb_rt_park_error";

/// Which errors `breakerror` stops on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ErrorCodes {
    Any,
    /// These codes, in the order they were asked for.
    Codes(Vec<i64>),
}

/// `breakerror`'s watch: the errors, and the live address of [`PARK_ERROR`],
/// where the debugger keeps a breakpoint of its own.
#[derive(Debug)]
struct ErrorWatch {
    codes: ErrorCodes,
    park: u64,
    /// The commands a stop for a watched error runs.
    commands: Vec<String>,
}

/// A raise `breakerror` stops for, caught as it entered the park.
struct Raise {
    thread: usize,
    code: i64,
    origin: Option<String>,
    /// Where the park returns to: in the function that raised the error.
    back: u64,
}

/// The live address of [`PARK_ERROR`].
fn park_address(inferior: &mut dyn Inferior) -> Result<u64, String> {
    inferior
        .functions()?
        .iter()
        .find(|function| function.name == PARK_ERROR && !function.linked)
        .and_then(|function| function.address)
        .ok_or_else(|| {
            "'breakerror' stops in the runtime's error function, which only a debug build \
             names; rebuild the program with: mfb build --debug -O0 <project>"
                .to_string()
        })
}

/// An error code as a sentence names it: `77050010 (errorCode::ErrOverflow)`.
pub(crate) fn code_shown(code: i64) -> String {
    match super::runtime::error_code_name(code) {
        Some(name) => format!("{code} ({name})"),
        None => code.to_string(),
    }
}

impl ErrorCodes {
    fn shown(&self) -> String {
        match self {
            ErrorCodes::Any => "any error".to_string(),
            ErrorCodes::Codes(codes) => codes
                .iter()
                .map(|code| code_shown(*code))
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    fn matches(&self, code: i64) -> bool {
        match self {
            ErrorCodes::Any => true,
            ErrorCodes::Codes(codes) => codes.contains(&code),
        }
    }
}

/// The written name of a function symbol, if it has one.
fn written(function: &FunctionSymbol) -> Option<String> {
    classify(function).1
}

/// The tag of `function`'s written name, when an MFBASIC and a native function
/// share it (`exit <mfb>`).
fn tag(function: &FunctionSymbol, collisions: &Collisions) -> Option<&'static str> {
    collisions.tag(&written(function)?, classify(function).0)
}

/// A name as `list breakpoint` prints it: `-` for none.
fn shown(name: &Option<String>) -> &str {
    name.as_deref().unwrap_or("-")
}

/// The breakpoints a name asks for: symbols spelled `name`, or else functions
/// written `name`; one per distinct address, and one per symbol not yet bound.
fn find(functions: &[FunctionSymbol], name: &str) -> Vec<Breakpoint> {
    let collisions = Collisions::of(functions);
    let mut matches = functions
        .iter()
        .filter(|function| function.name == name)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        matches = functions
            .iter()
            .filter(|function| written(function).as_deref() == Some(name))
            .collect();
    }
    let mut found: Vec<Breakpoint> = Vec::new();
    for function in matches {
        let duplicate = found.iter().any(|other| match function.address {
            Some(_) => other.address == function.address,
            None => other.address.is_none() && other.symbol.as_ref() == Some(&function.name),
        });
        let bound_elsewhere = function.address.is_none()
            && functions
                .iter()
                .any(|other| other.name == function.name && other.address.is_some());
        if !duplicate && !bound_elsewhere {
            found.push(Breakpoint {
                address: function.address,
                symbol: Some(function.name.clone()),
                written: written(function),
                tag: tag(function, &collisions),
                anchor: Some((function.name.clone(), 0)),
                commands: Vec::new(),
            });
        }
    }
    found
}

/// The breakpoint for `address`: named by the function that holds it (with the
/// offset into it), a linked function bound there, or neither.
fn describe(functions: &[FunctionSymbol], address: u64) -> Breakpoint {
    let collisions = Collisions::of(functions);
    let offset = |name: Option<String>, start: u64| {
        name.map(|name| {
            if address == start {
                name
            } else {
                format!("{name}+{:#x}", address - start)
            }
        })
    };
    let holder = functions.iter().find(|function| {
        !function.linked
            && function.address.is_some_and(|start| {
                start <= address && address - start < function.size.unwrap_or(0)
            })
    });
    if let Some(function) = holder {
        let start = function.address.unwrap_or(address);
        return Breakpoint {
            address: Some(address),
            symbol: offset(Some(function.name.clone()), start),
            written: offset(written(function), start),
            tag: tag(function, &collisions),
            anchor: Some((function.name.clone(), address - start)),
            commands: Vec::new(),
        };
    }
    match functions
        .iter()
        .find(|function| function.linked && function.address == Some(address))
    {
        Some(function) => Breakpoint {
            address: Some(address),
            symbol: Some(function.name.clone()),
            written: written(function),
            tag: tag(function, &collisions),
            anchor: Some((function.name.clone(), 0)),
            commands: Vec::new(),
        },
        None => Breakpoint {
            address: Some(address),
            symbol: None,
            written: None,
            tag: None,
            anchor: None,
            commands: Vec::new(),
        },
    }
}

/// How a breakpoint reads in a sentence: `0x… in isPrime`.
fn place(breakpoint: &Breakpoint) -> String {
    let name = written_shown(breakpoint).or_else(|| breakpoint.symbol.clone());
    match (breakpoint.address, name) {
        (Some(address), None) => format!("0x{address:016x}"),
        (Some(address), Some(name)) => format!("0x{address:016x} in {name}"),
        (None, _) => shown(&breakpoint.symbol).to_string(),
    }
}

/// A breakpoint's written name with its tag, if any.
fn written_shown(breakpoint: &Breakpoint) -> Option<String> {
    let written = breakpoint.written.clone()?;
    Some(match breakpoint.tag {
        Some(tag) => format!("{written} {tag}"),
        None => written,
    })
}

impl Breakpoints {
    /// Set breakpoints at `target`, each running `commands` at a stop.
    pub(crate) fn set(
        &mut self,
        inferior: &mut dyn Inferior,
        target: &Target,
        commands: &[String],
    ) -> Result<Vec<String>, String> {
        let functions = inferior.functions().unwrap_or_default();
        let wanted = match target {
            Target::Address(address) => vec![describe(&functions, *address)],
            Target::Name(name) => {
                let found = find(&functions, name);
                if found.is_empty() {
                    return Err(format!(
                        "no function is named '{name}'; type 'list func' for the functions"
                    ));
                }
                found
            }
            Target::Offset { name, offset } => offsets(&functions, name, *offset)?
                .into_iter()
                .map(|address| describe(&functions, address))
                .collect(),
        };
        let mut lines = Vec::new();
        for mut breakpoint in wanted {
            breakpoint.commands = commands.to_vec();
            let exists = self.list.iter().any(|other| match breakpoint.address {
                Some(_) => other.address == breakpoint.address,
                None => other.address.is_none() && other.symbol == breakpoint.symbol,
            });
            if exists {
                continue;
            }
            match breakpoint.address {
                Some(address) => {
                    // The park already holds `breakerror`'s breakpoint.
                    if self.own_park() != Some(address) {
                        inferior.insert_breakpoint(address)?;
                    }
                    lines.push(format!(
                        "Set a breakpoint at {}.{}",
                        place(&breakpoint),
                        runs(commands)
                    ));
                }
                None => lines.push(format!(
                    "Set a pending breakpoint on {}: it is placed when the program has \
                     loaded its libraries.{}",
                    shown(&breakpoint.symbol),
                    runs(commands)
                )),
            }
            self.list.push(breakpoint);
        }
        if lines.is_empty() {
            return Err(match target {
                Target::Name(name) => format!("there is already a breakpoint on '{name}'"),
                other => format!("there is already a breakpoint at {}", other.shown()),
            });
        }
        Ok(lines)
    }

    pub(crate) fn remove(
        &mut self,
        inferior: &mut dyn Inferior,
        target: &Target,
    ) -> Result<Vec<String>, String> {
        let chosen = match target {
            Target::Address(address) => self
                .list
                .iter()
                .filter(|breakpoint| breakpoint.address == Some(*address))
                .cloned()
                .collect::<Vec<_>>(),
            Target::Offset { name, offset } => {
                let functions = inferior.functions().unwrap_or_default();
                let addresses = offsets(&functions, name, *offset)?;
                self.list
                    .iter()
                    .filter(|breakpoint| {
                        breakpoint
                            .address
                            .is_some_and(|address| addresses.contains(&address))
                    })
                    .cloned()
                    .collect()
            }
            Target::Name(name) => {
                let by_symbol = self
                    .list
                    .iter()
                    .filter(|breakpoint| breakpoint.symbol.as_ref() == Some(name))
                    .cloned()
                    .collect::<Vec<_>>();
                if by_symbol.is_empty() {
                    self.list
                        .iter()
                        .filter(|breakpoint| breakpoint.written.as_ref() == Some(name))
                        .cloned()
                        .collect()
                } else {
                    by_symbol
                }
            }
        };
        if chosen.is_empty() {
            return Err(match target {
                Target::Name(name) => format!("there is no breakpoint on '{name}'"),
                other => format!("there is no breakpoint at {}", other.shown()),
            });
        }
        let mut lines = Vec::new();
        for breakpoint in chosen {
            let watched = self.errors.as_ref().map(|watch| watch.park);
            if let Some(address) = breakpoint
                .address
                .filter(|address| Some(*address) != watched)
            {
                inferior.remove_breakpoint(address)?;
            }
            self.list.retain(|other| *other != breakpoint);
            lines.push(format!("Removed the breakpoint at {}.", place(&breakpoint)));
        }
        Ok(lines)
    }

    /// The `list breakpoint` lines: `<address> <symbol> <written name>`, in
    /// address order, pending ones last with `-` for the address; then the
    /// errors `breakerror` watches for.
    pub(crate) fn lines(&self) -> Vec<String> {
        let mut lines = self.breakpoint_lines();
        if let Some(watch) = self.watch_line() {
            if self.list.is_empty() {
                lines.clear();
            }
            lines.push(watch);
        }
        lines
    }

    /// The breakpoint table; a `Commands` column joins it when any
    /// breakpoint has commands.
    fn breakpoint_lines(&self) -> Vec<String> {
        if self.list.is_empty() {
            return vec!["no breakpoints".to_string()];
        }
        let mut sorted = self.list.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|breakpoint| (breakpoint.address.is_none(), breakpoint.address));
        let rows = sorted
            .iter()
            .map(|breakpoint| {
                [
                    breakpoint
                        .address
                        .map_or_else(|| "-".to_string(), |address| format!("0x{address:016x}")),
                    shown(&breakpoint.symbol).to_string(),
                    shown(&written_shown(breakpoint)).to_string(),
                    listed(&breakpoint.commands),
                ]
            })
            .collect::<Vec<_>>();
        if sorted
            .iter()
            .all(|breakpoint| breakpoint.commands.is_empty())
        {
            return table(
                ["Address", "Symbol", "Written name"],
                rows.into_iter()
                    .map(|[address, symbol, written, _]| [address, symbol, written])
                    .collect(),
            );
        }
        table(["Address", "Symbol", "Written name", "Commands"], rows)
    }

    /// `watchpoint <where> [<size>] [<commands>]`: stop when a thread changes
    /// the `size` bytes at `address`.
    pub(crate) fn watch(
        &mut self,
        inferior: &mut dyn Inferior,
        address: u64,
        size: u64,
        commands: &[String],
    ) -> Result<Vec<String>, String> {
        watchable(address, size)?;
        if self.watches.iter().any(|watch| watch.address == address) {
            return Err(format!("there is already a watchpoint at 0x{address:016x}"));
        }
        let value = watched_value(inferior, address, size)?;
        inferior.insert_watchpoint(address, size)?;
        self.watches.push(Watch {
            address,
            size,
            commands: commands.to_vec(),
        });
        Ok(vec![format!(
            "Set a watchpoint at 0x{address:016x} ({size} {}): {}.{}",
            if size == 1 { "byte" } else { "bytes" },
            value_shown(value, size),
            runs(commands)
        )])
    }

    /// `watchpoint r <where>`: take away the watchpoint at `address`.
    pub(crate) fn unwatch(
        &mut self,
        inferior: &mut dyn Inferior,
        address: u64,
    ) -> Result<Vec<String>, String> {
        let Some(at) = self
            .watches
            .iter()
            .position(|watch| watch.address == address)
        else {
            return Err(format!("there is no watchpoint at 0x{address:016x}"));
        };
        inferior.remove_watchpoint(address)?;
        let watch = self.watches.remove(at);
        Ok(vec![format!(
            "Removed the watchpoint at 0x{address:016x} ({} {}).",
            watch.size,
            if watch.size == 1 { "byte" } else { "bytes" }
        )])
    }

    /// The `list watchpoint` lines: `<address> <size> <value>`, in address
    /// order, the value read now (`-` when it cannot be); a `Commands` column
    /// joins them when any watchpoint has commands.
    pub(crate) fn watch_lines(&self, inferior: &mut dyn Inferior) -> Vec<String> {
        if self.watches.is_empty() {
            return vec!["no watchpoints".to_string()];
        }
        let mut sorted = self.watches.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|watch| watch.address);
        let rows = sorted
            .iter()
            .map(|watch| {
                [
                    format!("0x{:016x}", watch.address),
                    watch.size.to_string(),
                    watched_value(inferior, watch.address, watch.size)
                        .map_or_else(|_| "-".to_string(), |value| value_shown(value, watch.size)),
                    listed(&watch.commands),
                ]
            })
            .collect::<Vec<_>>();
        if sorted.iter().all(|watch| watch.commands.is_empty()) {
            return table(
                ["Address", "Size", "Value"],
                rows.into_iter()
                    .map(|[address, size, value, _]| [address, size, value])
                    .collect(),
            );
        }
        table(["Address", "Size", "Value", "Commands"], rows)
    }

    /// The commands each stop in `stop` runs, by thread, and whether every
    /// stop in it has commands, so that the program runs on.
    pub(crate) fn stop_commands(&self, stop: &RunStop) -> (Vec<(usize, Vec<String>)>, bool) {
        let mut found = Vec::new();
        let mut every = true;
        let mut add = |thread: usize, commands: Option<&Vec<String>>| match commands {
            Some(commands) if !commands.is_empty() => found.push((thread, commands.clone())),
            _ => every = false,
        };
        match stop {
            RunStop::Breakpoints(hits) => {
                for (thread, address) in hits {
                    add(
                        *thread,
                        self.list
                            .iter()
                            .find(|breakpoint| breakpoint.address == Some(*address))
                            .map(|breakpoint| &breakpoint.commands),
                    );
                }
            }
            RunStop::Changed(changes) => {
                for change in changes {
                    add(
                        change.thread,
                        self.watches
                            .iter()
                            .find(|watch| watch.address == change.address)
                            .map(|watch| &watch.commands),
                    );
                }
            }
            RunStop::Raised { thread, .. } => {
                add(*thread, self.errors.as_ref().map(|watch| &watch.commands))
            }
            _ => every = false,
        }
        (found, every)
    }

    /// The notes made since the session last heard a stop.
    pub(crate) fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// Run the program, handing `hear` each stop the user should see as it
    /// happens, until `hear` says not to run on, the program ends, or it
    /// reaches breakpoints the caller placed itself. Pending breakpoints are
    /// placed as soon as their functions are bound; while any is pending, the
    /// program's entry point holds a breakpoint of the debugger's own, and
    /// stopping there only places them and runs on. A write a watchpoint
    /// covers that leaves its bytes as they were runs on unseen.
    pub(crate) fn run(
        &mut self,
        inferior: &mut dyn Inferior,
        hear: &mut Hear,
    ) -> Result<Ended, String> {
        loop {
            let landed = std::mem::take(&mut self.landed);
            let (stop, own) = if landed.is_empty() {
                self.run_once(inferior)?
            } else {
                (Some(RunStop::Breakpoints(landed)), Vec::new())
            };
            let Some(stop) = stop else {
                if own.is_empty() {
                    continue;
                }
                return Ok(Ended::Own(own));
            };
            let exited = matches!(stop, RunStop::Exited(_));
            if !exited {
                self.place_pending(inferior)?;
            }
            let notes = self.take_notes();
            let on = hear(inferior, self, notes, &stop)?;
            if !on || exited {
                return Ok(Ended::Heard(stop));
            }
            if !own.is_empty() {
                return Ok(Ended::Own(own));
            }
        }
    }

    /// Run the program once: the stop the user should see, if any, and the
    /// threads at breakpoints neither the user's nor the debugger's own.
    #[allow(clippy::type_complexity)]
    fn run_once(
        &mut self,
        inferior: &mut dyn Inferior,
    ) -> Result<(Option<RunStop>, Vec<(usize, u64)>), String> {
        self.place_pending(inferior)?;
        let entry = inferior.entry()?;
        let temporary = match entry {
            Some(entry)
                if !self.entry_passed
                    && self
                        .list
                        .iter()
                        .any(|breakpoint| breakpoint.address.is_none())
                    && !self
                        .list
                        .iter()
                        .any(|breakpoint| breakpoint.address == Some(entry)) =>
            {
                inferior.insert_breakpoint(entry)?;
                Some(entry)
            }
            _ => None,
        };
        let stop = inferior.run()?;
        if let (Some(entry), false) = (temporary, matches!(stop, RunStop::Exited(_))) {
            inferior.remove_breakpoint(entry)?;
        }
        self.settle(inferior, stop, entry, temporary)
    }

    /// Sort what the program stopped for: the user's breakpoints are a stop;
    /// the debugger's own — the entry point, the error park — are not, though
    /// a watched raise at the park runs on to the raiser; a watched write is a
    /// stop only when it changed something.
    #[allow(clippy::type_complexity)]
    fn settle(
        &mut self,
        inferior: &mut dyn Inferior,
        stop: RunStop,
        entry: Option<u64>,
        temporary: Option<u64>,
    ) -> Result<(Option<RunStop>, Vec<(usize, u64)>), String> {
        match stop {
            RunStop::Breakpoints(hits) => {
                if hits.iter().any(|(_, address)| Some(*address) == entry) {
                    self.entry_passed = true;
                }
                let park = self.own_park();
                let mut raised = None;
                let mut user = Vec::new();
                let mut own = Vec::new();
                for &(thread, address) in &hits {
                    if Some(address) == temporary {
                        continue;
                    }
                    if Some(address) == park {
                        if raised.is_none() {
                            raised = self.watched_raise(inferior, thread)?;
                        }
                        continue;
                    }
                    if self
                        .list
                        .iter()
                        .any(|breakpoint| breakpoint.address == Some(address))
                    {
                        user.push((thread, address));
                    } else {
                        own.push((thread, address));
                    }
                }
                if !user.is_empty() {
                    return Ok((Some(RunStop::Breakpoints(user)), own));
                }
                if !own.is_empty() {
                    return Ok((None, own));
                }
                match raised {
                    Some(raise) => {
                        let stop = self.run_to_raiser(inferior, raise)?;
                        self.settle(inferior, stop, entry, None)
                    }
                    None => Ok((None, Vec::new())),
                }
            }
            RunStop::Watchpoints(threads) => {
                Ok((self.watched_writes(inferior, &threads)?, Vec::new()))
            }
            other => Ok((Some(other), Vec::new())),
        }
    }

    /// Step each of `threads`, stopped before a watched write, over the write:
    /// a `Changed` stop for every watchpoint whose bytes it changed, or `None`
    /// when it changed none. A step that ends otherwise is that stop. A thread
    /// the step leaves on one of the user's breakpoints stops there next.
    fn watched_writes(
        &mut self,
        inferior: &mut dyn Inferior,
        threads: &[usize],
    ) -> Result<Option<RunStop>, String> {
        let mut changes = Vec::new();
        for &thread in threads {
            let Some(pc) = inferior.threads()?.get(thread).map(|facts| facts.pc) else {
                continue;
            };
            let before = self.watched_values(inferior);
            let step = inferior
                .step(&[thread], 1)?
                .pop()
                .ok_or("the step answered for no thread")?;
            match step.stop {
                StepStop::Finished | StepStop::Waiting => {}
                StepStop::Exception(what) => return Ok(Some(RunStop::Exception { thread, what })),
                // A program that has ended answers how at once.
                StepStop::Exited => return Ok(Some(inferior.run()?)),
            }
            let after = self.watched_values(inferior);
            for ((watch, old), new) in self.watches.iter().zip(before).zip(after) {
                if let (Some(old), Some(new)) = (old, new) {
                    if old != new {
                        changes.push(Change {
                            thread,
                            pc,
                            address: watch.address,
                            size: watch.size,
                            old,
                            new,
                        });
                    }
                }
            }
            if let Some(now) = inferior.threads()?.get(thread).map(|facts| facts.pc) {
                if self
                    .list
                    .iter()
                    .any(|breakpoint| breakpoint.address == Some(now))
                {
                    self.landed.push((thread, now));
                }
            }
        }
        Ok((!changes.is_empty()).then_some(RunStop::Changed(changes)))
    }

    /// Every watchpoint's bytes now, `None` where they cannot be read.
    fn watched_values(&self, inferior: &mut dyn Inferior) -> Vec<Option<u64>> {
        self.watches
            .iter()
            .map(|watch| watched_value(inferior, watch.address, watch.size).ok())
            .collect()
    }

    /// The park's address when the debugger's own breakpoint is there — a
    /// `breakerror` watch, and no breakpoint of the user's at the same place.
    fn own_park(&self) -> Option<u64> {
        let park = self.errors.as_ref()?.park;
        (!self
            .list
            .iter()
            .any(|breakpoint| breakpoint.address == Some(park)))
        .then_some(park)
    }

    /// Thread `thread` has just entered the park: the error it raises when the
    /// watch takes its code (from `x1`), with where it returns (`lr`) and the
    /// error's origin (the `ErrorLoc` in `x3`).
    fn watched_raise(
        &self,
        inferior: &mut dyn Inferior,
        thread: usize,
    ) -> Result<Option<Raise>, String> {
        let registers = inferior.registers(thread)?;
        let value = |name: &str| {
            registers
                .iter()
                .find(|register| register.name == name)
                .and_then(|register| match register.value {
                    super::RegisterValue::Bits64(value) => Some(value),
                    super::RegisterValue::Bits128(_) => None,
                })
                .ok_or_else(|| format!("thread {thread} has no register {name}"))
        };
        let code = value("x1")? as i64;
        let watch = self.errors.as_ref().expect("a watch holds the park");
        if !watch.codes.matches(code) {
            return Ok(None);
        }
        let origin = match value("x3")? {
            0 => None,
            location => super::runtime::error_origin(inferior, location).ok(),
        };
        Ok(Some(Raise {
            thread,
            code,
            origin,
            back: value("lr")?,
        }))
    }

    /// Run until the thread that raised `raise` is back in the function that
    /// raised it, the error parked: the `Raised` stop. Another stop first — a
    /// breakpoint, an exception, the end — is answered instead; a raise by
    /// another thread meanwhile is let go.
    fn run_to_raiser(
        &mut self,
        inferior: &mut dyn Inferior,
        raise: Raise,
    ) -> Result<RunStop, String> {
        let ours = !self
            .list
            .iter()
            .any(|breakpoint| breakpoint.address == Some(raise.back));
        if ours {
            inferior.insert_breakpoint(raise.back)?;
        }
        let park = self.own_park();
        let stop = loop {
            let stop = match inferior.run()? {
                RunStop::Watchpoints(threads) => match self.watched_writes(inferior, &threads)? {
                    Some(stop) => stop,
                    None => continue,
                },
                stop => stop,
            };
            let RunStop::Breakpoints(hits) = &stop else {
                break stop;
            };
            if hits.contains(&(raise.thread, raise.back)) {
                break RunStop::Raised {
                    thread: raise.thread,
                    code: raise.code,
                    origin: raise.origin.clone(),
                };
            }
            let user = hits
                .iter()
                .copied()
                .filter(|(_, address)| Some(*address) != park && !(ours && *address == raise.back))
                .collect::<Vec<_>>();
            if !user.is_empty() {
                break RunStop::Breakpoints(user);
            }
        };
        if ours && !matches!(stop, RunStop::Exited(_)) {
            inferior.remove_breakpoint(raise.back)?;
        }
        Ok(stop)
    }

    /// `breakerror <codes> [<commands>]`: stop when a thread raises one of
    /// them, running `commands` when there are any (else keeping the ones the
    /// watch has). The watch keeps a breakpoint of the debugger's own on the
    /// runtime's error park, which only a `--debug` build names.
    pub(crate) fn watch_errors(
        &mut self,
        inferior: &mut dyn Inferior,
        codes: ErrorCodes,
        commands: &[String],
    ) -> Result<Vec<String>, String> {
        let codes = match (self.errors.as_ref().map(|watch| &watch.codes), codes) {
            (Some(ErrorCodes::Any), _) | (_, ErrorCodes::Any) => ErrorCodes::Any,
            (Some(ErrorCodes::Codes(old)), ErrorCodes::Codes(new)) => {
                let mut all = old.clone();
                all.extend(new.into_iter().filter(|code| !old.contains(code)));
                ErrorCodes::Codes(all)
            }
            (None, codes) => codes,
        };
        if let Some(watch) = &mut self.errors {
            watch.codes = codes;
            if !commands.is_empty() {
                watch.commands = commands.to_vec();
            }
        } else {
            let park = park_address(inferior)?;
            if !self
                .list
                .iter()
                .any(|breakpoint| breakpoint.address == Some(park))
            {
                inferior.insert_breakpoint(park)?;
            }
            self.errors = Some(ErrorWatch {
                codes,
                park,
                commands: commands.to_vec(),
            });
        }
        Ok(vec![self.watch_line().expect("a watch was just set")])
    }

    /// `breakerror clear`: stop watching for errors.
    pub(crate) fn clear_errors(
        &mut self,
        inferior: &mut dyn Inferior,
    ) -> Result<Vec<String>, String> {
        if self.own_park().is_some() {
            let park = self.errors.as_ref().expect("a watch").park;
            inferior.remove_breakpoint(park)?;
        }
        match self.errors.take() {
            Some(_) => Ok(vec!["Stopped watching for errors.".to_string()]),
            None => Err("no errors are being watched for".to_string()),
        }
    }

    /// `Stopping when a thread raises <codes>.`, for a watch, and what it
    /// runs when it has commands.
    pub(crate) fn watch_line(&self) -> Option<String> {
        let watch = self.errors.as_ref()?;
        Some(format!(
            "Stopping when a thread raises {}.{}",
            watch.codes.shown(),
            runs(&watch.commands)
        ))
    }

    /// After a restart: place every breakpoint again by its function and
    /// offset in the new process (a linked function's is pending until its
    /// library loads); one no function held is dropped, and so is every
    /// watchpoint.
    pub(crate) fn restarted(&mut self, inferior: &mut dyn Inferior) -> Result<Vec<String>, String> {
        self.entry_passed = false;
        self.landed.clear();
        self.notes.clear();
        let mut notes = std::mem::take(&mut self.watches)
            .into_iter()
            .map(|watch| {
                format!(
                    "Dropped the watchpoint at 0x{:016x}: a restarted program's memory is laid \
                     out anew.",
                    watch.address
                )
            })
            .collect::<Vec<_>>();
        if let Some(watch) = &mut self.errors {
            watch.park = park_address(inferior)?;
            inferior.insert_breakpoint(watch.park)?;
        }
        let functions = inferior.functions().unwrap_or_default();
        let mut kept = Vec::new();
        for mut breakpoint in std::mem::take(&mut self.list) {
            let Some((symbol, offset)) = breakpoint.anchor.clone() else {
                notes.push(format!(
                    "Dropped the breakpoint at {}: no function holds it.",
                    place(&breakpoint)
                ));
                continue;
            };
            let function = functions.iter().find(|function| function.name == symbol);
            breakpoint.address = match function {
                Some(function) if !function.linked => function.address.map(|start| start + offset),
                Some(_) => None,
                None => {
                    notes.push(format!(
                        "Dropped the breakpoint on {symbol}: the program no longer has it."
                    ));
                    continue;
                }
            };
            if let Some(address) = breakpoint.address {
                inferior.insert_breakpoint(address)?;
            }
            kept.push(breakpoint);
        }
        self.list = kept;
        Ok(notes)
    }

    /// Run `thread` until the function it is in returns: breakpoints on every
    /// `ret` of the function, a stop there by `thread` with its frame popped
    /// (stack pointer at or above where it is now, so a recursive call's return
    /// does not count), then one step over the `ret`. Every stop on the way is
    /// handed to `hear`, as `run` does; one it does not run on from ends the
    /// finish early.
    pub(crate) fn finish(
        &mut self,
        inferior: &mut dyn Inferior,
        thread: usize,
        hear: &mut Hear,
    ) -> Result<Finish, String> {
        let threads = inferior.threads()?;
        let facts = threads
            .get(thread)
            .ok_or_else(|| format!("no thread {thread}"))?
            .clone();
        let range = inferior
            .function_at(facts.pc)?
            .ok_or_else(|| format!("cannot finish: no known function holds 0x{:016x}", facts.pc))?;
        let ends = returns(inferior, &range)?;
        if ends.is_empty() {
            return Err(format!(
                "cannot finish: no ret instruction found in the function at 0x{:016x}",
                range.start
            ));
        }
        let ours = ends
            .iter()
            .copied()
            .filter(|address| {
                !self
                    .list
                    .iter()
                    .any(|other| other.address == Some(*address))
            })
            .collect::<Vec<_>>();
        for address in &ours {
            inferior.insert_breakpoint(*address)?;
        }
        let outcome = loop {
            let hits = match self.run(inferior, hear)? {
                Ended::Heard(stop) => break Some(stop),
                Ended::Own(hits) => hits,
            };
            let popped = hits.iter().any(|(hit, _)| {
                *hit == thread
                    && inferior
                        .threads()
                        .ok()
                        .and_then(|threads| threads.get(thread).map(|now| now.sp >= facts.sp))
                        .unwrap_or(false)
            });
            if popped {
                break None;
            }
        };
        if !matches!(outcome, Some(RunStop::Exited(_))) {
            for address in &ours {
                inferior.remove_breakpoint(*address)?;
            }
        }
        if let Some(stop) = outcome {
            return Ok(Finish::Stopped {
                stop,
                from: range.name.clone(),
            });
        }
        let step = inferior
            .step(&[thread], 1)?
            .pop()
            .ok_or("the step answered for no thread")?;
        let stop = match step.stop {
            StepStop::Finished => {
                return Ok(Finish::Returned {
                    from: range.name.clone(),
                })
            }
            StepStop::Exception(what) => RunStop::Exception { thread, what },
            StepStop::Waiting => RunStop::Interrupted,
            StepStop::Exited => RunStop::Exited(super::Exit::Unknown),
        };
        let notes = self.take_notes();
        hear(inferior, self, notes, &stop)?;
        Ok(Finish::Stopped {
            stop,
            from: range.name.clone(),
        })
    }

    /// Place every pending breakpoint whose function is now bound, with a
    /// note for each.
    fn place_pending(&mut self, inferior: &mut dyn Inferior) -> Result<(), String> {
        if self
            .list
            .iter()
            .all(|breakpoint| breakpoint.address.is_some())
        {
            return Ok(());
        }
        let functions = inferior.functions()?;
        let mut index = 0;
        while index < self.list.len() {
            let pending = &self.list[index];
            let bound = (pending.address.is_none())
                .then(|| {
                    functions
                        .iter()
                        .find(|function| pending.symbol.as_ref() == Some(&function.name))
                        .and_then(|function| function.address)
                })
                .flatten();
            let Some(address) = bound else {
                index += 1;
                continue;
            };
            if self.list.iter().any(|other| other.address == Some(address)) {
                // Another breakpoint is already there; this one adds nothing.
                self.notes.push(format!(
                    "The pending breakpoint on {} is at 0x{address:016x}, which already has one.",
                    shown(&pending.symbol)
                ));
                self.list.remove(index);
                continue;
            }
            inferior.insert_breakpoint(address)?;
            self.list[index].address = Some(address);
            self.list[index].anchor = self.list[index].symbol.clone().map(|symbol| (symbol, 0));
            self.notes.push(format!(
                "Placed the pending breakpoint on {} at 0x{address:016x}.",
                shown(&self.list[index].symbol)
            ));
            index += 1;
        }
        Ok(())
    }
}

/// How a `finish` ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Finish {
    /// The thread returned from the function named here.
    Returned { from: Option<String> },
    /// Something else stopped the program before the function named here
    /// returned; the stop has been heard.
    Stopped { stop: RunStop, from: Option<String> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::{Ending, Exit, FunctionRange, Register, Step, ThreadFacts};

    /// A program whose `_exit` binds when it reaches its entry point, and
    /// which then stops at whatever breakpoints `stops` lists, in turn.
    struct Program {
        bound: bool,
        inserted: Vec<u64>,
        stops: Vec<RunStop>,
        runs: usize,
        /// The code in `x1` at each entry to the park, in turn.
        raised: Vec<u64>,
        /// The program's memory from [`MEMORY`].
        memory: Vec<u8>,
        /// What each step writes into memory, in turn: (address, byte).
        writes: Vec<Vec<(u64, u8)>>,
        /// Each watchpoint inserted: address, size.
        watched: Vec<(u64, u64)>,
        /// Thread 0's pc.
        pc: u64,
    }

    /// Where [`Program`]'s memory starts.
    const MEMORY: u64 = 0x8000;

    const PARK: u64 = 0x2000;
    /// Where the park returns to, in the function that raised the error.
    const RAISER: u64 = 0x1124;

    const ENTRY: u64 = 0x1000;
    /// A store in `isPrime`.
    const STORE: u64 = 0x1110;
    const IS_PRIME: u64 = 0x1100;
    const EXIT: u64 = 0x9000;

    fn function(
        name: &str,
        linked: bool,
        address: Option<u64>,
        size: Option<u64>,
    ) -> FunctionSymbol {
        FunctionSymbol {
            name: name.to_string(),
            linked,
            loaded: false,
            address,
            size,
        }
    }

    impl Program {
        fn new(stops: Vec<RunStop>) -> Self {
            Program {
                bound: false,
                inserted: Vec::new(),
                stops,
                runs: 0,
                raised: Vec::new(),
                memory: vec![0; 64],
                writes: Vec::new(),
                watched: Vec::new(),
                pc: STORE,
            }
        }
    }

    impl Inferior for Program {
        fn pid(&self) -> u32 {
            1
        }
        fn end(&mut self) -> Result<Ending, String> {
            Ok(Ending::Killed)
        }
        fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
            let exit = self.bound.then_some(EXIT);
            Ok(vec![
                function("_main", false, Some(ENTRY), Some(0x100)),
                function("_mfb_fn_isPrime", false, Some(IS_PRIME), Some(0x40)),
                function("_mfb_fn_isPrime$own1", false, Some(0x1200), Some(0x40)),
                function("_mfb_fn_exit", false, Some(0x1300), Some(0x10)),
                function("_exit", true, exit, None),
                function("_exit", true, exit, None),
                function(PARK_ERROR, false, Some(PARK), Some(0x100)),
            ])
        }
        fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
            Ok(None)
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
            let at = address
                .checked_sub(MEMORY)
                .map(|at| at as usize)
                .filter(|at| at + length <= self.memory.len())
                .ok_or_else(|| format!("unmapped {address:#x}"))?;
            Ok(self.memory[at..at + length].to_vec())
        }
        fn read_u64(&mut self, address: u64) -> Result<u64, String> {
            Err(format!("unmapped {address:#x}"))
        }
        fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
            Ok(vec![ThreadFacts {
                id: 1,
                handle: 1,
                name: None,
                pc: self.pc,
                sp: 0,
                arena: 0,
                stack: None,
            }])
        }
        fn registers(&mut self, _: usize) -> Result<Vec<Register>, String> {
            let register = |name: &str, value| Register {
                name: name.to_string(),
                group: crate::debugger::RegisterGroup::General,
                value: crate::debugger::RegisterValue::Bits64(value),
            };
            let code = if self.raised.is_empty() {
                0
            } else {
                self.raised.remove(0)
            };
            Ok(vec![
                register("x1", code),
                register("x3", 0),
                register("lr", RAISER),
            ])
        }
        /// Thread 0 runs one instruction: the next of `writes`.
        fn step(&mut self, _: &[usize], _: u64) -> Result<Vec<Step>, String> {
            if !self.writes.is_empty() {
                for (address, byte) in self.writes.remove(0) {
                    self.memory[(address - MEMORY) as usize] = byte;
                }
            }
            self.pc += 4;
            Ok(vec![Step {
                taken: 1,
                stop: StepStop::Finished,
            }])
        }
        fn restart(&mut self) -> Result<u32, String> {
            Err("restart needs a program the debugger launched".to_string())
        }
        fn entry(&mut self) -> Result<Option<u64>, String> {
            Ok(Some(ENTRY))
        }
        fn insert_breakpoint(&mut self, address: u64) -> Result<(), String> {
            if self.inserted.contains(&address) {
                return Err(format!("there is already a breakpoint at {address:#x}"));
            }
            self.inserted.push(address);
            Ok(())
        }
        fn remove_breakpoint(&mut self, address: u64) -> Result<(), String> {
            let before = self.inserted.len();
            self.inserted.retain(|other| *other != address);
            if self.inserted.len() == before {
                return Err(format!("there is no breakpoint at {address:#x}"));
            }
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
        fn run(&mut self) -> Result<RunStop, String> {
            self.runs += 1;
            if !self.bound && self.inserted.contains(&ENTRY) {
                self.bound = true;
                return Ok(RunStop::Breakpoints(vec![(0, ENTRY)]));
            }
            self.bound = true;
            Ok(self.stops.remove(0))
        }
    }

    fn name(word: &str) -> Target {
        Target::parse(word).unwrap()
    }

    /// Run until the first stop the session hears, which ends the run: the
    /// notes heard with it, and the stop; threads at breakpoints neither the
    /// user's nor the debugger's own are a `Breakpoints` stop.
    fn run(
        breakpoints: &mut Breakpoints,
        program: &mut Program,
    ) -> Result<(Vec<String>, RunStop), String> {
        let mut heard = None;
        let ended = breakpoints.run(
            program,
            &mut |_: &mut dyn Inferior, _: &Breakpoints, notes, stop| {
                heard = Some((notes, stop.clone()));
                Ok(false)
            },
        )?;
        Ok(match ended {
            Ended::Heard(_) => heard.expect("a heard stop"),
            Ended::Own(hits) => (breakpoints.take_notes(), RunStop::Breakpoints(hits)),
        })
    }

    #[test]
    fn an_address_or_a_name_is_parsed() {
        assert_eq!(Target::parse("0x1F00"), Ok(Target::Address(0x1f00)));
        assert_eq!(Target::parse("0X10"), Ok(Target::Address(0x10)));
        assert_eq!(
            Target::parse("isPrime"),
            Ok(Target::Name("isPrime".to_string()))
        );
        assert_eq!(
            Target::parse("0xzz"),
            Err("'0xzz' is not an address".to_string())
        );
    }

    #[test]
    fn a_symbol_wins_over_a_written_name_and_every_copy_is_covered() {
        let mut program = Program::new(Vec::new());
        let mut breakpoints = Breakpoints::default();
        // `isPrime` is written; it covers the function and its `$own` copy.
        assert_eq!(
            breakpoints
                .set(&mut program, &name("isPrime"), &[])
                .unwrap(),
            [
                "Set a breakpoint at 0x0000000000001100 in isPrime.",
                "Set a breakpoint at 0x0000000000001200 in isPrime.",
            ]
        );
        // `_mfb_fn_exit` is a symbol: only that one function.
        assert_eq!(
            breakpoints
                .set(&mut program, &name("_mfb_fn_exit"), &[])
                .unwrap(),
            ["Set a breakpoint at 0x0000000000001300 in exit <mfb>."]
        );
        assert_eq!(program.inserted, [0x1100, 0x1200, 0x1300]);
        assert_eq!(
            breakpoints.set(&mut program, &name("isPrime"), &[]),
            Err("there is already a breakpoint on 'isPrime'".to_string())
        );
        assert_eq!(
            breakpoints.set(&mut program, &name("nothing"), &[]),
            Err("no function is named 'nothing'; type 'list func' for the functions".to_string())
        );
    }

    #[test]
    fn a_function_and_an_offset_is_a_place_in_each_function_so_named() {
        let mut program = Program::new(Vec::new());
        let mut breakpoints = Breakpoints::default();
        // `isPrime` is two functions, the original and a hand-over copy.
        assert_eq!(
            breakpoints
                .set(&mut program, &name("isPrime+0x24"), &[])
                .unwrap(),
            [
                "Set a breakpoint at 0x0000000000001124 in isPrime+0x24.",
                "Set a breakpoint at 0x0000000000001224 in isPrime+0x24.",
            ]
        );
        // A symbol, with a decimal offset.
        assert_eq!(
            breakpoints
                .set(&mut program, &name("_mfb_fn_exit+4"), &[])
                .unwrap(),
            ["Set a breakpoint at 0x0000000000001304 in exit+0x4 <mfb>."]
        );
        assert_eq!(
            breakpoints.set(&mut program, &name("_mfb_fn_exit+4"), &[]),
            Err("there is already a breakpoint at _mfb_fn_exit+0x4".to_string())
        );
        assert_eq!(
            breakpoints
                .remove(&mut program, &name("_mfb_fn_isPrime+0x24"))
                .unwrap(),
            ["Removed the breakpoint at 0x0000000000001124 in isPrime+0x24."]
        );
        assert_eq!(program.inserted, [0x1224, 0x1304]);
        // `_exit` is not bound until the program reaches its entry point.
        assert_eq!(
            breakpoints.set(&mut program, &name("_exit+8"), &[]),
            Err(
                "'_exit' is not loaded yet; run the program until it has loaded its libraries"
                    .to_string()
            )
        );
        assert_eq!(
            breakpoints.set(&mut program, &name("nothing+8"), &[]),
            Err("no function is named 'nothing'; type 'list func' for the functions".to_string())
        );
    }

    #[test]
    fn breakerror_stops_in_the_raiser_only_for_a_watched_code() {
        // Two raises: one not watched (run on), then 77050010, stopped back
        // in the function that raised it.
        let mut program = Program::new(vec![
            RunStop::Breakpoints(vec![(0, PARK)]),
            RunStop::Breakpoints(vec![(0, PARK)]),
            RunStop::Breakpoints(vec![(0, RAISER)]),
        ]);
        program.raised = vec![77_050_004, 77_050_010];
        let mut breakpoints = Breakpoints::default();
        assert_eq!(
            breakpoints
                .watch_errors(&mut program, ErrorCodes::Codes(vec![77_050_010]), &[])
                .unwrap(),
            ["Stopping when a thread raises 77050010 (errorCode::ErrOverflow)."]
        );
        assert_eq!(program.inserted, [PARK]);
        let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
        assert_eq!(
            stop,
            RunStop::Raised {
                thread: 0,
                code: 77_050_010,
                origin: None
            }
        );
        // The stop's own breakpoint is gone; the watch's stays.
        assert_eq!(program.inserted, [PARK]);
        // More codes join the watch; `any` takes them all.
        breakpoints
            .watch_errors(&mut program, ErrorCodes::Codes(vec![42, 77_050_010]), &[])
            .unwrap();
        assert_eq!(
            breakpoints.lines(),
            ["Stopping when a thread raises 77050010 (errorCode::ErrOverflow), 42."]
        );
        breakpoints
            .watch_errors(&mut program, ErrorCodes::Any, &[])
            .unwrap();
        assert_eq!(
            breakpoints.lines(),
            ["Stopping when a thread raises any error."]
        );
        assert_eq!(
            breakpoints.clear_errors(&mut program).unwrap(),
            ["Stopped watching for errors."]
        );
        assert!(program.inserted.is_empty());
        assert_eq!(
            breakpoints.clear_errors(&mut program),
            Err("no errors are being watched for".to_string())
        );
    }

    #[test]
    fn a_breakpoint_of_the_user_s_on_the_park_shares_it_with_breakerror() {
        let mut program = Program::new(vec![RunStop::Breakpoints(vec![(0, PARK)])]);
        let mut breakpoints = Breakpoints::default();
        breakpoints
            .watch_errors(&mut program, ErrorCodes::Any, &[])
            .unwrap();
        breakpoints
            .set(&mut program, &name(PARK_ERROR), &[])
            .unwrap();
        assert_eq!(program.inserted, [PARK]);
        // The user's breakpoint is reported as theirs.
        let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
        assert_eq!(stop, RunStop::Breakpoints(vec![(0, PARK)]));
        // Removing it leaves the watch's breakpoint in place.
        breakpoints.remove(&mut program, &name(PARK_ERROR)).unwrap();
        assert_eq!(program.inserted, [PARK]);
        assert_eq!(
            breakpoints.lines(),
            ["Stopping when a thread raises any error."]
        );
    }

    #[test]
    fn an_address_is_described_by_the_function_that_holds_it() {
        let mut program = Program::new(Vec::new());
        let mut breakpoints = Breakpoints::default();
        breakpoints
            .set(&mut program, &Target::Address(0x1124), &[])
            .unwrap();
        breakpoints
            .set(&mut program, &Target::Address(0x5000), &[])
            .unwrap();
        assert_eq!(
            breakpoints.lines(),
            [
                "Address            Symbol               Written name",
                "0x0000000000001124 _mfb_fn_isPrime+0x24 isPrime+0x24",
                "0x0000000000005000 -                    -",
            ]
        );
        assert_eq!(
            breakpoints.set(&mut program, &Target::Address(0x1124), &[]),
            Err("there is already a breakpoint at 0x0000000000001124".to_string())
        );
    }

    #[test]
    fn an_unbound_linked_function_is_pending_until_the_entry_point() {
        let mut program = Program::new(vec![RunStop::Breakpoints(vec![(0, EXIT)])]);
        let mut breakpoints = Breakpoints::default();
        // `exit` is no symbol; as written it names both the developer's
        // function and the linked one, which is not bound yet.
        assert_eq!(
            breakpoints.set(&mut program, &name("exit"), &[]).unwrap(),
            [
                "Set a breakpoint at 0x0000000000001300 in exit <mfb>.",
                "Set a pending breakpoint on _exit: it is placed when the program has loaded \
                 its libraries.",
            ]
        );
        assert_eq!(
            breakpoints.lines(),
            [
                "Address            Symbol       Written name",
                "0x0000000000001300 _mfb_fn_exit exit <mfb>",
                "-                  _exit        exit <native>",
            ]
        );
        let (notes, stop) = run(&mut breakpoints, &mut program).unwrap();
        // The entry-point stop was the debugger's own: placed, and ran on.
        assert_eq!(program.runs, 2);
        assert_eq!(
            notes,
            ["Placed the pending breakpoint on _exit at 0x0000000000009000."]
        );
        assert_eq!(stop, RunStop::Breakpoints(vec![(0, EXIT)]));
        assert_eq!(program.inserted, [0x1300, EXIT]);
    }

    #[test]
    fn removing_by_name_takes_symbols_first_then_written_names() {
        let mut program = Program::new(Vec::new());
        let mut breakpoints = Breakpoints::default();
        breakpoints
            .set(&mut program, &name("isPrime"), &[])
            .unwrap();
        breakpoints.set(&mut program, &name("_exit"), &[]).unwrap();
        assert_eq!(
            breakpoints
                .remove(&mut program, &name("_mfb_fn_isPrime"))
                .unwrap(),
            ["Removed the breakpoint at 0x0000000000001100 in isPrime."]
        );
        assert_eq!(
            breakpoints.remove(&mut program, &name("isPrime")).unwrap(),
            ["Removed the breakpoint at 0x0000000000001200 in isPrime."]
        );
        assert_eq!(
            breakpoints.remove(&mut program, &name("exit")).unwrap(),
            ["Removed the breakpoint at _exit."]
        );
        assert_eq!(breakpoints.lines(), ["no breakpoints"]);
        assert!(program.inserted.is_empty());
        assert_eq!(
            breakpoints.remove(&mut program, &Target::Address(0x1100)),
            Err("there is no breakpoint at 0x0000000000001100".to_string())
        );
        assert_eq!(
            breakpoints.remove(&mut program, &name("isPrime")),
            Err("there is no breakpoint on 'isPrime'".to_string())
        );
    }

    #[test]
    fn a_user_breakpoint_at_the_entry_point_is_reported() {
        let mut program = Program::new(Vec::new());
        let mut breakpoints = Breakpoints::default();
        breakpoints.set(&mut program, &name("_main"), &[]).unwrap();
        breakpoints.set(&mut program, &name("_exit"), &[]).unwrap();
        let (notes, stop) = run(&mut breakpoints, &mut program).unwrap();
        assert_eq!(stop, RunStop::Breakpoints(vec![(0, ENTRY)]));
        assert_eq!(
            notes,
            ["Placed the pending breakpoint on _exit at 0x0000000000009000."]
        );
        assert_eq!(program.runs, 1);
    }

    #[test]
    fn an_exit_leaves_no_breakpoint_to_remove() {
        let mut program = Program::new(vec![RunStop::Exited(Exit::Status(3))]);
        program.bound = true;
        let mut breakpoints = Breakpoints::default();
        let (notes, stop) = run(&mut breakpoints, &mut program).unwrap();
        assert!(notes.is_empty());
        assert_eq!(stop, RunStop::Exited(Exit::Status(3)));
    }

    #[test]
    fn a_watchpoint_stops_only_when_a_write_changes_its_bytes() {
        // Three writes into the watched word: the same value (unseen), a new
        // one (a stop), and one beside it (unseen).
        let mut program = Program::new(vec![
            RunStop::Watchpoints(vec![0]),
            RunStop::Watchpoints(vec![0]),
            RunStop::Watchpoints(vec![0]),
            RunStop::Exited(Exit::Status(0)),
        ]);
        program.bound = true;
        program.memory[8] = 5;
        program.writes = vec![
            vec![(MEMORY + 8, 5)],
            vec![(MEMORY + 8, 6)],
            vec![(MEMORY + 16, 9)],
        ];
        let mut breakpoints = Breakpoints::default();
        assert_eq!(
            breakpoints.watch(&mut program, MEMORY + 8, 8, &[]).unwrap(),
            ["Set a watchpoint at 0x0000000000008008 (8 bytes): 0x0000000000000005 (5)."]
        );
        assert_eq!(program.watched, [(MEMORY + 8, 8)]);
        let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
        assert_eq!(
            stop,
            RunStop::Changed(vec![Change {
                thread: 0,
                pc: STORE + 4,
                address: MEMORY + 8,
                size: 8,
                old: 5,
                new: 6,
            }])
        );
        assert_eq!(program.runs, 2);
        let (_, stop) = run(&mut breakpoints, &mut program).unwrap();
        assert_eq!(stop, RunStop::Exited(Exit::Status(0)));
        assert_eq!(
            breakpoints.watch_lines(&mut program),
            [
                "Address            Size Value",
                "0x0000000000008008 8    0x0000000000000006 (6)",
            ]
        );
    }

    #[test]
    fn a_watchpoint_covers_1_to_8_bytes_within_a_multiple_of_8() {
        let mut program = Program::new(Vec::new());
        let mut breakpoints = Breakpoints::default();
        for (address, size) in [(MEMORY, 0), (MEMORY, 9), (MEMORY + 4, 8), (MEMORY + 7, 2)] {
            assert_eq!(
                breakpoints.watch(&mut program, address, size, &[]),
                Err(format!(
                    "cannot watch {size} bytes at 0x{address:016x}: a watchpoint covers 1 to 8 \
                     bytes that do not cross an address that is a multiple of 8"
                ))
            );
        }
        assert_eq!(
            breakpoints.watch(&mut program, MEMORY + 6, 2, &[]).unwrap(),
            ["Set a watchpoint at 0x0000000000008006 (2 bytes): 0x0000."]
        );
        assert_eq!(
            breakpoints.watch(&mut program, MEMORY + 6, 1, &[]),
            Err("there is already a watchpoint at 0x0000000000008006".to_string())
        );
        assert_eq!(
            breakpoints.watch(&mut program, 0x10, 1, &[]),
            Err("unmapped 0x10".to_string())
        );
        assert_eq!(
            breakpoints.unwatch(&mut program, MEMORY + 6).unwrap(),
            ["Removed the watchpoint at 0x0000000000008006 (2 bytes)."]
        );
        assert!(program.watched.is_empty());
        assert_eq!(
            breakpoints.unwatch(&mut program, MEMORY + 6),
            Err("there is no watchpoint at 0x0000000000008006".to_string())
        );
        assert_eq!(breakpoints.watch_lines(&mut program), ["no watchpoints"]);
    }

    #[test]
    fn a_restart_drops_every_watchpoint() {
        let mut program = Program::new(Vec::new());
        let mut breakpoints = Breakpoints::default();
        breakpoints
            .watch(&mut program, MEMORY, 4, &["bt".to_string()])
            .unwrap();
        assert_eq!(
            breakpoints.restarted(&mut program).unwrap(),
            [
                "Dropped the watchpoint at 0x0000000000008000: a restarted program's memory is \
                 laid out anew."
            ]
        );
        assert_eq!(breakpoints.watch_lines(&mut program), ["no watchpoints"]);
    }

    #[test]
    fn a_stop_with_commands_runs_on_and_one_without_ends_the_run() {
        let mut program = Program::new(vec![
            RunStop::Breakpoints(vec![(0, IS_PRIME)]),
            RunStop::Breakpoints(vec![(0, IS_PRIME), (1, 0x1300)]),
            RunStop::Exited(Exit::Status(0)),
        ]);
        program.bound = true;
        let mut breakpoints = Breakpoints::default();
        let commands = ["bt 1".to_string(), "list reg x0".to_string()];
        assert_eq!(
            breakpoints
                .set(&mut program, &name("_mfb_fn_isPrime"), &commands)
                .unwrap(),
            [
                "Set a breakpoint at 0x0000000000001100 in isPrime. Each stop runs 'bt 1; list \
                 reg x0' and continues."
            ]
        );
        breakpoints
            .set(&mut program, &name("_mfb_fn_exit"), &[])
            .unwrap();
        assert_eq!(
            breakpoints.lines(),
            [
                "Address            Symbol          Written name Commands",
                "0x0000000000001100 _mfb_fn_isPrime isPrime      bt 1; list reg x0",
                "0x0000000000001300 _mfb_fn_exit    exit <mfb>   -",
            ]
        );
        // Each stop is heard; the first, whose every breakpoint has
        // commands, runs on; the second has one without and ends the run.
        let mut heard = Vec::new();
        let ended = breakpoints
            .run(
                &mut program,
                &mut |_: &mut dyn Inferior, breakpoints: &Breakpoints, _, stop: &RunStop| {
                    let (commands, every) = breakpoints.stop_commands(stop);
                    heard.push((stop.clone(), commands));
                    Ok(every)
                },
            )
            .unwrap();
        assert_eq!(
            ended,
            Ended::Heard(RunStop::Breakpoints(vec![(0, IS_PRIME), (1, 0x1300)]))
        );
        assert_eq!(
            heard,
            [
                (
                    RunStop::Breakpoints(vec![(0, IS_PRIME)]),
                    vec![(0, commands.to_vec())]
                ),
                (
                    RunStop::Breakpoints(vec![(0, IS_PRIME), (1, 0x1300)]),
                    vec![(0, commands.to_vec())]
                ),
            ]
        );
    }

    #[test]
    fn a_watched_error_with_commands_runs_them_and_keeps_them() {
        let mut program = Program::new(Vec::new());
        let mut breakpoints = Breakpoints::default();
        let commands = ["show error".to_string()];
        assert_eq!(
            breakpoints
                .watch_errors(&mut program, ErrorCodes::Any, &commands)
                .unwrap(),
            ["Stopping when a thread raises any error. Each stop runs 'show error' and continues."]
        );
        // More codes without commands keep the watch's commands.
        breakpoints
            .watch_errors(&mut program, ErrorCodes::Codes(vec![42]), &[])
            .unwrap();
        let raised = RunStop::Raised {
            thread: 0,
            code: 42,
            origin: None,
        };
        assert_eq!(
            breakpoints.stop_commands(&raised),
            (vec![(0, commands.to_vec())], true)
        );
        assert_eq!(
            breakpoints.stop_commands(&RunStop::Interrupted),
            (Vec::new(), false)
        );
    }
}
