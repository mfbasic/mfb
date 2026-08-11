//! `process::sendBytes` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). process members are
//! `Implementation::Same`: they lower via the `_mfb_rt_process_*` runtime-call
//! seam (emission in `../native/`), so this file carries only the descriptor +
//! docs migrated from `src/docs/man/builtins/process/sendBytes.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const SEND_BYTES: BuiltinFunction = BuiltinFunction::same(
    super::SEND_BYTES,
    "sendBytes",
    INTRO,
    DESC,
    &[],
    super::OV_SEND_BYTES,
);
