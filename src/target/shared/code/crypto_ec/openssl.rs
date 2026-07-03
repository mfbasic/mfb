//! Linux `EVP_PKEY` (libcrypto) backend for the `crypto::` NIST-EC helpers.
//! Wire-compatible with the macOS `SecKey` backend (see the parent module).
//!
//! NOTE: implementation in progress — the body is filled in the Linux phase.

use std::collections::HashMap;

use super::super::*;
use super::{Curve, EcOp};
use crate::arch::aarch64::abi;

/// Read-only C strings referenced by the Linux EC helpers (filled in the Linux
/// phase alongside the real EVP_PKEY body).
pub(crate) fn data_objects() -> Vec<CodeDataObject> {
    Vec::new()
}

pub(super) fn lower(
    _op: EcOp,
    _curve: Curve,
    symbol: &str,
    _imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    let done = format!("{symbol}_done");
    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    super::emit_fail(
        symbol,
        ERR_UNKNOWN_CODE,
        ERR_UNKNOWN_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, slots) = finalize_vreg_body_with_locals(&mut ins, &[], 16);
    Ok((frame, ins, rel, slots))
}
