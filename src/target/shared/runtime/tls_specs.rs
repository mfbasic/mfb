use super::*;

use crate::target::shared::abi;

// TLS (transport-layer security; Linux/OpenSSL backend, plan-03-net.md §4).
// Split out of net_specs.rs by bug-329 so every spec file maps to exactly one
// RuntimeHelper family.

const TLS_CONNECT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "host",
        type_: "String",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "port",
        type_: "Integer",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: abi::ARG[2],
    },
    RuntimeAbiParam {
        name: "serverName",
        type_: "String",
        location: abi::ARG[3],
    },
];

const TLS_SOCKET_INT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "TlsSocket",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "maxBytes",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

const TLS_SOCKET_BYTES_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "TlsSocket",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "bytes",
        type_: "List OF Byte",
        location: abi::ARG[1],
    },
];

const TLS_SOCKET_STRING_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "TlsSocket",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: abi::ARG[1],
    },
];

const TLS_SOCKET_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "sock",
    type_: "TlsSocket",
    location: abi::ARG[0],
}];

const TLS_LISTEN_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "host",
        type_: "String",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "port",
        type_: "Integer",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "certPath",
        type_: "String",
        location: abi::ARG[2],
    },
    RuntimeAbiParam {
        name: "keyPath",
        type_: "String",
        location: abi::ARG[3],
    },
    RuntimeAbiParam {
        name: "backlog",
        type_: "Integer",
        location: abi::ARG[4],
    },
];

const TLS_ACCEPT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "listener",
        type_: "TlsListener",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

const TLS_LISTENER_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "listener",
    type_: "TlsListener",
    location: abi::ARG[0],
}];

pub(crate) const TLS_CONNECT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.connect",
    symbol: "_mfb_rt_tls_tls_connect",
    abi: RuntimeHelperAbi {
        params: TLS_CONNECT_PARAMS,
        returns: "TlsSocket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_LISTEN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.listen",
    symbol: "_mfb_rt_tls_tls_listen",
    abi: RuntimeHelperAbi {
        params: TLS_LISTEN_PARAMS,
        returns: "TlsListener",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_ACCEPT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.accept",
    symbol: "_mfb_rt_tls_tls_accept",
    abi: RuntimeHelperAbi {
        params: TLS_ACCEPT_PARAMS,
        returns: "TlsSocket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.read",
    symbol: "_mfb_rt_tls_tls_read",
    abi: RuntimeHelperAbi {
        params: TLS_SOCKET_INT_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_READ_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.readText",
    symbol: "_mfb_rt_tls_tls_readText",
    abi: RuntimeHelperAbi {
        params: TLS_SOCKET_INT_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_WRITE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.write",
    symbol: "_mfb_rt_tls_tls_write",
    abi: RuntimeHelperAbi {
        params: TLS_SOCKET_BYTES_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_WRITE_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.writeText",
    symbol: "_mfb_rt_tls_tls_writeText",
    abi: RuntimeHelperAbi {
        params: TLS_SOCKET_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_CLOSE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.close",
    symbol: "_mfb_rt_tls_tls_close",
    abi: RuntimeHelperAbi {
        params: TLS_SOCKET_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_CLOSE_LISTENER_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.closeListener",
    symbol: "_mfb_rt_tls_tls_closeListener",
    abi: RuntimeHelperAbi {
        params: TLS_LISTENER_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};
