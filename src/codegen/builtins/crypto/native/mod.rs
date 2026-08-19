//! Native code generation for the `crypto::randomBytes` CSPRNG helper.
//!
//! The elliptic-curve public-key operations that formerly lowered here now live in
//! the clean-room `crypto::generate`/`sign`/`verify` `AbiFunction`s
//! (`func_generate.rs`, `func_sign.rs`, `func_verify.rs`), which are self-contained
//! and do not route through this backend. This module is reduced to the one
//! remaining OS-seam crypto helper — the secure random-bytes generator, which draws
//! from the OS CSPRNG (`getentropy` / `BCryptGenRandom`), emitted per-platform in
//! `random.rs`.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::CodegenPlatform;
use crate::codegen::registry::OsLowerCtx;
use std::collections::HashMap;

mod random;

/// OsLower-shaped entry for `crypto::randomBytes` — the CSPRNG runtime helper.
/// The per-compilation [`OsLowerCtx`] carries no state this helper needs; it is
/// dispatched generically through `registry::os_helper` (`crypto/mod.rs`'s
/// `Body::native` slots point here) exactly like `os`/`fs`/`io`.
pub(crate) fn lower_crypto_random_bytes(
    _call: &str,
    symbol: &str,
    _ctx: &OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    random::lower_crypto_random_bytes_helper(symbol, platform_imports, platform)
}
