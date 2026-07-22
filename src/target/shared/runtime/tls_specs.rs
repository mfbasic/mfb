use super::*;

// TLS (transport-layer security; Linux/OpenSSL backend, plan-03-net.md §4).
// Split out of net_specs.rs by bug-329 so every spec file maps to exactly one
// RuntimeHelper family.

pub(crate) const TLS_CONNECT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.connect",
    abi: RuntimeHelperAbi {
        returns: "TlsSocket",
    },
};

pub(crate) const TLS_LISTEN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.listen",
    abi: RuntimeHelperAbi {
        returns: "TlsListener",
    },
};

pub(crate) const TLS_ACCEPT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.accept",
    abi: RuntimeHelperAbi {
        returns: "TlsSocket",
    },
};

pub(crate) const TLS_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.read",
    abi: RuntimeHelperAbi {
        returns: "List OF Byte",
    },
};

pub(crate) const TLS_READ_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.readText",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const TLS_WRITE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.write",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TLS_WRITE_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.writeText",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TLS_CLOSE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.close",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TLS_CLOSE_LISTENER_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.closeListener",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};
