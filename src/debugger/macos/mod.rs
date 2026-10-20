//! The macOS debugger backend: Mach task ports.
//!
//! A launched program is started with `posix_spawn` and Apple's
//! `POSIX_SPAWN_START_SUSPENDED`, so the kernel creates it already suspended and
//! no instruction of it — not even the loader's (`dyld`) — runs before the
//! prompt. An attached program is paused with `task_suspend`, which stops every
//! thread in the task at once.
//!
//! Both need the program's task port from `task_for_pid`, which macOS grants
//! only to a debugger signed with the `com.apple.security.cs.debugger`
//! entitlement (or running as root), and only for a target without the hardened
//! runtime; MFBASIC programs are ad-hoc signed without it. The kernel ties both
//! holds to this process: a `task_suspend` hold is dropped when the holder
//! exits, and a program still suspended at spawn does not outlive its parent,
//! so a debugger that dies never leaves a program frozen.
//!
//! Listing reads the executable's symbol table from its file
//! (`super::macho`) and the paused task's memory: the live address of the
//! Mach-O header (found by walking the task's memory regions) gives the load
//! offset to add to every file address, and an import's pointer slot holds the
//! function's address once `dyld` has bound it. Registers are read with
//! `thread_get_state`.
//!
//! A step runs a set of threads in rounds of one instruction each: every other
//! thread is held with `thread_suspend`, each stepping thread's `MDSCR_EL1.SS`
//! bit is set through `ARM_DEBUG_STATE64`, the task is resumed, and each
//! thread's single-step exception arrives on an exception port this process
//! installs on the task for the length of the step (and then puts the
//! program's own ports back). A thread that has raised its exception waits in
//! the kernel for the answer, so it runs exactly one instruction; the task is
//! suspended again before any exception is answered. A thread that raises no
//! exception in time (it is in a system call that blocks) or raises another
//! one is held for the rest of the step.
//!
//! A watchpoint is one of the processor's watchpoint register pairs
//! (`DBGWVR`/`DBGWCR`, in `ARM_DEBUG_STATE64`), set to stop a user-mode store
//! to the bytes it covers. It is written into every thread, and into the
//! task's own debug state, which the kernel copies into each thread the
//! program starts later. A hit is an `EXC_BREAKPOINT` whose first code is
//! `EXC_ARM_DA_DEBUG`, raised before the store runs. A step runs with every
//! stepping thread's watchpoints off, so a step can carry a thread past the
//! store; detaching clears them all.
//!
//! A breakpoint is a `brk #0` written over the instruction at its address:
//! the page is made writable with `mach_vm_protect(VM_PROT_COPY)` (a private
//! copy, so a shared-cache library page changes for this process only),
//! written with `mach_vm_write`, and made executable again; removing it writes
//! the original instruction back. `run` resumes the task and waits on the
//! exception port for a `brk` at a breakpoint's address, another exception,
//! the end of the process, or Ctrl-C. A launched program gets a process group
//! of its own, so a Ctrl-C at the terminal reaches only the debugger, whose
//! handler asks `run` to pause the program.
//!
//! Functions in the loaded libraries are found the way lldb finds them: the
//! loader's image list (`TASK_DYLD_INFO`) gives each library's Mach-O header in
//! memory, and that header's `LC_FUNCTION_STARTS` (every function's first
//! address) and `LC_SYMTAB` (the names the library exports) are read from the
//! library's `__LINKEDIT`, which the shared cache keeps mapped.
//!
//! Everything here is in libSystem, which every macOS binary already links.

use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_short, c_void, CStr, CString, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::arm64::{self, Float};
use super::macho::{self, Location};
use super::{
    Backend, Ending, Exit, FunctionRange, FunctionSymbol, Inferior, Register, RunStop, Step,
    StepStop, ThreadFacts,
};
// `libraries.rs` keeps its `super::shown` call from before the split.
use super::shown;

mod exceptions;
mod inferior;
mod libraries;
mod threads;
use exceptions::*;
use threads::*;

type MachPort = u32;
type KernReturn = c_int;
type SpawnAttr = *mut c_void;

const KERN_SUCCESS: KernReturn = 0;
/// `<spawn.h>`: create the process suspended (Apple extension).
const POSIX_SPAWN_START_SUSPENDED: c_short = 0x0080;
/// `<spawn.h>`: put the child in the process group `posix_spawnattr_setpgroup`
/// names (0: a new group led by the child).
const POSIX_SPAWN_SETPGROUP: c_short = 0x0002;
const SIGINT: c_int = 2;
/// `<sys/signal.h>`: `SIG_ERR`.
const SIG_ERR: usize = usize::MAX;
/// `<mach/vm_prot.h>`.
const VM_PROT_WRITE: c_int = 2;
const VM_PROT_EXECUTE: c_int = 4;
const VM_PROT_COPY: c_int = 0x10;
/// `brk #0`, the instruction a breakpoint writes.
const BREAKPOINT_INSTRUCTION: [u8; 4] = 0xd420_0000_u32.to_le_bytes();
/// `<mach/task_info.h>`: the loader's image list, and its size in `int`s.
const TASK_DYLD_INFO: c_int = 17;
const TASK_DYLD_INFO_COUNT: u32 = 5;
/// `<mach-o/dyld_images.h>`: `infoArrayCount` and `infoArray` in
/// `dyld_all_image_infos`, and the size of one `dyld_image_info`.
const ALL_IMAGES_COUNT: u64 = 4;
const ALL_IMAGES_ARRAY: u64 = 8;
const IMAGE_INFO_SIZE: u64 = 24;
/// `<mach-o/loader.h>`.
const LC_SEGMENT_64: u32 = 0x19;
const LC_SYMTAB: u32 = 0x2;
const LC_FUNCTION_STARTS: u32 = 0x26;
const LC_DYSYMTAB: u32 = 0xb;
/// `<mach-o/loader.h>`: a library's own install name, and the libraries an
/// image depends on (loaded, weakly loaded, re-exported, upward).
const LC_ID_DYLIB: u32 = 0xd;
const LC_LOAD_DYLIB: u32 = 0xc;
const LC_LOAD_WEAK_DYLIB: u32 = 0x8000_0018;
const LC_REEXPORT_DYLIB: u32 = 0x8000_001f;
const LC_LOAD_UPWARD_DYLIB: u32 = 0x8000_0023;
/// `<mach-o/nlist.h>`: an external (exported) symbol.
const N_EXT: u8 = 0x01;
/// `<mach-o/loader.h>`: a section holding instructions.
const S_ATTR_INSTRUCTIONS: u32 = 0x8000_0400;
/// `<mach-o/loader.h>`: a section of symbol stubs, `reserved2` bytes each.
const S_SYMBOL_STUBS: u32 = 0x8;
/// Reading a library's names: offsets closer than `STRING_GAP` share one read
/// of at most `STRING_RUN` bytes, plus `MAX_NAME` for the last name.
const STRING_GAP: u32 = 4096;
const STRING_RUN: u32 = 256 << 10;
const MAX_NAME: usize = 512;
/// The longest image path read.
const MAX_PATH: usize = 1024;
/// How often a `run` that is waiting looks for Ctrl-C and for the program's
/// end.
const RUN_POLL_MS: u32 = 100;
const SIGKILL: c_int = 9;
const WNOHANG: c_int = 1;
const EINTR: i32 = 4;
const ESRCH: i32 = 3;
/// `<sys/proc_info.h>`: `PROC_PIDPATHINFO_MAXSIZE`.
const PROC_PIDPATHINFO_MAXSIZE: usize = 4096;
/// `<mach/kern_return.h>`: no region at or above the address asked for.
const KERN_INVALID_ADDRESS: KernReturn = 1;
/// `<mach/vm_region.h>`: `VM_REGION_BASIC_INFO_64` and its size in `int`s.
const VM_REGION_BASIC_INFO_64: c_int = 9;
const VM_REGION_BASIC_INFO_COUNT_64: u32 = 9;
const VM_PROT_READ: i32 = 1;
/// `<mach-o/loader.h>`: the main executable's `filetype`.
const MH_EXECUTE: u32 = 0x2;
/// `<mach/arm/thread_status.h>`: the 64-bit general and NEON register sets.
const ARM_THREAD_STATE64: c_int = 6;
const ARM_THREAD_STATE64_COUNT: u32 = 68;
const ARM_NEON_STATE64: c_int = 17;
const ARM_NEON_STATE64_COUNT: u32 = 132;
/// The most memory regions the header search walks before giving up.
const MAX_REGIONS: usize = 1 << 16;
/// `<mach/thread_info.h>`: `THREAD_IDENTIFIER_INFO` and its size in `int`s
/// (`thread_id`, `thread_handle`, `dispatch_qaddr`).
const THREAD_IDENTIFIER_INFO: c_int = 4;
const THREAD_IDENTIFIER_INFO_COUNT: u32 = 6;
/// `<sys/proc_info.h>`: `PROC_PIDTHREADID64INFO`, the size of
/// `struct proc_threadinfo`, and where its `pth_name` starts.
const PROC_PIDTHREADID64INFO: c_int = 15;
const PROC_THREADINFO_SIZE: usize = 112;
const PROC_THREADINFO_NAME: usize = 48;
/// `<mach/arm/thread_status.h>`: the debug register set; `MDSCR_EL1.SS` is bit
/// 0 of its last word.
const ARM_DEBUG_STATE64: c_int = 15;
const ARM_DEBUG_STATE64_COUNT: u32 = 130;
const MDSCR_SS: u64 = 1;
/// `arm_debug_state64_t` as words: `bvr[16]`, `bcr[16]`, `wvr[16]`,
/// `wcr[16]`, then `MDSCR_EL1`.
const DEBUG_WORDS: usize = ARM_DEBUG_STATE64_COUNT as usize / 2;
const DEBUG_WVR: usize = 32;
const DEBUG_WCR: usize = 48;
const DEBUG_SLOTS: usize = 16;
/// `DBGWCR_EL1`: enabled (`E`), for EL0 (`PAC` = 0b10), on stores (`LSC` =
/// 0b10); the byte-address-select mask goes at bit 5.
const WCR_STORE_EL0: u64 = 1 | (0b10 << 1) | (0b10 << 3);
const WCR_BAS_SHIFT: u32 = 5;
/// The first code of an `EXC_BREAKPOINT` a watchpoint raised; the second is
/// the data address.
const EXC_ARM_DA_DEBUG: i64 = 0x102;
/// `<mach/exception_types.h>` and `<mach/message.h>`.
const EXC_BAD_ACCESS: i32 = 1;
const EXC_BAD_INSTRUCTION: i32 = 2;
const EXC_ARITHMETIC: i32 = 3;
const EXC_BREAKPOINT: i32 = 6;
const EXC_MASK_STEP: u32 = 0x2 | 0x4 | 0x8 | 0x40;
const EXC_TYPES_COUNT: usize = 14;
const EXCEPTION_DEFAULT: c_int = 1;
const MACH_EXCEPTION_CODES: c_int = 0x8000_0000_u32 as c_int;
const THREAD_STATE_NONE: c_int = 5;
const MACH_PORT_RIGHT_RECEIVE: u32 = 1;
const MACH_MSG_TYPE_MAKE_SEND: u32 = 20;
const MACH_SEND_MSG: c_int = 0x1;
const MACH_RCV_MSG: c_int = 0x2;
const MACH_RCV_TIMEOUT: c_int = 0x100;
const MACH_RCV_TIMED_OUT: KernReturn = 0x1000_4003;
/// `mach_exception_raise` as received: header, body, the thread and task port
/// descriptors, NDR, then `exception`, `codeCnt` and two 64-bit codes, packed
/// to 4 bytes.
const EXC_MESSAGE_THREAD: usize = 28;
const EXC_MESSAGE_TASK: usize = 40;
const EXC_MESSAGE_EXCEPTION: usize = 60;
const EXC_MESSAGE_CODES: usize = 68;
const EXC_MESSAGE_BUFFER: usize = 1024;
/// The reply: header, NDR (little-endian integers), return code.
const EXC_REPLY_SIZE: u32 = 36;
const NDR_RECORD: [u8; 8] = [0, 0, 0, 0, 1, 0, 0, 0];
/// How long a step waits for an instruction before looking at why it has not
/// finished: a thread sitting just after an `svc` is blocked in a system call
/// and is reported as waiting; any other is merely slow (its code paging in,
/// the machine busy) and is waited for again, up to `STEP_LIMIT_MS` in all.
const STEP_TIMEOUT_MS: u32 = 2000;
const STEP_LIMIT_MS: u64 = 30_000;
/// `svc #imm`: `1101 0100 000 imm16 000 01`.
const SVC_MASK: u32 = 0xffe0_001f;
const SVC_BITS: u32 = 0xd400_0001;
/// `brk #imm`: `1101 0100 001 imm16 000 00`.
const BRK_MASK: u32 = 0xffe0_001f;
const BRK_BITS: u32 = 0xd420_0000;

extern "C" {
    static mach_task_self_: MachPort;
    fn task_for_pid(target_tport: MachPort, pid: c_int, task: *mut MachPort) -> KernReturn;
    fn task_suspend(task: MachPort) -> KernReturn;
    fn task_resume(task: MachPort) -> KernReturn;
    fn mach_port_deallocate(task: MachPort, name: MachPort) -> KernReturn;
    fn mach_error_string(error: KernReturn) -> *const c_char;
    fn posix_spawnattr_init(attr: *mut SpawnAttr) -> c_int;
    fn posix_spawnattr_setflags(attr: *mut SpawnAttr, flags: c_short) -> c_int;
    fn posix_spawnattr_destroy(attr: *mut SpawnAttr) -> c_int;
    fn posix_spawn(
        pid: *mut c_int,
        path: *const c_char,
        file_actions: *const c_void,
        attr: *const SpawnAttr,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> c_int;
    fn kill(pid: c_int, signal: c_int) -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn proc_pidpath(pid: c_int, buffer: *mut c_void, size: u32) -> c_int;
    fn posix_spawnattr_setpgroup(attr: *mut SpawnAttr, group: c_int) -> c_int;
    fn signal(signal: c_int, handler: usize) -> usize;
    fn getpagesize() -> c_int;
    fn pid_for_task(task: MachPort, pid: *mut c_int) -> KernReturn;
    fn mach_vm_protect(
        task: MachPort,
        address: u64,
        size: u64,
        set_maximum: c_int,
        protection: c_int,
    ) -> KernReturn;
    fn mach_vm_write(task: MachPort, address: u64, data: *const u8, count: u32) -> KernReturn;
    fn task_info(task: MachPort, flavor: c_int, info: *mut i32, count: *mut u32) -> KernReturn;
    fn mach_vm_region(
        task: MachPort,
        address: *mut u64,
        size: *mut u64,
        flavor: c_int,
        info: *mut i32,
        count: *mut u32,
        object_name: *mut MachPort,
    ) -> KernReturn;
    fn mach_vm_read_overwrite(
        task: MachPort,
        address: u64,
        size: u64,
        data: u64,
        out_size: *mut u64,
    ) -> KernReturn;
    fn mach_vm_deallocate(task: MachPort, address: u64, size: u64) -> KernReturn;
    fn task_threads(task: MachPort, threads: *mut *mut MachPort, count: *mut u32) -> KernReturn;
    fn thread_get_state(
        thread: MachPort,
        flavor: c_int,
        state: *mut u32,
        count: *mut u32,
    ) -> KernReturn;
    fn thread_set_state(
        thread: MachPort,
        flavor: c_int,
        state: *const u32,
        count: u32,
    ) -> KernReturn;
    fn thread_info(thread: MachPort, flavor: c_int, info: *mut i32, count: *mut u32) -> KernReturn;
    fn thread_suspend(thread: MachPort) -> KernReturn;
    fn thread_resume(thread: MachPort) -> KernReturn;
    fn proc_pidinfo(pid: c_int, flavor: c_int, arg: u64, buffer: *mut c_void, size: c_int)
        -> c_int;
    fn mach_port_allocate(task: MachPort, right: u32, name: *mut MachPort) -> KernReturn;
    fn mach_port_insert_right(
        task: MachPort,
        name: MachPort,
        right: MachPort,
        disposition: u32,
    ) -> KernReturn;
    fn mach_port_mod_refs(task: MachPort, name: MachPort, right: u32, delta: i32) -> KernReturn;
    fn task_swap_exception_ports(
        task: MachPort,
        mask: u32,
        port: MachPort,
        behavior: c_int,
        flavor: c_int,
        masks: *mut u32,
        count: *mut u32,
        handlers: *mut MachPort,
        behaviors: *mut c_int,
        flavors: *mut c_int,
    ) -> KernReturn;
    fn task_set_state(task: MachPort, flavor: c_int, state: *const u32, count: u32) -> KernReturn;
    fn sysctlbyname(
        name: *const c_char,
        old: *mut c_void,
        old_size: *mut usize,
        new: *const c_void,
        new_size: usize,
    ) -> c_int;
    fn task_set_exception_ports(
        task: MachPort,
        mask: u32,
        port: MachPort,
        behavior: c_int,
        flavor: c_int,
    ) -> KernReturn;
    fn mach_msg(
        message: *mut u8,
        option: c_int,
        send_size: u32,
        receive_size: u32,
        receive_name: MachPort,
        timeout: u32,
        notify: MachPort,
    ) -> KernReturn;
}

pub(super) struct MacOs;

impl Backend for MacOs {
    fn executable_of(&self, pid: u32) -> Result<PathBuf, String> {
        let raw = raw_pid(pid)?;
        let mut buffer = vec![0u8; PROC_PIDPATHINFO_MAXSIZE];
        // SAFETY: `buffer` is writable for the length passed; `proc_pidpath`
        // writes at most that many bytes and returns how many it wrote.
        let written = unsafe {
            proc_pidpath(
                raw,
                buffer.as_mut_ptr().cast(),
                PROC_PIDPATHINFO_MAXSIZE as u32,
            )
        };
        if written <= 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ESRCH) {
                return Err(format!("no process with PID {pid}"));
            }
            return Err(format!(
                "cannot find the executable of process {pid}: {error}"
            ));
        }
        buffer.truncate(written as usize);
        Ok(PathBuf::from(OsString::from_vec(buffer)))
    }

    fn launch(
        &self,
        program: &Path,
        args: &[OsString],
        environment: &[(OsString, OsString)],
    ) -> Result<Box<dyn Inferior>, String> {
        let (pid, task) = spawn(program, args, environment)?;
        Ok(Box::new(Task::new(pid, task, Some(args.to_vec()), program)))
    }

    fn attach(&self, pid: u32, program: &Path) -> Result<Box<dyn Inferior>, String> {
        let task = task_port(pid)?;
        // SAFETY: `task` is a task port this process holds a send right to.
        let status = unsafe { task_suspend(task) };
        if status != KERN_SUCCESS {
            release(task);
            return Err(format!(
                "cannot pause process {pid}: task_suspend: {}",
                mach_error(status)
            ));
        }
        Ok(Box::new(Task::new(pid, task, None, program)))
    }
}

/// Start `program` with `args` and `environment`, suspended before its first
/// instruction, in a process group of its own, and take its task port.
fn spawn(
    program: &Path,
    args: &[OsString],
    environment: &[(OsString, OsString)],
) -> Result<(u32, MachPort), String> {
    let c_string = |value: &[u8]| {
        CString::new(value).map_err(|_| {
            format!(
                "cannot launch '{}': an argument contains a NUL byte",
                super::shown(program)
            )
        })
    };
    let path = c_string(program.as_os_str().as_bytes())?;
    let mut owned = vec![path.clone()];
    for arg in args {
        owned.push(c_string(arg.as_bytes())?);
    }
    let mut argv = owned.iter().map(|arg| arg.as_ptr()).collect::<Vec<_>>();
    argv.push(ptr::null());
    let entries = super::environment::entries(environment)
        .map_err(|message| format!("cannot launch '{}': {message}", super::shown(program)))?;
    let mut envp = entries
        .iter()
        .map(|entry| entry.as_ptr())
        .collect::<Vec<_>>();
    envp.push(ptr::null());

    let mut attr: SpawnAttr = ptr::null_mut();
    let mut pid: c_int = 0;
    // SAFETY: `attr` is initialized before use and destroyed after; `path`,
    // `argv` and `envp` (each NUL-terminated) and the strings they point at
    // outlive the call.
    let status = unsafe {
        let init = posix_spawnattr_init(&mut attr);
        if init != 0 {
            init
        } else {
            let flags = posix_spawnattr_setflags(
                &mut attr,
                POSIX_SPAWN_START_SUSPENDED | POSIX_SPAWN_SETPGROUP,
            );
            let group = posix_spawnattr_setpgroup(&mut attr, 0);
            let status = if flags != 0 {
                flags
            } else if group != 0 {
                group
            } else {
                posix_spawn(
                    &mut pid,
                    path.as_ptr(),
                    ptr::null(),
                    &attr,
                    argv.as_ptr(),
                    envp.as_ptr(),
                )
            };
            posix_spawnattr_destroy(&mut attr);
            status
        }
    };
    if status != 0 {
        return Err(format!(
            "cannot launch '{}': {}",
            super::shown(program),
            io::Error::from_raw_os_error(status)
        ));
    }
    let pid = pid as u32;
    match task_port(pid) {
        Ok(task) => Ok((pid, task)),
        Err(message) => {
            kill_and_reap(pid);
            Err(message)
        }
    }
}

/// A debuggee held through its task port. A launched program is paused by the
/// suspension it was spawned with; an attached one by this task's
/// `task_suspend` hold.
struct Task {
    pid: u32,
    task: MachPort,
    launched: bool,
    /// The arguments a launched program was started with, for `restart`.
    args: Vec<OsString>,
    ended: bool,
    program: PathBuf,
    /// The executable's symbol table, read on first use.
    image: Option<macho::Image>,
    /// The load offset, found on first use; it cannot change while the
    /// process lives.
    slide: Option<u64>,
    /// Each breakpoint written into the program, with the instruction it
    /// replaced.
    breakpoints: BTreeMap<u64, [u8; 4]>,
    /// Each watchpoint's address and size; its index is its register pair.
    watchpoints: Vec<(u64, u64)>,
    /// How the program ended, once a `run` or `step` saw it end.
    exited: Option<Exit>,
    /// The SIGINT handler this process had before the session installed its
    /// own.
    previous_interrupt: usize,
    /// The libraries the loader has mapped, read when first needed and again
    /// whenever the loader's list has grown.
    libraries: Vec<Library>,
    library_count: u32,
    /// The install names of the libraries the executable is linked against,
    /// read from its file on first use.
    dependencies: Option<Vec<String>>,
}

/// One library the loader has mapped, as its own headers in memory describe it.
struct Library {
    name: String,
    /// `LC_ID_DYLIB`'s install name; `None` for a bundle.
    install_name: Option<String>,
    /// The install names of the libraries it depends on.
    dependencies: Vec<String>,
    /// Loaded while the program ran (`dlopen`): neither the executable nor any
    /// library it was linked against depends on it.
    loaded: bool,
    /// Its `__TEXT` segment, live.
    text: (u64, u64),
    /// The live address of `__LINKEDIT` file offset 0, against which its
    /// tables' offsets are taken.
    linkedit: u64,
    slide: u64,
    symtab: Option<(u32, u32, u32, u32)>,
    function_starts: Option<(u32, u32)>,
    /// `LC_DYSYMTAB`'s indirect symbol table (offset, count).
    indirect: Option<(u32, u32)>,
    /// `__TEXT`'s sections, live: `[start, end)`, flags, `reserved1`,
    /// `reserved2`.
    sections: Vec<(u64, u64, u32, u32, u32)>,
    /// Read when first needed: every function's live start, ascending.
    starts: Option<Vec<u64>>,
    /// Read when first needed: each defined symbol's live address, name, and
    /// whether it is exported, by address.
    symbols: Option<Vec<(u64, String, bool)>>,
}

/// Set by the SIGINT handler; a `run` that sees it pauses the program.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_interrupt(_: c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

impl Task {
    fn new(pid: u32, task: MachPort, args: Option<Vec<OsString>>, program: &Path) -> Self {
        // SAFETY: `on_interrupt` only stores to an atomic, which is
        // async-signal-safe; the previous handler is put back in `end`.
        let previous_interrupt = unsafe { signal(SIGINT, on_interrupt as *const () as usize) };
        Self {
            pid,
            task,
            launched: args.is_some(),
            args: args.unwrap_or_default(),
            ended: false,
            program: program.to_path_buf(),
            image: None,
            slide: None,
            breakpoints: BTreeMap::new(),
            watchpoints: Vec::new(),
            exited: None,
            previous_interrupt,
            libraries: Vec::new(),
            library_count: 0,
            dependencies: None,
        }
    }
}

impl Drop for Task {
    /// A session left without `quit` (a failed write to the terminal, say)
    /// still ends the way it began.
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.end();
        }
    }
}

/// The task port of `pid`, or the reason macOS refused it.
fn task_port(pid: u32) -> Result<MachPort, String> {
    let raw = raw_pid(pid)?;
    let mut task: MachPort = 0;
    // SAFETY: `task` is a valid out-parameter; `mach_task_self_` is this
    // process's own task port, initialized by the runtime before `main`.
    let status = unsafe { task_for_pid(mach_task_self_, raw, &mut task) };
    if status == KERN_SUCCESS {
        return Ok(task);
    }
    let mfb = std::env::current_exe()
        .map(|exe| exe.display().to_string())
        .unwrap_or_else(|_| "<path to mfb>".to_string());
    Err(format!(
        "cannot attach to process {pid}: task_for_pid: {}\n\
         macOS lets one program debug another only when the debugger is signed with\n\
         the com.apple.security.cs.debugger entitlement, or runs as root. To sign\n\
         this mfb, run from the compiler's source tree:\n  \
         sh scripts/sign-debugger.sh {mfb}",
        mach_error(status)
    ))
}

fn raw_pid(pid: u32) -> Result<c_int, String> {
    c_int::try_from(pid).map_err(|_| format!("no process with PID {pid}"))
}

/// Kill a child this process launched and wait for it, so it leaves no zombie.
fn kill_and_reap(pid: u32) {
    let pid = pid as c_int;
    let mut status = 0;
    // SAFETY: `pid` is this process's own child; `status` is a valid
    // out-parameter.
    unsafe {
        kill(pid, SIGKILL);
        while waitpid(pid, &mut status, 0) == -1
            && io::Error::last_os_error().raw_os_error() == Some(EINTR)
        {}
    }
}

/// Whether `pid` still names a process (one this user may not signal counts).
fn alive(pid: u32) -> bool {
    let Ok(raw) = raw_pid(pid) else {
        return false;
    };
    // SAFETY: signal 0 performs only the existence and permission check.
    let signalled = unsafe { kill(raw, 0) == 0 };
    signalled || io::Error::last_os_error().raw_os_error() != Some(ESRCH)
}

fn release(task: MachPort) {
    // SAFETY: drops this process's send right to `task`; the port name is not
    // used again.
    unsafe {
        mach_port_deallocate(mach_task_self_, task);
    }
}

fn mach_error(status: KernReturn) -> String {
    // SAFETY: `mach_error_string` returns a static, NUL-terminated string for
    // every value.
    unsafe { CStr::from_ptr(mach_error_string(status)) }
        .to_string_lossy()
        .into_owned()
}
