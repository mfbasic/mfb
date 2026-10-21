//! The debug report of a `--debug` build, kept from the terminal for
//! `show report`.
//!
//! A `--debug` build writes its report to stderr as the last thing it does
//! (`_mfb_debug_shutdown`): one `write` to fd 2 per line. The session keeps a
//! breakpoint of its own on that function. A thread entering it opens a
//! window: a breakpoint where the function returns to, and one on each `write`
//! the program links. Inside the window a `write` to fd 2 by that thread is a
//! report line: its bytes are read from the program and kept, and the call is
//! skipped — the thread goes on from where `write` would return, with the
//! length it asked for as the result — so the line never reaches the terminal.
//! The thread's return closes the window and takes its breakpoints away.
//!
//! Only a `run` sees these breakpoints: a `step` runs the instructions they
//! replaced. So a window still open when the program stops for the user is
//! closed there, and the program writes the rest of the report itself; what
//! was kept until then is marked incomplete, as is a report the program ended
//! inside.

use super::Breakpoints;
use crate::codegen::debug::DEBUG_SHUTDOWN_SYMBOL;
use crate::debugger::names::linked_name;
use crate::debugger::{Inferior, Register, RegisterValue};
use crate::terminal_safe::safe;

/// The file descriptor the report is written to.
const STDERR: u64 = 2;

/// The C function each report line is written with.
const WRITE: &str = "write";

/// The most bytes one `write` may ask for and still be taken for a report
/// line; the runtime builds a line in a 128-byte window.
const LINE_LIMIT: u64 = 4096;

/// What ends the `--[EX]` line and follows a step's, when a report was kept.
pub(crate) const REPORT_HINT: &str =
    "type \"show report\" to see the performance and memory report";

const NOT_A_DEBUG_BUILD: &str =
    "No performance and memory report: the program was not built with --debug.";
const NOT_YET: &str = "No performance and memory report yet: the program writes it as it exits.";
const NONE_KEPT: &str =
    "No performance and memory report: the program ended without one being kept.";
const INCOMPLETE: &str = "The report is incomplete: the program stopped or ended while writing it.";

/// The session's hold on the program's debug report.
#[derive(Debug, Default)]
pub(super) struct Report {
    /// The live address of `_mfb_debug_shutdown`, where the debugger keeps a
    /// breakpoint of its own; `None` in a build without `--debug`.
    shutdown: Option<u64>,
    /// The report being written now.
    window: Option<Window>,
    /// The report the program wrote.
    kept: Option<Kept>,
}

/// One thread's pass through `_mfb_debug_shutdown`.
#[derive(Debug)]
struct Window {
    /// The system's identifier for the thread writing the report.
    thread: u64,
    /// Where `_mfb_debug_shutdown` returns to.
    back: u64,
    /// The live address of each `write` the program links.
    writes: Vec<u64>,
    /// The lines taken so far.
    text: Vec<u8>,
}

#[derive(Debug)]
struct Kept {
    text: Vec<u8>,
    /// The thread returned from `_mfb_debug_shutdown` with the window open.
    complete: bool,
}

impl Report {
    /// Whether one of the report's own breakpoints is at `address`.
    pub(super) fn holds(&self, address: u64) -> bool {
        self.shutdown == Some(address)
            || self
                .window
                .as_ref()
                .is_some_and(|window| window.back == address || window.writes.contains(&address))
    }
}

/// The 64-bit register `name` among thread `thread`'s `registers`.
fn register(registers: &[Register], thread: usize, name: &str) -> Result<u64, String> {
    registers
        .iter()
        .find(|register| register.name == name)
        .and_then(|register| match register.value {
            RegisterValue::Bits64(value) => Some(value),
            RegisterValue::Bits128(_) => None,
        })
        .ok_or_else(|| format!("thread {thread} has no register {name}"))
}

/// The system's identifier for thread `thread`, if there is such a thread.
fn thread_id(inferior: &mut dyn Inferior, thread: usize) -> Result<Option<u64>, String> {
    Ok(inferior.threads()?.get(thread).map(|facts| facts.id))
}

impl Breakpoints {
    /// Start keeping the program's debug report: forget any kept so far, and
    /// put a breakpoint of the debugger's own on `_mfb_debug_shutdown`, which
    /// only a `--debug` build has. `placed` is every address that holds an
    /// instruction of the session's already; the report's joins it.
    pub(crate) fn watch_report(
        &mut self,
        inferior: &mut dyn Inferior,
        placed: &mut Vec<u64>,
    ) -> Result<(), String> {
        self.report = Report::default();
        let shutdown = inferior
            .functions()
            .unwrap_or_default()
            .iter()
            .find(|function| function.name == DEBUG_SHUTDOWN_SYMBOL && !function.linked)
            .and_then(|function| function.address);
        if let Some(address) = shutdown {
            if !placed.contains(&address) {
                inferior.insert_breakpoint(address)?;
                placed.push(address);
            }
        }
        self.report.shutdown = shutdown;
        Ok(())
    }

    /// Whether a report was kept.
    pub(crate) fn has_report(&self) -> bool {
        self.report.kept.is_some()
    }

    /// `show report`: the report the program wrote, a line each as it wrote
    /// them, or why there is none; `exited` once the program has ended.
    pub(crate) fn report_lines(&self, exited: bool) -> Vec<String> {
        let Some(kept) = &self.report.kept else {
            let none = if self.report.shutdown.is_none() {
                NOT_A_DEBUG_BUILD
            } else if exited {
                NONE_KEPT
            } else {
                NOT_YET
            };
            return vec![none.to_string()];
        };
        let mut lines = String::from_utf8_lossy(&kept.text)
            .lines()
            .map(|line| safe(line).into_owned())
            .collect::<Vec<_>>();
        if !kept.complete {
            lines.push(INCOMPLETE.to_string());
        }
        lines
    }

    /// Look at breakpoint hits that stop for none of the user's breakpoints:
    /// a thread entering `_mfb_debug_shutdown` opens the window, the thread
    /// that opened it writing a line has the line taken, and its return closes
    /// the window.
    pub(super) fn report_hits(
        &mut self,
        inferior: &mut dyn Inferior,
        hits: &[(usize, u64)],
    ) -> Result<(), String> {
        for &(thread, address) in hits {
            if !self.report.holds(address) {
                continue;
            }
            let Some(id) = thread_id(inferior, thread)? else {
                continue;
            };
            match &self.report.window {
                None => self.open_report(inferior, thread, id)?,
                Some(window) if window.thread != id => {}
                Some(window) if window.back == address => self.close_report(inferior, true)?,
                Some(window) if window.writes.contains(&address) => {
                    self.take_line(inferior, thread)?
                }
                // The thread writing the report, entering it again.
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Thread `thread` (the system's `id`) has just entered
    /// `_mfb_debug_shutdown`: open the window, when the program links a
    /// `write` to stop at.
    fn open_report(
        &mut self,
        inferior: &mut dyn Inferior,
        thread: usize,
        id: u64,
    ) -> Result<(), String> {
        let mut writes = Vec::new();
        for function in inferior.functions()? {
            if let (true, Some(address)) = (function.linked, function.address) {
                if linked_name(&function.name) == WRITE && !writes.contains(&address) {
                    writes.push(address);
                }
            }
        }
        if writes.is_empty() {
            return Ok(());
        }
        let back = register(&inferior.registers(thread)?, thread, inferior.arch().link)?;
        for address in writes.iter().chain([&back]) {
            if !self.armed(*address) {
                inferior.insert_breakpoint(*address)?;
            }
        }
        self.report.window = Some(Window {
            thread: id,
            back,
            writes,
            text: Vec::new(),
        });
        Ok(())
    }

    /// The thread writing the report is at `write`'s first instruction: when
    /// it writes to stderr, keep the bytes and skip the call.
    fn take_line(&mut self, inferior: &mut dyn Inferior, thread: usize) -> Result<(), String> {
        let registers = inferior.registers(thread)?;
        let arch = inferior.arch();
        let value = |name| register(&registers, thread, name);
        // `write(fd, buffer, length)`, returning in the first argument register.
        let [fd, buffer, count, ..] = arch.arguments;
        let length = value(count)?;
        if value(fd)? != STDERR || length > LINE_LIMIT {
            return Ok(());
        }
        let bytes = match length {
            0 => Vec::new(),
            _ => inferior.read_bytes(value(buffer)?, length as usize)?,
        };
        let back = value(arch.link)?;
        inferior.set_registers(
            thread,
            &[(fd.to_string(), length), (arch.pc.to_string(), back)],
        )?;
        if let Some(window) = &mut self.report.window {
            window.text.extend(bytes);
        }
        Ok(())
    }

    /// Close the window, if one is open: take its breakpoints away, and keep
    /// what it took when no report is kept yet. (`_mfb_debug_shutdown` writes
    /// once: a second pass through it — a signal during the exit — takes
    /// nothing, and does not replace the report.)
    pub(super) fn close_report(
        &mut self,
        inferior: &mut dyn Inferior,
        complete: bool,
    ) -> Result<(), String> {
        let Some(window) = self.report.window.take() else {
            return Ok(());
        };
        for address in window.writes.iter().chain([&window.back]) {
            if !self.armed(*address) {
                inferior.remove_breakpoint(*address)?;
            }
        }
        self.keep(window.text, complete);
        Ok(())
    }

    /// The program has ended: a window still open took all it will.
    pub(super) fn report_ended(&mut self) {
        if let Some(window) = self.report.window.take() {
            self.keep(window.text, false);
        }
    }

    fn keep(&mut self, text: Vec<u8>, complete: bool) {
        if self.report.kept.is_none() && !text.is_empty() {
            self.report.kept = Some(Kept { text, complete });
        }
    }
}

#[cfg(test)]
mod tests;
