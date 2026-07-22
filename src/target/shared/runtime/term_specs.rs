use super::*;

pub(crate) const TERM_ON_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.on",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TERM_OFF_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.off",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TERM_IS_ON_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.isOn",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const TERM_SET_FOREGROUND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.setForeground",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TERM_SET_BACKGROUND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.setBackground",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TERM_SET_BOLD_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.setBold",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TERM_SET_UNDERLINE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.setUnderline",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TERM_SHOW_CURSOR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.showCursor",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TERM_HIDE_CURSOR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.hideCursor",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TERM_CLEAR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.clear",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TERM_SYNC_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.sync",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TERM_MOVE_TO_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.moveTo",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const TERM_GET_FOREGROUND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.getForeground",
    abi: RuntimeHelperAbi {
        returns: "TermColor",
    },
};

pub(crate) const TERM_GET_BACKGROUND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.getBackground",
    abi: RuntimeHelperAbi {
        returns: "TermColor",
    },
};

pub(crate) const TERM_GET_BOLD_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.getBold",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const TERM_GET_UNDERLINE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.getUnderline",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const TERM_TERMINAL_SIZE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.terminalSize",
    abi: RuntimeHelperAbi {
        returns: "TermSize",
    },
};
