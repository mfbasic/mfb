use super::*;

use crate::target::shared::abi;

const TERM_RGB_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "r",
        type_: "Byte",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "g",
        type_: "Byte",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "b",
        type_: "Byte",
        location: abi::ARG[2],
    },
];

const TERM_BOOL_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "enabled",
    type_: "Boolean",
    location: abi::ARG[0],
}];

const TERM_MOVE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "row",
        type_: "Integer",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "column",
        type_: "Integer",
        location: abi::ARG[1],
    },
];

pub(crate) const TERM_ON_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.on",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_OFF_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.off",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_IS_ON_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.isOn",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_SET_FOREGROUND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.setForeground",
    abi: RuntimeHelperAbi {
        params: TERM_RGB_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_SET_BACKGROUND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.setBackground",
    abi: RuntimeHelperAbi {
        params: TERM_RGB_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_SET_BOLD_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.setBold",
    abi: RuntimeHelperAbi {
        params: TERM_BOOL_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_SET_UNDERLINE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.setUnderline",
    abi: RuntimeHelperAbi {
        params: TERM_BOOL_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_SHOW_CURSOR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.showCursor",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_HIDE_CURSOR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.hideCursor",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_CLEAR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.clear",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_SYNC_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.sync",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_MOVE_TO_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.moveTo",
    abi: RuntimeHelperAbi {
        params: TERM_MOVE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_GET_FOREGROUND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.getForeground",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "TermColor",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_GET_BACKGROUND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.getBackground",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "TermColor",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_GET_BOLD_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.getBold",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_GET_UNDERLINE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.getUnderline",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TERM_TERMINAL_SIZE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Term,
    call: "term.terminalSize",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "TermSize",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};
