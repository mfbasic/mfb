//! plan-176-L: the Linux backend's threads — register sets, watchpoints, resuming and waiting,
//! thread birth and death, stop classification and stepping. The `Tracee` handle lives in
//! `debugger/linux/mod.rs`.

use super::*;

impl Tracee {
    /// One register set of thread `tid`.
    pub(super) fn regset<T: Copy>(&self, tid: c_int, kind: usize, empty: T) -> Result<T, String> {
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

    pub(super) fn set_regset<T: Copy>(
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

    pub(super) fn general(&self, tid: c_int) -> Result<General, String> {
        self.regset(tid, NT_PRSTATUS, cpu::NO_GENERAL)
    }

    pub(super) fn float(&self, tid: c_int) -> Result<Float, String> {
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

    /// Write the watchpoints into every thread.
    pub(super) fn arm_watchpoints(&self) -> Result<(), String> {
        for thread in &self.threads {
            if !thread.fresh {
                self.set_thread_watchpoints(thread.tid, true)?;
            }
        }
        Ok(())
    }

    /// Put every breakpoint's original instruction back, for a step or a
    /// detach; [`Tracee::reinsert`] writes them again.
    pub(super) fn lift(&self) -> Result<(), String> {
        for (address, original) in &self.breakpoints {
            self.write(*address, original)?;
        }
        Ok(())
    }

    pub(super) fn reinsert(&self) -> Result<(), String> {
        for (address, original) in &self.breakpoints {
            self.write(*address, cpu::breakpoint(original))?;
        }
        Ok(())
    }

    pub(super) fn index_of(&self, tid: c_int) -> Option<usize> {
        self.threads.iter().position(|thread| thread.tid == tid)
    }

    pub(super) fn thread_at(&self, index: usize) -> Result<c_int, String> {
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
    pub(super) fn resume(&mut self, index: usize, step: bool) -> Result<(), String> {
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
    pub(super) fn wait_any(&self, block: bool) -> Result<Option<(c_int, c_int)>, i32> {
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
    pub(super) fn thread_ended(&mut self, tid: c_int, status: c_int) -> bool {
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
    pub(super) fn adopt(&mut self, tid: c_int) -> usize {
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
    pub(super) fn welcome(&mut self, index: usize) -> Result<(), String> {
        self.threads[index].fresh = false;
        if !self.watchpoints.is_empty() {
            self.set_thread_watchpoints(self.threads[index].tid, true)?;
        }
        Ok(())
    }

    /// What a signal-delivery-stop of thread `index` for `signal` is: a report,
    /// or a signal passed on to the program.
    pub(super) fn signal_stop(
        &mut self,
        index: usize,
        signal: c_int,
    ) -> Result<Option<Report>, String> {
        let tid = self.threads[index].tid;
        let (code, address) = self.siginfo(tid);
        let report = match signal {
            SIGTRAP if code == TRAP_HWBKPT && !self.watchpoints.is_empty() => Some(Report::Watch),
            SIGTRAP if code == TRAP_BRKPT => {
                let pc = cpu::pc(&self.general(tid)?);
                if self.breakpoints.contains_key(&pc) {
                    Some(Report::Breakpoint(pc))
                } else if !cpu::HARDWARE_STEP && self.on_step_trap(tid)? {
                    // A step's own breakpoint, reached as the step was given
                    // up on: the step is over, and nothing is the program's.
                    None
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
    pub(super) fn stop_all(&mut self) -> Result<Vec<(c_int, Report)>, String> {
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
    pub(super) fn classify(&self, reports: &[(c_int, Report)]) -> RunStop {
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
    pub(super) fn check_exit(&mut self) -> Option<Exit> {
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
    /// instruction each; breakpoints are lifted and watchpoints off. Where the
    /// kernel steps a thread the round's threads step together; where a step
    /// plants breakpoints every thread could reach, they step one after
    /// another, so only the thread whose breakpoints they are runs.
    pub(super) fn step_rounds(
        &mut self,
        stepping: &[usize],
        count: u64,
    ) -> Result<Vec<Step>, String> {
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
            let groups = if cpu::HARDWARE_STEP {
                vec![active.clone()]
            } else {
                active.iter().map(|at| vec![*at]).collect()
            };
            let mut still = Vec::new();
            for group in groups {
                let ended = self.step_group(&tids, &group, &mut steps, &mut still);
                // A step's own breakpoints come out however it ended.
                let cleared = self.end_step_traps();
                if ended? {
                    return Ok(steps);
                }
                cleared?;
            }
            active = still;
        }
        Ok(steps)
    }

    /// One instruction of the threads `group` (positions in `tids`):
    /// start them, then wait until each has stepped, stopped for something
    /// else, or is found blocked in the kernel. A thread that stepped joins
    /// `still`. Answers `true` when the program ended.
    fn step_group(
        &mut self,
        tids: &[c_int],
        group: &[usize],
        steps: &mut [Step],
        still: &mut Vec<usize>,
    ) -> Result<bool, String> {
        let mut waiting = Vec::new();
        for at in group {
            if let Some(index) = self.index_of(tids[*at]) {
                if self.start_step(index)? {
                    waiting.push(*at);
                } else {
                    steps[*at].stop = StepStop::Exception("a breakpoint instruction".to_string());
                }
            }
        }
        let started = Instant::now();
        let mut deadline = started + STEP_SLICE;
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
                            for step in steps.iter_mut() {
                                step.stop = StepStop::Exited;
                            }
                            return Ok(true);
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
                    for step in steps.iter_mut() {
                        step.stop = StepStop::Exited;
                    }
                    return Ok(true);
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
                    self.continue_step(index)?;
                }
                PTRACE_EVENT_STOP => self.continue_step(index)?,
                0 => match stop_signal(status) {
                    SIGTRAP => {
                        let (code, _) = self.siginfo(tid);
                        // The kernel's step reports a trap that is no
                        // breakpoint; a planted step ends on one of its
                        // own breakpoints.
                        let stepped = if cpu::HARDWARE_STEP {
                            code != TRAP_BRKPT
                        } else {
                            code == TRAP_BRKPT && self.on_step_trap(tid)?
                        };
                        if stepped {
                            steps[at].taken += 1;
                            still.push(at);
                        } else if code == TRAP_BRKPT {
                            steps[at].stop =
                                StepStop::Exception("a breakpoint instruction".to_string());
                            self.threads[index].pending = SIGTRAP;
                        } else {
                            // A trap no step made: as at a run.
                            if let Some(Report::Fault(what)) = self.signal_stop(index, SIGTRAP)? {
                                steps[at].stop = StepStop::Exception(what);
                            } else {
                                self.continue_step(index)?;
                                continue;
                            }
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
                        _ => self.continue_step(index)?,
                    },
                },
                _ => self.continue_step(index)?,
            }
        }
        Ok(false)
    }

    /// The thread pointer less `pthread_self`: glibc's from the size its
    /// libthread_db description gives `struct pthread`; musl's from the
    /// descriptor's first word, which points at the descriptor itself.
    pub(super) fn pthread_gap(&mut self, tp: u64) -> Option<u64> {
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
                .map(cpu::glibc_gap)
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

    pub(super) fn facts(&mut self, tid: c_int) -> Result<ThreadFacts, String> {
        let general = self.general(tid)?;
        let tp = self.thread_pointer(tid, &general);
        let handle = self.pthread_gap(tp).map_or(0, |gap| tp.wrapping_sub(gap));
        let sp = cpu::sp(&general);
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
            pc: cpu::pc(&general),
            sp,
            arena: cpu::arena(&general),
            stack,
        })
    }

    /// Detach from a program left running: no `brk` and no watchpoint may be
    /// left behind.
    pub(super) fn detach(&mut self) -> Result<(), String> {
        self.watchpoints.clear();
        let lifted = self.lift().and_then(|()| self.arm_watchpoints());
        for thread in &self.threads {
            let _ = trace(PTRACE_DETACH, thread.tid, 0, thread.pending as usize);
        }
        lifted
    }
}
