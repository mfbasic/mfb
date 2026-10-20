//! plan-176-L: the Linux backend's process control — spawning or seizing the program, its
//! threads' task ids, decoding `waitpid` statuses, and killing it. The `Tracee` handle lives in
//! `debugger/linux/mod.rs`.

use super::*;

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
pub(super) fn seize_all(pid: c_int) -> Result<Vec<c_int>, String> {
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
pub(super) fn spawn(
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

pub(super) fn stopped(status: c_int) -> bool {
    status & 0xff == 0x7f
}

pub(super) fn exited(status: c_int) -> bool {
    status & 0x7f == 0
}

fn signaled(status: c_int) -> bool {
    !stopped(status) && !exited(status)
}

pub(super) fn stop_signal(status: c_int) -> c_int {
    (status >> 8) & 0xff
}

pub(super) fn event(status: c_int) -> c_int {
    (status >> 16) & 0xffff
}

/// Kill the child `pid` and reap it. Each of its threads the debugger traces
/// reports its own end first, and the leader can be reaped only after them,
/// so every end is waited for until the leader's.
pub(super) fn kill_and_reap(pid: c_int) {
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
