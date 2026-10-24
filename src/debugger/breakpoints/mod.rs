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

use super::arch::Arch;
use super::disassemble::returns;
use super::location::Condition;
use super::names::{ambiguity, classify, Collisions};
use super::prompt::table;
use super::{Change, FunctionSymbol, Inferior, RunStop, StepStop};
// From a part, `super` is `debugger::breakpoints`; part bodies spell `super::Exit`,
// `super::RegisterValue` and `super::runtime`, which live in `debugger`.
use super::{runtime, Exit, RegisterValue};

mod errors;
mod running;
pub(crate) use errors::*;
pub(crate) use running::*;

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

/// The bytes of a word: a value this wide is shown in decimal too.
const WORD: u64 = 8;

/// Refuse a watchpoint `arch`'s system cannot hold: one covers 1 to `span`
/// bytes that do not cross a multiple of `span` (8 on AArch64), and a system
/// with no watchpoints holds none.
pub(crate) fn watchable(arch: &Arch, address: u64, size: u64) -> Result<(), String> {
    let Some(span) = arch.watch_span else {
        return Err(format!(
            "this system has no watchpoints: the {} kernel does not provide them",
            arch.name
        ));
    };
    if size == 0 || size > span || address % span + size > span {
        return Err(format!(
            "cannot watch {size} bytes at 0x{address:016x}: a watchpoint covers 1 to {span} \
             bytes that do not cross an address that is a multiple of {span}"
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
    if size == WORD {
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

/// Refuse a breakpoint address that is not an instruction start for `arch`.
fn instruction_boundary(arch: &Arch, address: u64) -> Result<(), String> {
    if address % arch.alignment == 0 {
        return Ok(());
    }
    Err(format!(
        "0x{address:016x} is not the start of an instruction: they start every {} bytes",
        arch.alignment
    ))
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
        for breakpoint in &wanted {
            if let Some(address) = breakpoint.address {
                instruction_boundary(inferior.arch(), address)?;
            }
        }
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
        watchable(inferior.arch(), address, size)?;
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
}

#[cfg(test)]
mod instruction_alignment_tests {
    use super::instruction_boundary;
    use crate::debugger::arm64::AARCH64;

    #[test]
    fn breakpoint_addresses_must_start_on_instruction_boundaries() {
        assert_eq!(instruction_boundary(&AARCH64, 0x1100), Ok(()));
        assert_eq!(
            instruction_boundary(&AARCH64, 0x1101),
            Err(
                "0x0000000000001101 is not the start of an instruction: they start every 4 bytes"
                    .to_string()
            )
        );
    }
}

#[cfg(test)]
mod tests;
