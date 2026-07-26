use super::*;

// plan-62-B: the two `app::` presentation-mode helpers. `getMode` loads the
// per-arena presentation-mode slot; `setMode` stores it and calls the per-backend
// surface-reconcile seam (a no-op in B, filled by plan-62-C/D). Both are
// app-mode-only — a console build reserves no presentation-mode slot, so neither
// helper is ever emitted there.

pub(crate) const APP_GET_MODE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::App,
    call: "app.getMode",
    abi: RuntimeHelperAbi { returns: "Mode" },
};

pub(crate) const APP_SET_MODE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::App,
    call: "app.setMode",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};
