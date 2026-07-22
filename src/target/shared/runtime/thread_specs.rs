use super::*;

use crate::target::shared::abi;

const THREAD_START_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "f",
        type_: "ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "data",
        type_: "In",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "inboundLimit",
        type_: "Integer",
        location: abi::ARG[2],
    },
    RuntimeAbiParam {
        name: "outboundLimit",
        type_: "Integer",
        location: abi::ARG[3],
    },
];

const THREAD_HANDLE_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "t",
    type_: "Thread OF Msg TO Out",
    location: abi::ARG[0],
}];

const THREAD_WORKER_HANDLE_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "t",
    type_: "ThreadWorker OF Msg TO Out",
    location: abi::ARG[0],
}];

const THREAD_SEND_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "Thread OF Msg TO Out",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "data",
        type_: "Msg",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: abi::ARG[2],
    },
];

const THREAD_POLL_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "Thread OF Msg TO Out",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "ms",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

const THREAD_RECEIVE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "ThreadWorker OF Msg TO Out",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

const THREAD_PARENT_RECEIVE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "Thread OF Msg TO Out",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

const THREAD_WORKER_SEND_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "ThreadWorker OF Msg TO Out",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "data",
        type_: "Msg",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: abi::ARG[2],
    },
];

pub(crate) const THREAD_START_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.start",
    abi: RuntimeHelperAbi {
        params: THREAD_START_PARAMS,
        returns: "Thread OF Msg TO Out",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_IS_RUNNING_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.isRunning",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_WAIT_FOR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.waitFor",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Out",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_CANCEL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.cancel",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// plan-15 §4.5. The single handle param carries either a parent `Thread` handle
// (subscribe that worker) or a null sentinel (subscribe the calling thread); the
// no-arg source form is padded to the null sentinel in `lower_runtime_helper_call`.
pub(crate) const THREAD_OPEN_STD_IN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.openStdIn",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_CLOSE_STD_IN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.closeStdIn",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_DROP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.drop",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_SEND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.send",
    abi: RuntimeHelperAbi {
        params: THREAD_SEND_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_POLL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.poll",
    abi: RuntimeHelperAbi {
        params: THREAD_POLL_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.read",
    abi: RuntimeHelperAbi {
        params: THREAD_PARENT_RECEIVE_PARAMS,
        returns: "Msg",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_RECEIVE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.receive",
    abi: RuntimeHelperAbi {
        params: THREAD_RECEIVE_PARAMS,
        returns: "Msg",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_EMIT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.emit",
    abi: RuntimeHelperAbi {
        params: THREAD_WORKER_SEND_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_IS_CANCELLED_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.isCancelled",
    abi: RuntimeHelperAbi {
        params: THREAD_WORKER_HANDLE_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// Resource plane (§7): `thread::transfer`/`thread::accept` mirror `send`/`receive`
// but run on a separate per-thread resource queue.
pub(crate) const THREAD_TRANSFER_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.transferResource",
    abi: RuntimeHelperAbi {
        params: THREAD_SEND_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_ACCEPT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.acceptResource",
    abi: RuntimeHelperAbi {
        params: THREAD_RECEIVE_PARAMS,
        returns: "Msg",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// Resource plane, worker→parent direction: `emitResource` mirrors `emit` and
// `readResource` mirrors `read`, but run on the outbound resource queue. A
// source `thread::transfer`/`thread::accept` on a `ThreadWorker`/`Thread` handle
// respectively lowers here (see `builder_values`).
pub(crate) const THREAD_EMIT_RESOURCE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.emitResource",
    abi: RuntimeHelperAbi {
        params: THREAD_WORKER_SEND_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_READ_RESOURCE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.readResource",
    abi: RuntimeHelperAbi {
        params: THREAD_PARENT_RECEIVE_PARAMS,
        returns: "Msg",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};
