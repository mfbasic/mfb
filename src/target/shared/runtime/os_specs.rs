use super::*;

use crate::arch::aarch64::abi;

// `os::` environment-variable helpers (plan-31-A). Each wraps a libc primitive
// (`getenv`/`setenv`/`unsetenv`, plus the platform environ accessor for the
// snapshot). Values marshal through the standard result registers.
const OS_NAME_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "name",
    type_: "String",
    location: abi::RETURN_REGISTER,
}];

const OS_NAME_FALLBACK_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "name",
        type_: "String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "fallback",
        type_: "String",
        location: "x1",
    },
];

const OS_NAME_VALUE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "name",
        type_: "String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: "x1",
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

#[cfg(test)]
mod tests {
    use super::super::{helper_for_call, spec_for_call, RuntimeHelper};

    // Every `os::` env frontend name routes to `RuntimeHelper::Os` and has a
    // registered runtime-helper spec in the catalog (metadata ↔ spec parity).
    const OS_ENV_CALLS: &[&str] = &[
        "os.getEnv",
        "os.getEnvOr",
        "os.hasEnv",
        "os.setEnv",
        "os.unsetEnv",
        "os.environ",
    ];

    #[test]
    fn every_os_env_call_has_spec_and_helper() {
        for call in OS_ENV_CALLS {
            assert_eq!(
                helper_for_call(call),
                Some(RuntimeHelper::Os),
                "helper_for_call {call}"
            );
            let spec = spec_for_call(call).unwrap_or_else(|| panic!("no spec for {call}"));
            assert_eq!(spec.helper, RuntimeHelper::Os, "{call}");
            assert!(!spec.abi.returns.is_empty(), "{call} returns set");
        }
    }
}
