//! plan-176-L: the macOS backend's Mach exception port — installing it, receiving an exception
//! message, and answering it. The `Task` handle lives in `debugger/macos/mod.rs`.

use super::*;

/// A receive right this process listens on for the task's exceptions, with
/// the handlers it replaced; dropping it puts those back and destroys it.
pub(super) struct ExceptionPort {
    task: MachPort,
    port: MachPort,
    saved: Vec<(u32, MachPort, c_int, c_int)>,
}

impl ExceptionPort {
    /// Route `task`'s step-related exceptions to a fresh port.
    pub(super) fn install(task: MachPort) -> Result<Self, String> {
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
    pub(super) fn receive(&self, timeout: u32) -> Option<Exception> {
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
pub(super) struct Exception {
    bits: u32,
    reply: MachPort,
    id: u32,
    pub(super) thread: MachPort,
    task: MachPort,
    pub(super) kind: i32,
    pub(super) codes: [i64; 2],
}

impl Exception {
    /// Answer "handled", so the thread retries or continues when it next runs,
    /// and drop the thread and task rights the message carried.
    pub(super) fn answer(&self) {
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

    pub(super) fn describe(&self) -> String {
        match self.kind {
            EXC_BAD_ACCESS => format!("a bad memory access at 0x{:016x}", self.codes[1]),
            EXC_BAD_INSTRUCTION => "an illegal instruction".to_string(),
            EXC_ARITHMETIC => "an arithmetic exception".to_string(),
            EXC_BREAKPOINT => "a breakpoint instruction".to_string(),
            other => format!("exception {other}"),
        }
    }
}
