//! The Linux debugger backend (AArch64): ptrace.
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

use super::arm64::{self, Float, General};
use super::elf::{self, Location};
use super::{
    Backend, Ending, Exit, FunctionRange, FunctionSymbol, Inferior, Register, RunStop, Step,
    StepStop, ThreadFacts,
};

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
const NT_ARM_TLS: usize = 0x401;
const NT_ARM_HW_WATCH: usize = 0x403;
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
/// `<asm-generic/unistd.h>` on AArch64.
const SYS_GETTID: c_long = 178;
const SYS_TGKILL: c_long = 131;
const ESRCH: i32 = 3;
const EPERM: i32 = 1;
const EINTR: i32 = 4;
/// `<fcntl.h>`: `O_CLOEXEC`.
const O_CLOEXEC: c_int = 0o2_000_000;
/// `brk #0`, the instruction a breakpoint writes.
const BREAKPOINT_INSTRUCTION: [u8; 4] = 0xd420_0000_u32.to_le_bytes();
/// `DBGWCR`: enabled, for EL0 (`PAC` 0b10), on stores (`LSC` 0b10); the
/// byte-address-select bits start at bit 5.
const WCR_STORE_EL0: u64 = 0x1 | (0b10 << 1) | (0b10 << 3);
const WCR_BAS_SHIFT: u64 = 5;
/// `struct user_hwdebug_state`: `dbg_info` and padding, then 16 pairs of
/// `addr`, `ctrl` and padding.
const HW_DEBUG_HEADER: usize = 8;
const HW_DEBUG_SLOT: usize = 16;
const HW_DEBUG_SLOTS: usize = 16;
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

/// `struct sigaction` as glibc and musl lay it out on AArch64.
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

pub(super) struct LinuxAarch64;

impl Backend for LinuxAarch64 {
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

/// The refusal for a `PTRACE_SEIZE` the kernel denied.
fn seize_refusal(pid: c_int, error: i32) -> String {
    if error == ESRCH {
        return format!("no process with PID {pid}");
    }
    let scope = std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
        .ok()
        .map(|text| text.trim().to_string());
    match (error, scope.as_deref()) {
        (EPERM, Some(scope)) if scope != "0" => format!(
            "cannot attach to process {pid}: ptrace: {}. This system lets a debugger attach \
             only to its own children (kernel.yama.ptrace_scope is {scope}); launch the program \
             with mfb dbg <binary> instead, or run mfb dbg as root",
            os_error(error)
        ),
        _ => format!(
            "cannot attach to process {pid}: ptrace: {}",
            os_error(error)
        ),
    }
}

/// The thread ids of process `pid`, leader first, then ascending.
fn task_ids(pid: c_int) -> Result<Vec<c_int>, String> {
    let entries = std::fs::read_dir(format!("/proc/{pid}/task")).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            format!("no process with PID {pid}")
        } else {
            format!("cannot list the threads of process {pid}: {error}")
        }
    })?;
    let mut tids = entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<c_int>().ok())
        .collect::<Vec<_>>();
    tids.sort_by_key(|tid| (*tid != pid, *tid));
    Ok(tids)
}

/// Seize every thread of `pid`, again until no new one has appeared.
fn seize_all(pid: c_int) -> Result<Vec<c_int>, String> {
    let mut seized: Vec<c_int> = Vec::new();
    loop {
        let fresh = task_ids(pid)?
            .into_iter()
            .filter(|tid| !seized.contains(tid))
            .collect::<Vec<_>>();
        if fresh.is_empty() {
            return Ok(seized);
        }
        for tid in fresh {
            match trace(PTRACE_SEIZE, tid, 0, PTRACE_O_TRACECLONE as usize) {
                Ok(_) => seized.push(tid),
                // A thread that ended meanwhile is no thread.
                Err(ESRCH) if tid != pid => {}
                Err(error) => {
                    for tid in &seized {
                        let _ = trace(PTRACE_DETACH, *tid, 0, 0);
                    }
                    return Err(seize_refusal(pid, error));
                }
            }
        }
    }
}

/// Start `program` with `args` and `environment`, paused at its `execve`
/// before the loader runs, in a process group of its own, and seize it.
fn spawn(
    program: &Path,
    args: &[OsString],
    environment: &[(OsString, OsString)],
) -> Result<c_int, String> {
    let shown = super::shown(program);
    let c_string = |value: &[u8]| {
        CString::new(value)
            .map_err(|_| format!("cannot launch '{shown}': an argument contains a NUL byte"))
    };
    let path = c_string(program.as_os_str().as_bytes())?;
    let mut owned = vec![path.clone()];
    for arg in args {
        owned.push(c_string(arg.as_bytes())?);
    }
    let mut argv = owned.iter().map(|arg| arg.as_ptr()).collect::<Vec<_>>();
    argv.push(ptr::null());
    let entries = super::environment::entries(environment)
        .map_err(|message| format!("cannot launch '{shown}': {message}"))?;
    let mut envp = entries
        .iter()
        .map(|entry| entry.as_ptr())
        .collect::<Vec<_>>();
    envp.push(ptr::null());
    // A pipe the child writes `execve`'s errno into; close-on-exec, so a
    // successful exec closes it unwritten.
    let mut fds = [0 as c_int; 2];
    // SAFETY: `fds` holds the two descriptors `pipe2` writes.
    if unsafe { pipe2(fds.as_mut_ptr(), O_CLOEXEC) } != 0 {
        return Err(format!(
            "cannot launch '{shown}': {}",
            io::Error::last_os_error()
        ));
    }
    // SAFETY: the child only calls async-signal-safe functions on memory
    // prepared before the fork, then execs or exits.
    let pid = unsafe { fork() };
    if pid == 0 {
        // SAFETY: as above; nothing here allocates or takes a lock.
        unsafe {
            close(fds[0]);
            setpgid(0, 0);
            kill(getpid(), SIGSTOP);
            execve(path.as_ptr(), argv.as_ptr(), envp.as_ptr());
            let error = *__errno_location();
            write(
                fds[1],
                ptr::addr_of!(error).cast(),
                std::mem::size_of::<c_int>(),
            );
            _exit(EXEC_FAILED);
        }
    }
    // SAFETY: closes this process's copy of the write end.
    unsafe {
        close(fds[1]);
    }
    if pid < 0 {
        // SAFETY: closes the read end opened above.
        unsafe {
            close(fds[0]);
        }
        return Err(format!(
            "cannot launch '{shown}': fork: {}",
            io::Error::last_os_error()
        ));
    }
    let outcome = seize_launched(pid, &shown, fds[0]);
    // SAFETY: closes the read end opened above.
    unsafe {
        close(fds[0]);
    }
    if outcome.is_err() {
        kill_and_reap(pid);
    }
    outcome.map(|()| pid)
}

/// Seize the stopped child `pid`, let it run to its `execve`, and wait for the
/// exec to stop it.
fn seize_launched(pid: c_int, shown: &str, errors: c_int) -> Result<(), String> {
    let mut status = 0;
    // SAFETY: `status` is a valid out-parameter; `pid` is this process's child.
    let waited = unsafe { waitpid(pid, &mut status, WUNTRACED) };
    if waited != pid || !stopped(status) {
        return Err(format!("cannot launch '{shown}': it did not start"));
    }
    let options = PTRACE_O_TRACECLONE | PTRACE_O_TRACEEXEC | PTRACE_O_EXITKILL;
    trace(PTRACE_SEIZE, pid, 0, options as usize)
        .map_err(|error| format!("cannot launch '{shown}': ptrace: {}", os_error(error)))?;
    // SAFETY: `pid` is this process's own child.
    unsafe {
        kill(pid, SIGCONT);
    }
    loop {
        // SAFETY: as above.
        let waited = unsafe { waitpid(pid, &mut status, WALL) };
        if waited != pid {
            if errno() == EINTR {
                continue;
            }
            return Err(format!(
                "cannot launch '{shown}': {}",
                io::Error::last_os_error()
            ));
        }
        if exited(status) || signaled(status) {
            let mut error: c_int = 0;
            // SAFETY: `error` is writable for the bytes read.
            let got = unsafe {
                read(
                    errors,
                    ptr::addr_of_mut!(error).cast(),
                    std::mem::size_of::<c_int>(),
                )
            };
            return Err(if got == std::mem::size_of::<c_int>() as isize {
                format!("cannot launch '{shown}': {}", os_error(error))
            } else {
                format!("cannot launch '{shown}': it ended before it started")
            });
        }
        match (event(status), stop_signal(status)) {
            (PTRACE_EVENT_EXEC, _) => return Ok(()),
            // The stop it made itself, or the continue that ended it.
            (PTRACE_EVENT_STOP, _) | (0, SIGCONT) | (0, SIGSTOP) => {
                trace(PTRACE_CONT, pid, 0, 0).map_err(|error| {
                    format!("cannot launch '{shown}': ptrace: {}", os_error(error))
                })?;
            }
            (_, signal) => {
                trace(PTRACE_CONT, pid, 0, signal as usize).map_err(|error| {
                    format!("cannot launch '{shown}': ptrace: {}", os_error(error))
                })?;
            }
        }
    }
}

fn stopped(status: c_int) -> bool {
    status & 0xff == 0x7f
}
fn exited(status: c_int) -> bool {
    status & 0x7f == 0
}
fn signaled(status: c_int) -> bool {
    !stopped(status) && !exited(status)
}
fn stop_signal(status: c_int) -> c_int {
    (status >> 8) & 0xff
}
fn event(status: c_int) -> c_int {
    (status >> 16) & 0xffff
}

/// Kill the child `pid` and reap it. Each of its threads the debugger traces
/// reports its own end first, and the leader can be reaped only after them,
/// so every end is waited for until the leader's.
fn kill_and_reap(pid: c_int) {
    // SAFETY: `pid` is this process's own child, and every thread waited for
    // is one of its threads (the debugger has no other children).
    unsafe {
        kill(pid, SIGKILL);
        loop {
            let mut status = 0;
            let waited = waitpid(-1, &mut status, WALL);
            if waited == pid && !stopped(status) {
                break;
            }
            if waited == -1 && errno() != EINTR {
                break;
            }
        }
    }
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
    breakpoints: BTreeMap<u64, [u8; 4]>,
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

impl Tracee {
    fn new(
        pid: c_int,
        args: Option<Vec<OsString>>,
        program: &Path,
        tids: Vec<c_int>,
    ) -> Result<Self, String> {
        let memory = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!("/proc/{pid}/mem"))
            .map_err(|error| format!("cannot open the memory of process {pid}: {error}"))?;
        let action = SigAction {
            handler: on_interrupt as *const () as usize,
            mask: [0; 16],
            flags: 0,
            restorer: 0,
        };
        let mut previous_interrupt = action;
        // SAFETY: `on_interrupt` only stores to atomics and makes
        // async-signal-safe system calls; without `SA_RESTART` a wait it
        // interrupts returns `EINTR`, so a `run` sees Ctrl-C. The previous
        // action is put back in `end`.
        unsafe {
            SESSION_THREAD.store(syscall(SYS_GETTID) as c_int, Ordering::SeqCst);
            sigaction(SIGINT, &action, &mut previous_interrupt);
        }
        let launched = args.is_some();
        Ok(Tracee {
            pid,
            launched,
            args: args.unwrap_or_default(),
            program: program.to_path_buf(),
            ended: false,
            memory,
            threads: tids
                .into_iter()
                .map(|tid| Thread {
                    tid,
                    stopped: launched,
                    pending: 0,
                    fresh: false,
                })
                .collect(),
            image: None,
            bias: None,
            breakpoints: BTreeMap::new(),
            watchpoints: Vec::new(),
            exited: None,
            previous_interrupt,
            libraries: Vec::new(),
            maps: String::new(),
            files: HashMap::new(),
            pthread_gap: None,
        })
    }

    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let mut buffer = vec![0u8; length];
        let failure = |detail: String| {
            format!(
                "cannot read process {}'s memory at 0x{address:016x}: {detail}",
                self.pid
            )
        };
        match self.memory.read_at(&mut buffer, address) {
            Ok(read) if read == length => Ok(buffer),
            Ok(_) => Err(failure("the memory ends".to_string())),
            Err(error) => Err(failure(error.to_string())),
        }
    }

    fn write(&self, address: u64, bytes: &[u8]) -> Result<(), String> {
        match self.memory.write_at(bytes, address) {
            Ok(written) if written == bytes.len() => Ok(()),
            Ok(_) => Err(format!(
                "cannot change the instruction at 0x{address:016x}: the memory ends"
            )),
            Err(error) => Err(format!(
                "cannot change the instruction at 0x{address:016x}: {error}"
            )),
        }
    }

    fn image(&mut self) -> Result<&elf::Executable, String> {
        if self.image.is_none() {
            let shown = super::shown(&self.program);
            let bytes = std::fs::read(&self.program)
                .map_err(|err| format!("cannot read '{shown}': {err}"))?;
            let image = elf::executable(&bytes)
                .ok_or_else(|| format!("cannot read the symbol table of '{shown}'"))?;
            self.image = Some(image);
        }
        Ok(self.image.as_ref().expect("read above"))
    }

    /// `/proc/<pid>/maps`: its text, and each mapping in it.
    fn mappings(&self) -> Result<(String, Vec<Mapping>), String> {
        let text =
            std::fs::read_to_string(format!("/proc/{}/maps", self.pid)).map_err(|error| {
                format!(
                    "cannot read the memory map of process {}: {error}",
                    self.pid
                )
            })?;
        let mut mappings = Vec::new();
        for line in text.lines() {
            let mut fields = line.splitn(6, ' ');
            let (Some(span), _, Some(offset), _, _) = (
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
            ) else {
                continue;
            };
            let path = fields.next().unwrap_or("").trim_start().to_string();
            let Some((start, end)) = span.split_once('-') else {
                continue;
            };
            let (Ok(start), Ok(end), Ok(offset)) = (
                u64::from_str_radix(start, 16),
                u64::from_str_radix(end, 16),
                u64::from_str_radix(offset, 16),
            ) else {
                continue;
            };
            mappings.push(Mapping {
                start,
                end,
                offset,
                path,
            });
        }
        Ok((text, mappings))
    }

    /// The executable's load bias: where its first loaded page is, less that
    /// page's link-time address.
    fn bias(&mut self) -> Result<u64, String> {
        if let Some(bias) = self.bias {
            return Ok(bias);
        }
        let first = self
            .image()?
            .elf
            .segments
            .first()
            .copied()
            .ok_or("the executable maps no segment")?;
        let executable = std::fs::read_link(format!("/proc/{}/exe", self.pid))
            .unwrap_or_else(|_| self.program.clone());
        let (_, mappings) = self.mappings()?;
        let page = first.offset & !0xfff;
        let start = mappings
            .iter()
            .find(|mapping| mapping.offset == page && Path::new(&mapping.path) == executable)
            .map(|mapping| mapping.start)
            .ok_or_else(|| {
                format!(
                    "cannot find the executable in the memory map of process {}",
                    self.pid
                )
            })?;
        let bias = start.wrapping_sub(first.vaddr & !0xfff);
        self.bias = Some(bias);
        Ok(bias)
    }

    /// Re-read the libraries when the memory map has changed.
    fn refresh_libraries(&mut self) -> Result<(), String> {
        let (text, mappings) = self.mappings()?;
        if text == self.maps {
            return Ok(());
        }
        let executable = std::fs::read_link(format!("/proc/{}/exe", self.pid))
            .unwrap_or_else(|_| self.program.clone());
        let mut libraries: Vec<Library> = Vec::new();
        for Mapping {
            start,
            offset,
            path,
            ..
        } in &mappings
        {
            if !path.starts_with('/') || Path::new(path) == executable || *offset != 0 {
                continue;
            }
            let path = PathBuf::from(path);
            if libraries.iter().any(|library| library.path == path) {
                continue;
            }
            let Some(elf) = self.library_file(&path) else {
                continue;
            };
            let Some(first) = elf.segments.first() else {
                continue;
            };
            let bias = start.wrapping_sub(first.vaddr & !0xfff);
            let code = elf
                .code()
                .into_iter()
                .map(|(begin, end)| (begin.wrapping_add(bias), end.wrapping_add(bias)))
                .collect();
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            libraries.push(Library {
                name,
                path,
                bias,
                code,
                loaded: false,
            });
        }
        // Every library the executable's needs reach was loaded with it, and
        // so was the loader; any other was loaded while the program ran.
        let interpreter = self.image()?.elf.interpreter.clone();
        let mut reached = self
            .image()?
            .elf
            .needed
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        loop {
            let before = reached.len();
            for library in &libraries {
                let Some(Some(elf)) = self.files.get(&library.path) else {
                    continue;
                };
                let soname = elf.soname.clone().unwrap_or_else(|| library.name.clone());
                if reached.contains(&soname) {
                    reached.extend(elf.needed.iter().cloned());
                }
            }
            if reached.len() == before {
                break;
            }
        }
        for library in &mut libraries {
            let soname = self
                .files
                .get(&library.path)
                .and_then(|file| file.as_ref()?.soname.clone())
                .unwrap_or_else(|| library.name.clone());
            let is_loader = interpreter.as_deref().is_some_and(|loader| {
                Path::new(loader).file_name() == library.path.file_name()
                    || Path::new(loader) == library.path
            });
            library.loaded = !reached.contains(&soname) && !is_loader;
        }
        self.libraries = libraries;
        self.maps = text;
        Ok(())
    }

    /// A library's file, read on first use (through the program's own root
    /// when the path does not resolve here).
    fn library_file(&mut self, path: &Path) -> Option<elf::Elf> {
        if let Some(file) = self.files.get(path) {
            return file.clone();
        }
        let bytes = std::fs::read(path).ok().or_else(|| {
            std::fs::read(
                Path::new(&format!("/proc/{}/root", self.pid)).join(path.strip_prefix("/").ok()?),
            )
            .ok()
        });
        let elf = bytes.as_deref().and_then(elf::read);
        self.files.insert(path.to_path_buf(), elf.clone());
        elf
    }

    /// The functions the libraries the program loaded while running export,
    /// by address.
    fn loaded_functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        self.refresh_libraries()?;
        let mut functions = Vec::new();
        for library in &self.libraries {
            if !library.loaded {
                continue;
            }
            let Some(Some(elf)) = self.files.get(&library.path) else {
                continue;
            };
            functions.extend(
                elf.defined_functions()
                    .into_iter()
                    .filter(|(.., exported)| *exported)
                    .map(|(name, address, ..)| FunctionSymbol {
                        name,
                        linked: true,
                        loaded: true,
                        address: Some(address.wrapping_add(library.bias)),
                        size: None,
                    }),
            );
        }
        functions.sort_by_key(|function| function.address);
        Ok(functions)
    }

    /// The function of library `index` holding the live `address`.
    fn library_function_at(&self, index: usize, address: u64) -> Option<FunctionRange> {
        let library = &self.libraries[index];
        let elf = self.files.get(&library.path)?.as_ref()?;
        let at = address.wrapping_sub(library.bias);
        if let Some((start, name)) = elf.plt_entry(at) {
            return Some(FunctionRange {
                image: library.name.clone(),
                name: Some(format!("symbol stub for: {name}")),
                start: start.wrapping_add(library.bias),
                size: Some(16),
            });
        }
        let functions = elf.defined_functions();
        let holder = functions
            .iter()
            .filter(|(_, start, size, _)| *start <= at && at < start.saturating_add(*size))
            .max_by_key(|(_, start, ..)| *start)
            .or_else(|| {
                // A symbol without a size runs up to the next one.
                let before = functions
                    .iter()
                    .filter(|(_, start, ..)| *start <= at)
                    .max_by_key(|(_, start, ..)| *start)?;
                (before.2 == 0).then_some(before)
            })?;
        Some(FunctionRange {
            image: library.name.clone(),
            name: Some(holder.0.clone()),
            start: holder.1.wrapping_add(library.bias),
            size: (holder.2 > 0).then_some(holder.2),
        })
    }

    /// One register set of thread `tid`.
    fn regset<T: Copy>(&self, tid: c_int, kind: usize, empty: T) -> Result<T, String> {
        let mut value = empty;
        let mut vector = IoVec {
            base: ptr::addr_of_mut!(value).cast(),
            len: std::mem::size_of::<T>(),
        };
        trace(
            PTRACE_GETREGSET,
            tid,
            kind,
            ptr::addr_of_mut!(vector) as usize,
        )
        .map_err(|error| {
            format!(
                "cannot read thread {tid}'s registers: ptrace: {}",
                os_error(error)
            )
        })?;
        Ok(value)
    }

    fn set_regset<T: Copy>(
        &self,
        tid: c_int,
        kind: usize,
        value: &T,
        what: &str,
    ) -> Result<(), String> {
        let mut copy = *value;
        let mut vector = IoVec {
            base: ptr::addr_of_mut!(copy).cast(),
            len: std::mem::size_of::<T>(),
        };
        trace(
            PTRACE_SETREGSET,
            tid,
            kind,
            ptr::addr_of_mut!(vector) as usize,
        )
        .map(|_| ())
        .map_err(|error| {
            format!(
                "cannot set thread {tid}'s {what}: ptrace: {}",
                os_error(error)
            )
        })
    }

    fn general(&self, tid: c_int) -> Result<General, String> {
        self.regset(tid, NT_PRSTATUS, [0u64; 34])
    }

    fn float(&self, tid: c_int) -> Result<Float, String> {
        self.regset(tid, NT_FPREGSET, Float::zeroed())
    }

    /// Thread `tid`'s `siginfo_t`: `si_code`, and the word at offset 16 — the
    /// faulting address, or `si_pid` (low half) and `si_uid` for a signal a
    /// process sent.
    fn siginfo(&self, tid: c_int) -> (c_int, u64) {
        let mut info = [0u64; 16];
        match trace(PTRACE_GETSIGINFO, tid, 0, info.as_mut_ptr() as usize) {
            Ok(_) => ((info[1] & 0xffff_ffff) as u32 as c_int, info[2]),
            Err(_) => (0, 0),
        }
    }

    /// How many watchpoint register pairs the processor has.
    fn watch_slots(&self, tid: c_int) -> Result<usize, String> {
        let state = self.regset(
            tid,
            NT_ARM_HW_WATCH,
            [0u8; HW_DEBUG_HEADER + HW_DEBUG_SLOT * HW_DEBUG_SLOTS],
        )?;
        Ok(usize::from(state[0]).min(HW_DEBUG_SLOTS))
    }

    /// Write the watchpoints into thread `tid`, enabled or all off.
    fn set_thread_watchpoints(&self, tid: c_int, on: bool) -> Result<(), String> {
        let mut state = [0u8; HW_DEBUG_HEADER + HW_DEBUG_SLOT * HW_DEBUG_SLOTS];
        for (slot, &(address, size)) in self.watchpoints.iter().enumerate().take(HW_DEBUG_SLOTS) {
            let select = ((1u64 << size) - 1) << (address & 7);
            let control = WCR_STORE_EL0 | (select << WCR_BAS_SHIFT);
            let at = HW_DEBUG_HEADER + slot * HW_DEBUG_SLOT;
            state[at..at + 8].copy_from_slice(&(address & !7).to_le_bytes());
            let control = if on { control } else { control & !1 } as u32;
            state[at + 8..at + 12].copy_from_slice(&control.to_le_bytes());
        }
        // Every slot the processor has is written, so a removed watchpoint's
        // slot is cleared.
        let slots = self.watch_slots(tid)?;
        let mut vector = IoVec {
            base: state.as_mut_ptr().cast(),
            len: HW_DEBUG_HEADER + HW_DEBUG_SLOT * slots,
        };
        trace(
            PTRACE_SETREGSET,
            tid,
            NT_ARM_HW_WATCH,
            ptr::addr_of_mut!(vector) as usize,
        )
        .map(|_| ())
        .map_err(|error| {
            format!(
                "cannot set the program's watchpoints: ptrace: {}",
                os_error(error)
            )
        })
    }

    /// Write the watchpoints into every thread.
    fn arm_watchpoints(&self) -> Result<(), String> {
        for thread in &self.threads {
            if !thread.fresh {
                self.set_thread_watchpoints(thread.tid, true)?;
            }
        }
        Ok(())
    }

    /// Put every breakpoint's original instruction back, for a step or a
    /// detach; [`Tracee::reinsert`] writes them again.
    fn lift(&self) -> Result<(), String> {
        for (address, original) in &self.breakpoints {
            self.write(*address, original)?;
        }
        Ok(())
    }

    fn reinsert(&self) -> Result<(), String> {
        for address in self.breakpoints.keys() {
            self.write(*address, &BREAKPOINT_INSTRUCTION)?;
        }
        Ok(())
    }

    fn index_of(&self, tid: c_int) -> Option<usize> {
        self.threads.iter().position(|thread| thread.tid == tid)
    }

    fn thread_at(&self, index: usize) -> Result<c_int, String> {
        self.threads
            .get(index)
            .map(|thread| thread.tid)
            .ok_or_else(|| {
                format!(
                    "no thread {index}: process {} has {} threads",
                    self.pid,
                    self.threads.len()
                )
            })
    }

    /// Resume thread `index`, delivering its pending signal unless it steps.
    fn resume(&mut self, index: usize, step: bool) -> Result<(), String> {
        let thread = &mut self.threads[index];
        let (request, signal) = if step {
            (PTRACE_SINGLESTEP, 0)
        } else {
            (PTRACE_CONT, std::mem::take(&mut thread.pending))
        };
        thread.stopped = false;
        match trace(request, thread.tid, 0, signal as usize) {
            Ok(_) => Ok(()),
            // A thread that ended meanwhile reports its end to the wait.
            Err(ESRCH) => Ok(()),
            Err(error) => Err(format!(
                "cannot resume thread {}: ptrace: {}",
                thread.tid,
                os_error(error)
            )),
        }
    }

    /// Wait for a thread to change: blocking, or `None` at once when none has
    /// and `block` is false. `Err` is the errno (`EINTR` for Ctrl-C).
    fn wait_any(&self, block: bool) -> Result<Option<(c_int, c_int)>, i32> {
        let mut status = 0;
        let options = WALL | if block { 0 } else { WNOHANG };
        // SAFETY: `status` is a valid out-parameter.
        let tid = unsafe { waitpid(-1, &mut status, options) };
        match tid {
            0 => Ok(None),
            tid if tid > 0 => Ok(Some((tid, status))),
            _ => Err(errno()),
        }
    }

    /// A thread ended. Answers whether the program has.
    fn thread_ended(&mut self, tid: c_int, status: c_int) -> bool {
        if let Some(index) = self.index_of(tid) {
            self.threads.remove(index);
        }
        if tid == self.pid {
            self.exited = Some(if exited(status) {
                Exit::Status((status >> 8) & 0xff)
            } else {
                Exit::Signal(status & 0x7f)
            });
            self.threads.clear();
            return true;
        }
        false
    }

    /// A thread the debugger did not know reported: one the program started.
    fn adopt(&mut self, tid: c_int) -> usize {
        if let Some(index) = self.index_of(tid) {
            return index;
        }
        self.threads.push(Thread {
            tid,
            stopped: false,
            pending: 0,
            fresh: true,
        });
        self.threads.len() - 1
    }

    /// A new thread's first stop: give it the watchpoints.
    fn welcome(&mut self, index: usize) -> Result<(), String> {
        self.threads[index].fresh = false;
        if !self.watchpoints.is_empty() {
            self.set_thread_watchpoints(self.threads[index].tid, true)?;
        }
        Ok(())
    }

    /// What a signal-delivery-stop of thread `index` for `signal` is: a report,
    /// or a signal passed on to the program.
    fn signal_stop(&mut self, index: usize, signal: c_int) -> Result<Option<Report>, String> {
        let tid = self.threads[index].tid;
        let (code, address) = self.siginfo(tid);
        let report = match signal {
            SIGTRAP if code == TRAP_HWBKPT && !self.watchpoints.is_empty() => Some(Report::Watch),
            SIGTRAP if code == TRAP_BRKPT => {
                let pc = self.general(tid)?[32];
                if self.breakpoints.contains_key(&pc) {
                    Some(Report::Breakpoint(pc))
                } else {
                    self.threads[index].pending = SIGTRAP;
                    Some(Report::Fault("a breakpoint instruction".to_string()))
                }
            }
            // The debugger's own: a step's trap (one that finished as the
            // thread was interrupted in the kernel, or a step over a system
            // call, which the kernel reports as `SI_USER` from pid 0), or a
            // watchpoint's since removed. Passed on, it would end the program.
            SIGTRAP
                if code == TRAP_TRACE
                    || code == TRAP_HWBKPT
                    || (code == SI_USER && address as u32 == 0) =>
            {
                None
            }
            SIGSEGV | SIGBUS => {
                self.threads[index].pending = signal;
                Some(Report::Fault(format!(
                    "a bad memory access at 0x{address:016x}"
                )))
            }
            SIGILL => {
                self.threads[index].pending = signal;
                Some(Report::Fault("an illegal instruction".to_string()))
            }
            SIGFPE => {
                self.threads[index].pending = signal;
                Some(Report::Fault("an arithmetic exception".to_string()))
            }
            // A stop signal is the debugger's business, not the program's.
            SIGSTOP => None,
            other => {
                self.threads[index].pending = other;
                None
            }
        };
        Ok(report)
    }

    /// Interrupt every running thread and wait until each is stopped,
    /// collecting what any stopped for on its own meanwhile.
    fn stop_all(&mut self) -> Result<Vec<(c_int, Report)>, String> {
        for thread in &self.threads {
            if !thread.stopped {
                let _ = trace(PTRACE_INTERRUPT, thread.tid, 0, 0);
            }
        }
        let mut reports = Vec::new();
        while self.threads.iter().any(|thread| !thread.stopped) {
            let (tid, status) = match self.wait_any(true) {
                Ok(Some(found)) => found,
                Ok(None) => continue,
                Err(EINTR) => continue,
                Err(error) => {
                    return Err(format!(
                        "cannot wait for process {}: {}",
                        self.pid,
                        os_error(error)
                    ))
                }
            };
            if !stopped(status) {
                if self.thread_ended(tid, status) {
                    return Ok(reports);
                }
                continue;
            }
            let known = self.index_of(tid).is_some();
            let index = self.adopt(tid);
            self.threads[index].stopped = true;
            match event(status) {
                // The new thread makes a first stop of its own.
                PTRACE_EVENT_CLONE => {
                    let mut child: c_long = 0;
                    if trace(
                        PTRACE_GETEVENTMSG,
                        tid,
                        0,
                        ptr::addr_of_mut!(child) as usize,
                    )
                    .is_ok()
                    {
                        self.adopt(child as c_int);
                    }
                }
                PTRACE_EVENT_STOP => {
                    if !known || self.threads[index].fresh {
                        self.welcome(index)?;
                    }
                }
                0 => {
                    if let Some(report) = self.signal_stop(index, stop_signal(status))? {
                        reports.push((tid, report));
                    }
                }
                _ => {}
            }
        }
        Ok(reports)
    }

    /// The `RunStop` reports amount to: every breakpoint hit, else every
    /// thread about to write watched memory, else the first fault.
    fn classify(&self, reports: &[(c_int, Report)]) -> RunStop {
        let index = |tid: &c_int| self.index_of(*tid).unwrap_or(usize::MAX);
        let hits = reports
            .iter()
            .filter_map(|(tid, report)| match report {
                Report::Breakpoint(address) => Some((index(tid), *address)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !hits.is_empty() {
            return RunStop::Breakpoints(hits);
        }
        let mut watched = reports
            .iter()
            .filter(|(_, report)| *report == Report::Watch)
            .map(|(tid, _)| index(tid))
            .collect::<Vec<_>>();
        watched.dedup();
        if !watched.is_empty() {
            return RunStop::Watchpoints(watched);
        }
        match reports.first() {
            Some((tid, Report::Fault(what))) => RunStop::Exception {
                thread: index(tid),
                what: what.clone(),
            },
            _ => RunStop::Interrupted,
        }
    }

    /// Collect any end the program has reported without waiting.
    fn check_exit(&mut self) -> Option<Exit> {
        if self.exited.is_none() {
            while let Ok(Some((tid, status))) = self.wait_any(false) {
                if !stopped(status) {
                    if self.thread_ended(tid, status) {
                        break;
                    }
                } else if let Some(index) = self.index_of(tid) {
                    // A stop seen here waits to be handled by the next run.
                    self.threads[index].stopped = true;
                }
            }
        }
        self.exited
    }

    /// Whether thread `tid` is blocked in the kernel.
    fn blocked(&self, tid: c_int) -> bool {
        std::fs::read_to_string(format!("/proc/{}/task/{tid}/stat", self.pid))
            .ok()
            .and_then(|text| {
                let state = text.rsplit_once(')')?.1.trim_start().chars().next()?;
                Some(matches!(state, 'S' | 'D'))
            })
            .unwrap_or(false)
    }

    /// Run `stepping` (thread indices) for up to `count` rounds of one
    /// instruction each; breakpoints are lifted and watchpoints off.
    fn step_rounds(&mut self, stepping: &[usize], count: u64) -> Result<Vec<Step>, String> {
        let tids = stepping
            .iter()
            .map(|index| self.threads[*index].tid)
            .collect::<Vec<_>>();
        let mut steps = vec![
            Step {
                taken: 0,
                stop: StepStop::Finished,
            };
            tids.len()
        ];
        let mut active = (0..tids.len()).collect::<Vec<_>>();
        for _ in 0..count {
            if active.is_empty() {
                break;
            }
            for at in &active {
                if let Some(index) = self.index_of(tids[*at]) {
                    self.resume(index, true)?;
                }
            }
            let started = Instant::now();
            let mut deadline = started + STEP_SLICE;
            let mut still = Vec::new();
            let mut waiting = active.clone();
            while !waiting.is_empty() {
                let found = match self.wait_any(false) {
                    Ok(found) => found,
                    Err(EINTR) => None,
                    Err(error) => {
                        return Err(format!(
                            "cannot wait for process {}: {}",
                            self.pid,
                            os_error(error)
                        ))
                    }
                };
                let Some((tid, status)) = found else {
                    if Instant::now() >= deadline {
                        let blocked = waiting.iter().all(|at| self.blocked(tids[*at]));
                        if blocked || started.elapsed() >= STEP_LIMIT {
                            for at in &waiting {
                                let _ = trace(PTRACE_INTERRUPT, tids[*at], 0, 0);
                            }
                            for (tid, report) in self.stop_all()? {
                                if let (Some(at), Report::Fault(what)) =
                                    (tids.iter().position(|other| *other == tid), report)
                                {
                                    steps[at].stop = StepStop::Exception(what);
                                }
                            }
                            for at in waiting.drain(..) {
                                if steps[at].stop == StepStop::Finished {
                                    steps[at].stop = StepStop::Waiting;
                                }
                            }
                            if self.exited.is_some() {
                                for step in &mut steps {
                                    step.stop = StepStop::Exited;
                                }
                                return Ok(steps);
                            }
                            break;
                        }
                        deadline = Instant::now() + STEP_SLICE;
                    }
                    std::thread::sleep(POLL_PAUSE);
                    continue;
                };
                if !stopped(status) {
                    if self.thread_ended(tid, status) {
                        for step in &mut steps {
                            step.stop = StepStop::Exited;
                        }
                        return Ok(steps);
                    }
                    if let Some(at) = tids.iter().position(|other| *other == tid) {
                        steps[at].stop = StepStop::Exited;
                        waiting.retain(|other| *other != at);
                    }
                    continue;
                }
                let known = self.index_of(tid).is_some();
                let index = self.adopt(tid);
                self.threads[index].stopped = true;
                let Some(at) = tids.iter().position(|other| *other == tid) else {
                    // A new thread's first stop, or a stray one: it stays
                    // stopped for the rest of the step.
                    if event(status) == PTRACE_EVENT_STOP && (!known || self.threads[index].fresh) {
                        self.welcome(index)?;
                    }
                    continue;
                };
                match event(status) {
                    PTRACE_EVENT_CLONE => {
                        let mut child: c_long = 0;
                        if trace(
                            PTRACE_GETEVENTMSG,
                            tid,
                            0,
                            ptr::addr_of_mut!(child) as usize,
                        )
                        .is_ok()
                        {
                            self.adopt(child as c_int);
                        }
                        // The system call goes on to finish the instruction.
                        self.resume(index, true)?;
                    }
                    PTRACE_EVENT_STOP => self.resume(index, true)?,
                    0 => match stop_signal(status) {
                        SIGTRAP => {
                            let (code, _) = self.siginfo(tid);
                            if code == TRAP_BRKPT {
                                steps[at].stop =
                                    StepStop::Exception("a breakpoint instruction".to_string());
                                self.threads[index].pending = SIGTRAP;
                            } else {
                                steps[at].taken += 1;
                                still.push(at);
                            }
                            waiting.retain(|other| *other != at);
                        }
                        signal => match self.signal_stop(index, signal)? {
                            Some(Report::Fault(what)) => {
                                steps[at].stop = StepStop::Exception(what);
                                waiting.retain(|other| *other != at);
                            }
                            // Another signal waits for the next run; the
                            // instruction is tried again.
                            _ => self.resume(index, true)?,
                        },
                    },
                    _ => self.resume(index, true)?,
                }
            }
            active = still;
        }
        Ok(steps)
    }

    /// The thread pointer less `pthread_self`: glibc's from the size its
    /// libthread_db description gives `struct pthread`; musl's from the
    /// descriptor's first word, which points at the descriptor itself.
    fn pthread_gap(&mut self, tp: u64) -> Option<u64> {
        if let Some(gap) = self.pthread_gap {
            return gap;
        }
        if tp == 0 {
            return None;
        }
        let musl = self
            .image()
            .ok()?
            .elf
            .interpreter
            .as_deref()
            .is_some_and(|loader| loader.contains("musl"));
        let gap = if musl {
            (1..=MAX_PTHREAD_SIZE / 8)
                .map(|words| words * 8)
                .find(|gap| {
                    let at = tp.wrapping_sub(*gap);
                    self.read(at, 8)
                        .is_ok_and(|bytes| bytes[..] == at.to_le_bytes())
                })
        } else {
            self.refresh_libraries().ok()?;
            let size = self.libraries.iter().find_map(|library| {
                let elf = self.files.get(&library.path)?.as_ref()?;
                let symbol = elf
                    .dynamic
                    .iter()
                    .find(|symbol| symbol.defined && symbol.name == GLIBC_PTHREAD_SIZE)?;
                Some(symbol.value.wrapping_add(library.bias))
            });
            size.and_then(|address| self.read(address, 4).ok())
                .map(|bytes| u64::from(u32::from_le_bytes(bytes[..].try_into().expect("4 bytes"))))
        };
        if gap.is_some() {
            self.pthread_gap = Some(gap);
        }
        gap
    }

    /// The name thread `tid` gave itself: its `comm`, unless it is the one
    /// every thread starts with, the program's own.
    fn thread_name(&self, tid: c_int) -> Option<String> {
        let comm = |task: c_int| {
            std::fs::read_to_string(format!("/proc/{}/task/{task}/comm", self.pid))
                .ok()
                .map(|text| text.trim_end_matches('\n').to_string())
        };
        let name = comm(tid)?;
        if tid == self.pid || Some(&name) == comm(self.pid).as_ref() {
            return None;
        }
        Some(crate::terminal_safe::safe(&name).into_owned())
    }

    fn facts(&mut self, tid: c_int) -> Result<ThreadFacts, String> {
        let general = self.general(tid)?;
        let tp = self
            .regset(tid, NT_ARM_TLS, [0u64; 2])
            .map(|tls| tls[0])
            .unwrap_or(0);
        let handle = self.pthread_gap(tp).map_or(0, |gap| tp.wrapping_sub(gap));
        let sp = general[31];
        let stack = self.mappings().ok().and_then(|(_, mappings)| {
            mappings
                .iter()
                .find(|mapping| mapping.start <= sp && sp < mapping.end)
                .map(|mapping| (mapping.start, mapping.end))
        });
        Ok(ThreadFacts {
            id: tid as u64,
            handle,
            name: self.thread_name(tid),
            pc: general[32],
            sp,
            arena: general[19],
            stack,
        })
    }

    /// Detach from a program left running: no `brk` and no watchpoint may be
    /// left behind.
    fn detach(&mut self) -> Result<(), String> {
        self.watchpoints.clear();
        let lifted = self.lift().and_then(|()| self.arm_watchpoints());
        for thread in &self.threads {
            let _ = trace(PTRACE_DETACH, thread.tid, 0, thread.pending as usize);
        }
        lifted
    }
}

impl Drop for Tracee {
    /// A session left without `quit` still ends the way it began.
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.end();
        }
    }
}

impl Inferior for Tracee {
    fn pid(&self) -> u32 {
        self.pid as u32
    }

    fn end(&mut self) -> Result<Ending, String> {
        if !self.ended {
            self.ended = true;
            // SAFETY: puts back the action `new` replaced.
            unsafe {
                sigaction(SIGINT, &self.previous_interrupt, ptr::null_mut());
            }
        }
        if self.check_exit().is_some() {
            return Ok(Ending::AlreadyExited);
        }
        if self.launched {
            kill_and_reap(self.pid);
            return Ok(Ending::Killed);
        }
        self.detach().map(|()| Ending::Detached)
    }

    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        let bias = self.bias()?;
        let image = self.image.take().expect("bias reads the image");
        let mut functions = Vec::with_capacity(image.functions.len());
        let mut failure = None;
        for function in &image.functions {
            let (linked, address, size) = match &function.location {
                Location::Defined { address, size } => {
                    (false, Some(address.wrapping_add(bias)), Some(*size))
                }
                Location::Import(None) => (true, None, None),
                // The file's slot holds 0 until the loader binds it.
                Location::Import(Some(slot)) => match self.read(slot.wrapping_add(bias), 8) {
                    Ok(live) => {
                        let value = u64::from_le_bytes(live[..].try_into().expect("8 bytes"));
                        (true, (value != 0).then_some(value), None)
                    }
                    Err(message) => {
                        failure = Some(message);
                        break;
                    }
                },
            };
            functions.push(FunctionSymbol {
                name: function.name.clone(),
                linked,
                loaded: false,
                address,
                size,
            });
        }
        self.image = Some(image);
        if let Some(message) = failure {
            return Err(message);
        }
        functions.extend(self.loaded_functions()?);
        Ok(functions)
    }

    fn data_symbol(&mut self, name: &str) -> Result<Option<u64>, String> {
        let bias = self.bias()?;
        Ok(self
            .image()?
            .data
            .iter()
            .find(|(symbol, _)| symbol == name)
            .map(|(_, address)| address.wrapping_add(bias)))
    }

    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        let bytes = self.read(address, 8)?;
        Ok(u64::from_le_bytes(bytes[..].try_into().expect("8 bytes")))
    }

    fn read_bytes(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        self.read(address, length)
    }

    fn read_code(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let mut code = self.read(address, length)?;
        let end = address.saturating_add(length as u64);
        for (at, original) in self.breakpoints.range(address.saturating_sub(3)..end) {
            for (index, byte) in original.iter().enumerate() {
                let position = at + index as u64;
                if position >= address && position < end {
                    code[(position - address) as usize] = *byte;
                }
            }
        }
        Ok(code)
    }

    fn function_at(&mut self, address: u64) -> Result<Option<FunctionRange>, String> {
        let bias = self.bias()?;
        let executable = self
            .program
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let image = self.image()?;
        let defined = image
            .functions
            .iter()
            .find_map(|function| match function.location {
                Location::Defined {
                    address: start,
                    size,
                } if start.wrapping_add(bias) <= address
                    && address < start.wrapping_add(bias).wrapping_add(size) =>
                {
                    Some((function.name.clone(), start.wrapping_add(bias), size))
                }
                _ => None,
            });
        let stub = image.stubs.iter().find_map(|(name, start, size)| {
            let start = start.wrapping_add(bias);
            (start <= address && address < start + size)
                .then(|| (format!("symbol stub for: {name}"), start, *size))
        });
        if let Some((name, start, size)) = defined.or(stub) {
            return Ok(Some(FunctionRange {
                image: executable,
                name: Some(name),
                start,
                size: Some(size),
            }));
        }
        self.refresh_libraries()?;
        let Some(index) = self.libraries.iter().position(|library| {
            library
                .code
                .iter()
                .any(|(start, end)| *start <= address && address < *end)
        }) else {
            return Ok(None);
        };
        Ok(self.library_function_at(index, address))
    }

    fn data_symbol_at(&mut self, address: u64) -> Result<Option<String>, String> {
        let bias = self.bias()?;
        Ok(self
            .image()?
            .data
            .iter()
            .find(|(_, start)| start.wrapping_add(bias) == address)
            .map(|(name, _)| name.clone()))
    }

    fn library_functions(&mut self, name: &str) -> Result<Vec<FunctionRange>, String> {
        self.refresh_libraries()?;
        let mut found = Vec::new();
        for library in &self.libraries {
            let Some(Some(elf)) = self.files.get(&library.path) else {
                continue;
            };
            for (symbol, start, size, _) in elf.defined_functions() {
                if symbol == name {
                    found.push(FunctionRange {
                        image: library.name.clone(),
                        name: Some(symbol),
                        start: start.wrapping_add(library.bias),
                        size: (size > 0).then_some(size),
                    });
                }
            }
        }
        Ok(found)
    }

    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        let tids = self
            .threads
            .iter()
            .map(|thread| thread.tid)
            .collect::<Vec<_>>();
        tids.into_iter().map(|tid| self.facts(tid)).collect()
    }

    fn registers(&mut self, index: usize) -> Result<Vec<Register>, String> {
        let tid = self.thread_at(index)?;
        Ok(arm64::registers(&self.general(tid)?, &self.float(tid)?))
    }

    fn set_registers(&mut self, index: usize, values: &[(String, u64)]) -> Result<(), String> {
        let tid = self.thread_at(index)?;
        let mut general = self.general(tid)?;
        let mut float = None;
        for (name, value) in values {
            arm64::assign(
                &mut general,
                &mut float,
                &mut || self.float(tid),
                name,
                *value,
            )?;
        }
        self.set_regset(tid, NT_PRSTATUS, &general, "registers")?;
        if let Some(state) = float {
            self.set_regset(tid, NT_FPREGSET, &state, "floating-point control registers")?;
        }
        Ok(())
    }

    fn step(&mut self, indices: &[usize], count: u64) -> Result<Vec<Step>, String> {
        for index in indices {
            self.thread_at(*index)?;
        }
        let tids = indices
            .iter()
            .map(|index| self.threads[*index].tid)
            .collect::<Vec<_>>();
        let watched = !self.watchpoints.is_empty();
        let prepared = self.lift().and_then(|()| {
            if watched {
                tids.iter()
                    .try_for_each(|tid| self.set_thread_watchpoints(*tid, false))
            } else {
                Ok(())
            }
        });
        let steps = prepared.and_then(|()| self.step_rounds(indices, count));
        if self.exited.is_none() {
            let restored = self.reinsert().and_then(|()| {
                if watched {
                    tids.iter()
                        .filter(|tid| self.index_of(**tid).is_some())
                        .try_for_each(|tid| self.set_thread_watchpoints(*tid, true))
                } else {
                    Ok(())
                }
            });
            if let (Ok(_), Err(message)) = (&steps, restored) {
                return Err(message);
            }
        }
        steps
    }

    fn restart(&mut self, environment: &[(OsString, OsString)]) -> Result<u32, String> {
        if !self.launched {
            return Err(
                "restart needs a program the debugger launched; this one was attached to"
                    .to_string(),
            );
        }
        if self.check_exit().is_none() {
            kill_and_reap(self.pid);
        }
        let pid = spawn(&self.program, &self.args, environment)?;
        let fresh = Tracee::new(pid, Some(self.args.clone()), &self.program, vec![pid])?;
        // The new session keeps this one's saved SIGINT action, not the
        // handler `new` just read back.
        let previous = self.previous_interrupt;
        let mut old = std::mem::replace(self, fresh);
        old.ended = true;
        self.previous_interrupt = previous;
        Ok(pid as u32)
    }

    fn entry(&mut self) -> Result<Option<u64>, String> {
        let bias = self.bias()?;
        Ok(Some(self.image()?.elf.entry.wrapping_add(bias)))
    }

    fn insert_breakpoint(&mut self, address: u64) -> Result<(), String> {
        if self.breakpoints.contains_key(&address) {
            return Err(format!("there is already a breakpoint at 0x{address:016x}"));
        }
        let original: [u8; 4] = self.read(address, 4)?[..].try_into().expect("4 bytes");
        self.write(address, &BREAKPOINT_INSTRUCTION)?;
        self.breakpoints.insert(address, original);
        Ok(())
    }

    fn remove_breakpoint(&mut self, address: u64) -> Result<(), String> {
        let Some(original) = self.breakpoints.get(&address).copied() else {
            return Err(format!("there is no breakpoint at 0x{address:016x}"));
        };
        self.write(address, &original)?;
        self.breakpoints.remove(&address);
        Ok(())
    }

    fn insert_watchpoint(&mut self, address: u64, size: u64) -> Result<(), String> {
        if self.watchpoints.iter().any(|(other, _)| *other == address) {
            return Err(format!("there is already a watchpoint at 0x{address:016x}"));
        }
        let leader = self.thread_at(0)?;
        let slots = self.watch_slots(leader)?;
        if self.watchpoints.len() >= slots {
            return Err(format!(
                "all {slots} of the processor's watchpoints are in use; remove one with \
                 'wp r <where>'"
            ));
        }
        self.watchpoints.push((address, size));
        if let Err(message) = self.arm_watchpoints() {
            self.watchpoints.pop();
            let _ = self.arm_watchpoints();
            return Err(message);
        }
        Ok(())
    }

    fn remove_watchpoint(&mut self, address: u64) -> Result<(), String> {
        let Some(slot) = self
            .watchpoints
            .iter()
            .position(|(other, _)| *other == address)
        else {
            return Err(format!("there is no watchpoint at 0x{address:016x}"));
        };
        self.watchpoints.remove(slot);
        self.arm_watchpoints()
    }

    fn run(&mut self) -> Result<RunStop, String> {
        if let Some(exit) = self.check_exit() {
            return Ok(RunStop::Exited(exit));
        }
        // A thread paused on a breakpoint runs that one instruction first.
        let mut waiting = Vec::new();
        for index in 0..self.threads.len() {
            let tid = self.threads[index].tid;
            if self.threads[index].fresh {
                continue;
            }
            if self
                .general(tid)
                .is_ok_and(|general| self.breakpoints.contains_key(&general[32]))
            {
                waiting.push(index);
            }
        }
        if !waiting.is_empty() {
            for (index, step) in waiting.iter().zip(self.step(&waiting, 1)?) {
                match step.stop {
                    StepStop::Finished | StepStop::Waiting => {}
                    StepStop::Exited => {
                        return Ok(RunStop::Exited(self.exited.unwrap_or(Exit::Unknown)))
                    }
                    StepStop::Exception(what) => {
                        return Ok(RunStop::Exception {
                            thread: *index,
                            what,
                        })
                    }
                }
            }
        }
        INTERRUPTED.store(false, Ordering::SeqCst);
        for index in 0..self.threads.len() {
            if self.threads[index].stopped {
                self.resume(index, false)?;
            }
        }
        loop {
            let (tid, status) = match self.wait_any(true) {
                Ok(Some(found)) => found,
                Ok(None) => continue,
                Err(EINTR) => {
                    if INTERRUPTED.swap(false, Ordering::SeqCst) {
                        let reports = self.stop_all()?;
                        if let Some(exit) = self.exited {
                            return Ok(RunStop::Exited(exit));
                        }
                        return Ok(if reports.is_empty() {
                            RunStop::Interrupted
                        } else {
                            self.classify(&reports)
                        });
                    }
                    continue;
                }
                Err(error) => {
                    return Err(format!(
                        "cannot wait for process {}: {}",
                        self.pid,
                        os_error(error)
                    ))
                }
            };
            if !stopped(status) {
                if self.thread_ended(tid, status) {
                    return Ok(RunStop::Exited(self.exited.unwrap_or(Exit::Unknown)));
                }
                continue;
            }
            let known = self.index_of(tid).is_some();
            let index = self.adopt(tid);
            self.threads[index].stopped = true;
            match event(status) {
                PTRACE_EVENT_CLONE => {
                    let mut child: c_long = 0;
                    if trace(
                        PTRACE_GETEVENTMSG,
                        tid,
                        0,
                        ptr::addr_of_mut!(child) as usize,
                    )
                    .is_ok()
                    {
                        self.adopt(child as c_int);
                    }
                    self.resume(index, false)?;
                }
                PTRACE_EVENT_STOP => {
                    if !known || self.threads[index].fresh {
                        self.welcome(index)?;
                    }
                    // A job-control stop, or an interrupt that came late: the
                    // thread runs on.
                    self.resume(index, false)?;
                }
                0 => match self.signal_stop(index, stop_signal(status))? {
                    Some(report) => {
                        let mut reports = vec![(tid, report)];
                        reports.extend(self.stop_all()?);
                        if let Some(exit) = self.exited {
                            return Ok(RunStop::Exited(exit));
                        }
                        return Ok(self.classify(&reports));
                    }
                    // Passed on to the program at once.
                    None => self.resume(index, false)?,
                },
                _ => self.resume(index, false)?,
            }
        }
    }
}
