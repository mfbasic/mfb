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
    abi: RuntimeHelperAbi {
        params: OS_NAME_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_GET_ENV_OR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.getEnvOr",
    abi: RuntimeHelperAbi {
        params: OS_NAME_FALLBACK_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_HAS_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.hasEnv",
    abi: RuntimeHelperAbi {
        params: OS_NAME_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_SET_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.setEnv",
    abi: RuntimeHelperAbi {
        params: OS_NAME_VALUE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_UNSET_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.unsetEnv",
    abi: RuntimeHelperAbi {
        params: OS_NAME_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_ENVIRON_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.environ",
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
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "List OF String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_PID_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.pid",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Integer",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_EXECUTABLE_PATH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.executablePath",
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
    abi: RuntimeHelperAbi {
        params: OS_RELATIVE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_NAME_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.name",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_ARCH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.arch",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_HOST_NAME_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.hostName",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_USER_NAME_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.userName",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const OS_CPU_COUNT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.cpuCount",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Integer",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// Routing/spec parity for every os call is covered by the catalog-driven
// `catalog::tests::catalog_is_consistent` (bug-329), which replaced the
// hand-copied OS_ENV_CALLS array that used to live here.
