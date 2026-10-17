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
    fn _NSGetEnviron() -> *const *const *const c_char;
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

    fn launch(&self, program: &Path, args: &[OsString]) -> Result<Box<dyn Inferior>, String> {
        let (pid, task) = spawn(program, args)?;
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

/// Start `program` with `args`, suspended before its first instruction, in a
/// process group of its own, and take its task port.
fn spawn(program: &Path, args: &[OsString]) -> Result<(u32, MachPort), String> {
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

    let mut attr: SpawnAttr = ptr::null_mut();
    let mut pid: c_int = 0;
    // SAFETY: `attr` is initialized before use and destroyed after; `path`,
    // `argv` (NUL-terminated) and the strings it points at outlive the call;
    // `_NSGetEnviron` returns the live, NUL-terminated environment of this
    // process.
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
                    *_NSGetEnviron(),
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

    /// Refresh the list of loaded libraries when the loader's has changed.
    fn refresh_libraries(&mut self) -> Result<(), String> {
        let mut info = [0i32; 8];
        let mut count = TASK_DYLD_INFO_COUNT;
        // SAFETY: `info` holds more than `count` ints.
        let status = unsafe { task_info(self.task, TASK_DYLD_INFO, info.as_mut_ptr(), &mut count) };
        if status != KERN_SUCCESS {
            return Err(format!(
                "cannot read the loaded libraries of process {}: task_info: {}",
                self.pid,
                mach_error(status)
            ));
        }
        let all_images = u64::from(info[0] as u32) | (u64::from(info[1] as u32) << 32);
        if all_images == 0 {
            return Ok(());
        }
        let count = self.read(all_images + ALL_IMAGES_COUNT, 4)?;
        let count = u32::from_le_bytes(count[..].try_into().expect("4 bytes"));
        if count == self.library_count {
            return Ok(());
        }
        let array = self.read_u64(all_images + ALL_IMAGES_ARRAY)?;
        if array == 0 {
            return Ok(());
        }
        let executable = self.slide()?.wrapping_add(self.image()?.text_address);
        let mut libraries = Vec::new();
        for index in 0..u64::from(count) {
            let entry = array + index * IMAGE_INFO_SIZE;
            let header = self.read_u64(entry)?;
            if header == 0 || header == executable {
                continue;
            }
            let path = self.read_u64(entry + 8)?;
            let name = self.read_c_string(path).unwrap_or_default();
            let name = name.rsplit('/').next().unwrap_or(&name).to_string();
            if let Ok(library) = self.read_library(header, name) {
                libraries.push(library);
            }
        }
        // Every library the executable's dependencies reach was loaded with
        // it; any other was loaded while the program ran.
        let mut reached = self
            .executable_dependencies()
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        loop {
            let before = reached.len();
            for library in &libraries {
                if library
                    .install_name
                    .as_ref()
                    .is_some_and(|name| reached.contains(name))
                {
                    reached.extend(library.dependencies.iter().cloned());
                }
            }
            if reached.len() == before {
                break;
            }
        }
        for library in &mut libraries {
            library.loaded = !library
                .install_name
                .as_ref()
                .is_some_and(|name| reached.contains(name));
        }
        self.libraries = libraries;
        self.library_count = count;
        Ok(())
    }

    /// The install names of the libraries the executable is linked against.
    fn executable_dependencies(&mut self) -> Vec<String> {
        if self.dependencies.is_none() {
            let libraries = std::fs::read(&self.program)
                .ok()
                .and_then(|bytes| crate::os::inspect::inspect(&bytes))
                .map(|binary| binary.libraries)
                .unwrap_or_default();
            self.dependencies = Some(libraries);
        }
        self.dependencies.clone().unwrap_or_default()
    }

    /// The NUL-terminated string at `address`, read a page at a time.
    fn read_c_string(&self, address: u64) -> Result<String, String> {
        let mut bytes = Vec::new();
        let mut at = address;
        while bytes.len() < MAX_PATH {
            let chunk = 64 - (at % 64) as usize;
            let read = self.read(at, chunk)?;
            if let Some(end) = read.iter().position(|byte| *byte == 0) {
                bytes.extend_from_slice(&read[..end]);
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            bytes.extend_from_slice(&read);
            at += chunk as u64;
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// A library's segments and table locations, from its header in memory.
    fn read_library(&self, header: u64, name: String) -> Result<Library, String> {
        let fixed = self.read(header, 32)?;
        let word = |bytes: &[u8], at: usize| {
            u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
        };
        let wide = |bytes: &[u8], at: usize| {
            u64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
        };
        if word(&fixed, 0) != crate::os::inspect::MH_MAGIC_64 {
            return Err(format!("no Mach-O header at 0x{header:016x}"));
        }
        let commands = self.read(header + 32, word(&fixed, 20) as usize)?;
        let (mut text, mut linkedit, mut symtab, mut function_starts) = (None, None, None, None);
        let mut indirect = None;
        let mut sections = Vec::new();
        let (mut install_name, mut dependencies) = (None, Vec::new());
        let dylib_name = |command: &[u8]| {
            let offset = word(command, 8) as usize;
            command
                .get(offset..)
                .map(|name| String::from_utf8_lossy(crate::os::inspect::c_bytes(name)).into_owned())
        };
        let mut at = 0usize;
        for _ in 0..word(&fixed, 16) {
            if at + 8 > commands.len() {
                break;
            }
            let size = word(&commands, at + 4) as usize;
            if size < 8 || at + size > commands.len() {
                break;
            }
            let command = &commands[at..at + size];
            match word(command, 0) {
                LC_SEGMENT_64 if size >= 72 => {
                    let segment = crate::os::inspect::c_bytes(&command[8..24]);
                    if segment == b"__TEXT" {
                        text = Some((wide(command, 24), wide(command, 32)));
                        let count = word(command, 64) as usize;
                        for record in command[72..].chunks_exact(80).take(count) {
                            sections.push((
                                wide(record, 32),
                                wide(record, 40),
                                word(record, 64),
                                word(record, 68),
                                word(record, 72),
                            ));
                        }
                    } else if segment == b"__LINKEDIT" {
                        linkedit = Some((wide(command, 24), wide(command, 40)));
                    }
                }
                LC_SYMTAB if size >= 24 => {
                    symtab = Some((
                        word(command, 8),
                        word(command, 12),
                        word(command, 16),
                        word(command, 20),
                    ))
                }
                LC_FUNCTION_STARTS if size >= 16 => {
                    function_starts = Some((word(command, 8), word(command, 12)))
                }
                LC_DYSYMTAB if size >= 64 => {
                    indirect = Some((word(command, 56), word(command, 60)))
                }
                LC_ID_DYLIB if size >= 24 => install_name = dylib_name(command),
                LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB | LC_REEXPORT_DYLIB | LC_LOAD_UPWARD_DYLIB
                    if size >= 24 =>
                {
                    dependencies.extend(dylib_name(command))
                }
                _ => {}
            }
            at += size;
        }
        let (text_address, text_size) = text.ok_or("a library without __TEXT")?;
        let slide = header.wrapping_sub(text_address);
        let linkedit = linkedit
            .map(|(address, offset)| address.wrapping_add(slide).wrapping_sub(offset))
            .unwrap_or(0);
        Ok(Library {
            name,
            install_name,
            dependencies,
            loaded: false,
            text: (header, header.wrapping_add(text_size)),
            linkedit,
            slide,
            symtab: if linkedit == 0 { None } else { symtab },
            function_starts: if linkedit == 0 { None } else { function_starts },
            indirect: if linkedit == 0 { None } else { indirect },
            sections: sections
                .into_iter()
                .map(|(address, size, flags, reserved1, reserved2)| {
                    let start = address.wrapping_add(slide);
                    (start, start.wrapping_add(size), flags, reserved1, reserved2)
                })
                .collect(),
            starts: None,
            symbols: None,
        })
    }

    /// Library `index`'s function starts, read on first use.
    fn library_starts(&mut self, index: usize) -> Vec<u64> {
        if let Some(starts) = &self.libraries[index].starts {
            return starts.clone();
        }
        let library = &self.libraries[index];
        let mut starts = Vec::new();
        if let Some((offset, size)) = library.function_starts {
            if let Ok(bytes) = self.read(library.linkedit + u64::from(offset), size as usize) {
                let mut address = library.text.0;
                let mut at = 0;
                while at < bytes.len() {
                    let (delta, length) = uleb128(&bytes[at..]);
                    if delta == 0 || length == 0 {
                        break;
                    }
                    at += length;
                    address = address.wrapping_add(delta);
                    starts.push(address);
                }
            }
        }
        self.libraries[index].starts = Some(starts.clone());
        starts
    }

    /// Library `index`'s defined symbols, read on first use.
    fn library_symbols(&mut self, index: usize) -> Vec<(u64, String, bool)> {
        if let Some(symbols) = &self.libraries[index].symbols {
            return symbols.clone();
        }
        let library = &self.libraries[index];
        let mut symbols = Vec::new();
        if let Some((symoff, nsyms, stroff, _)) = library.symtab {
            if let Ok(table) = self.read(library.linkedit + u64::from(symoff), nsyms as usize * 16)
            {
                let entries = table
                    .chunks_exact(16)
                    .filter(|entry| entry[4] & 0xe0 == 0 && entry[4] & 0x0e == 0x0e)
                    .map(|entry| {
                        (
                            u32::from_le_bytes(entry[0..4].try_into().expect("4 bytes")),
                            u64::from_le_bytes(entry[8..16].try_into().expect("8 bytes")),
                            entry[4] & N_EXT != 0,
                        )
                    })
                    .collect::<Vec<_>>();
                // A shared-cache library's names lie scattered through one
                // string pool shared by every library, so the names are read
                // in runs of nearby offsets rather than as one span.
                let mut offsets = entries.iter().map(|(strx, ..)| *strx).collect::<Vec<_>>();
                offsets.sort_unstable();
                offsets.dedup();
                let mut names = std::collections::HashMap::new();
                let mut at = 0;
                while at < offsets.len() {
                    let first = offsets[at];
                    let mut last = first;
                    let mut next = at + 1;
                    while next < offsets.len()
                        && offsets[next] - last < STRING_GAP
                        && offsets[next] - first < STRING_RUN
                    {
                        last = offsets[next];
                        next += 1;
                    }
                    let base = library.linkedit + u64::from(stroff) + u64::from(first);
                    let length = (last - first) as usize + MAX_NAME;
                    if let Ok(strings) = self.read(base, length) {
                        for offset in &offsets[at..next] {
                            let name =
                                crate::os::inspect::c_bytes(&strings[(offset - first) as usize..]);
                            names.insert(*offset, String::from_utf8_lossy(name).into_owned());
                        }
                    }
                    at = next;
                }
                for (strx, value, external) in entries {
                    if let Some(name) = names.get(&strx).filter(|name| !name.is_empty()) {
                        symbols.push((value.wrapping_add(library.slide), name.clone(), external));
                    }
                }
            }
        }
        symbols.sort();
        self.libraries[index].symbols = Some(symbols.clone());
        symbols
    }

    /// The stub holding `address` in library `index`, named by the symbol its
    /// indirect-symbol entry selects (`symbol stub for: <symbol>`).
    fn library_stub(&self, index: usize, address: u64) -> Option<FunctionRange> {
        let library = &self.libraries[index];
        let &(begin, _, _, first, size) =
            library.sections.iter().find(|(begin, end, flags, ..)| {
                flags & 0xff == S_SYMBOL_STUBS && *begin <= address && address < *end
            })?;
        if size == 0 {
            return None;
        }
        let stub = (address - begin) / u64::from(size);
        let start = begin + stub * u64::from(size);
        let (indirect, _) = library.indirect?;
        let (symoff, _, stroff, _) = library.symtab?;
        let entry = self
            .read(
                library.linkedit + u64::from(indirect) + (u64::from(first) + stub) * 4,
                4,
            )
            .ok()?;
        let entry = u32::from_le_bytes(entry[..].try_into().ok()?);
        let name = if entry & 0xc000_0000 == 0 {
            let nlist = self
                .read(
                    library.linkedit + u64::from(symoff) + u64::from(entry) * 16,
                    4,
                )
                .ok()?;
            let strx = u32::from_le_bytes(nlist[..].try_into().ok()?);
            self.read_c_string(library.linkedit + u64::from(stroff) + u64::from(strx))
                .ok()
                .map(|symbol| format!("symbol stub for: {symbol}"))
        } else {
            None
        };
        Some(FunctionRange {
            image: library.name.clone(),
            name,
            start,
            size: Some(u64::from(size)),
        })
    }

    /// The range of the function at `start` in library `index`: up to the next
    /// function's start, and never past the end of its section.
    fn library_function(&mut self, index: usize, start: u64) -> FunctionRange {
        let starts = self.library_starts(index);
        let section_end = self.libraries[index]
            .sections
            .iter()
            .find(|(begin, end, ..)| *begin <= start && start < *end)
            .map_or(self.libraries[index].text.1, |(_, end, ..)| *end);
        let end = starts
            .iter()
            .copied()
            .find(|next| *next > start)
            .unwrap_or(section_end)
            .min(section_end);
        let symbols = self.library_symbols(index);
        let name = symbols
            .iter()
            .find(|(address, ..)| *address == start)
            .map(|(_, name, _)| name.clone());
        FunctionRange {
            image: self.libraries[index].name.clone(),
            name,
            start,
            size: starts.contains(&start).then(|| end - start),
        }
    }

    /// The functions the libraries the program loaded while running export,
    /// by address.
    fn loaded_functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        self.refresh_libraries()?;
        let mut functions = Vec::new();
        for index in 0..self.libraries.len() {
            if !self.libraries[index].loaded {
                continue;
            }
            let symbols = self.library_symbols(index);
            let library = &self.libraries[index];
            let code = |address: u64| {
                library.sections.iter().any(|(begin, end, flags, ..)| {
                    flags & S_ATTR_INSTRUCTIONS != 0 && *begin <= address && address < *end
                })
            };
            functions.extend(
                symbols
                    .into_iter()
                    .filter(|(address, _, external)| *external && code(*address))
                    .map(|(address, name, _)| FunctionSymbol {
                        name,
                        linked: true,
                        loaded: true,
                        address: Some(address),
                        size: None,
                    }),
            );
        }
        functions.sort_by_key(|function| function.address);
        Ok(functions)
    }

    /// Write `bytes` over the code at `address`: the page is made writable
    /// (as a private copy) for the write, then executable again.
    fn patch(&self, address: u64, bytes: &[u8; 4]) -> Result<(), String> {
        // SAFETY: `getpagesize` has no preconditions.
        let page_size = u64::try_from(unsafe { getpagesize() }).unwrap_or(16384);
        let page = address & !(page_size - 1);
        // SAFETY: the calls name memory in the debuggee only; `bytes` is
        // readable for the 4 bytes written.
        let status = unsafe {
            let status = mach_vm_protect(
                self.task,
                page,
                page_size,
                0,
                VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY,
            );
            if status != KERN_SUCCESS {
                status
            } else {
                let written = mach_vm_write(self.task, address, bytes.as_ptr(), 4);
                let restored = mach_vm_protect(
                    self.task,
                    page,
                    page_size,
                    0,
                    VM_PROT_READ | VM_PROT_EXECUTE,
                );
                if written != KERN_SUCCESS {
                    written
                } else {
                    restored
                }
            }
        };
        if status != KERN_SUCCESS {
            return Err(format!(
                "cannot change the instruction at 0x{address:016x}: {}",
                mach_error(status)
            ));
        }
        Ok(())
    }

    /// Write the watchpoints into every thread, and into the task's debug
    /// state, which each thread the program starts later takes.
    fn arm_watchpoints(&self) -> Result<(), String> {
        for thread in &self.thread_ports()?.0 {
            self.set_thread_watchpoints(*thread, true)?;
        }
        let mut debug = [0u64; DEBUG_WORDS];
        write_watchpoints(&mut debug, &self.watchpoints, true);
        // SAFETY: `debug` holds `ARM_DEBUG_STATE64_COUNT` ints of that flavor.
        let status = unsafe {
            task_set_state(
                self.task,
                ARM_DEBUG_STATE64,
                debug.as_ptr().cast(),
                ARM_DEBUG_STATE64_COUNT,
            )
        };
        if status != KERN_SUCCESS {
            return Err(format!(
                "cannot set the program's watchpoints: task_set_state: {}",
                mach_error(status)
            ));
        }
        Ok(())
    }

    /// Turn `threads`' watchpoints off for a step (`on` false), or back on.
    fn pause_watchpoints(&self, threads: &[MachPort], on: bool) -> Result<(), String> {
        if self.watchpoints.is_empty() {
            return Ok(());
        }
        threads
            .iter()
            .try_for_each(|thread| self.set_thread_watchpoints(*thread, on))
    }

    /// Write the watchpoints into `thread`, enabled or all off.
    fn set_thread_watchpoints(&self, thread: MachPort, on: bool) -> Result<(), String> {
        let mut debug = debug_state(thread)?;
        write_watchpoints(&mut debug, &self.watchpoints, on);
        set_debug_state(thread, &debug, "watchpoints")
    }

    /// Put every breakpoint's original instruction back, for a step or a
    /// detach; [`Task::reinsert`] writes them again.
    fn lift(&self) -> Result<(), String> {
        for (address, original) in &self.breakpoints {
            self.patch(*address, original)?;
        }
        Ok(())
    }

    fn reinsert(&self) -> Result<(), String> {
        for address in self.breakpoints.keys() {
            self.patch(*address, &BREAKPOINT_INSTRUCTION)?;
        }
        Ok(())
    }

    /// How the program ended, if it has: a launched child is reaped and its
    /// status read; an attached program is gone when its task port no longer
    /// names a process.
    fn check_exit(&mut self) -> Option<Exit> {
        if self.exited.is_some() {
            return self.exited;
        }
        let exit = if self.launched {
            let mut status = 0;
            // SAFETY: `status` is a valid out-parameter; `pid` is this
            // process's own child.
            if unsafe { waitpid(self.pid as c_int, &mut status, WNOHANG) } != self.pid as c_int {
                return None;
            }
            if status & 0x7f == 0 {
                Exit::Status((status >> 8) & 0xff)
            } else {
                Exit::Signal(status & 0x7f)
            }
        } else {
            let mut pid = 0;
            // SAFETY: `pid` is a valid out-parameter.
            if unsafe { pid_for_task(self.task, &mut pid) } == KERN_SUCCESS && alive(self.pid) {
                return None;
            }
            Exit::Unknown
        };
        self.exited = Some(exit);
        Some(exit)
    }

    fn image(&mut self) -> Result<&macho::Image, String> {
        if self.image.is_none() {
            let shown = super::shown(&self.program);
            let bytes = std::fs::read(&self.program)
                .map_err(|err| format!("cannot read '{shown}': {err}"))?;
            let image = macho::read(&bytes)
                .ok_or_else(|| format!("cannot read the symbol table of '{shown}'"))?;
            self.image = Some(image);
        }
        Ok(self.image.as_ref().expect("read above"))
    }

    /// How far the executable was loaded from the address it asks for.
    fn slide(&mut self) -> Result<u64, String> {
        if let Some(slide) = self.slide {
            return Ok(slide);
        }
        let header = self.executable_header()?;
        let slide = header.wrapping_sub(self.image()?.text_address);
        self.slide = Some(slide);
        Ok(slide)
    }

    /// The live address of the main executable's Mach-O header: the first
    /// readable region that starts with a 64-bit `MH_EXECUTE` header. The
    /// kernel maps the executable before `dyld` runs, so this holds from the
    /// first instruction on.
    fn executable_header(&self) -> Result<u64, String> {
        let mut address = 0u64;
        for _ in 0..MAX_REGIONS {
            let mut size = 0u64;
            let mut info = [0i32; VM_REGION_BASIC_INFO_COUNT_64 as usize];
            let mut count = VM_REGION_BASIC_INFO_COUNT_64;
            let mut object: MachPort = 0;
            // SAFETY: every out-parameter is a valid local; `info` holds
            // `count` ints, the size of `vm_region_basic_info_data_64_t`.
            let status = unsafe {
                mach_vm_region(
                    self.task,
                    &mut address,
                    &mut size,
                    VM_REGION_BASIC_INFO_64,
                    info.as_mut_ptr(),
                    &mut count,
                    &mut object,
                )
            };
            if status == KERN_INVALID_ADDRESS {
                break;
            }
            if status != KERN_SUCCESS {
                return Err(format!(
                    "cannot read the memory map of process {}: mach_vm_region: {}",
                    self.pid,
                    mach_error(status)
                ));
            }
            // `protection` is the first field of the region info.
            if info[0] & VM_PROT_READ != 0 {
                if let Ok(header) = self.read(address, 16) {
                    let magic = u32::from_le_bytes(header[0..4].try_into().expect("16 bytes"));
                    let kind = u32::from_le_bytes(header[12..16].try_into().expect("16 bytes"));
                    if magic == crate::os::inspect::MH_MAGIC_64 && kind == MH_EXECUTE {
                        return Ok(address);
                    }
                }
            }
            address = match address.checked_add(size) {
                Some(next) if size > 0 => next,
                _ => break,
            };
        }
        Err(format!(
            "cannot find the executable's header in process {}",
            self.pid
        ))
    }

    /// `length` bytes of the process's memory at `address`.
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let mut buffer = vec![0u8; length];
        let mut read = 0u64;
        // SAFETY: `buffer` is writable for `length` bytes; the kernel copies at
        // most that many into it and reports how many in `read`.
        let status = unsafe {
            mach_vm_read_overwrite(
                self.task,
                address,
                length as u64,
                buffer.as_mut_ptr() as u64,
                &mut read,
            )
        };
        if status != KERN_SUCCESS || read != length as u64 {
            return Err(format!(
                "cannot read process {}'s memory at 0x{address:016x}: {}",
                self.pid,
                mach_error(status)
            ));
        }
        Ok(buffer)
    }

    /// Every thread of the task, in the system's order.
    fn thread_ports(&self) -> Result<ThreadPorts, String> {
        let mut threads: *mut MachPort = ptr::null_mut();
        let mut count = 0u32;
        // SAFETY: both out-parameters are valid locals.
        let status = unsafe { task_threads(self.task, &mut threads, &mut count) };
        if status != KERN_SUCCESS {
            return Err(format!(
                "cannot list the threads of process {}: task_threads: {}",
                self.pid,
                mach_error(status)
            ));
        }
        // SAFETY: on success `threads` points at `count` thread ports the
        // kernel placed in this task. They are copied out, and the array is
        // returned to the kernel; the ports themselves are released when the
        // `ThreadPorts` is dropped.
        unsafe {
            let ports = std::slice::from_raw_parts(threads, count as usize).to_vec();
            mach_vm_deallocate(
                mach_task_self_,
                threads as u64,
                u64::from(count) * std::mem::size_of::<MachPort>() as u64,
            );
            Ok(ThreadPorts(ports))
        }
    }

    /// The thread at `index`, or the error naming how many there are.
    fn thread_at(&self, threads: &ThreadPorts, index: usize) -> Result<MachPort, String> {
        threads.0.get(index).copied().ok_or_else(|| {
            format!(
                "no thread {index}: process {} has {} threads",
                self.pid,
                threads.0.len()
            )
        })
    }

    /// The memory region holding `address`, `[start, end)`, if one does.
    fn region_containing(&self, address: u64) -> Option<(u64, u64)> {
        let mut start = address;
        let mut size = 0u64;
        let mut info = [0i32; VM_REGION_BASIC_INFO_COUNT_64 as usize];
        let mut count = VM_REGION_BASIC_INFO_COUNT_64;
        let mut object: MachPort = 0;
        // SAFETY: as in `executable_header`.
        let status = unsafe {
            mach_vm_region(
                self.task,
                &mut start,
                &mut size,
                VM_REGION_BASIC_INFO_64,
                info.as_mut_ptr(),
                &mut count,
                &mut object,
            )
        };
        (status == KERN_SUCCESS && start <= address).then(|| (start, start.saturating_add(size)))
    }

    fn facts(&self, thread: MachPort) -> Result<ThreadFacts, String> {
        let general = general_state(thread)?;
        let mut identity = [0u64; 3];
        let mut count = THREAD_IDENTIFIER_INFO_COUNT;
        // SAFETY: `identity` holds `count` ints of `thread_identifier_info`.
        let status = unsafe {
            thread_info(
                thread,
                THREAD_IDENTIFIER_INFO,
                identity.as_mut_ptr().cast(),
                &mut count,
            )
        };
        if status != KERN_SUCCESS {
            return Err(format!(
                "cannot identify a thread of process {}: thread_info: {}",
                self.pid,
                mach_error(status)
            ));
        }
        let [id, handle, _] = identity;
        Ok(ThreadFacts {
            id,
            handle,
            name: self.thread_name(id),
            pc: general[32],
            sp: general[31],
            arena: general[19],
            stack: self.region_containing(general[31]),
        })
    }

    /// The name the thread `id` gave itself, if any.
    fn thread_name(&self, id: u64) -> Option<String> {
        let mut info = [0u8; PROC_THREADINFO_SIZE];
        // SAFETY: `info` is writable for the size passed.
        let written = unsafe {
            proc_pidinfo(
                self.pid as c_int,
                PROC_PIDTHREADID64INFO,
                id,
                info.as_mut_ptr().cast(),
                PROC_THREADINFO_SIZE as c_int,
            )
        };
        if written != PROC_THREADINFO_SIZE as c_int {
            return None;
        }
        let name = crate::os::inspect::c_bytes(&info[PROC_THREADINFO_NAME..]);
        (!name.is_empty())
            .then(|| crate::terminal_safe::safe(&String::from_utf8_lossy(name)).into_owned())
    }

    /// Run `threads` for up to `count` rounds of one instruction each; every
    /// other thread is already held and the exception port installed. Answers
    /// one `Step` per thread, in order.
    fn step_rounds(
        &self,
        threads: &[MachPort],
        port: &ExceptionPort,
        count: u64,
    ) -> Result<Vec<Step>, String> {
        let mut steps = vec![
            Step {
                taken: 0,
                stop: StepStop::Finished,
            };
            threads.len()
        ];
        let mut active = (0..threads.len()).collect::<Vec<_>>();
        // Threads that stopped early, held until the step ends.
        let mut parked = Vec::new();
        let mut result = Ok(());
        for _ in 0..count {
            if active.is_empty() {
                break;
            }
            let before = active
                .iter()
                .map(|at| general_state(threads[*at]).map(|general| general[32]))
                .collect::<Vec<_>>();
            if let Err(message) = active
                .iter()
                .try_for_each(|at| set_single_step(threads[*at], true))
            {
                for at in &active {
                    let _ = set_single_step(threads[*at], false);
                }
                result = Err(message);
                break;
            }
            // SAFETY: `self.task` is this session's task port.
            let resumed = unsafe { task_resume(self.task) } == KERN_SUCCESS;
            let mut raised = (0..threads.len()).map(|_| None).collect::<Vec<_>>();
            if resumed {
                let started = Instant::now();
                let slice = Duration::from_millis(u64::from(STEP_TIMEOUT_MS));
                let mut deadline = started + slice;
                loop {
                    while active.iter().any(|at| raised[*at].is_none()) {
                        let left = deadline.saturating_duration_since(Instant::now());
                        if left.is_zero() {
                            break;
                        }
                        if let Some(exception) = port.receive(left.as_millis().max(1) as u32) {
                            place(threads, &mut raised, exception);
                        }
                    }
                    let blocked = active
                        .iter()
                        .filter(|at| raised[**at].is_none())
                        .all(|at| self.in_system_call(threads[*at]));
                    if blocked || started.elapsed() >= Duration::from_millis(STEP_LIMIT_MS) {
                        break;
                    }
                    deadline = Instant::now() + slice;
                }
            }
            // SAFETY: as above.
            let suspended = resumed && unsafe { task_suspend(self.task) } == KERN_SUCCESS;
            for at in &active {
                let _ = set_single_step(threads[*at], false);
            }
            // An exception may have arrived after the wait gave up.
            while let Some(late) = port.receive(0) {
                place(threads, &mut raised, late);
            }
            if !suspended {
                let stop = if alive(self.pid) {
                    StepStop::Waiting
                } else {
                    StepStop::Exited
                };
                for at in &active {
                    steps[*at].stop = stop.clone();
                }
                for exception in raised.into_iter().flatten() {
                    exception.answer();
                }
                active.clear();
                break;
            }
            let mut still = Vec::with_capacity(active.len());
            let mut stopped = Vec::new();
            for (at, before) in active.iter().zip(before) {
                match raised[*at].take() {
                    Some(exception) => {
                        exception.answer();
                        if self.is_step(&exception, before) {
                            steps[*at].taken += 1;
                            still.push(*at);
                        } else {
                            steps[*at].stop = StepStop::Exception(exception.describe());
                            stopped.push(threads[*at]);
                        }
                    }
                    None => {
                        steps[*at].stop = StepStop::Waiting;
                        stopped.push(threads[*at]);
                    }
                }
            }
            // Exceptions from threads not being stepped (none run) are
            // answered all the same.
            for exception in raised.into_iter().flatten() {
                exception.answer();
            }
            for thread in stopped {
                // SAFETY: a thread port of this task; the hold is given back
                // below, once the step is over.
                if unsafe { thread_suspend(thread) } == KERN_SUCCESS {
                    parked.push(thread);
                }
            }
            active = still;
        }
        for thread in parked {
            // SAFETY: drops the hold taken above.
            unsafe {
                thread_resume(thread);
            }
        }
        result.map(|()| steps)
    }

    /// Whether `thread` is blocked in a system call: its program counter is just
    /// past an `svc`, where the kernel holds it until the call returns.
    fn in_system_call(&self, thread: MachPort) -> bool {
        let Ok(general) = general_state(thread) else {
            return false;
        };
        self.read(general[32].wrapping_sub(4), 4)
            .map(|bytes| u32::from_le_bytes(bytes[..].try_into().expect("4 bytes")))
            .is_ok_and(|word| word & SVC_MASK == SVC_BITS)
    }

    /// A single-step and a `brk` both raise `EXC_BREAKPOINT`; a `brk` leaves
    /// the program counter on itself.
    fn is_step(&self, exception: &Exception, before: Result<u64, String>) -> bool {
        if exception.kind != EXC_BREAKPOINT {
            return false;
        }
        let Ok(before) = before else {
            return true;
        };
        let instruction = self
            .read(before, 4)
            .map(|bytes| u32::from_le_bytes(bytes[..].try_into().expect("4 bytes")));
        let after = general_state(exception.thread).map(|general| general[32]);
        !(instruction.is_ok_and(|word| word & BRK_MASK == BRK_BITS) && after == Ok(before))
    }
}

/// A ULEB128 value and how many bytes it took.
fn uleb128(bytes: &[u8]) -> (u64, usize) {
    let mut value = 0u64;
    for (index, byte) in bytes.iter().enumerate().take(10) {
        value |= u64::from(byte & 0x7f) << (7 * index);
        if byte & 0x80 == 0 {
            return (value, index + 1);
        }
    }
    (value, 0)
}

/// File `exception` under the thread that raised it; one from a thread not
/// being stepped, or a second from the same thread, is answered at once.
fn place(threads: &[MachPort], raised: &mut [Option<Exception>], exception: Exception) {
    match threads
        .iter()
        .position(|thread| *thread == exception.thread)
    {
        Some(at) if raised[at].is_none() => raised[at] = Some(exception),
        _ => exception.answer(),
    }
}

/// A task's thread ports, released when dropped.
struct ThreadPorts(Vec<MachPort>);

impl Drop for ThreadPorts {
    fn drop(&mut self) {
        for port in &self.0 {
            release(*port);
        }
    }
}

/// `ARM_THREAD_STATE64` as words: x0–x28, fp, lr, sp, pc, then CPSR.
fn general_state(thread: MachPort) -> Result<[u64; 34], String> {
    let mut general = [0u64; ARM_THREAD_STATE64_COUNT as usize / 2];
    thread_state(
        thread,
        ARM_THREAD_STATE64,
        general.as_mut_ptr().cast(),
        ARM_THREAD_STATE64_COUNT,
    )?;
    Ok(general)
}

/// `ARM_NEON_STATE64`: v0–v31, fpsr, fpcr.
fn neon_state(thread: MachPort) -> Result<Float, String> {
    let mut neon = Float::zeroed();
    thread_state(
        thread,
        ARM_NEON_STATE64,
        ptr::addr_of_mut!(neon).cast(),
        ARM_NEON_STATE64_COUNT,
    )?;
    Ok(neon)
}

/// Write one register set of `thread` from `state`, which holds `count` ints;
/// `what` names the set in an error.
fn set_thread_state(
    thread: MachPort,
    flavor: c_int,
    state: *const u32,
    count: u32,
    what: &str,
) -> Result<(), String> {
    // SAFETY: the caller passes a buffer of `count` ints for this flavor.
    let status = unsafe { thread_set_state(thread, flavor, state, count) };
    if status != KERN_SUCCESS {
        return Err(format!(
            "cannot set a thread's {what}: thread_set_state: {}",
            mach_error(status)
        ));
    }
    Ok(())
}

/// `thread`'s debug registers.
fn debug_state(thread: MachPort) -> Result<[u64; DEBUG_WORDS], String> {
    let mut debug = [0u64; DEBUG_WORDS];
    thread_state(
        thread,
        ARM_DEBUG_STATE64,
        debug.as_mut_ptr().cast(),
        ARM_DEBUG_STATE64_COUNT,
    )?;
    Ok(debug)
}

/// Write `thread`'s debug registers; `what` names the change in an error.
fn set_debug_state(thread: MachPort, debug: &[u64; DEBUG_WORDS], what: &str) -> Result<(), String> {
    set_thread_state(
        thread,
        ARM_DEBUG_STATE64,
        debug.as_ptr().cast(),
        ARM_DEBUG_STATE64_COUNT,
        what,
    )
}

/// Set or clear `MDSCR_EL1.SS`, the hardware single-step bit, for `thread`.
fn set_single_step(thread: MachPort, on: bool) -> Result<(), String> {
    let mut debug = debug_state(thread)?;
    let last = debug.len() - 1;
    if on {
        debug[last] |= MDSCR_SS;
    } else {
        debug[last] &= !MDSCR_SS;
    }
    set_debug_state(thread, &debug, "single-step bit")
}

/// Write `watchpoints` into the watchpoint registers of `debug`, the rest
/// cleared; with `on` false every one is written disabled.
fn write_watchpoints(debug: &mut [u64; DEBUG_WORDS], watchpoints: &[(u64, u64)], on: bool) {
    for slot in 0..DEBUG_SLOTS {
        let (value, control) = match watchpoints.get(slot) {
            Some(&(address, size)) => {
                let select = ((1u64 << size) - 1) << (address & 7);
                let control = WCR_STORE_EL0 | (select << WCR_BAS_SHIFT);
                (address & !7, if on { control } else { control & !1 })
            }
            None => (0, 0),
        };
        debug[DEBUG_WVR + slot] = value;
        debug[DEBUG_WCR + slot] = control;
    }
}

/// How many watchpoints the processor has.
fn watchpoint_slots() -> Result<usize, String> {
    let mut count: u32 = 0;
    let mut size = std::mem::size_of::<u32>();
    // SAFETY: the name is NUL-terminated; `count` is writable for `size` bytes.
    let status = unsafe {
        sysctlbyname(
            c"hw.optional.watchpoint".as_ptr(),
            ptr::addr_of_mut!(count).cast(),
            &mut size,
            ptr::null(),
            0,
        )
    };
    if status != 0 || count == 0 {
        return Err("this processor has no watchpoints the debugger can use".to_string());
    }
    Ok((count as usize).min(DEBUG_SLOTS))
}

/// A receive right this process listens on for the task's exceptions, with
/// the handlers it replaced; dropping it puts those back and destroys it.
struct ExceptionPort {
    task: MachPort,
    port: MachPort,
    saved: Vec<(u32, MachPort, c_int, c_int)>,
}

impl ExceptionPort {
    /// Route `task`'s step-related exceptions to a fresh port.
    fn install(task: MachPort) -> Result<Self, String> {
        let mut port: MachPort = 0;
        // SAFETY: `port` is a valid out-parameter; the right is created in
        // this task, and given a send right of its own for the kernel to use.
        let status = unsafe {
            let status = mach_port_allocate(mach_task_self_, MACH_PORT_RIGHT_RECEIVE, &mut port);
            if status == KERN_SUCCESS {
                mach_port_insert_right(mach_task_self_, port, port, MACH_MSG_TYPE_MAKE_SEND)
            } else {
                status
            }
        };
        if status != KERN_SUCCESS {
            return Err(format!(
                "cannot make an exception port: {}",
                mach_error(status)
            ));
        }
        let mut masks = [0u32; EXC_TYPES_COUNT];
        let mut handlers = [0 as MachPort; EXC_TYPES_COUNT];
        let mut behaviors = [0 as c_int; EXC_TYPES_COUNT];
        let mut flavors = [0 as c_int; EXC_TYPES_COUNT];
        let mut count = EXC_TYPES_COUNT as u32;
        // SAFETY: each array holds `count` entries, as the call requires.
        let status = unsafe {
            task_swap_exception_ports(
                task,
                EXC_MASK_STEP,
                port,
                EXCEPTION_DEFAULT | MACH_EXCEPTION_CODES,
                THREAD_STATE_NONE,
                masks.as_mut_ptr(),
                &mut count,
                handlers.as_mut_ptr(),
                behaviors.as_mut_ptr(),
                flavors.as_mut_ptr(),
            )
        };
        let mut installed = ExceptionPort {
            task,
            port,
            saved: Vec::new(),
        };
        if status != KERN_SUCCESS {
            return Err(format!(
                "cannot catch the program's exceptions: task_swap_exception_ports: {}",
                mach_error(status)
            ));
        }
        installed.saved = (0..count.min(EXC_TYPES_COUNT as u32) as usize)
            .map(|at| (masks[at], handlers[at], behaviors[at], flavors[at]))
            .collect();
        Ok(installed)
    }

    /// The next exception, waiting up to `timeout` milliseconds.
    fn receive(&self, timeout: u32) -> Option<Exception> {
        let mut buffer = [0u8; EXC_MESSAGE_BUFFER];
        // SAFETY: `buffer` is writable for the size passed.
        let status = unsafe {
            mach_msg(
                buffer.as_mut_ptr(),
                MACH_RCV_MSG | MACH_RCV_TIMEOUT,
                0,
                EXC_MESSAGE_BUFFER as u32,
                self.port,
                timeout,
                0,
            )
        };
        if status == MACH_RCV_TIMED_OUT || status != KERN_SUCCESS {
            return None;
        }
        let word = |at: usize| u32::from_le_bytes(buffer[at..at + 4].try_into().expect("4 bytes"));
        let wide = |at: usize| i64::from_le_bytes(buffer[at..at + 8].try_into().expect("8 bytes"));
        Some(Exception {
            bits: word(0),
            reply: word(8),
            id: word(20),
            thread: word(EXC_MESSAGE_THREAD),
            task: word(EXC_MESSAGE_TASK),
            kind: word(EXC_MESSAGE_EXCEPTION) as i32,
            codes: [wide(EXC_MESSAGE_CODES), wide(EXC_MESSAGE_CODES + 8)],
        })
    }
}

impl Drop for ExceptionPort {
    fn drop(&mut self) {
        // SAFETY: restores the handlers `install` replaced, then drops this
        // process's rights to them and to its own port.
        unsafe {
            for (mask, handler, behavior, flavor) in &self.saved {
                task_set_exception_ports(self.task, *mask, *handler, *behavior, *flavor);
                if *handler != 0 {
                    mach_port_deallocate(mach_task_self_, *handler);
                }
            }
            mach_port_deallocate(mach_task_self_, self.port);
            mach_port_mod_refs(mach_task_self_, self.port, MACH_PORT_RIGHT_RECEIVE, -1);
        }
    }
}

/// One `mach_exception_raise` message.
struct Exception {
    bits: u32,
    reply: MachPort,
    id: u32,
    thread: MachPort,
    task: MachPort,
    kind: i32,
    codes: [i64; 2],
}

impl Exception {
    /// Answer "handled", so the thread retries or continues when it next runs,
    /// and drop the thread and task rights the message carried.
    fn answer(&self) {
        let mut reply = [0u8; EXC_REPLY_SIZE as usize];
        // The reply's remote right is the request's reply right, as sent.
        reply[0..4].copy_from_slice(&(self.bits & 0x1f).to_le_bytes());
        reply[4..8].copy_from_slice(&EXC_REPLY_SIZE.to_le_bytes());
        reply[8..12].copy_from_slice(&self.reply.to_le_bytes());
        reply[20..24].copy_from_slice(&(self.id + 100).to_le_bytes());
        reply[24..32].copy_from_slice(&NDR_RECORD);
        reply[32..36].copy_from_slice(&KERN_SUCCESS.to_le_bytes());
        // SAFETY: `reply` is a complete reply message of the size passed; the
        // two port names came in this message and are not used again.
        unsafe {
            mach_msg(
                reply.as_mut_ptr(),
                MACH_SEND_MSG,
                EXC_REPLY_SIZE,
                0,
                0,
                0,
                0,
            );
            mach_port_deallocate(mach_task_self_, self.thread);
            mach_port_deallocate(mach_task_self_, self.task);
        }
    }

    fn describe(&self) -> String {
        match self.kind {
            EXC_BAD_ACCESS => format!("a bad memory access at 0x{:016x}", self.codes[1]),
            EXC_BAD_INSTRUCTION => "an illegal instruction".to_string(),
            EXC_ARITHMETIC => "an arithmetic exception".to_string(),
            EXC_BREAKPOINT => "a breakpoint instruction".to_string(),
            other => format!("exception {other}"),
        }
    }
}

/// Read one register set of `thread` into `state`, which holds `count` ints.
fn thread_state(
    thread: MachPort,
    flavor: c_int,
    state: *mut u32,
    count: u32,
) -> Result<(), String> {
    let mut got = count;
    // SAFETY: the caller passes a buffer of `count` ints for this flavor.
    let status = unsafe { thread_get_state(thread, flavor, state, &mut got) };
    if status != KERN_SUCCESS {
        return Err(format!(
            "cannot read a thread's registers: thread_get_state: {}",
            mach_error(status)
        ));
    }
    Ok(())
}

impl Inferior for Task {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn end(&mut self) -> Result<Ending, String> {
        self.ended = true;
        // SAFETY: puts back the handler `new` replaced.
        unsafe {
            if self.previous_interrupt != SIG_ERR {
                signal(SIGINT, self.previous_interrupt);
            }
        }
        let ending = if self.exited.is_some() {
            Ok(Ending::AlreadyExited)
        } else if self.launched {
            let pid = self.pid as c_int;
            let mut status = 0;
            // SAFETY: `status` is a valid out-parameter; `pid` is this
            // process's own child, so reaping it touches nothing else.
            if unsafe { waitpid(pid, &mut status, WNOHANG) } == pid {
                Ok(Ending::AlreadyExited)
            } else {
                kill_and_reap(self.pid);
                Ok(Ending::Killed)
            }
        } else {
            // A program left running must not meet a `brk`, or a watchpoint,
            // with no debugger.
            self.watchpoints.clear();
            let lifted = self.lift().and_then(|()| self.arm_watchpoints());
            // SAFETY: `self.task` is the task port `attach` acquired.
            let status = unsafe { task_resume(self.task) };
            if let Err(message) = lifted {
                Err(message)
            } else if status == KERN_SUCCESS {
                Ok(Ending::Detached)
            } else if !alive(self.pid) {
                Ok(Ending::AlreadyExited)
            } else {
                Err(format!(
                    "cannot resume process {}: task_resume: {}",
                    self.pid,
                    mach_error(status)
                ))
            }
        };
        release(self.task);
        ending
    }

    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        let slide = self.slide()?;
        let image = self.image.take().expect("slide reads the image");
        let mut functions = Vec::with_capacity(image.functions.len());
        let mut failure = None;
        for function in &image.functions {
            let (linked, address, size) = match &function.location {
                Location::Defined { address, size } => {
                    (false, Some(address.wrapping_add(slide)), Some(*size))
                }
                Location::Import(None) => (true, None, None),
                Location::Import(Some(slot)) => {
                    match self.read(slot.address.wrapping_add(slide), 8) {
                        // A slot that still holds the file's bytes is unbound.
                        Ok(live) if live[..] == slot.unbound[..] => (true, None, None),
                        Ok(live) => (
                            true,
                            Some(u64::from_le_bytes(live[..].try_into().expect("8 bytes"))),
                            None,
                        ),
                        Err(message) => {
                            failure = Some(message);
                            break;
                        }
                    }
                }
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
        let slide = self.slide()?;
        Ok(self
            .image()?
            .data
            .iter()
            .find(|(symbol, _)| symbol == name)
            .map(|(_, address)| address.wrapping_add(slide)))
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
        let slide = self.slide()?;
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
                } if start.wrapping_add(slide) <= address
                    && address < start.wrapping_add(slide).wrapping_add(size) =>
                {
                    Some((function.name.clone(), start.wrapping_add(slide), size))
                }
                _ => None,
            });
        let stub = image.stubs.iter().find_map(|(name, start, size)| {
            let start = start.wrapping_add(slide);
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
        let Some(index) = self
            .libraries
            .iter()
            .position(|library| library.text.0 <= address && address < library.text.1)
        else {
            return Ok(None);
        };
        if let Some(stub) = self.library_stub(index, address) {
            return Ok(Some(stub));
        }
        let starts = self.library_starts(index);
        let Some(start) = starts
            .iter()
            .copied()
            .filter(|start| *start <= address)
            .max()
        else {
            return Ok(None);
        };
        let function = self.library_function(index, start);
        if function.size.is_some_and(|size| address >= start + size) {
            return Ok(None);
        }
        Ok(Some(function))
    }

    fn data_symbol_at(&mut self, address: u64) -> Result<Option<String>, String> {
        let slide = self.slide()?;
        Ok(self
            .image()?
            .data
            .iter()
            .find(|(_, start)| start.wrapping_add(slide) == address)
            .map(|(name, _)| name.clone()))
    }

    fn library_functions(&mut self, name: &str) -> Result<Vec<FunctionRange>, String> {
        self.refresh_libraries()?;
        let mut found = Vec::new();
        for index in 0..self.libraries.len() {
            let starts = self
                .library_symbols(index)
                .into_iter()
                .filter(|(_, symbol, _)| symbol == name)
                .map(|(address, ..)| address)
                .collect::<Vec<_>>();
            for start in starts {
                found.push(self.library_function(index, start));
            }
        }
        Ok(found)
    }

    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        let threads = self.thread_ports()?;
        threads.0.iter().map(|thread| self.facts(*thread)).collect()
    }

    fn registers(&mut self, index: usize) -> Result<Vec<Register>, String> {
        let threads = self.thread_ports()?;
        let thread = self.thread_at(&threads, index)?;
        let general = general_state(thread)?;
        let neon = neon_state(thread)?;
        Ok(arm64::registers(&general, &neon))
    }

    fn set_registers(&mut self, index: usize, values: &[(String, u64)]) -> Result<(), String> {
        let threads = self.thread_ports()?;
        let thread = self.thread_at(&threads, index)?;
        let mut general = general_state(thread)?;
        let mut neon = None;
        // The flags are CPSR's top four bits; the rest of the word (and
        // `__pad` above it) is left as it is.
        for (name, value) in values {
            arm64::assign(
                &mut general,
                &mut neon,
                &mut || neon_state(thread),
                name,
                *value,
            )?;
        }
        set_thread_state(
            thread,
            ARM_THREAD_STATE64,
            general.as_ptr().cast(),
            ARM_THREAD_STATE64_COUNT,
            "registers",
        )?;
        if let Some(state) = neon {
            set_thread_state(
                thread,
                ARM_NEON_STATE64,
                ptr::addr_of!(state).cast(),
                ARM_NEON_STATE64_COUNT,
                "floating-point control registers",
            )?;
        }
        Ok(())
    }

    fn step(&mut self, indices: &[usize], count: u64) -> Result<Vec<Step>, String> {
        let threads = self.thread_ports()?;
        let stepping = indices
            .iter()
            .map(|index| self.thread_at(&threads, *index))
            .collect::<Result<Vec<_>, _>>()?;
        let port = ExceptionPort::install(self.task)?;
        // SAFETY: each is a thread port of this task; every hold taken here is
        // given back below.
        let held = threads
            .0
            .iter()
            .copied()
            .filter(|thread| !stepping.contains(thread))
            .filter(|thread| unsafe { thread_suspend(*thread) } == KERN_SUCCESS)
            .collect::<Vec<_>>();
        let steps = self
            .lift()
            .and_then(|()| self.pause_watchpoints(&stepping, false))
            .and_then(|()| self.step_rounds(&stepping, &port, count));
        for thread in held {
            // SAFETY: drops the hold taken above.
            unsafe {
                thread_resume(thread);
            }
        }
        drop(port);
        if let Ok(steps) = &steps {
            if steps.iter().any(|step| step.stop == StepStop::Exited) {
                self.exited = Some(self.check_exit().unwrap_or(Exit::Unknown));
                return Ok(steps.clone());
            }
        }
        // A step that failed part way still puts every breakpoint and
        // watchpoint back: the session holds them as set.
        let restored = self
            .reinsert()
            .and_then(|()| self.pause_watchpoints(&stepping, true));
        let steps = steps?;
        restored?;
        Ok(steps)
    }

    fn restart(&mut self) -> Result<u32, String> {
        if !self.launched {
            return Err(
                "restart needs a program the debugger launched; this one was attached to"
                    .to_string(),
            );
        }
        if self.exited.is_none() {
            kill_and_reap(self.pid);
        }
        release(self.task);
        let (pid, task) = spawn(&self.program, &self.args)?;
        self.pid = pid;
        self.task = task;
        // A new process: nothing of the old one carries over, and the file is
        // read again in case it was rebuilt.
        self.image = None;
        self.slide = None;
        self.breakpoints.clear();
        self.watchpoints.clear();
        self.exited = None;
        self.libraries.clear();
        self.library_count = 0;
        Ok(pid)
    }

    fn entry(&mut self) -> Result<Option<u64>, String> {
        let slide = self.slide()?;
        Ok(self.image()?.entry.map(|entry| entry.wrapping_add(slide)))
    }

    fn insert_breakpoint(&mut self, address: u64) -> Result<(), String> {
        if self.breakpoints.contains_key(&address) {
            return Err(format!("there is already a breakpoint at 0x{address:016x}"));
        }
        let original: [u8; 4] = self.read(address, 4)?[..].try_into().expect("4 bytes");
        self.patch(address, &BREAKPOINT_INSTRUCTION)?;
        self.breakpoints.insert(address, original);
        Ok(())
    }

    fn remove_breakpoint(&mut self, address: u64) -> Result<(), String> {
        let Some(original) = self.breakpoints.get(&address).copied() else {
            return Err(format!("there is no breakpoint at 0x{address:016x}"));
        };
        self.patch(address, &original)?;
        self.breakpoints.remove(&address);
        Ok(())
    }

    fn insert_watchpoint(&mut self, address: u64, size: u64) -> Result<(), String> {
        if self.watchpoints.iter().any(|(other, _)| *other == address) {
            return Err(format!("there is already a watchpoint at 0x{address:016x}"));
        }
        let slots = watchpoint_slots()?;
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
        let waiting = {
            let threads = self.thread_ports()?;
            threads
                .0
                .iter()
                .enumerate()
                .filter(|(_, thread)| {
                    general_state(**thread)
                        .is_ok_and(|general| self.breakpoints.contains_key(&general[32]))
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        };
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
        let port = ExceptionPort::install(self.task)?;
        INTERRUPTED.store(false, Ordering::SeqCst);
        // SAFETY: `self.task` is this session's task port.
        if unsafe { task_resume(self.task) } != KERN_SUCCESS {
            return Ok(RunStop::Exited(self.check_exit().unwrap_or(Exit::Unknown)));
        }
        loop {
            if let Some(first) = port.receive(RUN_POLL_MS) {
                // SAFETY: as above.
                let suspended = unsafe { task_suspend(self.task) } == KERN_SUCCESS;
                let mut raised = vec![first];
                while let Some(more) = port.receive(0) {
                    raised.push(more);
                }
                let stop = self.classify(&raised);
                for exception in &raised {
                    exception.answer();
                }
                if !suspended {
                    return Ok(RunStop::Exited(self.check_exit().unwrap_or(Exit::Unknown)));
                }
                return stop;
            }
            if INTERRUPTED.swap(false, Ordering::SeqCst) {
                // SAFETY: as above.
                if unsafe { task_suspend(self.task) } == KERN_SUCCESS {
                    return Ok(RunStop::Interrupted);
                }
            }
            if let Some(exit) = self.check_exit() {
                return Ok(RunStop::Exited(exit));
            }
        }
    }
}

impl Task {
    /// What the exceptions a `run` caught mean: every `brk` at one of the
    /// debugger's breakpoints, else every thread a watchpoint stopped, else
    /// the first other exception. A thread whose watchpoint hit is passed
    /// over runs the store again, and stops again, when the program next
    /// runs.
    fn classify(&self, raised: &[Exception]) -> Result<RunStop, String> {
        let threads = self.thread_ports()?;
        let index = |exception: &Exception| {
            threads
                .0
                .iter()
                .position(|thread| *thread == exception.thread)
                .unwrap_or(usize::MAX)
        };
        let hits = raised
            .iter()
            .filter(|exception| {
                exception.kind == EXC_BREAKPOINT
                    && exception.codes[0] != EXC_ARM_DA_DEBUG
                    && self.breakpoints.contains_key(&(exception.codes[1] as u64))
            })
            .map(|exception| (index(exception), exception.codes[1] as u64))
            .collect::<Vec<_>>();
        if !hits.is_empty() {
            return Ok(RunStop::Breakpoints(hits));
        }
        let mut watched = raised
            .iter()
            .filter(|exception| {
                exception.kind == EXC_BREAKPOINT && exception.codes[0] == EXC_ARM_DA_DEBUG
            })
            .map(index)
            .collect::<Vec<_>>();
        watched.dedup();
        if !watched.is_empty() {
            return Ok(RunStop::Watchpoints(watched));
        }
        let first = &raised[0];
        Ok(RunStop::Exception {
            thread: index(first),
            what: first.describe(),
        })
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
