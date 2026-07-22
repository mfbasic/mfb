use super::*;

// `os::` environment-variable helpers (plan-31-A). Each wraps a libc primitive
// (`getenv`/`setenv`/`unsetenv`, plus the platform environ accessor for the
// snapshot). Values marshal through the standard result registers.
pub(crate) const OS_GET_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.getEnv",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const OS_GET_ENV_OR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.getEnvOr",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const OS_HAS_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.hasEnv",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const OS_SET_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.setEnv",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const OS_UNSET_ENV_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.unsetEnv",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const OS_ENVIRON_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.environ",
    abi: RuntimeHelperAbi {
        returns: "Map OF String TO String",
    },
};

// Process & platform introspection (plan-31-B). All nullary.
pub(crate) const OS_ARGS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.args",
    abi: RuntimeHelperAbi {
        returns: "List OF String",
    },
};

pub(crate) const OS_PID_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.pid",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

pub(crate) const OS_EXECUTABLE_PATH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.executablePath",
    abi: RuntimeHelperAbi { returns: "String" },
};

// The resource locator (plan-55-B) is the one `os::` call taking an argument: a
// `String` build-relative resource path, in ARG[0].
pub(crate) const OS_RESOURCE_PATH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.resourcePath",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const OS_NAME_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.name",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const OS_ARCH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.arch",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const OS_HOST_NAME_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.hostName",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const OS_USER_NAME_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.userName",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const OS_CPU_COUNT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Os,
    call: "os.cpuCount",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

// Routing/spec parity for every os call is covered by the catalog-driven
// `catalog::tests::catalog_is_consistent` (bug-329), which replaced the
// hand-copied OS_ENV_CALLS array that used to live here.
