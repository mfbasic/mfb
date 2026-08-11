//! `process::spawn` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). process members are
//! `Implementation::Same`: they lower via the `_mfb_rt_process_*` runtime-call
//! seam (emission in `../native/`), so this file carries only the descriptor +
//! docs migrated from `src/docs/man/builtins/process/spawn.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = "";
const DESC: &str = "";

pub(crate) const SPAWN: BuiltinFunction =
    BuiltinFunction::same(super::SPAWN, "spawn", INTRO, DESC, &[], super::OV_SPAWN);
