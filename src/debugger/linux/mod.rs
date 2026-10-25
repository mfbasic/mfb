//! The Linux debugger backend: ptrace. What differs by processor is in
//! `aarch64.rs`, which this code reads as `cpu::`.
//!
//! Every thread of the program is traced with `PTRACE_SEIZE`, with
//! `PTRACE_O_TRACECLONE` so each thread it starts later is traced from its
//! first instruction. A launched program is forked into a process group of its
//! own and stops itself before `execve`; once it is seized, the exec stops it
//! again (`PTRACE_EVENT_EXEC`) before the loader runs, so no instruction of the
//! program — not even `ld.so`'s — runs before the prompt. An attached program
//! is paused by `PTRACE_INTERRUPT` on every thread.
//!
//! At the prompt every thread is in a ptrace-stop. `run` resumes them all and
//! waits (`waitpid(-1, __WALL)`) for a thread to reach one of the debugger's
//! `brk`s (`SIGTRAP`, `TRAP_BRKPT`), be about to write watched memory
//! (`TRAP_HWBKPT`), or fault; then it interrupts every other thread and
//! reports what they all stopped for. A signal that is no fault (`SIGCHLD`,
//! `SIGPIPE`, a timer) is passed on to the program at once, unseen; a fault
//! is reported, and passed on when the program next runs, so its own handler
//! (or the default action) runs then.
//!
//! A step runs a set of threads in rounds of one instruction each with
//! `PTRACE_SINGLESTEP`, every other thread staying stopped; a thread that does
//! not finish its instruction in time because it is blocked in the kernel is
//! interrupted and reported as waiting. Registers are `NT_PRSTATUS` and
//! `NT_FPREGSET`; memory is `/proc/<pid>/mem`, through which a breakpoint's
//! `brk #0` is written over read-only code (the kernel makes the page a
//! private copy). A watchpoint is one of the processor's watchpoint register
//! pairs, written into every thread through `NT_ARM_HW_WATCH`; the kernel does
//! not copy them into a new thread, so each new thread gets them at its first
//! stop.
//!
//! The executable's functions come from its file (`super::elf`) plus the load
//! bias `/proc/<pid>/maps` shows; a library's from its own file, mapped where
//! `/proc/<pid>/maps` says.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_char, c_int, c_long, c_void, CString, OsString};
use std::fs::File;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};

use super::arch::Arch;
use super::elf::{self, Location};
use super::{
    Backend, Ending, Exit, FunctionRange, FunctionSymbol, Inferior, Register, RunStop, Step,
    StepStop, ThreadFacts,
};
// `process.rs` and `images.rs` keep their `super::shown` / `super::environment` calls from
// before the split.
use super::{environment, shown};

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "aarch64")]
use aarch64 as cpu;
#[cfg(target_arch = "riscv64")]
mod riscv64;
#[cfg(target_arch = "riscv64")]
use riscv64 as cpu;
mod images;
mod inferior;
mod process;
mod threads;
use cpu::{Float, General};
use process::*;

/// `<sys/ptrace.h>`.
const PTRACE_CONT: c_int = 7;
const PTRACE_SINGLESTEP: c_int = 9;
const PTRACE_DETACH: c_int = 17;
const PTRACE_GETEVENTMSG: c_int = 0x4201;
const PTRACE_GETSIGINFO: c_int = 0x4202;
const PTRACE_GETREGSET: c_int = 0x4204;
const PTRACE_SETREGSET: c_int = 0x4205;
const PTRACE_SEIZE: c_int = 0x4206;
const PTRACE_INTERRUPT: c_int = 0x4207;
const PTRACE_O_TRACECLONE: c_long = 0x8;
const PTRACE_O_TRACEEXEC: c_long = 0x10;
const PTRACE_O_EXITKILL: c_long = 0x10_0000;
const PTRACE_EVENT_CLONE: c_int = 3;
const PTRACE_EVENT_EXEC: c_int = 4;
const PTRACE_EVENT_STOP: c_int = 128;
/// `<elf.h>`: the register sets.
const NT_PRSTATUS: usize = 1;
const NT_FPREGSET: usize = 2;
/// `<sys/wait.h>`: wait for every child, threads included; do not block.
const WALL: c_int = 0x4000_0000;
const WNOHANG: c_int = 1;
const WUNTRACED: c_int = 2;
/// `<signal.h>`.
const SIGINT: c_int = 2;
const SIGILL: c_int = 4;
const SIGTRAP: c_int = 5;
const SIGBUS: c_int = 7;
const SIGFPE: c_int = 8;
const SIGKILL: c_int = 9;
const SIGSEGV: c_int = 11;
const SIGCONT: c_int = 18;
const SIGSTOP: c_int = 19;
/// `<signal.h>`: `si_code` of a `SIGTRAP` the kernel raised: a `brk`, a
/// single step, a watchpoint. One sent by a process has a code of 0 or less
/// and names the sender; the kernel's report of a single step over a system
/// call is `SI_USER` from no process (pid 0).
const SI_USER: c_int = 0;
const TRAP_BRKPT: c_int = 1;
const TRAP_TRACE: c_int = 2;
const TRAP_HWBKPT: c_int = 4;
/// `<asm-generic/unistd.h>`, the same on AArch64 and riscv64.
const SYS_GETTID: c_long = 178;
const SYS_TGKILL: c_long = 131;
const ESRCH: i32 = 3;
const EPERM: i32 = 1;
const EINTR: i32 = 4;
/// `<fcntl.h>`: `O_CLOEXEC`.
const O_CLOEXEC: c_int = 0o2_000_000;
/// A step waits for each thread in slices of this long, and gives up on one
/// still not done that is blocked in the kernel, or after the limit.
const STEP_SLICE: Duration = Duration::from_millis(2000);
const STEP_LIMIT: Duration = Duration::from_millis(30_000);
/// How long a poll for a thread's stop sleeps between looks.
const POLL_PAUSE: Duration = Duration::from_millis(1);
/// The exit status `fork`'s child reports when `execve` failed.
const EXEC_FAILED: i32 = 127;
/// The widest `struct pthread` a musl `pthread_self` search tries.
const MAX_PTHREAD_SIZE: u64 = 4096;
/// glibc's libthread_db description of `struct pthread`'s size.
const GLIBC_PTHREAD_SIZE: &str = "_thread_db_sizeof_pthread";

#[repr(C)]
struct IoVec {
    base: *mut c_void,
    len: usize,
}

/// `struct sigaction` as glibc and musl lay it out on AArch64 and riscv64:
/// the handler, a 128-byte mask, the flags at 136, the restorer.
#[repr(C)]
#[derive(Clone, Copy)]
struct SigAction {
    handler: usize,
    mask: [u64; 16],
    flags: c_int,
    restorer: usize,
}

extern "C" {
    fn ptrace(request: c_int, ...) -> c_long;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn kill(pid: c_int, signal: c_int) -> c_int;
    fn fork() -> c_int;
    fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char)
        -> c_int;
    fn _exit(status: c_int) -> !;
    fn setpgid(pid: c_int, group: c_int) -> c_int;
    fn getpid() -> c_int;
    fn pipe2(fds: *mut c_int, flags: c_int) -> c_int;
    fn read(fd: c_int, buffer: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buffer: *const c_void, count: usize) -> isize;
    fn close(fd: c_int) -> c_int;
    fn sigaction(signal: c_int, action: *const SigAction, old: *mut SigAction) -> c_int;
    fn syscall(number: c_long, ...) -> c_long;
    fn __errno_location() -> *mut c_int;
}

/// Set by the SIGINT handler; a `run` that sees it pauses the program.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// The thread the session runs on, which waits for the program. `mfb` runs a
/// command on a thread of its own, and the kernel may hand a SIGINT to any
/// thread, so the handler sends one that lands elsewhere on to this one,
/// whose blocking wait it then interrupts (`EINTR`).
static SESSION_THREAD: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_interrupt(signal: c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
    let session = SESSION_THREAD.load(Ordering::SeqCst);
    // SAFETY: `gettid`, `getpid` and `tgkill` are async-signal-safe system
    // calls; the signal is sent only to the session's own thread.
    unsafe {
        if session != 0 && syscall(SYS_GETTID) as c_int != session {
            syscall(
                SYS_TGKILL,
                getpid() as c_long,
                session as c_long,
                signal as c_long,
            );
        }
    }
}

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

pub(super) struct Linux;

impl Backend for Linux {
    fn executable_of(&self, pid: u32) -> Result<PathBuf, String> {
        let link = format!("/proc/{pid}/exe");
        std::fs::read_link(&link).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                format!("no process with PID {pid}")
            } else {
                format!("cannot find the executable of process {pid}: {error}")
            }
        })
    }

    fn launch(
        &self,
        program: &Path,
        args: &[OsString],
        environment: &[(OsString, OsString)],
    ) -> Result<Box<dyn Inferior>, String> {
        let pid = spawn(program, args, environment)?;
        Tracee::new(pid, Some(args.to_vec()), program, vec![pid]).map(|tracee| {
            let boxed: Box<dyn Inferior> = Box::new(tracee);
            boxed
        })
    }

    fn attach(&self, pid: u32, program: &Path) -> Result<Box<dyn Inferior>, String> {
        let pid = c_int::try_from(pid).map_err(|_| format!("no process with PID {pid}"))?;
        let tids = seize_all(pid)?;
        let mut tracee = Tracee::new(pid, None, program, tids)?;
        tracee.stop_all()?;
        Ok(Box::new(tracee))
    }
}

/// `ptrace(request, tid, addr, data)`, or the errno it failed with.
fn trace(request: c_int, tid: c_int, addr: usize, data: usize) -> Result<c_long, i32> {
    // SAFETY: every request this file makes passes an address and data the
    // kernel reads or writes as the request defines, and the buffers they
    // point at outlive the call.
    let result = unsafe { ptrace(request, tid, addr as *mut c_void, data as *mut c_void) };
    if result == -1 {
        let error = errno();
        if error != 0 {
            return Err(error);
        }
    }
    Ok(result)
}

fn os_error(error: i32) -> io::Error {
    io::Error::from_raw_os_error(error)
}

/// One thread of the program.
struct Thread {
    tid: c_int,
    /// In a ptrace-stop the debugger has seen and not resumed.
    stopped: bool,
    /// A signal to deliver when it next runs.
    pending: c_int,
    /// Started by the program since; its first stop has not been seen.
    fresh: bool,
}

/// What a stopped thread stopped for, when it is worth reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Report {
    Breakpoint(u64),
    Watch,
    Fault(String),
}

/// A program held with ptrace.
struct Tracee {
    pid: c_int,
    launched: bool,
    args: Vec<OsString>,
    program: PathBuf,
    ended: bool,
    memory: File,
    threads: Vec<Thread>,
    /// The executable's symbols, read on first use.
    image: Option<elf::Executable>,
    /// The load bias, found on first use; it cannot change while the
    /// process lives.
    bias: Option<u64>,
    /// Each breakpoint written into the program, with the instruction it
    /// replaced.
    breakpoints: BTreeMap<u64, Vec<u8>>,
    /// Each watchpoint's address and size; its index is its register pair.
    watchpoints: Vec<(u64, u64)>,
    /// How the program ended, once a wait saw it end.
    exited: Option<Exit>,
    /// The SIGINT action this process had before the session installed its
    /// own.
    previous_interrupt: SigAction,
    /// The libraries mapped, and the maps they were read from.
    libraries: Vec<Library>,
    maps: String,
    /// Each library file read, by path.
    files: HashMap<PathBuf, Option<elf::Elf>>,
    /// `pthread_self` is the thread pointer less this, once found.
    pthread_gap: Option<Option<u64>>,
    /// The breakpoints a step planted, with what they replaced.
    #[cfg(target_arch = "riscv64")]
    step_traps: Vec<(u64, Vec<u8>)>,
}

/// One line of `/proc/<pid>/maps`: `[start, end)`, the file offset mapped
/// there, and the file (empty for none).
struct Mapping {
    start: u64,
    end: u64,
    offset: u64,
    path: String,
}

/// One library mapped into the program.
struct Library {
    /// The file's name, as a frame shows it.
    name: String,
    path: PathBuf,
    /// The live address of the file's link-time address 0.
    bias: u64,
    /// Its code, live: `[start, end)`.
    code: Vec<(u64, u64)>,
    /// Loaded while the program ran (`dlopen`): neither the executable nor
    /// any library it was linked against needs it.
    loaded: bool,
}

impl Drop for Tracee {
    /// A session left without `quit` still ends the way it began.
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.end();
        }
    }
}
