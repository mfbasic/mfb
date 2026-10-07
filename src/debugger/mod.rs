//! The native debugger behind `mfb dbg`.
//!
//! The debugger is a property of the compiler's host, not of a compiled program:
//! an `mfb` built on macOS debugs macOS programs, and nothing else. This module
//! is the platform-neutral half — the MFBASIC/native check every target must
//! pass, the [`Backend`] and [`Inferior`] seams a platform implements, and the
//! `DBG>` prompt ([`prompt`]) that drives an [`Inferior`] without knowing which
//! platform produced it. A platform joins by implementing [`Backend`] and
//! answering it from [`host_backend`]; the prompt and the CLI need no change.

pub(crate) mod backtrace;
pub(crate) mod breakpoints;
pub(crate) mod disassemble;
pub(crate) mod location;
pub(crate) mod names;
pub(crate) mod prompt;
pub(crate) mod runtime;
pub(crate) mod values;

#[cfg(any(target_os = "macos", test))]
mod macho;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(test)]
pub(crate) mod tests;

use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use crate::os::inspect::inspect;
use crate::terminal_safe::safe;

/// One platform's way to start, attach to, and identify a debuggee.
pub(crate) trait Backend {
    /// The executable file the running process `pid` was started from.
    fn executable_of(&self, pid: u32) -> Result<PathBuf, String>;
    /// Start `program` with `args`, attached and paused before its first
    /// instruction runs.
    fn launch(&self, program: &Path, args: &[OsString]) -> Result<Box<dyn Inferior>, String>;
    /// Attach to the running process `pid`, whose executable is `program`,
    /// and pause every thread in it.
    fn attach(&self, pid: u32, program: &Path) -> Result<Box<dyn Inferior>, String>;
}

/// A paused debuggee the session owns.
pub(crate) trait Inferior {
    fn pid(&self) -> u32;
    /// End the session the way it began: kill a program the debugger launched,
    /// or let an attached program run on and detach from it. Called once; an
    /// implementation also ends an unended session when it is dropped.
    fn end(&mut self) -> Result<Ending, String>;
    /// Every function symbol the executable's symbol table names, in table
    /// order and with repeats, at its address in the running process; then
    /// every function exported by a library the program loaded while running
    /// (one no library it was linked against depends on), by address.
    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String>;
    /// The live address of a data symbol the executable defines, or `None`
    /// when it defines no such symbol (always, in a build without `--debug`).
    fn data_symbol(&mut self, name: &str) -> Result<Option<u64>, String>;
    /// The 64-bit little-endian word at `address` in the process.
    fn read_u64(&mut self, address: u64) -> Result<u64, String>;
    /// `length` bytes of the process's memory at `address`.
    fn read_bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String>;
    /// `length` bytes of code at `address`, with the instruction each of the
    /// debugger's breakpoints replaced put back.
    fn read_code(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String>;
    /// The function holding `address`: in the executable (from its symbols or
    /// import stubs), or in a loaded library (from its function starts).
    fn function_at(&mut self, address: u64) -> Result<Option<FunctionRange>, String>;
    /// The functions the loaded libraries define under the symbol `name`.
    fn library_functions(&mut self, name: &str) -> Result<Vec<FunctionRange>, String>;
    /// The data symbol the executable defines at exactly `address`.
    fn data_symbol_at(&mut self, address: u64) -> Result<Option<String>, String>;
    /// Every thread, in the system's order; index 0 is the initial thread.
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String>;
    /// The registers of thread `thread` (an index into [`Inferior::threads`]),
    /// in display order.
    fn registers(&mut self, thread: usize) -> Result<Vec<Register>, String>;
    /// Give registers of thread `thread` new values, each named as
    /// [`Inferior::registers`] names it: a general-purpose register, `sp`,
    /// `pc`, `nzcv` (its flag bits), `fpsr` or `fpcr` (32 bits). Every value is
    /// written before the thread next runs.
    fn set_registers(&mut self, thread: usize, values: &[(String, u64)]) -> Result<(), String>;
    /// Run `threads` (indices into [`Inferior::threads`]) in up to `count`
    /// rounds of one instruction each, with every other thread held; the
    /// process is paused again afterwards. A thread that stops early sits out
    /// the remaining rounds. Answers one [`Step`] per thread, in order.
    fn step(&mut self, threads: &[usize], count: u64) -> Result<Vec<Step>, String>;
    /// Kill a program the debugger launched and start it again, the same
    /// way, paused before its first instruction; every breakpoint is gone.
    /// Answers the new process id. A program attached to cannot restart.
    fn restart(&mut self) -> Result<u32, String>;
    /// The live address of the program's entry point (where the loader hands
    /// over once every library is loaded and bound), when the file names one.
    fn entry(&mut self) -> Result<Option<u64>, String>;
    /// Make the instruction at `address` stop the program when it runs.
    fn insert_breakpoint(&mut self, address: u64) -> Result<(), String>;
    /// Put back the instruction a breakpoint at `address` replaced.
    fn remove_breakpoint(&mut self, address: u64) -> Result<(), String>;
    /// Make every thread, and every thread the program starts later, stop
    /// before it writes any of the `size` bytes at `address`: 1 to 8 bytes
    /// that do not cross a multiple of 8 ([`breakpoints::watchable`]). A
    /// `step` runs with the watchpoints off.
    fn insert_watchpoint(&mut self, address: u64, size: u64) -> Result<(), String>;
    /// Take away the watchpoint at `address`.
    fn remove_watchpoint(&mut self, address: u64) -> Result<(), String>;
    /// Let every thread run until one reaches a breakpoint, is about to write
    /// memory a watchpoint covers, or raises an exception, the program ends,
    /// or the user interrupts it; the process is paused again afterwards. A
    /// thread sitting on a breakpoint goes past it first. Once the program has
    /// ended, answers how it ended at once.
    fn run(&mut self) -> Result<RunStop, String>;
}

/// Why a `run` stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunStop {
    /// These threads (indices into [`Inferior::threads`]) reached a breakpoint
    /// at these addresses.
    Breakpoints(Vec<(usize, u64)>),
    /// These threads are about to write memory a watchpoint covers: each is
    /// paused on the instruction that writes it, which has not run.
    Watchpoints(Vec<usize>),
    /// Writes changed watched memory, one [`Change`] per watchpoint whose
    /// value changed. Only the session's `run` makes this stop; an
    /// [`Inferior`] never answers it.
    Changed(Vec<Change>),
    /// A thread raised an exception that is not one of the debugger's
    /// breakpoints; the text names it.
    Exception { thread: usize, what: String },
    /// The user interrupted the run (Ctrl-C).
    Interrupted,
    /// A thread raised an error `breakerror` watches for, and stopped in the
    /// function that raised it as the runtime returned with the error parked
    /// (`origin` is where in the source, when the error has one). Only the
    /// session's `run` makes this stop; an [`Inferior`] never answers it.
    Raised {
        thread: usize,
        code: i64,
        origin: Option<String>,
    },
    /// The program ended.
    Exited(Exit),
}

/// A write that changed the memory a watchpoint covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change {
    /// The thread that wrote it.
    pub(crate) thread: usize,
    /// The instruction that wrote it.
    pub(crate) pc: u64,
    /// The watchpoint's address and size.
    pub(crate) address: u64,
    pub(crate) size: u64,
    /// The value before and after, little-endian, `size` bytes wide.
    pub(crate) old: u64,
    pub(crate) new: u64,
}

/// How a program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exit {
    Status(i32),
    Signal(i32),
    /// An attached program: its parent, not the debugger, sees how it ended.
    Unknown,
}

/// What the debugger can read about one thread without the program's help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThreadFacts {
    /// The system's identifier for the thread.
    pub(crate) id: u64,
    /// The thread's `pthread_t`: the handle the runtime stores for its own
    /// threads.
    pub(crate) handle: u64,
    /// The name the thread gave itself, if any.
    pub(crate) name: Option<String>,
    pub(crate) pc: u64,
    pub(crate) sp: u64,
    /// The register MFBASIC code keeps its arena state in (x19 on AArch64);
    /// meaningful only while the thread is in MFBASIC code.
    pub(crate) arena: u64,
    /// The memory region holding the thread's stack, `[start, end)`.
    pub(crate) stack: Option<(u64, u64)>,
}

/// How a `step` ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Step {
    /// Instructions completed.
    pub(crate) taken: u64,
    pub(crate) stop: StepStop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StepStop {
    /// Every requested instruction ran.
    Finished,
    /// The thread raised an exception other than the step's own (a crash, a
    /// breakpoint instruction); the text names it.
    Exception(String),
    /// An instruction did not finish in time (a system call that waits); the
    /// thread was paused where it was.
    Waiting,
    /// The process ended during the step.
    Exited,
}

/// One function symbol of the debuggee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FunctionSymbol {
    /// The symbol exactly as the executable spells it.
    pub(crate) name: String,
    /// `true` for a function in a linked library, `false` for one the
    /// executable defines.
    pub(crate) linked: bool,
    /// `true` for a function (and so `linked`) in a library the program loaded
    /// while running, which the executable does not import.
    pub(crate) loaded: bool,
    /// The live address; `None` for a linked function the loader has not bound
    /// yet.
    pub(crate) address: Option<u64>,
    /// The bytes the function spans, for one the executable defines.
    pub(crate) size: Option<u64>,
}

/// Where a function's code is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FunctionRange {
    /// The file the code is in: the executable's or a library's name.
    pub(crate) image: String,
    /// The function's symbol, when the image names it; an import stub is
    /// `symbol stub for: <import>`.
    pub(crate) name: Option<String>,
    pub(crate) start: u64,
    /// The bytes it spans, when known.
    pub(crate) size: Option<u64>,
}

/// One register and its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Register {
    pub(crate) name: String,
    pub(crate) group: RegisterGroup,
    pub(crate) value: RegisterValue,
}

/// The `list reg` group a register belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisterGroup {
    /// `gpr`: the general-purpose registers.
    General,
    /// `simd`: the SIMD and floating-point registers.
    Simd,
    /// `sys`: the stack pointer, program counter and status registers.
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisterValue {
    Bits64(u64),
    Bits128(u128),
}

/// What [`Inferior::end`] did to the debuggee.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    Killed,
    Detached,
    /// The process was gone before the session ended (killed from outside).
    AlreadyExited,
}

/// The debugger for the platform this `mfb` was built for, or `None` where
/// `mfb dbg` is not supported.
pub(crate) fn host_backend() -> Option<&'static dyn Backend> {
    #[cfg(target_os = "macos")]
    {
        Some(&macos::MacOs)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Refuse `path` unless it is an MFBASIC executable built for this host. The
/// provenance note (`./mfb spec linker provenance-marker`) identifies an MFBASIC
/// binary; its container format and architecture must be the host's, since the
/// debugger only drives programs the host runs natively.
pub(crate) fn check_debuggable(path: &Path) -> Result<Program, String> {
    let shown = shown(path);
    let bytes = std::fs::read(path).map_err(|err| format!("cannot read '{shown}': {err}"))?;
    let Some(binary) = inspect(&bytes) else {
        return Err(format!("'{shown}' is not an MFBASIC executable"));
    };
    let (format, arch) = host_image();
    if binary.format != format || binary.arch != arch {
        return Err(format!(
            "'{shown}' is built for {} {}; this debugger runs {format} {arch} programs",
            binary.format,
            safe(&binary.arch)
        ));
    }
    Ok(Program {
        app: binary
            .libraries
            .iter()
            .any(|library| library.contains("AppKit.framework")),
    })
}

/// What the debugger knows about the program from its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Program {
    /// Built with `mfb build --app` (or as a Canvas program): the initial
    /// thread runs the window system and `main` runs on a thread of its own.
    pub(crate) app: bool,
}

/// `path` as it may be printed: a path read back from the system (the
/// executable of a PID) is not trusted to be free of terminal control codes.
pub(crate) fn shown(path: &Path) -> String {
    safe(&path.to_string_lossy()).into_owned()
}

/// The container format and architecture of a native executable on this host,
/// spelled as [`inspect`] reports them.
fn host_image() -> (&'static str, &'static str) {
    let format = match std::env::consts::OS {
        "macos" => "Mach-O",
        "windows" => "PE",
        _ => "ELF",
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86-64",
        other => other,
    };
    (format, arch)
}
