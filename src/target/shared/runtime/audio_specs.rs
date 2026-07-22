use super::*;

// `audio::` raw-PCM helpers (plan-33-A §5). Fourteen symbols: the
// direction-specific `read`/`write`/`open*`/`close*` bodies plus the
// direction-agnostic `poll`/`available`/`xruns`, which share one symbol each and
// branch on `AudioHandle.kind` internally. The bodies land with the macOS
// (plan-33-B) and Linux (plan-33-C) backends; these rows exist so
// `spec_for_call`/`spec_for_symbol` resolve an `audio::` call. Argument shapes
// are owned by the front-end table in `src/builtins/audio.rs` (bug-329).

pub(crate) const AUDIO_DEVICES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.devices",
    abi: RuntimeHelperAbi {
        returns: "List OF AudioDevice",
    },
};

pub(crate) const AUDIO_OPEN_INPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.openInput",
    abi: RuntimeHelperAbi {
        returns: "AudioInput",
    },
};

pub(crate) const AUDIO_OPEN_INPUT_DEVICE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.openInputDevice",
    abi: RuntimeHelperAbi {
        returns: "AudioInput",
    },
};

pub(crate) const AUDIO_OPEN_OUTPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.openOutput",
    abi: RuntimeHelperAbi {
        returns: "AudioOutput",
    },
};

pub(crate) const AUDIO_OPEN_OUTPUT_DEVICE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.openOutputDevice",
    abi: RuntimeHelperAbi {
        returns: "AudioOutput",
    },
};

pub(crate) const AUDIO_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.read",
    abi: RuntimeHelperAbi {
        returns: "List OF Byte",
    },
};

pub(crate) const AUDIO_READ_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.readTimeout",
    abi: RuntimeHelperAbi {
        returns: "List OF Byte",
    },
};

pub(crate) const AUDIO_WRITE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.write",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const AUDIO_POLL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.poll",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const AUDIO_POLL_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.pollTimeout",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const AUDIO_AVAILABLE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.available",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

pub(crate) const AUDIO_XRUNS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.xruns",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

pub(crate) const AUDIO_CLOSE_INPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.closeInput",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const AUDIO_CLOSE_OUTPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Audio,
    call: "audio.closeOutput",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

// Routing/spec/symbol parity for every audio call is covered by the
// catalog-driven `catalog::tests::catalog_is_consistent` (bug-329), which
// replaced the hand-copied AUDIO_CALLS array that used to live here. The
// validate.rs is-implemented gate is covered by validate.rs's own tests.
