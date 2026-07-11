use super::*;

use crate::target::shared::abi;

const NET_HOST_PORT_PARAMS: &[RuntimeAbiParam] = &[
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
];

const NET_CONNECT_TCP_PARAMS: &[RuntimeAbiParam] = &[
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
];

const NET_LISTEN_TCP_PARAMS: &[RuntimeAbiParam] = &[
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
        name: "backlog",
        type_: "Integer",
        location: abi::ARG[2],
    },
];

const NET_SOCKET_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "sock",
    type_: "Socket",
    location: abi::ARG[0],
}];

const NET_SOCKET_TIMEOUT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "Socket",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

const NET_LISTENER_TIMEOUT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "listener",
        type_: "Listener",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

const NET_SOCKET_INT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "Socket",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "maxBytes",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

const NET_SOCKET_BYTES_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "Socket",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "bytes",
        type_: "List OF Byte",
        location: abi::ARG[1],
    },
];

const NET_SOCKET_STRING_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "Socket",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: abi::ARG[1],
    },
];

pub(crate) const NET_LOOKUP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.lookup",
    symbol: "_mfb_rt_net_net_lookup",
    abi: RuntimeHelperAbi {
        params: NET_HOST_PORT_PARAMS,
        returns: "List OF Address",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_CONNECT_TCP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.connectTcp",
    symbol: "_mfb_rt_net_net_connectTcp",
    abi: RuntimeHelperAbi {
        params: NET_CONNECT_TCP_PARAMS,
        returns: "Socket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

const NET_CONNECT_TCP_ADDR_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "address",
        type_: "Address",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

pub(crate) const NET_CONNECT_TCP_ADDR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.connectTcpAddr",
    symbol: "_mfb_rt_net_net_connectTcpAddr",
    abi: RuntimeHelperAbi {
        params: NET_CONNECT_TCP_ADDR_PARAMS,
        returns: "Socket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_LISTEN_TCP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.listenTcp",
    symbol: "_mfb_rt_net_net_listenTcp",
    abi: RuntimeHelperAbi {
        params: NET_LISTEN_TCP_PARAMS,
        returns: "Listener",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_ACCEPT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.accept",
    symbol: "_mfb_rt_net_net_accept",
    abi: RuntimeHelperAbi {
        params: NET_LISTENER_TIMEOUT_PARAMS,
        returns: "Socket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_POLL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.poll",
    symbol: "_mfb_rt_net_net_poll",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_TIMEOUT_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.read",
    symbol: "_mfb_rt_net_net_read",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_INT_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_READ_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.readText",
    symbol: "_mfb_rt_net_net_readText",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_INT_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_WRITE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.write",
    symbol: "_mfb_rt_net_net_write",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_BYTES_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_WRITE_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.writeText",
    symbol: "_mfb_rt_net_net_writeText",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_CLOSE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.close",
    symbol: "_mfb_rt_net_net_close",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_LOCAL_ADDRESS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.localAddress",
    symbol: "_mfb_rt_net_net_localAddress",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_PARAMS,
        returns: "Address",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_REMOTE_ADDRESS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.remoteAddress",
    symbol: "_mfb_rt_net_net_remoteAddress",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_PARAMS,
        returns: "Address",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SET_READ_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.setReadTimeout",
    symbol: "_mfb_rt_net_net_setReadTimeout",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_TIMEOUT_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SET_WRITE_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.setWriteTimeout",
    symbol: "_mfb_rt_net_net_setWriteTimeout",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_TIMEOUT_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// ---------------------------------------------------------------------------
// UDP datagram sockets
// ---------------------------------------------------------------------------

const NET_UDP_SOCKET_INT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "UdpSocket",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "maxBytes",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

const NET_SEND_TO_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "UdpSocket",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "address",
        type_: "Address",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "bytes",
        type_: "List OF Byte",
        location: abi::ARG[2],
    },
];

const NET_SEND_TEXT_TO_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "UdpSocket",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "address",
        type_: "Address",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: abi::ARG[2],
    },
];

pub(crate) const NET_BIND_UDP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.bindUdp",
    symbol: "_mfb_rt_net_net_bindUdp",
    abi: RuntimeHelperAbi {
        params: NET_HOST_PORT_PARAMS,
        returns: "UdpSocket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_RECEIVE_FROM_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.receiveFrom",
    symbol: "_mfb_rt_net_net_receiveFrom",
    abi: RuntimeHelperAbi {
        params: NET_UDP_SOCKET_INT_PARAMS,
        returns: "Datagram",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_RECEIVE_TEXT_FROM_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.receiveTextFrom",
    symbol: "_mfb_rt_net_net_receiveTextFrom",
    abi: RuntimeHelperAbi {
        params: NET_UDP_SOCKET_INT_PARAMS,
        returns: "DatagramText",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SEND_TO_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.sendTo",
    symbol: "_mfb_rt_net_net_sendTo",
    abi: RuntimeHelperAbi {
        params: NET_SEND_TO_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SEND_TEXT_TO_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.sendTextTo",
    symbol: "_mfb_rt_net_net_sendTextTo",
    abi: RuntimeHelperAbi {
        params: NET_SEND_TEXT_TO_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// ---------------------------------------------------------------------------
// TLS (transport-layer security; Linux/OpenSSL backend, plan-03-net.md §4)
// ---------------------------------------------------------------------------

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
