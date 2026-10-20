//! plan-176-L: the macOS backend's threads — memory patching, watchpoints, thread state,
//! stepping and stop classification. The `Task` handle lives in `debugger/macos/mod.rs`.

use super::*;

impl Task {
    /// Write `bytes` over the code at `address`: the page is made writable
    /// (as a private copy) for the write, then executable again.
    pub(super) fn patch(&self, address: u64, bytes: &[u8; 4]) -> Result<(), String> {
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
    pub(super) fn arm_watchpoints(&self) -> Result<(), String> {
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
    pub(super) fn pause_watchpoints(&self, threads: &[MachPort], on: bool) -> Result<(), String> {
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
    pub(super) fn lift(&self) -> Result<(), String> {
        for (address, original) in &self.breakpoints {
            self.patch(*address, original)?;
        }
        Ok(())
    }

    pub(super) fn reinsert(&self) -> Result<(), String> {
        for address in self.breakpoints.keys() {
            self.patch(*address, &BREAKPOINT_INSTRUCTION)?;
        }
        Ok(())
    }

    /// How the program ended, if it has: a launched child is reaped and its
    /// status read; an attached program is gone when its task port no longer
    /// names a process.
    pub(super) fn check_exit(&mut self) -> Option<Exit> {
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

    /// `length` bytes of the process's memory at `address`.
    pub(super) fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
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
    pub(super) fn thread_ports(&self) -> Result<ThreadPorts, String> {
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
    pub(super) fn thread_at(
        &self,
        threads: &ThreadPorts,
        index: usize,
    ) -> Result<MachPort, String> {
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

    pub(super) fn facts(&self, thread: MachPort) -> Result<ThreadFacts, String> {
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
    pub(super) fn step_rounds(
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
pub(super) struct ThreadPorts(pub(super) Vec<MachPort>);

impl Drop for ThreadPorts {
    fn drop(&mut self) {
        for port in &self.0 {
            release(*port);
        }
    }
}

/// `ARM_THREAD_STATE64` as words: x0–x28, fp, lr, sp, pc, then CPSR.
pub(super) fn general_state(thread: MachPort) -> Result<[u64; 34], String> {
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
pub(super) fn neon_state(thread: MachPort) -> Result<Float, String> {
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
pub(super) fn set_thread_state(
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
pub(super) fn watchpoint_slots() -> Result<usize, String> {
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

impl Task {
    /// What the exceptions a `run` caught mean: every `brk` at one of the
    /// debugger's breakpoints, else every thread a watchpoint stopped, else
    /// the first other exception. A thread whose watchpoint hit is passed
    /// over runs the store again, and stops again, when the program next
    /// runs.
    pub(super) fn classify(&self, raised: &[Exception]) -> Result<RunStop, String> {
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
