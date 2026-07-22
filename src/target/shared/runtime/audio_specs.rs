use super::*;

use crate::target::shared::abi;

// `audio::` raw-PCM helpers (plan-33-A §5). Fourteen symbols: the
// direction-specific `read`/`write`/`open*`/`close*` bodies plus the
// direction-agnostic `poll`/`available`/`xruns`, which share one symbol each and
// branch on `AudioHandle.kind` internally. The bodies land with the macOS
// (plan-33-B) and Linux (plan-33-C) backends; these rows carry the full ABI
// metadata so `spec_for_symbol` resolves an `audio::` call. Before a backend
// landed, the pre-emit gate was `capabilities.runtime_calls` (which rejects a
// call whose helper the target cannot emit) — not a "does not emit runtime
// helper" error from these specs, which always resolve. (Moot now that the
// macOS and Linux audio backends have landed.)
//
// A shared-symbol call (`poll`/`available`/`xruns`) accepts either resource
// type; its param type is the representative pointer-sized `AudioInput`, exactly
// as `net.close` names `Socket` while serving `Listener`/`UdpSocket` too.

const AUDIO_OPEN_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sampleRate",
        type_: "Integer",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "channels",
        type_: "Integer",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "bufferFrames",
        type_: "Integer",
        location: abi::ARG[2],
    },
];

const AUDIO_OPEN_INPUT_DEVICE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "device",
        type_: "AudioDevice",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "sampleRate",
        type_: "Integer",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "channels",
        type_: "Integer",
        location: abi::ARG[2],
    },
    RuntimeAbiParam {
        name: "bufferFrames",
        type_: "Integer",
        location: abi::ARG[3],
    },
];

const AUDIO_READ_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "input",
        type_: "AudioInput",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "frames",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

const AUDIO_READ_TIMEOUT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "input",
        type_: "AudioInput",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "frames",
        type_: "Integer",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: abi::ARG[2],
    },
];

const AUDIO_WRITE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "output",
        type_: "AudioOutput",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "bytes",
        type_: "List OF Byte",
        location: abi::ARG[1],
    },
];

// Shared-symbol streams (`poll`/`available`/`xruns`): either direction, one
// pointer-sized handle in ARG[0].
const AUDIO_STREAM_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "stream",
    type_: "AudioInput",
    location: abi::ARG[0],
}];

const AUDIO_STREAM_TIMEOUT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "stream",
        type_: "AudioInput",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

const AUDIO_INPUT_STREAM_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "input",
    type_: "AudioInput",
    location: abi::ARG[0],
}];

const AUDIO_OUTPUT_STREAM_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "output",
    type_: "AudioOutput",
    location: abi::ARG[0],
}];

pub(crate) const AUDIO_DEVICES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.devices",
    symbol: "_mfb_rt_audio_audio_devices",
    abi: RuntimeHelperAbi {
        // Nullary, like `os.pid`; the family rides on the open/read/write specs
        // for the `validate.rs` completeness predicate (plan-33-A §5).
        params: &[],
        returns: "List OF AudioDevice",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_OPEN_INPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.openInput",
    symbol: "_mfb_rt_audio_audio_openInput",
    abi: RuntimeHelperAbi {
        params: AUDIO_OPEN_PARAMS,
        returns: "AudioInput",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_OPEN_INPUT_DEVICE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.openInputDevice",
    symbol: "_mfb_rt_audio_audio_openInputDevice",
    abi: RuntimeHelperAbi {
        params: AUDIO_OPEN_INPUT_DEVICE_PARAMS,
        returns: "AudioInput",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_OPEN_OUTPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.openOutput",
    symbol: "_mfb_rt_audio_audio_openOutput",
    abi: RuntimeHelperAbi {
        params: AUDIO_OPEN_PARAMS,
        returns: "AudioOutput",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_OPEN_OUTPUT_DEVICE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.openOutputDevice",
    symbol: "_mfb_rt_audio_audio_openOutputDevice",
    abi: RuntimeHelperAbi {
        params: AUDIO_OPEN_INPUT_DEVICE_PARAMS,
        returns: "AudioOutput",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.read",
    symbol: "_mfb_rt_audio_audio_read",
    abi: RuntimeHelperAbi {
        params: AUDIO_READ_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_READ_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.readTimeout",
    symbol: "_mfb_rt_audio_audio_readTimeout",
    abi: RuntimeHelperAbi {
        params: AUDIO_READ_TIMEOUT_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_WRITE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.write",
    symbol: "_mfb_rt_audio_audio_write",
    abi: RuntimeHelperAbi {
        params: AUDIO_WRITE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_POLL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.poll",
    symbol: "_mfb_rt_audio_audio_poll",
    abi: RuntimeHelperAbi {
        params: AUDIO_STREAM_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_POLL_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.pollTimeout",
    symbol: "_mfb_rt_audio_audio_pollTimeout",
    abi: RuntimeHelperAbi {
        params: AUDIO_STREAM_TIMEOUT_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_AVAILABLE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.available",
    symbol: "_mfb_rt_audio_audio_available",
    abi: RuntimeHelperAbi {
        params: AUDIO_STREAM_PARAMS,
        returns: "Integer",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_XRUNS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.xruns",
    symbol: "_mfb_rt_audio_audio_xruns",
    abi: RuntimeHelperAbi {
        params: AUDIO_STREAM_PARAMS,
        returns: "Integer",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_CLOSE_INPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.closeInput",
    symbol: "_mfb_rt_audio_audio_closeInput",
    abi: RuntimeHelperAbi {
        params: AUDIO_INPUT_STREAM_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const AUDIO_CLOSE_OUTPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.closeOutput",
    symbol: "_mfb_rt_audio_audio_closeOutput",
    abi: RuntimeHelperAbi {
        params: AUDIO_OUTPUT_STREAM_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

#[cfg(test)]
mod tests {
    use super::super::spec_for_call;

    // Routing/spec/symbol parity for every audio call is covered by the
    // catalog-driven `catalog::tests::catalog_is_consistent` (bug-329), which
    // replaced the hand-copied AUDIO_CALLS array that used to live here.

    #[test]
    fn audio_family_is_complete_for_validate() {
        // The `validate.rs:210` predicate treats the family as implemented as
        // soon as ONE spec has non-empty params/returns/clobbers.
        let spec = spec_for_call("audio.openOutput").unwrap();
        assert!(!spec.abi.params.is_empty());
        assert!(!spec.abi.returns.is_empty());
        assert!(!spec.abi.clobbers.is_empty());
    }
}
