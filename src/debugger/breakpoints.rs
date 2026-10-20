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
//! A breakpoint may carry a test (`breakpointif`): a thread that reaches it
//! stops only when the test holds for that thread, and otherwise runs on
//! unseen. Several breakpoints may share an address, each with its own test;
//! one instruction there serves them all.
//!
//! A breakpoint, a watchpoint and the `breakerror` watch may carry commands:
//! a stop at one runs them, and the program stays stopped unless they end in
//! `run`. `run` hands each stop to the session as it happens. Each breakpoint
//! and watchpoint counts the stops it made.
//!
//! The session also keeps the debug report a `--debug` build writes as it
//! exits ([`report`]), with breakpoints of its own that no stop shows.

mod report;

pub(crate) use report::REPORT_HINT;

use super::disassemble::returns;
use super::location::Condition;
use super::names::{ambiguity, classify, Collisions};
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
    /// The commands a stop here runs.
    pub(crate) commands: Vec<String>,
    /// The test a thread must pass to stop here; `None` stops every one.
    pub(crate) condition: Option<Condition>,
    /// The session's number for it, unique among its breakpoints.
    pub(crate) id: u64,
    /// The stops it has made since the program last started.
    pub(crate) hits: u64,
}

/// One watchpoint the session holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Watch {
    pub(crate) address: u64,
    pub(crate) size: u64,
    /// The commands a change here runs.
    pub(crate) commands: Vec<String>,
    /// The stops it has made.
    pub(crate) hits: u64,
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
        format!(" Each stop runs '{}'.", commands_shown(commands))
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
    let found = find(functions, name)?;
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
    /// The id the next breakpoint gets.
    next_id: u64,
    watches: Vec<Watch>,
    /// The program has been seen at its entry point: every library is bound.
    entry_passed: bool,
    /// The errors `breakerror` stops on, if any.
    errors: Option<ErrorWatch>,
    /// Notes made since the session last heard a stop.
    notes: Vec<String>,
    /// Threads a step over a watched write left on a breakpoint the session
    /// holds: the next `run` stops there before it runs anything.
    landed: Vec<(usize, u64)>,
    /// The `ret`s a `finish` is waiting on.
    finishing: Vec<u64>,
    /// The breakpoints each hit of the last stop stopped for, by id.
    matched: Vec<((usize, u64), Vec<u64>)>,
    /// The program's debug report, and the breakpoints that keep it.
    report: report::Report,
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

/// The hits of `hits` that stopped for none of the user's breakpoints.
fn unseen(hits: &[(usize, u64)], user: &[(usize, u64)]) -> Vec<(usize, u64)> {
    hits.iter()
        .copied()
        .filter(|hit| !user.contains(hit))
        .collect()
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

/// The breakpoints a name asks for: symbols spelled `name`, or else the one
/// function written `name` (an error when several are); one per distinct
/// address, and one per symbol not yet bound.
fn find(functions: &[FunctionSymbol], name: &str) -> Result<Vec<Breakpoint>, String> {
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
        if let Some(error) = ambiguity(name, matches.iter().map(|f| f.name.as_str())) {
            return Err(error);
        }
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
                condition: None,
                id: 0,
                hits: 0,
            });
        }
    }
    Ok(found)
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
            condition: None,
            id: 0,
            hits: 0,
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
            condition: None,
            id: 0,
            hits: 0,
        },
        None => Breakpoint {
            address: Some(address),
            symbol: None,
            written: None,
            tag: None,
            anchor: None,
            commands: Vec::new(),
            condition: None,
            id: 0,
            hits: 0,
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

/// A breakpoint's test as a sentence ends with it: ` if <test>`, or nothing.
fn tested(breakpoint: &Breakpoint) -> String {
    if_shown(breakpoint.condition.as_ref())
}

/// ` if <test>` for a test, nothing for none.
fn if_shown(condition: Option<&Condition>) -> String {
    condition.map_or_else(String::new, |condition| {
        format!(" if {}", condition.shown())
    })
}

/// How a breakpoint reads in a sentence, with its test: `0x… in isPrime if x0
/// eq 0x… (5)`.
fn described(breakpoint: &Breakpoint) -> String {
    format!("{}{}", place(breakpoint), tested(breakpoint))
}

/// Whether two breakpoints are at the same place: the same address, or, both
/// pending, on the same symbol.
fn same_place(one: &Breakpoint, other: &Breakpoint) -> bool {
    match one.address {
        Some(_) => one.address == other.address,
        None => other.address.is_none() && one.symbol == other.symbol,
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
    /// Set breakpoints at `target`, each stopping a thread only when
    /// `condition` holds for it (every thread, without one) and running
    /// `commands` at a stop.
    pub(crate) fn set(
        &mut self,
        inferior: &mut dyn Inferior,
        target: &Target,
        condition: Option<&Condition>,
        commands: &[String],
    ) -> Result<Vec<String>, String> {
        let functions = inferior.functions().unwrap_or_default();
        let wanted = match target {
            Target::Address(address) => vec![describe(&functions, *address)],
            Target::Name(name) => {
                let found = find(&functions, name)?;
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
            breakpoint.condition = condition.cloned();
            let exists = self.list.iter().any(|other| {
                same_place(other, &breakpoint) && other.condition == breakpoint.condition
            });
            if exists {
                continue;
            }
            match breakpoint.address {
                Some(address) => {
                    // One instruction serves every breakpoint at an address,
                    // the park's and a `finish`'s included.
                    if !self.armed(address) {
                        inferior.insert_breakpoint(address)?;
                    }
                    lines.push(format!(
                        "Set a breakpoint at {}.{}",
                        described(&breakpoint),
                        runs(commands)
                    ));
                }
                None => lines.push(format!(
                    "Set a pending breakpoint on {}{}: it is placed when the program has \
                     loaded its libraries.{}",
                    shown(&breakpoint.symbol),
                    tested(&breakpoint),
                    runs(commands)
                )),
            }
            breakpoint.id = self.next_id;
            self.next_id += 1;
            self.list.push(breakpoint);
        }
        if lines.is_empty() {
            let test = if_shown(condition);
            return Err(match target {
                Target::Name(name) => format!("there is already a breakpoint on '{name}'{test}"),
                other => format!("there is already a breakpoint at {}{test}", other.shown()),
            });
        }
        Ok(lines)
    }

    /// Whether a breakpoint the session holds is at `address`: one of the
    /// user's, `breakerror`'s on the park, one on a `ret` a `finish` waits
    /// on, or one that keeps the debug report. One instruction there serves
    /// them all.
    fn armed(&self, address: u64) -> bool {
        self.user_at(address)
            || self.is_park(address)
            || self.finishing.contains(&address)
            || self.report.holds(address)
    }

    /// Whether one of the user's breakpoints is at `address`.
    fn user_at(&self, address: u64) -> bool {
        self.list
            .iter()
            .any(|breakpoint| breakpoint.address == Some(address))
    }

    /// Whether `address` is the park `breakerror` keeps a breakpoint on.
    fn is_park(&self, address: u64) -> bool {
        self.errors
            .as_ref()
            .is_some_and(|watch| watch.park == address)
    }

    /// Take away the breakpoints at `target`: every one there, whatever its
    /// test, when `only` is `None` (`breakpoint r`); else just those with the
    /// test `only` (`breakpointif r`).
    pub(crate) fn remove(
        &mut self,
        inferior: &mut dyn Inferior,
        target: &Target,
        only: Option<&Condition>,
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
                    let written = self
                        .list
                        .iter()
                        .filter(|breakpoint| breakpoint.written.as_ref() == Some(name))
                        .cloned()
                        .collect::<Vec<_>>();
                    let symbols = written
                        .iter()
                        .filter_map(|breakpoint| breakpoint.symbol.as_deref());
                    if let Some(error) = ambiguity(name, symbols) {
                        return Err(error);
                    }
                    written
                } else {
                    by_symbol
                }
            }
        };
        let chosen = chosen
            .into_iter()
            .filter(|breakpoint| {
                only.is_none_or(|condition| breakpoint.condition.as_ref() == Some(condition))
            })
            .collect::<Vec<_>>();
        if chosen.is_empty() {
            let test = if_shown(only);
            return Err(match target {
                Target::Name(name) => format!("there is no breakpoint on '{name}'{test}"),
                other => format!("there is no breakpoint at {}{test}", other.shown()),
            });
        }
        let mut lines = Vec::new();
        for breakpoint in &chosen {
            self.list.retain(|other| other.id != breakpoint.id);
            lines.push(format!(
                "Removed the breakpoint at {}.",
                described(breakpoint)
            ));
        }
        let mut addresses = chosen
            .iter()
            .filter_map(|breakpoint| breakpoint.address)
            .collect::<Vec<_>>();
        addresses.sort_unstable();
        addresses.dedup();
        // The instruction stays while any breakpoint there is left.
        for address in addresses {
            if !self.armed(address) {
                inferior.remove_breakpoint(address)?;
            }
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

    /// The breakpoint table, in address order and, at one address, those
    /// without a test first; an `If` column joins it when any breakpoint has
    /// a test, and a `Commands` column when any has commands.
    fn breakpoint_lines(&self) -> Vec<String> {
        if self.list.is_empty() {
            return vec!["no breakpoints".to_string()];
        }
        let mut sorted = self.list.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|breakpoint| {
            (
                breakpoint.address.is_none(),
                breakpoint.address,
                breakpoint.condition.is_some(),
                breakpoint.id,
            )
        });
        let tests = sorted
            .iter()
            .any(|breakpoint| breakpoint.condition.is_some());
        let commands = sorted
            .iter()
            .any(|breakpoint| !breakpoint.commands.is_empty());
        let mut headers = vec!["Address", "Count", "Symbol", "Written name"];
        if tests {
            headers.push("If");
        }
        if commands {
            headers.push("Commands");
        }
        let rows = sorted
            .iter()
            .map(|breakpoint| {
                let mut row = vec![
                    breakpoint
                        .address
                        .map_or_else(|| "-".to_string(), |address| format!("0x{address:016x}")),
                    breakpoint.hits.to_string(),
                    shown(&breakpoint.symbol).to_string(),
                    shown(&written_shown(breakpoint)).to_string(),
                ];
                if tests {
                    row.push(
                        breakpoint
                            .condition
                            .as_ref()
                            .map_or_else(|| "-".to_string(), Condition::shown),
                    );
                }
                if commands {
                    row.push(listed(&breakpoint.commands));
                }
                row
            })
            .collect();
        table(&headers, rows)
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
            hits: 0,
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

    /// The `list watchpoint` lines: `<address> <count> <size> <value>`, in address
    /// order, the value read now (`-` when it cannot be); a `Commands` column
    /// joins them when any watchpoint has commands.
    pub(crate) fn watch_lines(&self, inferior: &mut dyn Inferior) -> Vec<String> {
        if self.watches.is_empty() {
            return vec!["no watchpoints".to_string()];
        }
        let mut sorted = self.watches.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|watch| watch.address);
        let commands = sorted.iter().any(|watch| !watch.commands.is_empty());
        let rows = sorted
            .iter()
            .map(|watch| {
                let mut row = vec![
                    format!("0x{:016x}", watch.address),
                    watch.hits.to_string(),
                    watch.size.to_string(),
                    watched_value(inferior, watch.address, watch.size)
                        .map_or_else(|_| "-".to_string(), |value| value_shown(value, watch.size)),
                ];
                if commands {
                    row.push(listed(&watch.commands));
                }
                row
            })
            .collect();
        if commands {
            return table(&["Address", "Count", "Size", "Value", "Commands"], rows);
        }
        table(&["Address", "Count", "Size", "Value"], rows)
    }

    /// Count `stop` against what made it: each breakpoint a hit stopped for,
    /// and each watchpoint whose bytes changed.
    fn count(&mut self, stop: &RunStop) {
        match stop {
            RunStop::Breakpoints(hits) => {
                for (_, ids) in self.matched.iter().filter(|(hit, _)| hits.contains(hit)) {
                    for breakpoint in &mut self.list {
                        if ids.contains(&breakpoint.id) {
                            breakpoint.hits += 1;
                        }
                    }
                }
            }
            RunStop::Changed(changes) => {
                for change in changes {
                    for watch in &mut self.watches {
                        if watch.address == change.address {
                            watch.hits += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// The commands each stop in `stop` runs, by thread, and whether every
    /// stop in it has commands.
    pub(crate) fn stop_commands(&self, stop: &RunStop) -> (Vec<(usize, Vec<String>)>, bool) {
        let mut found = Vec::new();
        let mut every = true;
        let mut add = |thread: usize, commands: Option<&Vec<String>>| match commands {
            Some(commands) if !commands.is_empty() => found.push((thread, commands.clone())),
            _ => every = false,
        };
        match stop {
            // Each breakpoint a hit stopped for runs its commands.
            RunStop::Breakpoints(hits) => {
                for hit in hits {
                    let stopped_for = self
                        .matched
                        .iter()
                        .find(|(matched, _)| matched == hit)
                        .map_or(&[][..], |(_, ids)| ids.as_slice());
                    if stopped_for.is_empty() {
                        add(hit.0, None);
                    }
                    for id in stopped_for {
                        add(
                            hit.0,
                            self.list
                                .iter()
                                .find(|breakpoint| breakpoint.id == *id)
                                .map(|breakpoint| &breakpoint.commands),
                        );
                    }
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
            let (stop, own) = self.run_once(inferior)?;
            let Some(stop) = stop else {
                if own.is_empty() {
                    continue;
                }
                self.close_report(inferior, false)?;
                return Ok(Ended::Own(own));
            };
            let exited = matches!(stop, RunStop::Exited(_));
            // The program is the user's again: no `step` runs the report's
            // breakpoints, so a report half written is the program's to end.
            if exited {
                self.report_ended();
            } else {
                self.close_report(inferior, false)?;
                self.place_pending(inferior)?;
            }
            let notes = self.take_notes();
            self.count(&stop);
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
        let stop = self.run_inferior(inferior, temporary)?;
        if let (Some(entry), false) = (temporary, matches!(stop, RunStop::Exited(_))) {
            inferior.remove_breakpoint(entry)?;
        }
        self.settle(inferior, stop, entry, temporary)
    }

    /// Sort what the program stopped for: the user's breakpoints are a stop
    /// when one there stops the thread; the debugger's own — the entry point,
    /// the error park, the debug report's — are not, though a watched raise at
    /// the park runs on to the raiser; a watched write is a stop only when it
    /// changed something.
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
                let hits = hits
                    .into_iter()
                    .filter(|(_, address)| Some(*address) != temporary)
                    .collect::<Vec<_>>();
                let (user, own) = self.sort_hits(inferior, &hits);
                self.report_hits(inferior, &unseen(&hits, &user))?;
                if !user.is_empty() {
                    return Ok((Some(RunStop::Breakpoints(user)), own));
                }
                if !own.is_empty() {
                    return Ok((None, own));
                }
                // A thread at the park that no breakpoint of the user's
                // stopped is the watch's to look at.
                let mut raised = None;
                for &(thread, address) in &hits {
                    if self.is_park(address) {
                        raised = self.watched_raise(inferior, thread)?;
                        if raised.is_some() {
                            break;
                        }
                    }
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
                Ok((self.watched_writes(inferior, &threads, None)?, Vec::new()))
            }
            other => Ok((Some(other), Vec::new())),
        }
    }

    /// Sort breakpoint hits into the user's — a hit where a breakpoint stops
    /// the thread (it has no test, or its test holds for that thread), each
    /// remembered for its commands — and the caller's own: a `ret` a `finish`
    /// waits on, or a breakpoint the session does not hold. A hit only at
    /// breakpoints whose tests fail is neither, and neither is the park's or
    /// the debug report's.
    #[allow(clippy::type_complexity)]
    fn sort_hits(
        &mut self,
        inferior: &mut dyn Inferior,
        hits: &[(usize, u64)],
    ) -> (Vec<(usize, u64)>, Vec<(usize, u64)>) {
        self.matched.clear();
        let mut user = Vec::new();
        let mut own = Vec::new();
        for &(thread, address) in hits {
            let here = self
                .list
                .iter()
                .filter(|breakpoint| breakpoint.address == Some(address))
                .collect::<Vec<_>>();
            let held = !here.is_empty();
            let stopping = here
                .into_iter()
                .filter(|breakpoint| {
                    breakpoint
                        .condition
                        .as_ref()
                        .is_none_or(|condition| condition.holds(inferior, thread))
                })
                .map(|breakpoint| breakpoint.id)
                .collect::<Vec<_>>();
            if !stopping.is_empty() {
                user.push((thread, address));
                self.matched.push(((thread, address), stopping));
            }
            if self.finishing.contains(&address)
                || (!held && !self.is_park(address) && !self.report.holds(address))
            {
                own.push((thread, address));
            }
        }
        (user, own)
    }

    /// Run the program, as [`Inferior::run`] does, without passing a watched
    /// write unseen. A thread a step left on a breakpoint is a hit there
    /// before anything runs. With watchpoints set, a thread paused on a
    /// breakpoint (one the session holds, or `own`, one the caller placed for
    /// this run) is first stepped off it here, where its write is seen: the
    /// backend's `run` steps such a thread with watchpoints off.
    fn run_inferior(
        &mut self,
        inferior: &mut dyn Inferior,
        own: Option<u64>,
    ) -> Result<RunStop, String> {
        let landed = self.take_landed(inferior)?;
        if !landed.is_empty() {
            return Ok(RunStop::Breakpoints(landed));
        }
        if !self.watches.is_empty() {
            let paused = inferior
                .threads()?
                .iter()
                .enumerate()
                .filter(|(_, facts)| self.armed(facts.pc) || own == Some(facts.pc))
                .map(|(thread, _)| thread)
                .collect::<Vec<_>>();
            if !paused.is_empty() {
                if let Some(stop) = self.watched_writes(inferior, &paused, own)? {
                    return Ok(stop);
                }
                let landed = self.take_landed(inferior)?;
                if !landed.is_empty() {
                    return Ok(RunStop::Breakpoints(landed));
                }
            }
        }
        inferior.run()
    }

    /// The threads a step left on a breakpoint, each still there (a `jump`
    /// since moves one away), with the breakpoint's address.
    fn take_landed(&mut self, inferior: &mut dyn Inferior) -> Result<Vec<(usize, u64)>, String> {
        let landed = std::mem::take(&mut self.landed);
        if landed.is_empty() {
            return Ok(landed);
        }
        let threads = inferior.threads()?;
        Ok(landed
            .into_iter()
            .filter(|(thread, address)| {
                threads
                    .get(*thread)
                    .is_some_and(|facts| facts.pc == *address)
            })
            .collect())
    }

    /// Step each of `threads`, stopped before a watched write, over the write:
    /// a `Changed` stop for every watchpoint whose bytes it changed, or `None`
    /// when it changed none. A step that ends otherwise is that stop. A thread
    /// the step leaves on a breakpoint the session holds, or on `own`, stops
    /// there next.
    fn watched_writes(
        &mut self,
        inferior: &mut dyn Inferior,
        threads: &[usize],
        own: Option<u64>,
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
                if self.armed(now) || own == Some(now) {
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
        let ours = !self.armed(raise.back);
        if ours {
            inferior.insert_breakpoint(raise.back)?;
        }
        let back = ours.then_some(raise.back);
        let stop = loop {
            let stop = match self.run_inferior(inferior, back)? {
                RunStop::Watchpoints(threads) => {
                    match self.watched_writes(inferior, &threads, back)? {
                        Some(stop) => stop,
                        None => continue,
                    }
                }
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
            // Another thread at the raiser's place, or at a breakpoint that
            // does not stop it, runs on; so does a raise at the park.
            let hits = hits
                .iter()
                .copied()
                .filter(|(_, address)| !(ours && *address == raise.back))
                .collect::<Vec<_>>();
            let (user, own) = self.sort_hits(inferior, &hits);
            self.report_hits(inferior, &unseen(&hits, &user))?;
            if !user.is_empty() || !own.is_empty() {
                break RunStop::Breakpoints(
                    hits.into_iter()
                        .filter(|hit| user.contains(hit) || own.contains(hit))
                        .collect(),
                );
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
            if !self.armed(park) {
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
        let Some(watch) = self.errors.take() else {
            return Err("no errors are being watched for".to_string());
        };
        if !self.armed(watch.park) {
            inferior.remove_breakpoint(watch.park)?;
        }
        Ok(vec!["Stopped watching for errors.".to_string()])
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
        self.matched.clear();
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
        // The addresses that hold an instruction again: one per address.
        let mut placed = Vec::new();
        if let Some(watch) = &mut self.errors {
            watch.park = park_address(inferior)?;
            inferior.insert_breakpoint(watch.park)?;
            placed.push(watch.park);
        }
        self.watch_report(inferior, &mut placed)?;
        let functions = inferior.functions().unwrap_or_default();
        let mut kept = Vec::new();
        for mut breakpoint in std::mem::take(&mut self.list) {
            breakpoint.hits = 0;
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
                if !placed.contains(&address) {
                    inferior.insert_breakpoint(address)?;
                    placed.push(address);
                }
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
        // A `ret` that holds a breakpoint already needs no instruction of the
        // finish's own, but is still one it waits on.
        let ours = ends
            .iter()
            .copied()
            .filter(|address| !self.armed(*address))
            .collect::<Vec<_>>();
        for address in &ours {
            inferior.insert_breakpoint(*address)?;
        }
        self.finishing = ends;
        let outcome = self.run_to_return(inferior, thread, facts.sp, hear);
        self.finishing.clear();
        let outcome = outcome?;
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

    /// Run until `thread` reaches a `ret` the finish waits on with its frame
    /// popped (its stack pointer at or above `sp`): `None`; or the stop heard
    /// first, which ends the finish.
    fn run_to_return(
        &mut self,
        inferior: &mut dyn Inferior,
        thread: usize,
        sp: u64,
        hear: &mut Hear,
    ) -> Result<Option<RunStop>, String> {
        loop {
            let hits = match self.run(inferior, hear)? {
                Ended::Heard(stop) => return Ok(Some(stop)),
                Ended::Own(hits) => hits,
            };
            let popped = hits.iter().any(|(hit, _)| {
                *hit == thread
                    && inferior
                        .threads()
                        .ok()
                        .and_then(|threads| threads.get(thread).map(|now| now.sp >= sp))
                        .unwrap_or(false)
            });
            if popped {
                return Ok(None);
            }
        }
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
            if self
                .list
                .iter()
                .any(|other| other.address == Some(address) && other.condition == pending.condition)
            {
                // One with the same test is already there; this adds nothing.
                self.notes.push(format!(
                    "The pending breakpoint on {}{} is at 0x{address:016x}, which already has one.",
                    shown(&pending.symbol),
                    tested(pending)
                ));
                self.list.remove(index);
                continue;
            }
            if !self.armed(address) {
                inferior.insert_breakpoint(address)?;
            }
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
mod tests;
