use super::*;

// plan-67-B: the four internal runtime performance-tracking helpers. They are
// NOT part of any `perf::` MFB package — there is no language surface — and are
// invoked only by compiler-injected calls in a `--cfg perf`-built, macOS-entry program
// (see `plan::symbols::runtime_symbols`, which force-adds their symbols under
// that gate). They are catalogued only so `spec_for_call`/`spec_for_symbol`
// resolve them during code emission and object planning, exactly like the
// code-layer-synthesized calls (`thread.drop`, `net.connectTcpAddr`).
//
// - `perf.init` / `perf.done`: zero-arg lifecycle (map the region; print the
//   table + free). Injected at program entry / exit.
// - `perf.start` / `perf.end`: one `String` arg (a region name). Injected around
//   the instrumented code regions (bodies filled in plan-67-C/D).
//
// All four return nothing (`Nothing`): they are pure side-effecting helpers with
// no result value threaded back to a caller.
pub(crate) const PERF_INIT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Perf,
    call: "perf.init",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const PERF_START_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Perf,
    call: "perf.start",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const PERF_END_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Perf,
    call: "perf.end",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const PERF_DONE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Perf,
    call: "perf.done",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};
