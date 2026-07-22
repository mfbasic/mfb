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
    abi: RuntimeHelperAbi {
        params: NET_HOST_PORT_PARAMS,
        returns: "List OF Address",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_CONNECT_TCP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.connectTcp",
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
    abi: RuntimeHelperAbi {
        params: NET_CONNECT_TCP_ADDR_PARAMS,
        returns: "Socket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_LISTEN_TCP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.listenTcp",
    abi: RuntimeHelperAbi {
        params: NET_LISTEN_TCP_PARAMS,
        returns: "Listener",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_ACCEPT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.accept",
    abi: RuntimeHelperAbi {
        params: NET_LISTENER_TIMEOUT_PARAMS,
        returns: "Socket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_POLL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.poll",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_TIMEOUT_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.read",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_INT_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_READ_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.readText",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_INT_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_WRITE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.write",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_BYTES_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_WRITE_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.writeText",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_CLOSE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.close",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_LOCAL_ADDRESS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.localAddress",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_PARAMS,
        returns: "Address",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_REMOTE_ADDRESS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.remoteAddress",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_PARAMS,
        returns: "Address",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SET_READ_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.setReadTimeout",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_TIMEOUT_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SET_WRITE_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.setWriteTimeout",
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
    abi: RuntimeHelperAbi {
        params: NET_HOST_PORT_PARAMS,
        returns: "UdpSocket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_RECEIVE_FROM_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.receiveFrom",
    abi: RuntimeHelperAbi {
        params: NET_UDP_SOCKET_INT_PARAMS,
        returns: "Datagram",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_RECEIVE_TEXT_FROM_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.receiveTextFrom",
    abi: RuntimeHelperAbi {
        params: NET_UDP_SOCKET_INT_PARAMS,
        returns: "DatagramText",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SEND_TO_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.sendTo",
    abi: RuntimeHelperAbi {
        params: NET_SEND_TO_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SEND_TEXT_TO_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.sendTextTo",
    abi: RuntimeHelperAbi {
        params: NET_SEND_TEXT_TO_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};
