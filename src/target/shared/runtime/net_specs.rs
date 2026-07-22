use super::*;

pub(crate) const NET_LOOKUP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.lookup",
    abi: RuntimeHelperAbi {
        returns: "List OF Address",
    },
};

pub(crate) const NET_CONNECT_TCP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.connectTcp",
    abi: RuntimeHelperAbi { returns: "Socket" },
};

pub(crate) const NET_CONNECT_TCP_ADDR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.connectTcpAddr",
    abi: RuntimeHelperAbi { returns: "Socket" },
};

pub(crate) const NET_LISTEN_TCP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.listenTcp",
    abi: RuntimeHelperAbi {
        returns: "Listener",
    },
};

pub(crate) const NET_ACCEPT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.accept",
    abi: RuntimeHelperAbi { returns: "Socket" },
};

pub(crate) const NET_POLL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.poll",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const NET_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.read",
    abi: RuntimeHelperAbi {
        returns: "List OF Byte",
    },
};

pub(crate) const NET_READ_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.readText",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const NET_WRITE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.write",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const NET_WRITE_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.writeText",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const NET_CLOSE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.close",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const NET_LOCAL_ADDRESS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.localAddress",
    abi: RuntimeHelperAbi { returns: "Address" },
};

pub(crate) const NET_REMOTE_ADDRESS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.remoteAddress",
    abi: RuntimeHelperAbi { returns: "Address" },
};

pub(crate) const NET_SET_READ_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.setReadTimeout",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const NET_SET_WRITE_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.setWriteTimeout",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

// ---------------------------------------------------------------------------
// UDP datagram sockets
// ---------------------------------------------------------------------------

pub(crate) const NET_BIND_UDP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.bindUdp",
    abi: RuntimeHelperAbi {
        returns: "UdpSocket",
    },
};

pub(crate) const NET_RECEIVE_FROM_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.receiveFrom",
    abi: RuntimeHelperAbi {
        returns: "Datagram",
    },
};

pub(crate) const NET_RECEIVE_TEXT_FROM_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.receiveTextFrom",
    abi: RuntimeHelperAbi {
        returns: "DatagramText",
    },
};

pub(crate) const NET_SEND_TO_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.sendTo",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const NET_SEND_TEXT_TO_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.sendTextTo",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};
