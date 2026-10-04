//! The session's breakpoints and `run`.
//!
//! A breakpoint is set by address or by function name. A name is looked up as
//! a symbol first (`_mfb_rt_print`, `_exit`) and, only when no symbol has it,
//! as a written name (`isPrime`, `exit`); every distinct address it matches
//! gets a breakpoint. A linked function the loader has not bound yet has no
//! address: its breakpoint is pending, and `run` places it once the program
//! reaches its entry point, by which time every library is loaded — stopping
//! there on a breakpoint of its own when no other is in the way.

use super::names::{classify_defined, linked_name};
use super::{FunctionSymbol, Inferior, RunStop};

/// One breakpoint the session holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Breakpoint {
    /// Where it is written; `None` while its function is not loaded.
    pub(crate) address: Option<u64>,
    /// The symbol it is on (with `+0x<offset>` inside a function), or `-`.
    pub(crate) symbol: String,
    /// The name the developer wrote (likewise), or `-`.
    pub(crate) written: String,
}

/// What a `breakpoint` command names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Address(u64),
    Name(String),
}

impl Target {
    /// `0x` and hex digits is an address; anything else is a name.
    pub(crate) fn parse(word: &str) -> Result<Target, String> {
        match word.strip_prefix("0x").or_else(|| word.strip_prefix("0X")) {
            Some(hex) => u64::from_str_radix(hex, 16)
                .map(Target::Address)
                .map_err(|_| format!("'{word}' is not an address")),
            None => Ok(Target::Name(word.to_string())),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Breakpoints {
    list: Vec<Breakpoint>,
    /// The program has been seen at its entry point: every library is bound.
    entry_passed: bool,
}

/// The written name of a function symbol, or `-`.
fn written(function: &FunctionSymbol) -> String {
    if function.linked {
        linked_name(&function.name)
    } else {
        classify_defined(&function.name)
            .1
            .unwrap_or_else(|| "-".to_string())
    }
}

/// The breakpoints a name asks for: symbols spelled `name`, or else functions
/// written `name`; one per distinct address, and one per symbol not yet bound.
fn find(functions: &[FunctionSymbol], name: &str) -> Vec<Breakpoint> {
    let mut matches = functions
        .iter()
        .filter(|function| function.name == name)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        matches = functions
            .iter()
            .filter(|function| written(function) == name)
            .collect();
    }
    let mut found: Vec<Breakpoint> = Vec::new();
    for function in matches {
        let duplicate = found.iter().any(|other| match function.address {
            Some(_) => other.address == function.address,
            None => other.address.is_none() && other.symbol == function.name,
        });
        let bound_elsewhere = function.address.is_none()
            && functions
                .iter()
                .any(|other| other.name == function.name && other.address.is_some());
        if !duplicate && !bound_elsewhere {
            found.push(Breakpoint {
                address: function.address,
                symbol: function.name.clone(),
                written: written(function),
            });
        }
    }
    found
}

/// The symbol and written name at `address`: the function that holds it (with
/// the offset into it), a linked function bound there, or neither.
fn describe(functions: &[FunctionSymbol], address: u64) -> (String, String) {
    let offset = |name: String, start: u64| {
        if name == "-" || address == start {
            name
        } else {
            format!("{name}+{:#x}", address - start)
        }
    };
    let holder = functions.iter().find(|function| {
        !function.linked
            && function.address.is_some_and(|start| {
                start <= address && address - start < function.size.unwrap_or(0)
            })
    });
    if let Some(function) = holder {
        let start = function.address.unwrap_or(address);
        return (
            offset(function.name.clone(), start),
            offset(written(function), start),
        );
    }
    match functions
        .iter()
        .find(|function| function.linked && function.address == Some(address))
    {
        Some(function) => (function.name.clone(), written(function)),
        None => ("-".to_string(), "-".to_string()),
    }
}

/// How a breakpoint reads in a sentence: `0x… in isPrime`.
fn place(breakpoint: &Breakpoint) -> String {
    let name = if breakpoint.written != "-" {
        &breakpoint.written
    } else {
        &breakpoint.symbol
    };
    match (breakpoint.address, name.as_str()) {
        (Some(address), "-") => format!("0x{address:016x}"),
        (Some(address), name) => format!("0x{address:016x} in {name}"),
        (None, _) => breakpoint.symbol.clone(),
    }
}

impl Breakpoints {
    pub(crate) fn set(
        &mut self,
        inferior: &mut dyn Inferior,
        target: &Target,
    ) -> Result<Vec<String>, String> {
        let functions = inferior.functions().unwrap_or_default();
        let wanted = match target {
            Target::Address(address) => {
                let (symbol, written) = describe(&functions, *address);
                vec![Breakpoint {
                    address: Some(*address),
                    symbol,
                    written,
                }]
            }
            Target::Name(name) => {
                let found = find(&functions, name);
                if found.is_empty() {
                    return Err(format!(
                        "no function is named '{name}'; type 'list func' for the functions"
                    ));
                }
                found
            }
        };
        let mut lines = Vec::new();
        for breakpoint in wanted {
            let exists = self.list.iter().any(|other| match breakpoint.address {
                Some(_) => other.address == breakpoint.address,
                None => other.address.is_none() && other.symbol == breakpoint.symbol,
            });
            if exists {
                continue;
            }
            match breakpoint.address {
                Some(address) => {
                    inferior.insert_breakpoint(address)?;
                    lines.push(format!("Set a breakpoint at {}.", place(&breakpoint)));
                }
                None => lines.push(format!(
                    "Set a pending breakpoint on {}: it is placed when the program has \
                     loaded its libraries.",
                    breakpoint.symbol
                )),
            }
            self.list.push(breakpoint);
        }
        if lines.is_empty() {
            return Err(match target {
                Target::Address(address) => {
                    format!("there is already a breakpoint at {address:#x}")
                }
                Target::Name(name) => format!("there is already a breakpoint on '{name}'"),
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
            Target::Name(name) => {
                let by_symbol = self
                    .list
                    .iter()
                    .filter(|breakpoint| &breakpoint.symbol == name)
                    .cloned()
                    .collect::<Vec<_>>();
                if by_symbol.is_empty() {
                    self.list
                        .iter()
                        .filter(|breakpoint| &breakpoint.written == name)
                        .cloned()
                        .collect()
                } else {
                    by_symbol
                }
            }
        };
        if chosen.is_empty() {
            return Err(match target {
                Target::Address(address) => format!("there is no breakpoint at {address:#x}"),
                Target::Name(name) => format!("there is no breakpoint on '{name}'"),
            });
        }
        let mut lines = Vec::new();
        for breakpoint in chosen {
            if let Some(address) = breakpoint.address {
                inferior.remove_breakpoint(address)?;
            }
            self.list.retain(|other| *other != breakpoint);
            lines.push(format!("Removed the breakpoint at {}.", place(&breakpoint)));
        }
        Ok(lines)
    }

    /// The `list breakpoint` lines: `<address> <symbol> <written name>`, in
    /// address order, pending ones last with `-` for the address.
    pub(crate) fn lines(&self) -> Vec<String> {
        if self.list.is_empty() {
            return vec!["no breakpoints".to_string()];
        }
        let mut sorted = self.list.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|breakpoint| (breakpoint.address.is_none(), breakpoint.address));
        let width = sorted
            .iter()
            .map(|breakpoint| breakpoint.symbol.len())
            .max()
            .unwrap_or(0);
        sorted
            .iter()
            .map(|breakpoint| {
                let address = breakpoint
                    .address
                    .map_or_else(|| "-".to_string(), |address| format!("0x{address:016x}"));
                format!(
                    "{address:<18} {:<width$} {}",
                    breakpoint.symbol, breakpoint.written
                )
            })
            .collect()
    }

    /// Run the program to the next stop the user should see. Pending
    /// breakpoints are placed as soon as their functions are bound; while any
    /// is pending, the program's entry point holds a breakpoint of the
    /// debugger's own, and stopping there only places them and runs on.
    /// Answers notes about placed breakpoints, and the stop.
    pub(crate) fn run(
        &mut self,
        inferior: &mut dyn Inferior,
    ) -> Result<(Vec<String>, RunStop), String> {
        let mut notes = Vec::new();
        loop {
            self.place_pending(inferior, &mut notes)?;
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
            if let RunStop::Breakpoints(hits) = &stop {
                if hits.iter().any(|(_, address)| Some(*address) == entry) {
                    self.entry_passed = true;
                }
                if temporary.is_some()
                    && hits.iter().all(|(_, address)| Some(*address) == temporary)
                {
                    continue;
                }
            }
            if !matches!(stop, RunStop::Exited(_)) {
                self.place_pending(inferior, &mut notes)?;
            }
            return Ok((notes, stop));
        }
    }

    /// Place every pending breakpoint whose function is now bound.
    fn place_pending(
        &mut self,
        inferior: &mut dyn Inferior,
        notes: &mut Vec<String>,
    ) -> Result<(), String> {
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
                        .find(|function| function.name == pending.symbol)
                        .and_then(|function| function.address)
                })
                .flatten();
            let Some(address) = bound else {
                index += 1;
                continue;
            };
            if self.list.iter().any(|other| other.address == Some(address)) {
                // Another breakpoint is already there; this one adds nothing.
                notes.push(format!(
                    "The pending breakpoint on {} is at 0x{address:016x}, which already has one.",
                    pending.symbol
                ));
                self.list.remove(index);
                continue;
            }
            inferior.insert_breakpoint(address)?;
            self.list[index].address = Some(address);
            notes.push(format!(
                "Placed the pending breakpoint on {} at 0x{address:016x}.",
                self.list[index].symbol
            ));
            index += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::{Ending, Exit, Register, Step, ThreadFacts};

    /// A program whose `_exit` binds when it reaches its entry point, and
    /// which then stops at whatever breakpoints `stops` lists, in turn.
    struct Program {
        bound: bool,
        inserted: Vec<u64>,
        stops: Vec<RunStop>,
        runs: usize,
    }

    const ENTRY: u64 = 0x1000;
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
            ])
        }
        fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
            Ok(None)
        }
        fn read_bytes(&mut self, address: u64, _: usize) -> Result<Vec<u8>, String> {
            Err(format!("unmapped {address:#x}"))
        }
        fn read_u64(&mut self, address: u64) -> Result<u64, String> {
            Err(format!("unmapped {address:#x}"))
        }
        fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
            Ok(Vec::new())
        }
        fn registers(&mut self, _: usize) -> Result<Vec<Register>, String> {
            Ok(Vec::new())
        }
        fn step(&mut self, _: &[usize], _: u64) -> Result<Vec<Step>, String> {
            Ok(Vec::new())
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
            breakpoints.set(&mut program, &name("isPrime")).unwrap(),
            [
                "Set a breakpoint at 0x0000000000001100 in isPrime.",
                "Set a breakpoint at 0x0000000000001200 in isPrime.",
            ]
        );
        // `_mfb_fn_exit` is a symbol: only that one function.
        assert_eq!(
            breakpoints
                .set(&mut program, &name("_mfb_fn_exit"))
                .unwrap(),
            ["Set a breakpoint at 0x0000000000001300 in exit."]
        );
        assert_eq!(program.inserted, [0x1100, 0x1200, 0x1300]);
        assert_eq!(
            breakpoints.set(&mut program, &name("isPrime")),
            Err("there is already a breakpoint on 'isPrime'".to_string())
        );
        assert_eq!(
            breakpoints.set(&mut program, &name("nothing")),
            Err("no function is named 'nothing'; type 'list func' for the functions".to_string())
        );
    }

    #[test]
    fn an_address_is_described_by_the_function_that_holds_it() {
        let mut program = Program::new(Vec::new());
        let mut breakpoints = Breakpoints::default();
        breakpoints
            .set(&mut program, &Target::Address(0x1124))
            .unwrap();
        breakpoints
            .set(&mut program, &Target::Address(0x5000))
            .unwrap();
        assert_eq!(
            breakpoints.lines(),
            [
                "0x0000000000001124 _mfb_fn_isPrime+0x24 isPrime+0x24",
                "0x0000000000005000 -                    -",
            ]
        );
        assert_eq!(
            breakpoints.set(&mut program, &Target::Address(0x1124)),
            Err("there is already a breakpoint at 0x1124".to_string())
        );
    }

    #[test]
    fn an_unbound_linked_function_is_pending_until_the_entry_point() {
        let mut program = Program::new(vec![RunStop::Breakpoints(vec![(0, EXIT)])]);
        let mut breakpoints = Breakpoints::default();
        // `exit` is no symbol; as written it names both the developer's
        // function and the linked one, which is not bound yet.
        assert_eq!(
            breakpoints.set(&mut program, &name("exit")).unwrap(),
            [
                "Set a breakpoint at 0x0000000000001300 in exit.",
                "Set a pending breakpoint on _exit: it is placed when the program has loaded \
                 its libraries.",
            ]
        );
        assert_eq!(
            breakpoints.lines(),
            [
                "0x0000000000001300 _mfb_fn_exit exit",
                "-                  _exit        exit",
            ]
        );
        let (notes, stop) = breakpoints.run(&mut program).unwrap();
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
        breakpoints.set(&mut program, &name("isPrime")).unwrap();
        breakpoints.set(&mut program, &name("_exit")).unwrap();
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
            Err("there is no breakpoint at 0x1100".to_string())
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
        breakpoints.set(&mut program, &name("_main")).unwrap();
        breakpoints.set(&mut program, &name("_exit")).unwrap();
        let (notes, stop) = breakpoints.run(&mut program).unwrap();
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
        let (notes, stop) = breakpoints.run(&mut program).unwrap();
        assert!(notes.is_empty());
        assert_eq!(stop, RunStop::Exited(Exit::Status(3)));
    }
}
