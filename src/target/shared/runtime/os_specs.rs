use super::*;

use crate::target::shared::abi;

// `os::` environment-variable helpers (plan-31-A). Each wraps a libc primitive
// (`getenv`/`setenv`/`unsetenv`, plus the platform environ accessor for the
// snapshot). Values marshal through the standard result registers.
const OS_NAME_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "name",
    type_: "String",
    location: abi::ARG[0],
}];

const OS_NAME_FALLBACK_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "name",
        type_: "String",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "fallback",
        type_: "String",
        location: abi::ARG[1],
    },
];

const OS_NAME_VALUE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "name",
        type_: "String",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: abi::ARG[1],
    },
];

pub(crate) const OS_GET_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.getEnv",
    symbol: "_mfb_rt_os_os_getEnv",
    abi: RuntimeHelperAbi {
        params: OS_NAME_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_GET_ENV_OR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.getEnvOr",
    symbol: "_mfb_rt_os_os_getEnvOr",
    abi: RuntimeHelperAbi {
        params: OS_NAME_FALLBACK_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_HAS_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.hasEnv",
    symbol: "_mfb_rt_os_os_hasEnv",
    abi: RuntimeHelperAbi {
        params: OS_NAME_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_SET_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.setEnv",
    symbol: "_mfb_rt_os_os_setEnv",
    abi: RuntimeHelperAbi {
        params: OS_NAME_VALUE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_UNSET_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.unsetEnv",
    symbol: "_mfb_rt_os_os_unsetEnv",
    abi: RuntimeHelperAbi {
        params: OS_NAME_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_ENVIRON_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.environ",
    symbol: "_mfb_rt_os_os_environ",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Map OF String TO String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// Process & platform introspection (plan-31-B). All nullary.
pub(crate) const OS_ARGS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.args",
    symbol: "_mfb_rt_os_os_args",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "List OF String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_PID_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.pid",
    symbol: "_mfb_rt_os_os_pid",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Integer",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_EXECUTABLE_PATH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.executablePath",
    symbol: "_mfb_rt_os_os_executablePath",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// The resource locator (plan-55-B) is the one `os::` call taking an argument: a
// `String` build-relative resource path, in ARG[0].
const OS_RELATIVE_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "relative",
    type_: "String",
    location: abi::ARG[0],
}];

pub(crate) const OS_RESOURCE_PATH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.resourcePath",
    symbol: "_mfb_rt_os_os_resourcePath",
    abi: RuntimeHelperAbi {
        params: OS_RELATIVE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_NAME_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.name",
    symbol: "_mfb_rt_os_os_name",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_ARCH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.arch",
    symbol: "_mfb_rt_os_os_arch",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_HOST_NAME_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.hostName",
    symbol: "_mfb_rt_os_os_hostName",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_USER_NAME_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.userName",
    symbol: "_mfb_rt_os_os_userName",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_CPU_COUNT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.cpuCount",
    symbol: "_mfb_rt_os_os_cpuCount",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Integer",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// Routing/spec parity for every os call is covered by the catalog-driven
// `catalog::tests::catalog_is_consistent` (bug-329), which replaced the
// hand-copied OS_ENV_CALLS array that used to live here.
