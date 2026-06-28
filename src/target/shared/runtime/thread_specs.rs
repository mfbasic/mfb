use super::*;

use crate::arch::aarch64::abi;

const THREAD_START_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "f",
        type_: "ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "data",
        type_: "In",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "inboundLimit",
        type_: "Integer",
        location: "x2",
    },
    RuntimeAbiParam {
        name: "outboundLimit",
        type_: "Integer",
        location: "x3",
    },
];

const THREAD_HANDLE_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "t",
    type_: "Thread OF Msg TO Out",
    location: "x0",
}];

const THREAD_WORKER_HANDLE_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "t",
    type_: "ThreadWorker OF Msg TO Out",
    location: "x0",
}];

const THREAD_SEND_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "Thread OF Msg TO Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "data",
        type_: "Msg",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x2",
    },
];

const THREAD_POLL_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "Thread OF Msg TO Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "ms",
        type_: "Integer",
        location: "x1",
    },
];

const THREAD_RECEIVE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "ThreadWorker OF Msg TO Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x1",
    },
];

const THREAD_PARENT_RECEIVE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "Thread OF Msg TO Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x1",
    },
];

const THREAD_WORKER_SEND_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "ThreadWorker OF Msg TO Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "data",
        type_: "Msg",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x2",
    },
];

pub(crate) const THREAD_START_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.start",
    symbol: "_mfb_rt_thread_thread_start",
    abi: RuntimeHelperAbi {
        params: THREAD_START_PARAMS,
        returns: "Thread OF Msg TO Out",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_IS_RUNNING_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.isRunning",
    symbol: "_mfb_rt_thread_thread_isRunning",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_WAIT_FOR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.waitFor",
    symbol: "_mfb_rt_thread_thread_waitFor",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Out",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_CANCEL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.cancel",
    symbol: "_mfb_rt_thread_thread_cancel",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_DROP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.drop",
    symbol: "_mfb_rt_thread_thread_drop",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_SEND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.send",
    symbol: "_mfb_rt_thread_thread_send",
    abi: RuntimeHelperAbi {
        params: THREAD_SEND_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_POLL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.poll",
    symbol: "_mfb_rt_thread_thread_poll",
    abi: RuntimeHelperAbi {
        params: THREAD_POLL_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.read",
    symbol: "_mfb_rt_thread_thread_read",
    abi: RuntimeHelperAbi {
        params: THREAD_PARENT_RECEIVE_PARAMS,
        returns: "Msg",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_RECEIVE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.receive",
    symbol: "_mfb_rt_thread_thread_receive",
    abi: RuntimeHelperAbi {
        params: THREAD_RECEIVE_PARAMS,
        returns: "Msg",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_EMIT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.emit",
    symbol: "_mfb_rt_thread_thread_emit",
    abi: RuntimeHelperAbi {
        params: THREAD_WORKER_SEND_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_IS_CANCELLED_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.isCancelled",
    symbol: "_mfb_rt_thread_thread_isCancelled",
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
    symbol: "_mfb_rt_thread_thread_transferResource",
    abi: RuntimeHelperAbi {
        params: THREAD_SEND_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_ACCEPT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.acceptResource",
    symbol: "_mfb_rt_thread_thread_acceptResource",
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
    symbol: "_mfb_rt_thread_thread_emitResource",
    abi: RuntimeHelperAbi {
        params: THREAD_WORKER_SEND_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_READ_RESOURCE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.readResource",
    symbol: "_mfb_rt_thread_thread_readResource",
    abi: RuntimeHelperAbi {
        params: THREAD_PARENT_RECEIVE_PARAMS,
        returns: "Msg",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};
