//! `process::receive` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). process members are
//! `Implementation::Same`: they lower via the `_mfb_rt_process_*` runtime-call
//! seam (emission in `../native/`), so this file carries only the descriptor +
//! docs migrated from `src/docs/man/builtins/process/receive.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const RECEIVE: BuiltinFunction = BuiltinFunction::same(
    super::RECEIVE,
    "receive",
    INTRO,
    DESC,
    &[],
    super::OV_RECEIVE,
);
