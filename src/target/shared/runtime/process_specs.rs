use super::*;

// plan-90 `process` package runtime helpers. `spawn` has two code-layer forms
// selected by argument count in `builder_values` (like `net.connectTcp` →
// `net.connectTcpAddr`): the bare argv form and the full form carrying a working
// directory + environment map. `__drop` is the internal scope-drop op (SIGKILL +
// waitpid); it is not a source-level call but is dispatched as a runtime helper
// so the resource-cleanup path can emit it.

pub(crate) const PROCESS_SPAWN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Process,
    call: "process.spawn",
    abi: RuntimeHelperAbi { returns: "Process" },
};

pub(crate) const PROCESS_SPAWN_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Process,
    call: "process.spawnEnv",
    abi: RuntimeHelperAbi { returns: "Process" },
};

pub(crate) const PROCESS_SHELL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Process,
    call: "process.shell",
    abi: RuntimeHelperAbi { returns: "Process" },
};

pub(crate) const PROCESS_PID_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Process,
    call: "process.pid",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

pub(crate) const PROCESS_IS_RUNNING_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Process,
    call: "process.isRunning",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const PROCESS_WAIT_FOR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Process,
    call: "process.waitFor",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

pub(crate) const PROCESS_CLOSE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Process,
    call: "process.close",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

// The scope-drop cleanup op (SIGKILL + waitpid + close pipes). Code-layer-only:
// synthesized by the resource-cleanup path, never written in source.
pub(crate) const PROCESS_DROP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Process,
    call: "process.__drop",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};
