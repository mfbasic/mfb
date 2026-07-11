use super::*;

use crate::target::shared::abi;

const STRING_VALUE_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "value",
    type_: "String",
    location: abi::ARG[0],
}];

const STRING_VALUE_PATTERN_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "pattern",
        type_: "String",
        location: abi::ARG[1],
    },
];

const STRING_LIST_SEPARATOR_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "values",
        type_: "List OF String",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "separator",
        type_: "String",
        location: abi::ARG[1],
    },
];

pub(crate) const STRINGS_TRIM_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.trim",
    symbol: "_mfb_rt_strings_strings_trim",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_TRIM_START_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.trimStart",
    symbol: "_mfb_rt_strings_strings_trimStart",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_TRIM_END_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.trimEnd",
    symbol: "_mfb_rt_strings_strings_trimEnd",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_UPPER_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.upper",
    symbol: "_mfb_rt_strings_strings_upper",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_LOWER_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.lower",
    symbol: "_mfb_rt_strings_strings_lower",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_CASE_FOLD_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.caseFold",
    symbol: "_mfb_rt_strings_strings_caseFold",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_NORMALIZE_NFC_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.normalizeNfc",
    symbol: "_mfb_rt_strings_strings_normalizeNfc",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_GRAPHEMES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.graphemes",
    symbol: "_mfb_rt_strings_strings_graphemes",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "List OF String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_STARTS_WITH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.startsWith",
    symbol: "_mfb_rt_strings_strings_startsWith",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PATTERN_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_ENDS_WITH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.endsWith",
    symbol: "_mfb_rt_strings_strings_endsWith",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PATTERN_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_CONTAINS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.contains",
    symbol: "_mfb_rt_strings_strings_contains",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PATTERN_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_SPLIT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.split",
    symbol: "_mfb_rt_strings_strings_split",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PATTERN_PARAMS,
        returns: "List OF String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_JOIN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.join",
    symbol: "_mfb_rt_strings_strings_join",
    abi: RuntimeHelperAbi {
        params: STRING_LIST_SEPARATOR_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_BYTE_LEN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.byteLen",
    symbol: "_mfb_rt_strings_strings_byteLen",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "Integer",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};
