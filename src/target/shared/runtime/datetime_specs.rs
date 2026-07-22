use super::*;

use crate::target::shared::abi;

// `datetime::` OS-seam intrinsics (plan-01-datetime.md §8.2). `nowNanos` /
// `monotonicNanos` take no arguments; `localOffset` takes the epoch-seconds
// instant in `x0`. All return an `Integer` in the standard result-value
// register with the OK tag set. `nowNanos` / `monotonicNanos` cannot fail;
// `localOffset` raises `ErrInvalidArgument` (ERR tag) for an instant
// `localtime_r` cannot represent (bug-42).
const DATETIME_LOCAL_OFFSET_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "epochSeconds",
    type_: "Integer",
    location: abi::ARG[0],
}];

pub(crate) const DATETIME_NOW_NANOS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Datetime,
    call: "datetime.nowNanos",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Integer",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const DATETIME_MONOTONIC_NANOS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Datetime,
    call: "datetime.monotonicNanos",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Integer",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const DATETIME_LOCAL_OFFSET_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Datetime,
    call: "datetime.localOffset",
    abi: RuntimeHelperAbi {
        params: DATETIME_LOCAL_OFFSET_PARAMS,
        returns: "Integer",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};
