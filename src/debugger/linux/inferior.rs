//! plan-176-L: the Linux backend's `Inferior` implementation. The `Tracee` handle lives in
//! `debugger/linux/mod.rs`.

use super::*;

impl Inferior for Tracee {
    fn pid(&self) -> u32 {
        self.pid as u32
    }

    fn arch(&self) -> &'static crate::debugger::arch::Arch {
        &crate::debugger::arm64::AARCH64
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
