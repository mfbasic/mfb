use super::*;

pub(crate) const THREAD_START_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.start",
    abi: RuntimeHelperAbi {
        returns: "Thread OF Msg TO Out",
    },
};

pub(crate) const THREAD_IS_RUNNING_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.isRunning",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const THREAD_WAIT_FOR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.waitFor",
    abi: RuntimeHelperAbi { returns: "Out" },
};

pub(crate) const THREAD_CANCEL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.cancel",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

// plan-15 §4.5. The single handle param carries either a parent `Thread` handle
// (subscribe that worker) or a null sentinel (subscribe the calling thread); the
// no-arg source form is padded to the null sentinel in `lower_runtime_helper_call`.
pub(crate) const THREAD_OPEN_STD_IN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.openStdIn",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const THREAD_CLOSE_STD_IN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.closeStdIn",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const THREAD_DROP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.drop",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const THREAD_SEND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.send",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const THREAD_POLL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.poll",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const THREAD_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.read",
    abi: RuntimeHelperAbi { returns: "Msg" },
};

pub(crate) const THREAD_RECEIVE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.receive",
    abi: RuntimeHelperAbi { returns: "Msg" },
};

pub(crate) const THREAD_EMIT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.emit",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const THREAD_IS_CANCELLED_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.isCancelled",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

// Resource plane (§7): `thread::transfer`/`thread::accept` mirror `send`/`receive`
// but run on a separate per-thread resource queue.
pub(crate) const THREAD_TRANSFER_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.transferResource",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const THREAD_ACCEPT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.acceptResource",
    abi: RuntimeHelperAbi { returns: "Msg" },
};

// Resource plane, worker→parent direction: `emitResource` mirrors `emit` and
// `readResource` mirrors `read`, but run on the outbound resource queue. A
// source `thread::transfer`/`thread::accept` on a `ThreadWorker`/`Thread` handle
// respectively lowers here (see `builder_values`).
pub(crate) const THREAD_EMIT_RESOURCE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.emitResource",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const THREAD_READ_RESOURCE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.readResource",
    abi: RuntimeHelperAbi { returns: "Msg" },
};
