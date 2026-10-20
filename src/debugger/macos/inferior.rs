//! plan-176-L: the macOS backend's `Inferior` implementation. The `Task` handle lives in
//! `debugger/macos/mod.rs`.

use super::*;

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

    fn restart(&mut self, environment: &[(OsString, OsString)]) -> Result<u32, String> {
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
        let (pid, task) = spawn(&self.program, &self.args, environment)?;
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
